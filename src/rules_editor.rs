//! The rules editor, in the world (`/felix rules edit <name>`): an NPC's rules as a list (left),
//! the one picked beside it: when (the event and its word, how far it looks, how long it rests),
//! if (up to three conditions, all of them), then and else (what it does). Save sends them to the
//! server, which keeps them as `felixrules:<name>` for NPCs (`/felix rules <npc> felixrules:<name>`).
use crate::creator::{button, field, head, hint_field, inside, picker};
use crate::rules_edit::{self, DOS, DoEd, IFS, IfEd, MAX_ELSE, MAX_IFS, MAX_RULES, MAX_THEN, RuleEd, WHENS};
use crate::serde_json::{Value, json};
use mod_api::{
    ClientHost, DrawContext, Flow, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId, Registrar,
    ScreenCharTypedPreEvent, ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenRenderPostEvent, SoundPlay,
};
use std::sync::{Arc, LazyLock, Mutex};

/// Server to client: open the editor (JSON: id, name, file).
pub const OPEN: &str = "projectfelix:rules_open";
/// Client to server: rules to keep (JSON: name, file).
pub const SAVE: &str = "projectfelix:rules_save";

const SCREEN: &str = "projectfelix";
const W: f32 = 416.0;
const H: f32 = 228.0;
const ROW: f32 = 12.0;
const LIST: f32 = 150.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum F {
    Word,
    Radius,
    Cooldown,
    If(usize),
    Then(usize),
    Else(usize),
    SaveAs,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Do {
    Field(F),
    Select(usize),
    AddRule,
    DelRule,
    When,
    AddIf,
    DelIf(usize),
    IfKind(usize),
    IfNot(usize),
    AddThen,
    DelThen(usize),
    ThenKind(usize),
    AddElse,
    DelElse(usize),
    ElseKind(usize),
    CopyRule,
    Save,
    Try,
    Close,
}

struct Editor {
    id: String,
    rules: Vec<RuleEd>,
    selected: usize,
    focus: Option<F>,
    save_as: String,
    buttons: Vec<([f32; 4], Do)>,
    status: String,
    shown: bool,
    close: bool,
    outbox: Vec<Vec<u8>>,
    history: Vec<Vec<RuleEd>>,
}

static EDITOR: LazyLock<Mutex<Option<Editor>>> = LazyLock::new(Default::default);

pub fn active() -> bool {
    EDITOR.lock().is_ok_and(|e| e.is_some())
}

pub fn register(r: &mut dyn Registrar) {
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_client(PayloadId(OPEN.into()), Some(Arc::new(OpenHandler)));
    reg.play_to_server(PayloadId(SAVE.into()), Arc::new(crate::rulesets::SaveRules));
}

struct OpenHandler;

impl IPayloadHandler for OpenHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(host) = context.client_host() else { return Ok(()) };
        let Ok(v) = crate::serde_json::from_slice::<Value>(&payload.0) else { return Ok(()) };
        let id = v.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let name = v.get("name").and_then(Value::as_str).unwrap_or("mine").to_string();
        let mut rules = v.get("file").map(rules_edit::from_json).unwrap_or_default();
        if rules.is_empty() {
            rules.push(RuleEd::new());
        }
        crate::menu::yield_screen();
        let shown = host.mod_screen() == Some(SCREEN) || host.mod_screen_open(SCREEN);
        let e = Editor {
            id,
            rules,
            selected: 0,
            focus: None,
            save_as: name,
            buttons: Vec::new(),
            status: "Pick a rule on the left. Pickers: click on, right-click back. Ctrl+Z undoes; Try gives them to the nearest NPC.".into(),
            shown,
            close: false,
            outbox: Vec::new(),
            history: Vec::new(),
        };
        if let Ok(mut lock) = EDITOR.lock() {
            *lock = Some(e);
        }
        Ok(())
    }
}

impl Editor {
    fn text(&mut self, f: F) -> Option<&mut String> {
        if let F::SaveAs = f {
            return Some(&mut self.save_as);
        }
        let r = self.rules.get_mut(self.selected)?;
        Some(match f {
            F::Word => &mut r.word,
            F::Radius => &mut r.radius,
            F::Cooldown => &mut r.cooldown,
            F::If(i) => &mut r.ifs.get_mut(i)?.value,
            F::Then(i) => &mut r.then.get_mut(i)?.value,
            F::Else(i) => &mut r.otherwise.get_mut(i)?.value,
            F::SaveAs => return None,
        })
    }

