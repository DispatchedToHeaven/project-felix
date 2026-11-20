//! The village book on the client (key K, or `/felix book`): the NPCs near on the left (your own
//! companions first), the one picked on the right — how it stands with you in words, what it does
//! now, its work, home, food and family — and the next event on top. Find makes it glow and points
//! the way; Call has it come and talk. The server sends the pages (see `books`).
use crate::books::{ASK, OPEN};
use crate::creator::{button, head, inside};
use crate::serde_json::{Value, json};
use mod_api::{
    ClientHost, Flow, IPayloadContext, IPayloadHandler, KeyRegistration, PayloadBytes, PayloadId, Registrar,
    ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenRenderPostEvent, SoundPlay,
};
use std::sync::{Arc, LazyLock, Mutex};

const SCREEN: &str = "projectfelix";
const MOD: &str = "projectfelix";
const KEY: &str = "village_book";
const W: f32 = 380.0;
const H: f32 = 210.0;
const LIST: f32 = 140.0;
const ROW: f32 = 13.0;

#[derive(Clone)]
struct Page {
    id: u32,
    name: String,
    mine: bool,
    mark: String,
    whereabouts: String,
    brief: String,
    lines: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Do {
    Pick(usize),
    Find,
    Call,
    Refresh,
    Close,
}

struct Book {
    pages: Vec<Page>,
    next: String,
    picked: usize,
    scroll: usize,
    buttons: Vec<([f32; 4], Do)>,
    shown: bool,
    close: bool,
    outbox: Vec<Vec<u8>>,
}

static BOOK: LazyLock<Mutex<Option<Book>>> = LazyLock::new(Default::default);
/// Asks to send from the next client tick (key presses and the opening).
static ASKS: LazyLock<Mutex<Vec<Vec<u8>>>> = LazyLock::new(Default::default);

pub fn active() -> bool {
    BOOK.lock().is_ok_and(|b| b.is_some())
}

pub fn register(r: &mut dyn Registrar) {
    r.register_key_mapping(KeyRegistration::new(KEY, "key.keyboard.k").label(&format!("key.projectfelix.{KEY}")).category("key.categories.projectfelix"));
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_client(PayloadId(OPEN.into()), Some(Arc::new(OpenHandler)));
    reg.play_to_server(PayloadId(ASK.into()), Arc::new(crate::books::Ask));
}

struct OpenHandler;

impl IPayloadHandler for OpenHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(host) = context.client_host() else { return Ok(()) };
        let Ok(v) = crate::serde_json::from_slice::<Value>(&payload.0) else { return Ok(()) };
        let s = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let pages: Vec<Page> = v
            .get("npcs")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .map(|x| Page {
                        id: x.get("id").and_then(Value::as_u64).unwrap_or(0) as u32,
                        name: s(x, "name"),
                        mine: x.get("mine").and_then(Value::as_bool).unwrap_or(false),
                        mark: s(x, "mark"),
                        whereabouts: s(x, "where"),
                        brief: s(x, "brief"),
                        lines: x.get("lines").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let next = s(&v, "next");
        let Ok(mut lock) = BOOK.lock() else { return Ok(()) };
        // Opened again (Refresh): the same NPC stays picked.
        let before = lock.as_ref().and_then(|b| b.pages.get(b.picked).map(|p| p.id));
        let picked = before.and_then(|id| pages.iter().position(|p| p.id == id)).unwrap_or(0);
        let scroll = lock.as_ref().map_or(0, |b| b.scroll).min(pages.len().saturating_sub(1));
        if lock.is_none() {
            crate::menu::yield_screen();
        }
        let shown = host.mod_screen() == Some(SCREEN) || host.mod_screen_open(SCREEN);
        *lock = Some(Book { pages, next, picked, scroll, buttons: Vec::new(), shown, close: false, outbox: Vec::new() });
        Ok(())
    }
}

fn act(b: &mut Book, what: Do) {
    let id = b.pages.get(b.picked).map(|p| p.id);
    match what {
        Do::Pick(i) => b.picked = i,
        Do::Find => {
            if let Some(id) = id {
                b.outbox.push(crate::serde_json::to_vec(&json!({ "find": id })).unwrap_or_default());
                b.close = true;
            }
        }
        Do::Call => {
            if let Some(id) = id {
                b.outbox.push(crate::serde_json::to_vec(&json!({ "call": id })).unwrap_or_default());
                b.close = true;
            }
        }
        Do::Refresh => b.outbox.push(b"{}".to_vec()),
        Do::Close => b.close = true,
    }
}

fn rows_shown() -> usize {
    ((H - 40.0) / ROW) as usize
}

pub fn render(host: &dyn ClientHost, ev: &mut ScreenRenderPostEvent<'_>) {
    if host.mod_screen() != Some(SCREEN) {
        return;
    }
    let Ok(mut lock) = BOOK.lock() else { return };
    let Some(b) = lock.as_mut() else { return };
    let draw = &mut *ev.draw;
    let mouse = ev.screen.mouse;
    let [sw, sh] = draw.size();
    let (x0, y0) = (((sw - W) / 2.0).max(0.0), ((sh - H) / 2.0).max(0.0));
    draw.fill([x0 - 6.0, y0 - 6.0, W + 12.0, H + 12.0], 0xFA14_1210);
    draw.text("Village book", [x0, y0], 0xFFFF_D86A, false);
    if !b.next.is_empty() {
        let next = head(draw, &b.next, W - 80.0);
        draw.text(&next, [x0 + W - draw.text_width(&next), y0], 0xFFB0_C8FF, false);
    }
    let mut buttons = Vec::new();
    let top = y0 + 12.0;
    draw.fill([x0, top, LIST, H - 32.0], 0xFF1C_1A18);
    if b.pages.is_empty() {
        draw.text("No one near.", [x0 + 4.0, top + 4.0], 0xFFA0_A0A0, false);
    }
    let shown = rows_shown();
    b.scroll = b.scroll.min(b.pages.len().saturating_sub(shown));
    let mut y = top + 1.0;
    for (i, p) in b.pages.iter().enumerate().skip(b.scroll).take(shown) {
        let rect = [x0 + 1.0, y, LIST - 2.0, ROW];
        if i == b.picked {
            draw.fill(rect, 0xF050_4020);
        } else if inside(rect, mouse) {
            draw.fill(rect, 0x6048_4038);
        }
        let mark_w = 8.0;
        draw.text(&p.mark, [x0 + 3.0, y + 3.0], if p.mark == "\u{2665}" { 0xFFFF_6A8A } else { 0xFFC0_C0A0 }, false);
        let name = head(draw, &p.name, LIST - 52.0);
        draw.text(&name, [x0 + 3.0 + mark_w, y + 3.0], if p.mine { 0xFF9A_E89A } else { 0xFFFF_FFFF }, false);
        let w = p.brief.clone();
        draw.text(&w, [x0 + LIST - 4.0 - draw.text_width(&w), y + 3.0], 0xFF80_8088, false);
        buttons.push((rect, Do::Pick(i)));
        y += ROW;
    }
    if b.pages.len() > shown {
        draw.text(&format!("{}-{} of {}", b.scroll + 1, (b.scroll + shown).min(b.pages.len()), b.pages.len()), [x0 + 3.0, top + H - 42.0], 0xFF70_7080, false);
    }
    // The page.
    let px = x0 + LIST + 8.0;
    let pw = W - LIST - 8.0;
    if let Some(p) = b.pages.get(b.picked) {
        draw.text(&p.name, [px, top + 2.0], 0xFFFF_D86A, false);
        let w = format!("{}{}", p.whereabouts, if p.mine { ", your companion" } else { "" });
        draw.text(&w, [px, top + 13.0], 0xFF90_90A0, false);
        let mut y = top + 27.0;
        for line in &p.lines {
            for part in crate::menu::wrap(line, pw, &|t| draw.text_width(t)) {
                if y > top + H - 50.0 {
                    break;
                }
                draw.text(&part, [px, y], 0xFFE8_E4DC, false);
                y += 10.0;
            }
        }
        let by = top + H - 44.0;
        button(draw, &mut buttons, [px, by, 60.0, 14.0], "Find", Do::Find, mouse);
        button(draw, &mut buttons, [px + 64.0, by, 60.0, 14.0], "Call", Do::Call, mouse);
    }
    let y = y0 + H - 14.0;
    button(draw, &mut buttons, [x0, y, 60.0, 14.0], "Refresh", Do::Refresh, mouse);
    button(draw, &mut buttons, [x0 + W - 50.0, y, 50.0, 14.0], "Close", Do::Close, mouse);
    b.buttons = buttons;
}

pub fn mouse_pressed(host: &dyn ClientHost, ev: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = BOOK.lock() else { return Flow::Continue };
    let Some(b) = lock.as_mut() else { return Flow::Continue };
    if let Some(what) = b.buttons.iter().rev().find(|(r, _)| inside(*r, ev.screen.mouse)).map(|(_, w)| *w) {
        host.play_local_sound(&SoundPlay::ui(if matches!(what, Do::Pick(_)) { "minecraft:item.book.page_turn" } else { "minecraft:ui.button.click" }), 0);
        act(b, what);
    }
    Flow::Cancel
}

pub fn mouse_scrolled(host: &dyn ClientHost, up: bool) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = BOOK.lock() else { return Flow::Continue };
    let Some(b) = lock.as_mut() else { return Flow::Continue };
    if up {
        b.scroll = b.scroll.saturating_sub(1);
    } else if b.scroll + rows_shown() < b.pages.len() {
        b.scroll += 1;
    }
    Flow::Cancel
}

