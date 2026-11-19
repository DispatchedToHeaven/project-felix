//! The scene editor, in the world (`/felix scene edit <name>`): the stage seen from above, the
//! lead in the middle facing up, the other roles dragged where they stand (the wheel turns them);
//! beside it the scene's length, the roles, and the timeline: at what second who says what and
//! plays which animation. Save keeps it on the server as `felixscene:<name>`; Save & play has the
//! NPC nearest you play it at once, with the NPCs nearest it in the other roles.
use crate::creator::{button, field, head, hint_field, inside, picker};
use crate::scene_edit::{CueEd, SceneEd};
use crate::serde_json::{Value, json};
use mod_api::{
    ClientHost, DrawContext, Flow, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId, ScreenCharTypedPreEvent,
    ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenMouseDraggedPreEvent, ScreenRenderPostEvent,
    SoundPlay,
};
use std::sync::{Arc, LazyLock, Mutex};

const SCREEN: &str = "projectfelix";
const W: f32 = 404.0;
const H: f32 = 226.0;
const ROW: f32 = 14.0;
/// The stage: its size, where the lead stands on it, pixels a block.
const STAGE: [f32; 2] = [150.0, 168.0];
const LEAD: [f32; 2] = [75.0, 118.0];
const BLOCK: f32 = 10.0;
const MARK: f32 = 10.0;
const ROWS: usize = 8;
const MAX_CUES: usize = 40;
const MAX_CAST: usize = 6;

#[derive(Clone, Copy, PartialEq, Debug)]
enum F {
    Length,
    Role,
    At(usize),
    Say(usize),
    SaveAs,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Do {
    Field(F),
    LeadAnim,
    Who(usize),
    Anim(usize),
    DelCue(usize),
    AddCue,
    AddRole,
    DelRole,
    Save(bool),
    Close,
}

struct Editor {
    id: String,
    ed: SceneEd,
    /// The role picked on the stage.
    selected: Option<usize>,
    focus: Option<F>,
    save_as: String,
    scroll: usize,
    drag: bool,
    stage: [f32; 2],
    list: [f32; 4],
    anims: Vec<String>,
    buttons: Vec<([f32; 4], Do)>,
    status: String,
    shown: bool,
    close: bool,
    outbox: Vec<Vec<u8>>,
}

static EDITOR: LazyLock<Mutex<Option<Editor>>> = LazyLock::new(Default::default);

pub fn active() -> bool {
    EDITOR.lock().is_ok_and(|e| e.is_some())
}

pub struct OpenHandler;

impl IPayloadHandler for OpenHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(host) = context.client_host() else { return Ok(()) };
        let Ok(v) = crate::serde_json::from_slice::<Value>(&payload.0) else { return Ok(()) };
        let ed = v.get("file").filter(|f| !f.is_null()).map(SceneEd::from_json).unwrap_or_else(SceneEd::blank);
        let mut anims: Vec<String> = crate::animations::names().into_iter().filter(|a| a.starts_with("animation.felix.")).collect();
        anims.sort();
        anims.insert(0, String::new());
        crate::menu::yield_screen();
        let shown = host.mod_screen() == Some(SCREEN) || host.mod_screen_open(SCREEN);
        let e = Editor {
            id: v.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
            ed,
            selected: None,
            focus: None,
            save_as: v.get("name").and_then(Value::as_str).unwrap_or("mine").to_string(),
            scroll: 0,
            drag: false,
            stage: [0.0; 2],
            list: [0.0; 4],
            anims,
            buttons: Vec::new(),
            status: "Drag the roles; the wheel turns one. Save & play: the NPC nearest you leads.".into(),
            shown,
            close: false,
            outbox: Vec::new(),
        };
        if let Ok(mut lock) = EDITOR.lock() {
            *lock = Some(e);
        }
        Ok(())
    }
}

impl Editor {
    fn text(&mut self, f: F) -> Option<&mut String> {
        Some(match f {
            F::Length => &mut self.ed.length,
            F::Role => &mut self.ed.cast.get_mut(self.selected?)?.role,
            F::At(i) => &mut self.ed.cues.get_mut(i)?.at,
            F::Say(i) => &mut self.ed.cues.get_mut(i)?.say,
            F::SaveAs => &mut self.save_as,
        })
    }

