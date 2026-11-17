//! The NPC's menu on the client: right-click an NPC, pick an order.
//!
//! The menu is the mod's own screen over the world. A right-click on an NPC opens it, whether the
//! game's own aim found the NPC (`entity_interact`) or our ray did: the client keeps a wolf's
//! small box for other players' NPCs, so the use key also tests a person-sized box of its own.
//! Orders go to the server from the next client tick (input hooks can't send).
use crate::companion::{self, Mode, NPC, Order, ORDER};
use crate::runtime;
use mod_api::{
    ClientHost, Flow, Hand, HostSide, InteractionKey, InteractionKeyMappingTriggeredEvent, InteractionResult,
    PayloadBytes, PayloadId, ScreenMouseButtonPressedPreEvent, ScreenRenderPostEvent, ScreenView, SoundPlay, Text,
};
use std::sync::{LazyLock, Mutex};

/// The mod's own screen, by its mod id.
const SCREEN: &str = "projectfelix";
const WIDTH: f32 = 180.0;
const BUTTON: f32 = 20.0;
const GAP: f32 = 4.0;
/// How far the use key reaches for an NPC.
const REACH: f64 = 4.5;

const BUTTONS: [(&str, Option<Order>); 5] = [
    ("Follow me", Some(Order::Follow)),
    ("Wait here", Some(Order::Wait)),
    ("Guard this place", Some(Order::Guard)),
    ("Wave", Some(Order::Wave)),
    ("Close", None),
];

#[derive(Default)]
struct Menu {
    /// The NPC the menu is for, while it is open.
    entity: Option<u32>,
    shown: bool,
    outbox: Vec<(u32, Order)>,
    close: bool,
}

static MENU: LazyLock<Mutex<Menu>> = LazyLock::new(Default::default);

fn open(host: &mut dyn ClientHost, entity: u32) {
    let Ok(mut m) = MENU.lock() else { return };
    m.entity = Some(entity);
    m.shown = host.mod_screen_open(SCREEN);
}

/// `entity_interact`: a right-click on an NPC opens the menu here and does nothing else on the
/// server (no wolf feeding, sitting or collar dyeing).
pub fn interact(host: HostSide, entity: i32, kind: &str, hand: Hand) -> Option<InteractionResult> {
    if kind != NPC {
        return None;
    }
    if let HostSide::Client(h) = host
        && hand == Hand::Main
    {
        open(h, entity as u32);
    }
    Some(InteractionResult::Success)
}

/// `interaction_key_mapping_triggered`: the use key aimed at an NPC's upper body, which the
/// game's own aim misses on a client (it keeps the wolf's box).
pub fn use_key(host: &mut dyn ClientHost, e: &mut InteractionKeyMappingTriggeredEvent) -> Flow {
    if e.key != InteractionKey::Use || e.hand != Hand::Main || host.mod_screen().is_some() {
        return Flow::Continue;
    }
    let Some(me) = host.local_player().map(|p| p.entity()) else { return Flow::Continue };
    let (Some(eye), Some(look)) = (me.eye_position(&*host), me.view_vector(&*host)) else { return Flow::Continue };
    let mut best: Option<(f64, u32)> = None;
    for id in runtime::visible_npcs() {
        let Some(at) = host.entity(id).and_then(|e| e.position(&*host)) else { continue };
        let min = [at[0] - 0.3, at[1], at[2] - 0.3];
        let max = [at[0] + 0.3, at[1] + 1.95, at[2] + 0.3];
        if let Some(t) = ray_box(eye, look, min, max).filter(|t| *t <= REACH) {
            if best.is_none_or(|(b, _)| t < b) {
                best = Some((t, id));
            }
        }
    }
    let Some((_, id)) = best else { return Flow::Continue };
    open(host, id);
    e.swing_hand = false;
    Flow::Cancel
}

