//! Talking to an NPC on the client: right-click it, read what it says, pick an answer.
//!
//! The screen is the mod's own, over the world. A right-click on an NPC asks the server to start
//! talking (`talk`), whether the game's own aim found the NPC (`entity_interact`) or our ray did:
//! the client keeps a wolf's small box for other players' NPCs, so the use key also tests a
//! person-sized box of its own. The server answers with what to show (`view`) or ends it
//! (`close`); a pick goes back as `choose`, Escape as `bye`. Payloads go from the next client tick
//! (input hooks can't send).
use crate::companion::NPC;
use crate::runtime;
use crate::talk::{BYE, CHOOSE, TALK};
use mod_api::{
    ClientHost, Flow, Hand, HostSide, IPayloadContext, IPayloadHandler, InteractionKey,
    InteractionKeyMappingTriggeredEvent, InteractionResult, PayloadBytes, PayloadId, ScreenMouseButtonPressedPreEvent,
    ScreenRenderPostEvent, ScreenView, SoundPlay, Text,
};
use std::sync::{LazyLock, Mutex};

/// The mod's own screen, by its mod id.
const SCREEN: &str = "projectfelix";
const WIDTH: f32 = 300.0;
const BUTTON: f32 = 20.0;
const GAP: f32 = 4.0;
const LINE: f32 = 11.0;
/// How far the use key reaches for an NPC.
const REACH: f64 = 4.5;

/// What the server last said to show.
#[derive(Default, Clone)]
struct View {
    npc: u32,
    title: String,
    lines: Vec<String>,
    options: Vec<String>,
}

#[derive(Default)]
struct Talk {
    view: Option<View>,
    shown: bool,
    outbox: Vec<(&'static str, Vec<u8>)>,
    /// Close the screen at the next tick (the server ended the talk).
    close: bool,
    /// Where the last frame drew the answers, for clicks.
    buttons: Vec<[f32; 4]>,
}

static TALKING: LazyLock<Mutex<Talk>> = LazyLock::new(Default::default);

fn send(name: &'static str, bytes: Vec<u8>) {
    if let Ok(mut t) = TALKING.lock() {
        t.outbox.push((name, bytes));
    }
}

/// The board took the screen: forget the talk without closing it.
pub fn yield_screen() {
    if let Ok(mut t) = TALKING.lock() {
        t.view = None;
        t.shown = false;
        t.close = false;
    }
}

/// `entity_interact`: a right-click on an NPC starts talking, and does nothing else on the
/// server (no wolf feeding, sitting or collar dyeing).
pub fn interact(host: HostSide, entity: i32, kind: &str, hand: Hand) -> Option<InteractionResult> {
    if kind != NPC {
        return None;
    }
    if let HostSide::Client(_) = host
        && hand == Hand::Main
    {
        send(TALK, (entity as u32).to_le_bytes().to_vec());
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
        if let Some(t) = ray_box(eye, look, min, max).filter(|t| *t <= REACH)
            && best.is_none_or(|(b, _)| t < b)
        {
            best = Some((t, id));
        }
    }
    let Some((_, id)) = best else { return Flow::Continue };
    send(TALK, id.to_le_bytes().to_vec());
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

/// The client tick: sends what the hooks queued, closes the screen when the server said so, and
/// tells the server when the player closed it.
pub fn tick(host: &mut dyn ClientHost) {
    let (out, close, gone) = {
        let Ok(mut t) = TALKING.lock() else { return };
        let gone = t.shown && t.view.is_some() && host.mod_screen() != Some(SCREEN);
        if gone {
            t.view = None;
            t.shown = false;
        }
        (std::mem::take(&mut t.outbox), std::mem::take(&mut t.close), gone)
    };
    for (name, bytes) in out {
        let _ = host.send_payload_to_server(&PayloadId(name.into()), PayloadBytes(bytes));
    }
    if gone {
        let _ = host.send_payload_to_server(&PayloadId(BYE.into()), PayloadBytes(Vec::new()));
    }
    if close && !crate::game_screen::active() && !crate::editor::active() {
        host.mod_screen_close(SCREEN);
    }
}

/// `view`: what the NPC says now, and the answers; opens the screen if it isn't open.
pub struct ViewHandler;

impl IPayloadHandler for ViewHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(v) = crate::serde_json::from_slice::<crate::serde_json::Value>(&payload.0) else { return Ok(()) };
        let strings = |k: &str| -> Vec<String> {
            v.get(k).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
        };
        let view = View {
            npc: v.get("npc").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            title: v.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            lines: strings("lines"),
            options: strings("options"),
        };
        let Ok(host) = context.client_host() else { return Ok(()) };
        let open = host.mod_screen() == Some(SCREEN) || host.mod_screen_open(SCREEN);
        if let Ok(mut t) = TALKING.lock() {
            t.view = Some(view);
            t.shown = open;
        }
        Ok(())
    }
}

