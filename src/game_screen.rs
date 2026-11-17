//! The board screen on the client: the position as the server sent it, from the player's side,
//! clicks to move (a piece, then where it goes), a choice of piece when a pawn promotes, and
//! buttons to resign and to close. Watchers see the board and can't move.
use crate::games::{MOVE, RESIGN};
use mod_api::{
    ClientHost, DrawContext, Flow, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId,
    ScreenMouseButtonPressedPreEvent, ScreenRenderPostEvent, ScreenView, SoundPlay, Text,
};
use std::sync::{LazyLock, Mutex};

/// The mod's own screen, which the talk screen shares.
const SCREEN: &str = "projectfelix";
const SQ: f32 = 22.0;
const ICON: f32 = 16.0;
const PIECES: &str = "projectfelix:gui/pieces";

#[derive(Clone, Default)]
struct View {
    npc: u32,
    kind: String,
    board: Vec<char>,
    you: String,
    turn: String,
    moves: Vec<(u8, u8, bool)>,
    last: Option<(u8, u8)>,
    status: String,
    check: bool,
    over: bool,
    white: String,
    black: String,
}

#[derive(Default)]
struct Board {
    view: Option<View>,
    shown: bool,
    selected: Option<u8>,
    /// A promotion waiting for its piece: from, to.
    promoting: Option<(u8, u8)>,
    outbox: Vec<(&'static str, Vec<u8>)>,
    /// Buttons as the last frame drew them: rect and what they do.
    buttons: Vec<([f32; 4], Button)>,
    origin: [f32; 2],
    /// Close the screen at the next tick.
    close: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Resign,
    Close,
    Promote(u8),
}

static BOARD: LazyLock<Mutex<Board>> = LazyLock::new(Default::default);

/// Whether the board has the mod's screen now (the talk screen keeps away).
pub fn active() -> bool {
    BOARD.lock().is_ok_and(|b| b.view.is_some())
}

/// `game_view`: what the board shows; opens the screen.
pub struct ViewHandler;

impl IPayloadHandler for ViewHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(v) = crate::serde_json::from_slice::<crate::serde_json::Value>(&payload.0) else { return Ok(()) };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let moves = v
            .get("moves")
            .and_then(|m| m.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        let m = m.as_array()?;
                        Some((m.first()?.as_u64()? as u8, m.get(1)?.as_u64()? as u8, m.get(2)?.as_u64()? == 1))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let last = v.get("last").and_then(|l| l.as_array()).and_then(|l| Some((l.first()?.as_u64()? as u8, l.get(1)?.as_u64()? as u8)));
        let view = View {
            npc: v.get("npc").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            kind: s("kind"),
            board: s("board").chars().collect(),
            you: s("you"),
            turn: s("turn"),
            moves,
            last,
            status: s("status"),
            check: v.get("check").and_then(|x| x.as_bool()).unwrap_or(false),
            over: v.get("over").and_then(|x| x.as_bool()).unwrap_or(false),
            white: s("white"),
            black: s("black"),
        };
        crate::menu::yield_screen();
        let Ok(host) = context.client_host() else { return Ok(()) };
        let open = host.mod_screen() == Some(SCREEN) || host.mod_screen_open(SCREEN);
        if let Ok(mut b) = BOARD.lock() {
            if b.view.as_ref().is_some_and(|old| old.board != view.board) {
                host.play_local_sound(&SoundPlay::ui("minecraft:block.wood.place"), 0);
            }
            b.view = Some(view);
            b.shown = open;
            b.selected = None;
            b.promoting = None;
        }
        Ok(())
    }
}

/// The client tick: sends moves, forgets the board once its screen is closed.
pub fn tick(host: &mut dyn ClientHost) {
    let (out, close) = {
        let Ok(mut b) = BOARD.lock() else { return };
        if b.shown && host.mod_screen() != Some(SCREEN) {
            b.view = None;
            b.shown = false;
        }
        (std::mem::take(&mut b.outbox), std::mem::take(&mut b.close))
    };
    if close {
        host.mod_screen_close(SCREEN);
    }
    for (name, bytes) in out {
        let _ = host.send_payload_to_server(&PayloadId(name.into()), PayloadBytes(bytes));
    }
}