    fn order(&self) -> Vec<F> {
        let mut o = vec![F::Length];
        if self.selected.is_some() {
            o.push(F::Role);
        }
        for i in self.scroll..(self.scroll + ROWS).min(self.ed.cues.len()) {
            o.extend([F::At(i), F::Say(i)]);
        }
        o.push(F::SaveAs);
        o
    }

    fn save(&mut self, play: bool) {
        match self.ed.to_json() {
            Ok(file) => {
                let name = crate::profile::make_id(self.save_as.trim());
                let body = json!({ "name": name, "file": file, "play": play });
                self.outbox.push(crate::serde_json::to_vec(&body).unwrap_or_default());
                self.status = format!("Sent: felixscene:{name}{}", if play { ", to play" } else { "" });
                if play {
                    self.close = true;
                }
            }
            Err(e) => self.status = e,
        }
    }

    fn anim_index(&self, name: &str) -> usize {
        self.anims.iter().position(|a| a == name).unwrap_or(0)
    }
}

fn cycle(i: usize, by: i32, n: usize) -> usize {
    if n == 0 { 0 } else { (i as i64 + by as i64).rem_euclid(n as i64) as usize }
}

fn short_anim(a: &str) -> String {
    if a.is_empty() { "(none)".into() } else { a.strip_prefix("animation.felix.").unwrap_or(a).to_string() }
}

fn act(e: &mut Editor, what: Do, by: i32) {
    match what {
        Do::Field(f) => e.focus = Some(f),
        Do::LeadAnim => {
            let i = cycle(e.anim_index(&e.ed.lead_animation), by, e.anims.len());
            e.ed.lead_animation = e.anims[i].clone();
        }
        Do::Who(i) => {
            let n = e.ed.cast.len() + 1;
            if let Some(c) = e.ed.cues.get_mut(i) {
                c.who = cycle(c.who, by, n);
            }
        }
        Do::Anim(i) => {
            let cur = e.ed.cues.get(i).map(|c| e.anim_index(&c.animation)).unwrap_or(0);
            let next = e.anims.get(cycle(cur, by, e.anims.len())).cloned().unwrap_or_default();
            if let Some(c) = e.ed.cues.get_mut(i) {
                c.animation = next;
            }
        }
        Do::DelCue(i) => {
            if i < e.ed.cues.len() {
                e.ed.cues.remove(i);
                e.focus = None;
            }
        }
        Do::AddCue => {
            if e.ed.cues.len() < MAX_CUES {
                let last = e.ed.cues.last().and_then(|c| c.at.trim().parse::<f32>().ok()).unwrap_or(0.0);
                e.ed.cues.push(CueEd { at: format!("{}", (last + 2.0).round()), who: e.selected.map_or(0, |s| s + 1), ..CueEd::default() });
                let i = e.ed.cues.len() - 1;
                e.scroll = i.saturating_sub(ROWS - 1);
                e.focus = Some(F::Say(i));
            }
        }
        Do::AddRole => {
            if e.ed.cast.len() < MAX_CAST {
                e.selected = Some(e.ed.add_role());
                e.focus = Some(F::Role);
            }
        }
        Do::DelRole => {
            if let Some(s) = e.selected {
                e.ed.remove_role(s);
                e.selected = None;
                e.focus = None;
            }
        }
        Do::Save(play) => e.save(play),
        Do::Close => e.close = true,
    }
}

// ---- drawing --------------------------------------------------------------------------------------------

fn mark_at(c: [f32; 2], s0: [f32; 2]) -> [f32; 2] {
    [s0[0] + LEAD[0] + c[0] * BLOCK, s0[1] + LEAD[1] - c[1] * BLOCK]
}

fn draw_mark(draw: &mut dyn DrawContext, centre: [f32; 2], turn: f32, letter: &str, fill: u32) {
    draw.fill([centre[0] - MARK / 2.0, centre[1] - MARK / 2.0, MARK, MARK], fill);
    // The way it faces: up is the lead's forward.
    let a = turn.to_radians();
    let (dx, dy) = (-a.sin() * 7.0, -a.cos() * 7.0);
    draw.fill([centre[0] + dx - 1.5, centre[1] + dy - 1.5, 3.0, 3.0], 0xFFFF_FFFF);
    let w = draw.text_width(letter);
    draw.text(letter, [centre[0] - w / 2.0, centre[1] - 4.0], 0xFF10_1018, false);
}

