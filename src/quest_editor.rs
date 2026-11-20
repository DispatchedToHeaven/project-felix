//! The quest editor, in the world (`/felix quest edit <name>`): the quest's settings and its steps
//! as a list (left); beside it, the one picked: the quest's title, what the NPC says, the reward
//! at the end, what must be done before, a time limit; or a step's goal (bring, kill, go to,
//! walk me, deliver to), what is said as it begins, along it and when it is done, and its reward.
//! Save sends it to the server, kept as `felixquest:<name>`; Try has the nearest NPC offer it.
use crate::creator::{button, field, head, hint_field, inside, picker};
use crate::quest_edit::{KINDS, MAX_STEPS, QuestEd, StepEd};
use crate::serde_json::{Value, json};
use mod_api::{
    ClientHost, DrawContext, Flow, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId, Registrar,
    ScreenCharTypedPreEvent, ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenRenderPostEvent, SoundPlay,
};
use std::sync::{Arc, LazyLock, Mutex};

pub const OPEN: &str = "projectfelix:quest_open";
pub const SAVE: &str = "projectfelix:quest_save";

const SCREEN: &str = "projectfelix";
const W: f32 = 416.0;
const H: f32 = 228.0;
const ROW: f32 = 14.0;
const LIST: f32 = 130.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum F {
    Title,
    Offer,
    Done,
    Fail,
    Reward,
    Gives,
    Requires,
    Scene,
    Time,
    MinRel,
    What,
    Count,
    Item,
    Say,
    Progress,
    SDone,
    SReward,
    SaveAs,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Do {
    Field(F),
    Select(usize),
    Kind,
    Repeat,
    AddStep,
    DelStep,
    CopyStep,
    Save,
    Try,
    Close,
}

struct Editor {
    id: String,
    q: QuestEd,
    /// 0: the quest; n: step n.
    selected: usize,
    focus: Option<F>,
    save_as: String,
    buttons: Vec<([f32; 4], Do)>,
    status: String,
    shown: bool,
    close: bool,
    outbox: Vec<Vec<u8>>,
    /// What it was when opened or last saved, and whether closing unsaved was warned of.
    saved: QuestEd,
    warned: bool,
    history: Vec<QuestEd>,
    /// What was undone, for Ctrl+Y.
    future: Vec<QuestEd>,
}

static EDITOR: LazyLock<Mutex<Option<Editor>>> = LazyLock::new(Default::default);

pub fn active() -> bool {
    EDITOR.lock().is_ok_and(|e| e.is_some())
}

pub fn register(r: &mut dyn Registrar) {
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_client(PayloadId(OPEN.into()), Some(Arc::new(OpenHandler)));
    reg.play_to_server(PayloadId(SAVE.into()), Arc::new(crate::quest_store::SaveQuest));
}

struct OpenHandler;

impl IPayloadHandler for OpenHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(host) = context.client_host() else { return Ok(()) };
        let Ok(v) = crate::serde_json::from_slice::<Value>(&payload.0) else { return Ok(()) };
        let id = v.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let name = v.get("name").and_then(Value::as_str).unwrap_or("mine").to_string();
        let q = v.get("file").filter(|f| f.is_object()).map(QuestEd::from_json).unwrap_or_else(QuestEd::blank);
        crate::menu::yield_screen();
        let shown = host.mod_screen() == Some(SCREEN) || host.mod_screen_open(SCREEN);
        let e = Editor {
            id,
            q,
            selected: 0,
            focus: None,
            save_as: name,
            buttons: Vec::new(),
            status: "Pick a part on the left. Ctrl+Z undoes; Try: the nearest NPC offers it.".into(),
            shown,
            close: false,
            outbox: Vec::new(),
            saved: Default::default(),
            warned: false,
            history: Vec::new(),
            future: Vec::new(),
        };
        let mut e = e;
        e.saved = e.q.clone();
        if let Ok(mut lock) = EDITOR.lock() {
            *lock = Some(e);
        }
        Ok(())
    }
}