    fn order(&self) -> Vec<F> {
        let mut o = vec![F::Word, F::Radius, F::Cooldown];
        if let Some(r) = self.rules.get(self.selected) {
            o.extend((0..r.ifs.len()).map(F::If));
            o.extend((0..r.then.len()).map(F::Then));
            o.extend((0..r.otherwise.len()).map(F::Else));
        }
        o.push(F::SaveAs);
        o
    }

    fn remember(&mut self) {
        if self.history.last() != Some(&self.rules) {
            self.history.push(self.rules.clone());
            if self.history.len() > 40 {
                self.history.remove(0);
            }
        }
    }

    fn undo(&mut self) {
        if let Some(before) = self.history.pop() {
            self.rules = before;
            self.selected = self.selected.min(self.rules.len().saturating_sub(1));
            self.focus = None;
            self.status = format!("Undone ({} more back)", self.history.len());
        }
    }

    fn save(&mut self) {
        self.send(false);
    }

    /// Saves them; trying, the NPC nearest the player follows them at once.
    fn send(&mut self, try_it: bool) {
        match rules_edit::to_json(&self.rules) {
            Ok(file) => {
                let name = crate::profile::make_id(self.save_as.trim());
                self.outbox.push(crate::serde_json::to_vec(&json!({ "name": name, "file": file, "try": try_it })).unwrap_or_default());
                if try_it {
                    self.close = true;
                }
                self.status = format!("Sent: felixrules:{name}");
            }
            Err(e) => self.status = e,
        }
    }
}

fn cycle(i: usize, by: i32, n: usize) -> usize {
    if n == 0 { 0 } else { (i as i64 + by as i64).rem_euclid(n as i64) as usize }
}

fn act(e: &mut Editor, what: Do, by: i32) {
    if let Do::Field(f) = what {
        if e.focus != Some(f) {
            e.remember();
        }
        e.focus = Some(f);
        return;
    }
    if !matches!(what, Do::Select(_) | Do::Save | Do::Try | Do::Close) {
        e.remember();
    }
    match what {
        Do::Select(i) => {
            e.selected = i;
            e.focus = None;
            return;
        }
        Do::AddRule => {
            if e.rules.len() < MAX_RULES {
                e.rules.push(RuleEd::new());
                e.selected = e.rules.len() - 1;
            }
            return;
        }
        Do::DelRule => {
            if e.rules.len() > 1 {
                e.rules.remove(e.selected);
                e.selected = e.selected.min(e.rules.len() - 1);
            }
            return;
        }
        Do::Save => return e.save(),
        Do::Try => return e.send(true),
        Do::CopyRule => {
            if e.rules.len() < MAX_RULES {
                let r = e.rules[e.selected].clone();
                e.rules.insert(e.selected + 1, r);
                e.selected += 1;
            }
            return;
        }
        Do::Close => {
            e.close = true;
            return;
        }
        _ => {}
    }
    let Some(r) = e.rules.get_mut(e.selected) else { return };
    e.focus = None;
    match what {
        Do::When => {
            r.when = cycle(r.when, by, WHENS.len());
            r.raw_when = None;
        }
        Do::AddIf => {
            if r.ifs.len() < MAX_IFS {
                r.ifs.push(IfEd::default());
            }
        }
        Do::DelIf(i) => {
            if i < r.ifs.len() {
                r.ifs.remove(i);
            }
        }
        Do::IfKind(i) => {
            if let Some(c) = r.ifs.get_mut(i) {
                c.kind = cycle(c.kind, by, IFS.len() - 1);
                c.raw = None;
            }
        }
        Do::IfNot(i) => {
            if let Some(c) = r.ifs.get_mut(i) {
                c.not = !c.not;
            }
        }
        Do::AddThen => {
            if r.then.len() < MAX_THEN {
                r.then.push(DoEd { kind: 1, ..DoEd::default() });
            }
        }
        Do::DelThen(i) => {
            if i < r.then.len() {
                r.then.remove(i);
            }
        }
        Do::ThenKind(i) => {
            if let Some(d) = r.then.get_mut(i) {
                d.kind = cycle(d.kind, by, DOS.len() - 1);
                d.raw = None;
            }
        }
        Do::AddElse => {
            if r.otherwise.len() < MAX_ELSE {
                r.otherwise.push(DoEd { kind: 1, ..DoEd::default() });
            }
        }
        Do::DelElse(i) => {
            if i < r.otherwise.len() {
                r.otherwise.remove(i);
            }
        }
        Do::ElseKind(i) => {
            if let Some(d) = r.otherwise.get_mut(i) {
                d.kind = cycle(d.kind, by, DOS.len() - 1);
                d.raw = None;
            }
        }
        _ => {}
    }
}