fn draw_stage(draw: &mut dyn DrawContext, e: &Editor, s0: [f32; 2]) {
    draw.fill([s0[0], s0[1], STAGE[0], STAGE[1]], 0xFF20_2A20);
    // A block grid.
    let mut x = (LEAD[0] % BLOCK) + s0[0];
    while x < s0[0] + STAGE[0] {
        draw.fill([x, s0[1], 1.0, STAGE[1]], 0x2080_A080);
        x += BLOCK;
    }
    let mut y = (LEAD[1] % BLOCK) + s0[1];
    while y < s0[1] + STAGE[1] {
        draw.fill([s0[0], y, STAGE[0], 1.0], 0x2080_A080);
        y += BLOCK;
    }
    draw_mark(draw, mark_at([0.0, 0.0], s0), 0.0, "L", 0xFFE8_C640);
    for (i, c) in e.ed.cast.iter().enumerate() {
        let fill = if e.selected == Some(i) { 0xFFFF_D86A } else { 0xFF6A_B0FF };
        let letter: String = c.role.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_else(|| "?".into());
        draw_mark(draw, mark_at(c.at, s0), c.turn, &letter, fill);
    }
    draw.text("forward", [s0[0] + LEAD[0] - 16.0, s0[1] + 2.0], 0xFF80_9080, false);
}

pub fn render(host: &dyn ClientHost, ev: &mut ScreenRenderPostEvent<'_>) {
    if host.mod_screen() != Some(SCREEN) {
        return;
    }
    let Ok(mut lock) = EDITOR.lock() else { return };
    let Some(e) = lock.as_mut() else { return };
    let draw = &mut *ev.draw;
    let mouse = ev.screen.mouse;
    let [sw, sh] = draw.size();
    let (x0, y0) = (((sw - W) / 2.0).max(0.0), ((sh - H) / 2.0).max(0.0));
    let blink = (crate::runtime::clock() as i64 / 6) % 2 == 0;
    draw.fill([x0 - 6.0, y0 - 6.0, W + 12.0, H + 12.0], 0xFA10_1018);
    let title = if e.id.is_empty() { "New scene".to_string() } else { format!("Scene {}", e.id) };
    draw.text(&title, [x0, y0], 0xFFFF_D86A, false);
    let s0 = [x0, y0 + 12.0];
    e.stage = s0;
    draw_stage(draw, e, s0);
    let mut b = Vec::new();
    let grey = 0xFFA0_A0A0;
    let h = ROW - 2.0;
    let x = x0 + 156.0;
    let mut y = y0 + 12.0;
    draw.text("length", [x, y + 3.0], grey, false);
    hint_field(draw, &mut b, [x + 34.0, y, 26.0, h], &e.ed.length.clone(), "s", e.focus == Some(F::Length), Do::Field(F::Length), mouse, blink);
    draw.text("lead plays", [x + 66.0, y + 3.0], grey, false);
    picker(draw, &mut b, [x + 120.0, y, W - 156.0 - 120.0, h], &short_anim(&e.ed.lead_animation), Do::LeadAnim, mouse);
    y += ROW;
    match e.selected.and_then(|s| e.ed.cast.get(s)) {
        Some(c) => {
            draw.text("role", [x, y + 3.0], grey, false);
            let role = c.role.clone();
            field(draw, &mut b, [x + 34.0, y, 90.0, h], &role, e.focus == Some(F::Role), Do::Field(F::Role), mouse, blink);
            button(draw, &mut b, [x + 128.0, y, 56.0, h], "- Actor", Do::DelRole, mouse);
        }
        None => draw.text("Pick a role on the stage", [x, y + 3.0], 0xFF70_7080, false),
    }
    if e.ed.cast.len() < MAX_CAST {
        button(draw, &mut b, [x + 188.0, y, 56.0, h], "+ Actor", Do::AddRole, mouse);
    }
    y += ROW + 2.0;
    draw.text("sec   who        says                       plays", [x, y], 0xFF70_7080, false);
    y += 10.0;
    e.list = [x, y, W - 156.0, ROWS as f32 * ROW];
    let names = e.ed.who_names();
    let end = (e.scroll + ROWS).min(e.ed.cues.len());
    for i in e.scroll..end {
        let c = e.ed.cues[i].clone();
        hint_field(draw, &mut b, [x, y, 24.0, h], &c.at, "s", e.focus == Some(F::At(i)), Do::Field(F::At(i)), mouse, blink);
        picker(draw, &mut b, [x + 26.0, y, 56.0, h], names.get(c.who).map(String::as_str).unwrap_or("lead"), Do::Who(i), mouse);
        hint_field(draw, &mut b, [x + 84.0, y, 90.0, h], &c.say, "a line, @player", e.focus == Some(F::Say(i)), Do::Field(F::Say(i)), mouse, blink);
        picker(draw, &mut b, [x + 176.0, y, 56.0, h], &short_anim(&c.animation), Do::Anim(i), mouse);
        button(draw, &mut b, [x + 234.0, y, 12.0, h], "x", Do::DelCue(i), mouse);
        y += ROW;
    }
    if e.ed.cues.len() > ROWS {
        draw.text(&format!("{}-{} of {} (wheel)", e.scroll + 1, end, e.ed.cues.len()), [x + 60.0, y + 3.0], 0xFF70_7080, false);
    }
    if e.ed.cues.len() < MAX_CUES {
        button(draw, &mut b, [x, y, 56.0, h], "+ Line", Do::AddCue, mouse);
    }
    let by = y0 + H - 30.0;
    draw.text("save as", [x0, by + 3.0], grey, false);
    let saving = e.save_as.clone();
    field(draw, &mut b, [x0 + 38.0, by, 96.0, 14.0], &saving, e.focus == Some(F::SaveAs), Do::Field(F::SaveAs), mouse, blink);
    button(draw, &mut b, [x0 + 138.0, by, 44.0, 14.0], "Save", Do::Save(false), mouse);
    button(draw, &mut b, [x0 + 186.0, by, 76.0, 14.0], "Save & play", Do::Save(true), mouse);
    button(draw, &mut b, [x0 + W - 50.0, by, 50.0, 14.0], "Close", Do::Close, mouse);
    let status = head(draw, &e.status, W);
    draw.text(&status, [x0, by + 18.0], 0xFFC0_C0C8, false);
    e.buttons = b;
}