impl Editor {
    fn step(&mut self) -> Option<&mut StepEd> {
        let i = self.selected.checked_sub(1)?;
        self.q.steps.get_mut(i)
    }

    fn text(&mut self, f: F) -> Option<&mut String> {
        Some(match f {
            F::SaveAs => &mut self.save_as,
            F::Title => &mut self.q.title,
            F::Offer => &mut self.q.offer,
            F::Done => &mut self.q.done,
            F::Fail => &mut self.q.fail,
            F::Reward => &mut self.q.reward,
            F::Gives => &mut self.q.gives,
            F::Requires => &mut self.q.requires,
            F::Scene => &mut self.q.scene,
            F::Time => &mut self.q.time,
            F::MinRel => &mut self.q.min_relation,
            F::What => &mut self.step()?.what,
            F::Count => &mut self.step()?.count,
            F::Item => &mut self.step()?.item,
            F::Say => &mut self.step()?.say,
            F::Progress => &mut self.step()?.progress,
            F::SDone => &mut self.step()?.done,
            F::SReward => &mut self.step()?.reward,
        })
    }

    fn order(&self) -> Vec<F> {
        let mut o = if self.selected == 0 {
            vec![F::Title, F::Offer, F::Done, F::Fail, F::Reward, F::Gives, F::Requires, F::Scene, F::Time, F::MinRel]
        } else {
            vec![F::What, F::Count, F::Item, F::Say, F::Progress, F::SDone, F::SReward]
        };
        o.push(F::SaveAs);
        o
    }

    fn remember(&mut self) {
        if self.history.last() != Some(&self.q) {
            self.history.push(self.q.clone());
            self.future.clear();
            if self.history.len() > 40 {
                self.history.remove(0);
            }
        }
    }

    fn undo(&mut self) {
        if let Some(before) = self.history.pop() {
            self.future.push(std::mem::replace(&mut self.q, before));
            self.selected = self.selected.min(self.q.steps.len());
            self.focus = None;
            self.status = format!("Undone ({} more back)", self.history.len());
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.future.pop() {
            self.history.push(std::mem::replace(&mut self.q, next));
            self.focus = None;
            self.status = format!("Redone ({} more on)", self.future.len());
        }
    }

    fn dirty(&self) -> bool {
        self.q != self.saved
    }

    fn send(&mut self, try_it: bool) {
        match self.q.to_json() {
            Ok(file) => {
                let name = crate::profile::make_id(self.save_as.trim());
                self.outbox.push(crate::serde_json::to_vec(&json!({ "name": name, "file": file, "try": try_it })).unwrap_or_default());
                if try_it {
                    self.close = true;
                }
                self.saved = self.q.clone();
                self.status = format!("Sent: felixquest:{name}");
            }
            Err(e) => self.status = e,
        }
    }
}

fn act(e: &mut Editor, what: Do, by: i32) {
    if !matches!(what, Do::Close) {
        e.warned = false;
    }
    match what {
        Do::Field(f) => {
            if e.focus != Some(f) {
                e.remember();
            }
            e.focus = Some(f);
            return;
        }
        Do::Select(i) => {
            e.selected = i;
            e.focus = None;
            return;
        }
        Do::Save => return e.send(false),
        Do::Try => return e.send(true),
        Do::Close => {
            e.close = crate::creator::may_close(e.dirty(), &mut e.warned, &mut e.status);
            return;
        }
        _ => e.remember(),
    }
    e.focus = None;
    match what {
        Do::Kind => {
            if let Some(s) = e.step() {
                s.kind = (s.kind as i64 + by as i64).rem_euclid(KINDS.len() as i64) as usize;
            }
        }
        Do::Repeat => e.q.repeat = !e.q.repeat,
        Do::AddStep => {
            if e.q.steps.len() < MAX_STEPS {
                let at = e.selected.min(e.q.steps.len());
                e.q.steps.insert(at, StepEd { kind: 1, what: "monster".into(), count: "3".into(), ..StepEd::default() });
                e.selected = at + 1;
            }
        }
        Do::DelStep => {
            if e.selected > 0 && e.q.steps.len() > 1 {
                e.q.steps.remove(e.selected - 1);
                e.selected = e.selected.min(e.q.steps.len());
            }
        }
        Do::CopyStep => {
            if e.selected > 0 && e.q.steps.len() < MAX_STEPS {
                let s = e.q.steps[e.selected - 1].clone();
                e.q.steps.insert(e.selected, s);
                e.selected += 1;
            }
        }
        _ => {}
    }
}

// ---- drawing --------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn row(draw: &mut dyn DrawContext, b: &mut Vec<([f32; 4], Do)>, e: &Editor, label: &str, r: [f32; 4], f: F, value: &str, hint: &str, mouse: [f32; 2], blink: bool) {
    draw.text(label, [r[0] - draw.text_width(label) - 3.0, r[1] + 3.0], 0xFFA0_A0A0, false);
    hint_field(draw, b, r, value, hint, e.focus == Some(f), Do::Field(f), mouse, blink);
}