/// Where the ray from `from` along `dir` first enters the box, as a distance along it.
fn ray_box(from: [f64; 3], dir: [f64; 3], min: [f64; 3], max: [f64; 3]) -> Option<f64> {
    let (mut near, mut far) = (0.0f64, f64::MAX);
    for i in 0..3 {
        if dir[i].abs() < 1e-9 {
            if from[i] < min[i] || from[i] > max[i] {
                return None;
            }
            continue;
        }
        let (a, b) = ((min[i] - from[i]) / dir[i], (max[i] - from[i]) / dir[i]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far).then_some(near)
}

/// The client tick: sends queued orders, closes the menu after one, forgets it once closed.
pub fn tick(host: &mut dyn ClientHost) {
    let (out, close) = {
        let Ok(mut m) = MENU.lock() else { return };
        if m.shown && host.mod_screen() != Some(SCREEN) {
            m.entity = None;
            m.shown = false;
        }
        (std::mem::take(&mut m.outbox), std::mem::take(&mut m.close))
    };
    for (id, order) in out {
        let _ = host.send_payload_to_server(&PayloadId(ORDER.into()), PayloadBytes(companion::order_bytes(id, order)));
    }
    if close {
        host.mod_screen_close(SCREEN);
    }
}

fn layout(screen: &ScreenView<'_>) -> (f32, f32) {
    let [x, y, w, h] = screen.rect;
    let height = 30.0 + BUTTONS.len() as f32 * (BUTTON + GAP);
    (x + ((w - WIDTH) / 2.0).floor(), y + ((h - height) / 2.0).floor())
}

fn button_rect(left: f32, top: f32, i: usize) -> [f32; 4] {
    [left, top + 30.0 + i as f32 * (BUTTON + GAP), WIDTH, BUTTON]
}

fn inside(r: [f32; 4], at: [f32; 2]) -> bool {
    at[0] >= r[0] && at[0] < r[0] + r[2] && at[1] >= r[1] && at[1] < r[1] + r[3]
}

/// `screen_render_post`: the menu, while it is open.
pub fn render(host: &dyn ClientHost, e: &mut ScreenRenderPostEvent<'_>) {
    if host.mod_screen() != Some(SCREEN) {
        return;
    }
    let Some(entity) = MENU.lock().ok().and_then(|m| m.entity) else { return };
    let draw = &mut *e.draw;
    let [w, h] = draw.size();
    draw.draw_menu_background([0.0, 0.0, w, h], true);
    let (left, top) = layout(e.screen);
    let title = "Companion";
    let mode = match runtime::orders(entity).map(|o| o.mode) {
        Some(Mode::Wait) => "Waiting",
        Some(Mode::Guard) => "Guarding",
        _ => "Following",
    };
    draw.text(title, [left + (WIDTH - draw.text_width(title)) / 2.0, top], 0xFFFF_FFFF, true);
    draw.text(mode, [left + (WIDTH - draw.text_width(mode)) / 2.0, top + 12.0], 0xFFA0_A0A0, true);
    let mouse = e.screen.mouse;
    for (i, (label, _)) in BUTTONS.iter().enumerate() {
        let r = button_rect(left, top, i);
        let _ = draw.widget_button(r, &Text::literal(*label), true, inside(r, mouse));
    }
}

/// `screen_mouse_button_pressed_pre`: a button pressed.
pub fn mouse_pressed(host: &dyn ClientHost, e: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) || e.button != 0 {
        return Flow::Continue;
    }
    let Ok(mut m) = MENU.lock() else { return Flow::Continue };
    let Some(entity) = m.entity else { return Flow::Continue };
    let (left, top) = layout(e.screen);
    let Some(i) = (0..BUTTONS.len()).find(|i| inside(button_rect(left, top, *i), e.screen.mouse)) else {
        return Flow::Cancel;
    };
    host.play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0);
    if let Some(order) = BUTTONS[i].1 {
        m.outbox.push((entity, order));
    }
    m.close = true;
    Flow::Cancel
}