// ---- input ----------------------------------------------------------------------------------------------

fn role_at(e: &Editor, mouse: [f32; 2]) -> Option<usize> {
    e.ed.cast.iter().rposition(|c| {
        let m = mark_at(c.at, e.stage);
        (mouse[0] - m[0]).abs() <= MARK / 2.0 + 1.0 && (mouse[1] - m[1]).abs() <= MARK / 2.0 + 1.0
    })
}

pub fn mouse_pressed(host: &dyn ClientHost, ev: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(e) = lock.as_mut() else { return Flow::Continue };
    let mouse = ev.screen.mouse;
    if let Some(what) = e.buttons.iter().rev().find(|(r, _)| inside(*r, mouse)).map(|(_, w)| *w) {
        if !matches!(what, Do::Field(_)) {
            host.play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0);
        }
        act(e, what, if ev.button == 1 { -1 } else { 1 });
        return Flow::Cancel;
    }
    e.focus = None;
    if let Some(i) = role_at(e, mouse) {
        e.selected = Some(i);
        e.drag = true;
    } else if inside([e.stage[0], e.stage[1], STAGE[0], STAGE[1]], mouse) {
        e.selected = None;
    }
    Flow::Cancel
}

pub fn mouse_dragged(host: &dyn ClientHost, ev: &mut ScreenMouseDraggedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(e) = lock.as_mut() else { return Flow::Continue };
    if e.drag
        && let Some(s) = e.selected
    {
        let m = ev.screen.mouse;
        let x = ((m[0] - e.stage[0] - LEAD[0]) / BLOCK * 2.0).round() / 2.0;
        let z = ((e.stage[1] + LEAD[1] - m[1]) / BLOCK * 2.0).round() / 2.0;
        let max_x = LEAD[0] / BLOCK - 0.5;
        if let Some(c) = e.ed.cast.get_mut(s) {
            c.at = [x.clamp(-max_x, max_x), z.clamp(-(STAGE[1] - LEAD[1]) / BLOCK + 0.5, LEAD[1] / BLOCK - 0.5)];
        }
    }
    Flow::Cancel
}