fn draw_panel(draw: &mut dyn DrawContext, b: &mut Vec<([f32; 4], Do)>, e: &Editor, at: [f32; 2], mouse: [f32; 2], blink: bool) {
    let [x, mut y] = at;
    let wide = W - LIST - 8.0;
    let h = ROW - 2.0;
    let lx = x + 44.0;
    let lw = wide - 44.0;
    if e.selected == 0 {
        draw.text("The quest", [x, y], 0xFFFF_D86A, false);
        y += 12.0;
        let q = &e.q;
        for (label, f, v, hint) in [
            ("title", F::Title, &q.title, "A favour for @npc"),
            ("offers", F::Offer, &q.offer, "what it asks, offering it"),
            ("thanks", F::Done, &q.done, "said when all is done"),
            ("fails", F::Fail, &q.fail, "said when given up or late"),
            ("reward", F::Reward, &q.reward, "give minecraft:emerald 3; remember x"),
            ("gives", F::Gives, &q.gives, "when taken: give minecraft:paper"),
            ("after", F::Requires, &q.requires, "quests done before (ids)"),
            ("scene", F::Scene, &q.scene, "played when done"),
        ] {
            row(draw, b, e, label, [lx, y, lw, h], f, v, hint, mouse, blink);
            y += ROW;
        }
        row(draw, b, e, "minutes", [lx, y, 34.0, h], F::Time, &q.time, "none", mouse, blink);
        row(draw, b, e, "min rel", [lx + 82.0, y, 30.0, h], F::MinRel, &q.min_relation, "-100", mouse, blink);
        button(draw, b, [x + wide - 64.0, y, 64.0, h], if q.repeat { "repeats" } else { "once" }, Do::Repeat, mouse);
        y += ROW + 4.0;
        draw.text(&format!("{} step{}; the reward and thanks come after the last.", q.steps.len(), if q.steps.len() == 1 { "" } else { "s" }), [x, y], 0xFF70_7080, false);
        return;
    }
    let Some(s) = e.q.steps.get(e.selected - 1) else { return };
    draw.text(&format!("Step {}", e.selected), [x, y], 0xFFFF_D86A, false);
    y += 12.0;
    draw.text("goal", [x + 20.0, y + 3.0], 0xFFA0_A0A0, false);
    picker(draw, b, [lx, y, 70.0, h], KINDS.get(s.kind).map_or("?", |k| k.0), Do::Kind, mouse);
    hint_field(draw, b, [lx + 72.0, y, lw - 72.0, h], &s.what, KINDS.get(s.kind).map_or("", |k| k.1), e.focus == Some(F::What), Do::Field(F::What), mouse, blink);
    y += ROW;
    if s.kind != 2 && s.kind != 3 {
        row(draw, b, e, "count", [lx, y, 34.0, h], F::Count, &s.count, "1", mouse, blink);
    }
    if s.kind == 4 {
        row(draw, b, e, "hands", [lx + 80.0, y, lw - 80.0, h], F::Item, &s.item, "item (none: a word)", mouse, blink);
    }
    y += ROW;
    for (label, f, v, hint) in [
        ("says", F::Say, &s.say, "as the step begins"),
        ("asks", F::Progress, &s.progress, "while it is under way"),
        ("done", F::SDone, &s.done, "when the step is done"),
        ("gives", F::SReward, &s.reward, "its reward: give minecraft:bread"),
    ] {
        row(draw, b, e, label, [lx, y, lw, h], f, v, hint, mouse, blink);
        y += ROW;
    }
    y += 4.0;
    draw.text("Steps go one by one; empty words use the quest's.", [x, y], 0xFF70_7080, false);
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
    let title = if e.id.is_empty() { "New quest".to_string() } else { format!("Quest {}", e.id) };
    draw.text(&title, [x0, y0], 0xFFFF_D86A, false);
    let mut b = Vec::new();
    let mut y = y0 + 12.0;
    draw.fill([x0, y, LIST, H - 42.0], 0xFF18_1C24);
    let rows: Vec<String> = std::iter::once(format!("Quest: {}", e.q.title)).chain(e.q.steps.iter().enumerate().map(|(i, s)| format!("{}. {}", i + 1, s.summary()))).collect();
    for (i, r) in rows.iter().enumerate() {
        let rect = [x0 + 1.0, y + 1.0, LIST - 2.0, 14.0];
        if i == e.selected {
            draw.fill(rect, 0xF050_4020);
        } else if inside(rect, mouse) {
            draw.fill(rect, 0x6040_4860);
        }
        let label = head(draw, r, LIST - 6.0);
        draw.text(&label, [x0 + 3.0, y + 4.0], if i == 0 { 0xFFFF_D86A } else { 0xFFFF_FFFF }, false);
        b.push((rect, Do::Select(i)));
        y += 15.0;
    }
    button(draw, &mut b, [x0 + 2.0, y + 2.0, 40.0, 12.0], "+ Step", Do::AddStep, mouse);
    button(draw, &mut b, [x0 + 45.0, y + 2.0, 40.0, 12.0], "- Step", Do::DelStep, mouse);
    button(draw, &mut b, [x0 + 88.0, y + 2.0, 38.0, 12.0], "Copy", Do::CopyStep, mouse);
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
    if e.buttons.iter().any(|(r, w)| inside(*r, mouse) && *w == Do::Kind) {
        act(e, Do::Kind, if up { -1 } else { 1 });
    }
    Flow::Cancel
}