// ---- drawing --------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn do_rows(draw: &mut dyn DrawContext, b: &mut Vec<([f32; 4], Do)>, e: &Editor, list: &[DoEd], at: [f32; 2], wide: f32, kind: fn(usize) -> Do, del: fn(usize) -> Do, f: fn(usize) -> F, mouse: [f32; 2], blink: bool) -> f32 {
    let [x, mut y] = at;
    let h = ROW - 1.0;
    for (i, d) in list.iter().enumerate() {
        picker(draw, b, [x, y, 66.0, h], DOS.get(d.kind).map_or("?", |k| k.0), kind(i), mouse);
        let hint = DOS.get(d.kind).map_or("", |k| k.1);
        if !hint.is_empty() {
            hint_field(draw, b, [x + 68.0, y, wide - 82.0, h], &d.value, hint, e.focus == Some(f(i)), Do::Field(f(i)), mouse, blink);
        }
        button(draw, b, [x + wide - 12.0, y, 12.0, h], "x", del(i), mouse);
        y += ROW;
    }
    y
}

fn draw_panel(draw: &mut dyn DrawContext, b: &mut Vec<([f32; 4], Do)>, e: &Editor, at: [f32; 2], mouse: [f32; 2], blink: bool) {
    let [x, mut y] = at;
    let wide = W - LIST - 8.0;
    let h = ROW - 1.0;
    let grey = 0xFFA0_A0A0;
    let dim = 0xFF70_7080;
    let Some(r) = e.rules.get(e.selected) else { return };
    draw.text("when", [x, y + 2.0], grey, false);
    let when_name = if r.raw_when.is_some() { "(custom)" } else { WHENS[r.when.min(WHENS.len() - 1)].0 };
    picker(draw, b, [x + 28.0, y, 92.0, h], when_name, Do::When, mouse);
    if let Some(hint) = WHENS[r.when.min(WHENS.len() - 1)].2.filter(|_| r.raw_when.is_none()) {
        hint_field(draw, b, [x + 122.0, y, wide - 122.0, h], &r.word, hint, e.focus == Some(F::Word), Do::Field(F::Word), mouse, blink);
    }
    y += ROW;
    draw.text("reach", [x, y + 2.0], grey, false);
    hint_field(draw, b, [x + 30.0, y, 28.0, h], &r.radius, "12", e.focus == Some(F::Radius), Do::Field(F::Radius), mouse, blink);
    draw.text("rests", [x + 64.0, y + 2.0], grey, false);
    hint_field(draw, b, [x + 94.0, y, 28.0, h], &r.cooldown, "5 s", e.focus == Some(F::Cooldown), Do::Field(F::Cooldown), mouse, blink);
    draw.text("blocks, seconds", [x + 128.0, y + 2.0], dim, false);
    y += ROW + 2.0;
    draw.text("if (all of them)", [x, y + 2.0], 0xFF6A_B0FF, false);
    if r.ifs.len() < MAX_IFS {
        button(draw, b, [x + 80.0, y, 34.0, h], "+ If", Do::AddIf, mouse);
    }
    if r.ifs.is_empty() {
        draw.text("always", [x + 120.0, y + 2.0], dim, false);
    }
    y += ROW;
    for (i, c) in r.ifs.iter().enumerate() {
        picker(draw, b, [x, y, 66.0, h], IFS.get(c.kind).map_or("?", |k| k.0), Do::IfKind(i), mouse);
        let hint = IFS.get(c.kind).map_or("", |k| k.1);
        if !hint.is_empty() {
            hint_field(draw, b, [x + 68.0, y, wide - 98.0, h], &c.value, hint, e.focus == Some(F::If(i)), Do::Field(F::If(i)), mouse, blink);
        }
        button(draw, b, [x + wide - 28.0, y, 14.0, h], if c.not { "!" } else { "=" }, Do::IfNot(i), mouse);
        button(draw, b, [x + wide - 12.0, y, 12.0, h], "x", Do::DelIf(i), mouse);
        y += ROW;
    }
    y += 2.0;
    draw.text("then", [x, y + 2.0], 0xFF9A_E6A0, false);
    if r.then.len() < MAX_THEN {
        button(draw, b, [x + 80.0, y, 40.0, h], "+ Then", Do::AddThen, mouse);
    }
    y += ROW;
    y = do_rows(draw, b, e, &r.then, [x, y], wide, Do::ThenKind, Do::DelThen, F::Then, mouse, blink);
    y += 2.0;
    draw.text("else", [x, y + 2.0], 0xFFFF_9A6A, false);
    if r.otherwise.len() < MAX_ELSE {
        button(draw, b, [x + 80.0, y, 40.0, h], "+ Else", Do::AddElse, mouse);
    }
    if r.ifs.is_empty() {
        draw.text("(with an if)", [x + 124.0, y + 2.0], dim, false);
    }
    y += ROW;
    do_rows(draw, b, e, &r.otherwise, [x, y], wide, Do::ElseKind, Do::DelElse, F::Else, mouse, blink);
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
    let title = if e.id.is_empty() { "New rules".to_string() } else { format!("Rules {}", e.id) };
    draw.text(&title, [x0, y0], 0xFFFF_D86A, false);
    let mut b = Vec::new();
    // The list: each rule in a few words.
    let mut y = y0 + 12.0;
    draw.fill([x0, y, LIST, H - 42.0], 0xFF18_1C24);
    for (i, r) in e.rules.iter().enumerate() {
        let rect = [x0 + 1.0, y + 1.0, LIST - 2.0, 14.0];
        if i == e.selected {
            draw.fill(rect, 0xF050_4020);
        } else if inside(rect, mouse) {
            draw.fill(rect, 0x6040_4860);
        }
        let label = head(draw, &format!("{}. {}", i + 1, r.summary()), LIST - 6.0);
        draw.text(&label, [x0 + 3.0, y + 4.0], 0xFFFF_FFFF, false);
        b.push((rect, Do::Select(i)));
        y += 15.0;
    }
    if e.rules.len() < MAX_RULES {
        button(draw, &mut b, [x0 + 2.0, y + 2.0, 50.0, 12.0], "+ Rule", Do::AddRule, mouse);
    }
    button(draw, &mut b, [x0 + 56.0, y + 2.0, 50.0, 12.0], "- Rule", Do::DelRule, mouse);
    button(draw, &mut b, [x0 + 110.0, y + 2.0, 38.0, 12.0], "Copy", Do::CopyRule, mouse);
    draw_panel(draw, &mut b, e, [x0 + LIST + 8.0, y0 + 12.0], mouse, blink);
    let y = y0 + H - 26.0;
    draw.text("save as", [x0, y + 3.0], 0xFFA0_A0A0, false);
    let saving = e.save_as.clone();
    field(draw, &mut b, [x0 + 42.0, y, 90.0, 14.0], &saving, e.focus == Some(F::SaveAs), Do::Field(F::SaveAs), mouse, blink);
    button(draw, &mut b, [x0 + 136.0, y, 40.0, 14.0], "Save", Do::Save, mouse);
    button(draw, &mut b, [x0 + 180.0, y, 30.0, 14.0], "Try", Do::Try, mouse);
    button(draw, &mut b, [x0 + W - 44.0, y, 44.0, 14.0], "Close", Do::Close, mouse);
    let status = crate::creator::tail(draw, &e.status, W);
    draw.text(&status, [x0, y + 17.0], 0xFFC0_C0C8, false);
    e.buttons = b;
}