/// The square at row `row` (0 at the top) and column `col` as `you` see the board.
fn square_at(you: &str, col: u8, row: u8) -> u8 {
    if you == "black" { (row) * 8 + (7 - col) } else { (7 - row) * 8 + col }
}

fn icon(kind: &str, c: char) -> Option<[f32; 4]> {
    let (col, row) = if kind == "checkers" {
        match c {
            'w' => (0, 2),
            'W' => (1, 2),
            'b' => (2, 2),
            'B' => (3, 2),
            _ => return None,
        }
    } else {
        let col = "kqrbnp".find(c.to_ascii_lowercase())?;
        (col, if c.is_ascii_uppercase() { 0 } else { 1 })
    };
    Some([col as f32 * ICON, row as f32 * ICON, ICON, ICON])
}

fn layout(screen: &ScreenView<'_>) -> [f32; 2] {
    let [x, y, w, h] = screen.rect;
    let total = [8.0 * SQ + 120.0, 8.0 * SQ + 30.0];
    [x + ((w - total[0]) / 2.0).floor(), y + ((h - total[1]) / 2.0).floor() + 10.0]
}

fn inside(r: [f32; 4], at: [f32; 2]) -> bool {
    at[0] >= r[0] && at[0] < r[0] + r[2] && at[1] >= r[1] && at[1] < r[1] + r[3]
}

fn label(draw: &mut dyn DrawContext, text: &str, at: [f32; 2], color: u32) {
    draw.text(text, at, color, true);
}

/// `screen_render_post`: the board.
pub fn render(host: &dyn ClientHost, e: &mut ScreenRenderPostEvent<'_>) {
    if host.mod_screen() != Some(SCREEN) {
        return;
    }
    let Ok(mut b) = BOARD.lock() else { return };
    let Some(v) = b.view.clone() else { return };
    let draw = &mut *e.draw;
    let [w, h] = draw.size();
    draw.draw_menu_background([0.0, 0.0, w, h], true);
    let [left, top] = layout(e.screen);
    b.origin = [left, top];
    let mouse = e.screen.mouse;
    // Names: the far side on top, yours below.
    let (far, near) = if v.you == "black" { (&v.white, &v.black) } else { (&v.black, &v.white) };
    let (far_c, near_c) = if v.you == "black" { ("White", "Black") } else { ("Black", "White") };
    label(draw, &format!("{far_c}: {far}"), [left, top - 12.0], 0xFFE0_E0E0);
    label(draw, &format!("{near_c}: {near}"), [left, top + 8.0 * SQ + 3.0], 0xFFE0_E0E0);
    let targets: Vec<u8> = b.selected.map(|s| v.moves.iter().filter(|m| m.0 == s).map(|m| m.1).collect()).unwrap_or_default();
    for row in 0..8u8 {
        for col in 0..8u8 {
            let sq = square_at(&v.you, col, row);
            let r = [left + col as f32 * SQ, top + row as f32 * SQ, SQ, SQ];
            let dark = (sq % 8 + sq / 8) % 2 == 0;
            draw.fill(r, if dark { 0xFF9A_6A42 } else { 0xFFEA_D8B0 });
            if v.last.is_some_and(|(a, z)| a == sq || z == sq) {
                draw.fill(r, 0x66F0_E040);
            }
            if b.selected == Some(sq) {
                draw.fill(r, 0x8840_C040);
            }
            let c = v.board.get(sq as usize).copied().unwrap_or('.');
            // The king in check, in red.
            if v.check && c == if v.turn == "black" { 'k' } else { 'K' } {
                draw.fill(r, 0x88FF_3030);
            }
            if let Some(src) = icon(&v.kind, c) {
                let pad = (SQ - ICON) / 2.0;
                draw.sprite(PIECES, src, [r[0] + pad, r[1] + pad, ICON, ICON], None);
            }
            if targets.contains(&sq) {
                draw.fill([r[0] + SQ / 2.0 - 3.0, r[1] + SQ / 2.0 - 3.0, 6.0, 6.0], 0xAA20_A020);
            }
            if inside(r, mouse) && !v.over {
                draw.fill(r, 0x22FF_FFFF);
            }
        }
    }
    // The side panel: whose game, the status, the buttons.
    let px = left + 8.0 * SQ + 10.0;
    let title = if v.kind == "checkers" { "Draughts" } else { "Chess" };
    label(draw, title, [px, top], 0xFFFF_D86A);
    let mut y = top + 14.0;
    let measure = |s: &str| draw.text_width(s);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in v.status.split_whitespace() {
        let next = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if measure(&next) > 108.0 && !line.is_empty() {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = next;
        }
    }
    lines.push(line);
    for l in lines {
        label(draw, &l, [px, y], if v.check { 0xFFFF_7070 } else { 0xFFF0_F0F0 });
        y += 11.0;
    }
    let mut buttons = Vec::new();
    let mut by = top + 8.0 * SQ - 44.0;
    if b.promoting.is_some() {
        label(draw, "Promote to:", [px, by - 50.0], 0xFFF0_F0F0);
        for (i, k) in ['q', 'r', 'b', 'n'].into_iter().enumerate() {
            let r = [px + i as f32 * 26.0, by - 38.0, 24.0, 24.0];
            draw.fill(r, 0xFFE8_D3A8);
            let letter = if v.you == "black" { k } else { k.to_ascii_uppercase() };
            if let Some(src) = icon("chess", letter) {
                draw.sprite(PIECES, src, [r[0] + 4.0, r[1] + 4.0, ICON, ICON], None);
            }
            buttons.push((r, Button::Promote(k as u8)));
        }
    }
    if !v.over && v.you != "watch" {
        let r = [px, by, 108.0, 20.0];
        let _ = draw.widget_button(r, &Text::literal("Resign"), true, inside(r, mouse));
        buttons.push((r, Button::Resign));
    }
    by += 24.0;
    let r = [px, by, 108.0, 20.0];
    let _ = draw.widget_button(r, &Text::literal("Close"), true, inside(r, mouse));
    buttons.push((r, Button::Close));
    b.buttons = buttons;
}