fn type_text(e: &mut Editor, text: &str) {
    let Some(f) = e.focus else { return };
    let Some(s) = e.text(f) else { return };
    crate::creator::type_in(s, text, 240);
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
    if ev.key == "key.keyboard.escape" {
        let dirty = e.dirty();
        return if crate::creator::may_close(dirty, &mut e.warned, &mut e.status) { Flow::Continue } else { Flow::Cancel };
    }
    e.warned = false;
    if let Some(s) = e.focus.and_then(|f| e.text(f))
        && crate::creator::typing_key(host, s, ev.key, ev.modifiers, 240)
    {
        return Flow::Cancel;
    }
    match ev.key {
        "key.keyboard.tab" | "key.keyboard.enter" | "key.keyboard.keypad.enter" => {
            let order = e.order();
            let at = e.focus.and_then(|f| order.iter().position(|x| *x == f));
            let next = match at {
                Some(p) if shift => (p + order.len() - 1) % order.len(),
                Some(p) => (p + 1) % order.len(),
                None => 0,
            };
            e.focus = order.get(next).copied();
            crate::creator::typing_reset();
        }
        "key.keyboard.s" if ctrl => e.send(false),
        "key.keyboard.z" if ctrl && !shift => e.undo(),
        "key.keyboard.y" | "key.keyboard.z" if ctrl => e.redo(),
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