pub fn mouse_released(host: &dyn ClientHost) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    if let Ok(mut lock) = EDITOR.lock()
        && let Some(e) = lock.as_mut()
    {
        e.drag = false;
    }
    Flow::Cancel
}

pub fn mouse_scrolled(host: &dyn ClientHost, mouse: [f32; 2], up: bool) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(e) = lock.as_mut() else { return Flow::Continue };
    let by = if up { -1 } else { 1 };
    if let Some(what) = e.buttons.iter().find(|(r, w)| inside(*r, mouse) && matches!(w, Do::Who(_) | Do::Anim(_) | Do::LeadAnim)).map(|(_, w)| *w) {
        act(e, what, by);
    } else if let Some(i) = role_at(e, mouse).or(e.selected.filter(|_| inside([e.stage[0], e.stage[1], STAGE[0], STAGE[1]], mouse))) {
        if let Some(c) = e.ed.cast.get_mut(i) {
            c.turn = (c.turn + 45.0 * by as f32).rem_euclid(360.0);
        }
    } else if inside(e.list, mouse) {
        let max = e.ed.cues.len().saturating_sub(ROWS);
        e.scroll = (e.scroll as i64 + by as i64).clamp(0, max as i64) as usize;
    }
    Flow::Cancel
}

fn type_text(e: &mut Editor, text: &str) {
    let Some(f) = e.focus else { return };
    let Some(s) = e.text(f) else { return };
    for ch in text.chars().filter(|ch| !ch.is_control()) {
        if s.chars().count() >= 200 {
            break;
        }
        s.push(ch);
    }
}

pub fn char_typed(host: &dyn ClientHost, ev: &mut ScreenCharTypedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(e) = lock.as_mut() else { return Flow::Continue };
    type_text(e, &ev.c.to_string());
    Flow::Cancel
}

pub fn key_pressed(host: &dyn ClientHost, ev: &mut ScreenKeyPressedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(e) = lock.as_mut() else { return Flow::Continue };
    let ctrl = ev.modifiers & 2 != 0;
    let shift = ev.modifiers & 1 != 0;
    match ev.key {
        "key.keyboard.escape" => return Flow::Continue,
        "key.keyboard.backspace" => {
            if let Some(s) = e.focus.and_then(|f| e.text(f)) {
                if ctrl {
                    s.clear();
                } else {
                    s.pop();
                }
            }
        }
        "key.keyboard.tab" | "key.keyboard.enter" | "key.keyboard.keypad.enter" => {
            let order = e.order();
            let at = e.focus.and_then(|f| order.iter().position(|x| *x == f));
            let next = match at {
                Some(p) if shift => (p + order.len() - 1) % order.len(),
                Some(p) => (p + 1) % order.len(),
                None => 0,
            };
            e.focus = order.get(next).copied();
        }
        "key.keyboard.v" if ctrl => {
            if let Some(text) = host.clipboard_text() {
                type_text(e, &text);
            }
        }
        "key.keyboard.s" if ctrl => e.save(false),
        _ => {}
    }
    Flow::Cancel
}

pub fn tick(host: &mut dyn ClientHost) {
    let (out, close) = {
        let Ok(mut lock) = EDITOR.lock() else { return };
        let Some(e) = lock.as_mut() else { return };
        let out = std::mem::take(&mut e.outbox);
        let close = e.close;
        if close || (e.shown && host.mod_screen() != Some(SCREEN)) {
            *lock = None;
        } else if !e.shown && host.mod_screen() == Some(SCREEN) {
            e.shown = true;
        }
        (out, close)
    };
    for bytes in out {
        let _ = host.send_payload_to_server(&PayloadId(crate::scenes::EDIT_SAVE.into()), PayloadBytes(bytes));
    }
    if close {
        host.mod_screen_close(SCREEN);
    }
}

pub fn register(r: &mut dyn mod_api::Registrar) {
    crate::scenes::register(r, Arc::new(OpenHandler));
}