/// `screen_mouse_button_pressed_pre`: squares and buttons.
pub fn mouse_pressed(host: &dyn ClientHost, e: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) || e.button != 0 {
        return Flow::Continue;
    }
    let Ok(mut b) = BOARD.lock() else { return Flow::Continue };
    let Some(v) = b.view.clone() else { return Flow::Continue };
    let at = e.screen.mouse;
    let click = || host.play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0);
    if let Some((_, button)) = b.buttons.iter().find(|(r, _)| inside(*r, at)).copied() {
        click();
        match button {
            Button::Close => {
                b.view = None;
                b.shown = false;
                b.close = true;
            }
            Button::Resign => b.outbox.push((RESIGN, v.npc.to_le_bytes().to_vec())),
            Button::Promote(letter) => {
                if let Some((from, to)) = b.promoting.take() {
                    let mut bytes = v.npc.to_le_bytes().to_vec();
                    bytes.extend([from, to, letter]);
                    b.outbox.push((MOVE, bytes));
                    b.selected = None;
                }
            }
        }
        return Flow::Cancel;
    }
    let [left, top] = b.origin;
    let (cx, cy) = ((at[0] - left) / SQ, (at[1] - top) / SQ);
    if !(0.0..8.0).contains(&cx) || !(0.0..8.0).contains(&cy) || v.over {
        return Flow::Cancel;
    }
    let sq = square_at(&v.you, cx as u8, cy as u8);
    if let Some(from) = b.selected
        && let Some(m) = v.moves.iter().find(|m| m.0 == from && m.1 == sq)
    {
        if m.2 {
            b.promoting = Some((from, sq));
        } else {
            let mut bytes = v.npc.to_le_bytes().to_vec();
            bytes.extend([from, sq, 0]);
            b.outbox.push((MOVE, bytes));
            b.selected = None;
        }
        click();
        return Flow::Cancel;
    }
    b.promoting = None;
    b.selected = v.moves.iter().any(|m| m.0 == sq).then_some(sq);
    Flow::Cancel
}