/// `close`: the server ended the talk.
pub struct CloseHandler;

impl IPayloadHandler for CloseHandler {
    fn handle(&self, _: PayloadBytes, _context: &mut dyn IPayloadContext) -> Result<(), String> {
        if let Ok(mut t) = TALKING.lock() {
            t.view = None;
            t.shown = false;
            t.close = true;
        }
        Ok(())
    }
}

/// `text` broken into lines no wider than `width`, by words.
fn wrap(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let next = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if measure(&next) > width && !line.is_empty() {
            out.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// The panel's top and the first button's top, for the lines it shows.
fn layout(screen: &ScreenView<'_>, lines: usize, options: usize) -> (f32, f32, f32) {
    let [x, y, w, h] = screen.rect;
    let text = 14.0 + lines as f32 * LINE + 8.0;
    let height = text + options as f32 * (BUTTON + GAP);
    let left = x + ((w - WIDTH) / 2.0).floor();
    let top = y + (h - height - 24.0).max(8.0).min((h - height) / 2.0 + 40.0).floor();
    (left, top, top + text)
}

fn button_rect(left: f32, first: f32, i: usize) -> [f32; 4] {
    [left + 20.0, first + i as f32 * (BUTTON + GAP), WIDTH - 40.0, BUTTON]
}

fn inside(r: [f32; 4], at: [f32; 2]) -> bool {
    at[0] >= r[0] && at[0] < r[0] + r[2] && at[1] >= r[1] && at[1] < r[1] + r[3]
}

/// `screen_render_post`: the talk, while the screen is open.
pub fn render(host: &dyn ClientHost, e: &mut ScreenRenderPostEvent<'_>) {
    if host.mod_screen() != Some(SCREEN) || crate::game_screen::active() || crate::editor::active() {
        return;
    }
    let Some(view) = TALKING.lock().ok().and_then(|t| t.view.clone()) else { return };
    let draw = &mut *e.draw;
    let lines: Vec<String> = {
        let measure = |s: &str| draw.text_width(s);
        view.lines.iter().flat_map(|l| wrap(l, WIDTH - 20.0, &measure)).collect()
    };
    let (left, top, first) = layout(e.screen, lines.len(), view.options.len());
    let bottom = first + view.options.len() as f32 * (BUTTON + GAP) + 4.0;
    draw.fill([left - 6.0, top - 6.0, WIDTH + 12.0, bottom - top + 6.0], 0xC010_1018);
    draw.fill([left - 6.0, top - 6.0, WIDTH + 12.0, 1.0], 0xFF60_6070);
    draw.text(&view.title, [left + 4.0, top], 0xFFFF_D86A, true);
    for (i, l) in lines.iter().enumerate() {
        draw.text(l, [left + 10.0, top + 14.0 + i as f32 * LINE], 0xFFF0_F0F0, true);
    }
    let mouse = e.screen.mouse;
    let mut rects = Vec::new();
    for (i, label) in view.options.iter().enumerate() {
        let r = button_rect(left, first, i);
        let _ = draw.widget_button(r, &Text::literal(format!("{}. {label}", i + 1)), true, inside(r, mouse));
        rects.push(r);
    }
    if let Ok(mut t) = TALKING.lock() {
        t.buttons = rects;
    }
}

/// `screen_mouse_button_pressed_pre`: an answer picked.
pub fn mouse_pressed(host: &dyn ClientHost, e: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) || e.button != 0 || crate::game_screen::active() || crate::editor::active() {
        return Flow::Continue;
    }
    let Some((view, buttons)) = TALKING.lock().ok().and_then(|t| Some((t.view.clone()?, t.buttons.clone()))) else {
        return Flow::Continue;
    };
    let pick = buttons.iter().position(|r| inside(*r, e.screen.mouse)).filter(|i| *i < view.options.len());
    if let Some(i) = pick {
        host.play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0);
        let mut b = view.npc.to_le_bytes().to_vec();
        b.push(i as u8);
        send(CHOOSE, b);
    }
    Flow::Cancel
}