pub fn key_pressed(host: &dyn ClientHost, ev: &mut ScreenKeyPressedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = BOOK.lock() else { return Flow::Continue };
    let Some(b) = lock.as_mut() else { return Flow::Continue };
    let n = b.pages.len();
    match ev.key {
        "key.keyboard.escape" => return Flow::Continue,
        "key.keyboard.k" => b.close = true,
        "key.keyboard.down" if n > 0 => {
            b.picked = (b.picked + 1) % n;
            if b.picked >= b.scroll + rows_shown() || b.picked < b.scroll {
                b.scroll = b.picked.saturating_sub(rows_shown() - 1);
            }
        }
        "key.keyboard.up" if n > 0 => {
            b.picked = (b.picked + n - 1) % n;
            if b.picked < b.scroll || b.picked >= b.scroll + rows_shown() {
                b.scroll = b.picked.min(n.saturating_sub(rows_shown()));
            }
        }
        "key.keyboard.f" => act(b, Do::Find),
        "key.keyboard.c" => act(b, Do::Call),
        _ => {}
    }
    Flow::Cancel
}

pub fn tick(host: &mut dyn ClientHost) {
    while host.consume_click(MOD, KEY) {
        if host.mod_screen().is_none()
            && let Ok(mut a) = ASKS.lock()
        {
            a.push(b"{}".to_vec());
        }
    }
    let asks = ASKS.lock().map(|mut a| std::mem::take(&mut *a)).unwrap_or_default();
    let (out, close) = {
        let Ok(mut lock) = BOOK.lock() else { return };
        match lock.as_mut() {
            Some(b) => {
                let out = std::mem::take(&mut b.outbox);
                let close = b.close;
                if close || (b.shown && host.mod_screen() != Some(SCREEN)) {
                    *lock = None;
                } else if !b.shown && host.mod_screen() == Some(SCREEN) {
                    b.shown = true;
                }
                (out, close)
            }
            None => (Vec::new(), false),
        }
    };
    for bytes in asks.into_iter().chain(out) {
        let _ = host.send_payload_to_server(&PayloadId(ASK.into()), PayloadBytes(bytes));
    }
    if close {
        host.mod_screen_close(SCREEN);
    }
}