// ---- input --------------------------------------------------------------------------------------------

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
    } else {
        e.focus = None;
    }
    Flow::Cancel
}

pub fn mouse_scrolled(host: &dyn ClientHost, mouse: [f32; 2], up: bool) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(e) = lock.as_mut() else { return Flow::Continue };
    if let Some(what) = e.buttons.iter().find(|(r, w)| inside(*r, mouse) && matches!(w, Do::When | Do::IfKind(_) | Do::ThenKind(_) | Do::ElseKind(_))).map(|(_, w)| *w) {
        act(e, what, if up { -1 } else { 1 });
    }
    Flow::Cancel
}

fn type_text(e: &mut Editor, text: &str) {
    let Some(f) = e.focus else { return };
    let Some(s) = e.text(f) else { return };
    for ch in text.chars().filter(|ch| !ch.is_control()) {
        if s.chars().count() >= 240 {
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
    let ctrl = ev.modifiers & 2 != 0 && ev.modifiers & 4 == 0;
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
        "key.keyboard.s" if ctrl => e.save(),
        "key.keyboard.z" if ctrl => e.undo(),
        _ => {}
    }
    Flow::Cancel
}

/// The client tick: sends what was saved, and forgets the editor once its screen is closed.
pub fn tick(host: &mut dyn ClientHost) {
    let (out, close) = {
        let Ok(mut lock) = EDITOR.lock() else { return };
        let Some(e) = lock.as_mut() else { return };
        let out = std::mem::take(&mut e.outbox);
        let close = e.close;
        if close || (e.shown && host.mod_screen() != Some(SCREEN)) {
            *lock = None;
        } else if !e.shown && (host.mod_screen() == Some(SCREEN) || host.mod_screen_open(SCREEN)) {
            e.shown = true;
        }
        (out, close)
    };
    for bytes in out {
        let _ = host.send_payload_to_server(&PayloadId(SAVE.into()), PayloadBytes(bytes));
    }
    if close {
        host.mod_screen_close(SCREEN);
    }
}
