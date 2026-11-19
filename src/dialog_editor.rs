//! The dialog editor, in the world (`/felix dialog edit <name>`): the dialog's nodes as boxes on a
//! canvas, with arrows for where answers (yellow, by letter), checks (blue) and going on (green)
//! lead. Beside it, the node picked: a talk's lines (a page each, or all together), what it does
//! and where it goes on; its answers, each with when it is shown and what it does that the player
//! doesn't see (relation, a feeling, actions); or a check's conditions and where each leads. Save
//! sends it to the server, which keeps it as `felixdialog:<name>` for NPCs to speak.
use crate::creator::{button, field, head, hint_field, inside, picker};
use crate::dialog_edit::{AnswerEd, BOX, CANVAS, CONDS, CheckEd, CondEd, DialogEd, END, MAX_ANSWERS, MAX_CHECKS, MAX_LINES};
use crate::serde_json::{Value, json};
use mod_api::{
    ClientHost, DrawContext, Flow, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId, Registrar,
    ScreenCharTypedPreEvent, ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenMouseDraggedPreEvent,
    ScreenRenderPostEvent, SoundPlay,
};
use std::sync::{Arc, LazyLock, Mutex};

/// Server to client: open the editor (JSON: id, name, file).
pub const OPEN: &str = "projectfelix:dialog_open";
/// Client to server: a dialog to keep (JSON: name, file).
pub const SAVE: &str = "projectfelix:dialog_save";

const SCREEN: &str = "projectfelix";
const W: f32 = 416.0;
const H: f32 = 228.0;
const ROW: f32 = 14.0;
/// Rows of an answer.
const AROW: f32 = 12.0;
const PANEL: f32 = 236.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum F {
    Name,
    Line(usize),
    Anim,
    More,
    Act,
    AText(usize),
    AValue(usize),
    ARel(usize),
    AFeel(usize),
    AAct(usize),
    CValue(usize),
    SaveAs,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Do {
    Field(F),
    Tab(u8),
    Start,
    Paged,
    AddLine,
    DelLine(usize),
    Next,
    AddAnswer,
    DelAnswer(usize),
    ACond(usize),
    ANot(usize),
    AGoto(usize),
    AddCheck,
    DelCheck(usize),
    CCond(usize),
    CNot(usize),
    CGoto(usize),
    AddTalk,
    AddCheckNode,
    DelNode,
    Save,
    Close,
}

struct Editor {
    id: String,
    ed: DialogEd,
    selected: usize,
    tab: u8,
    focus: Option<F>,
    save_as: String,
    drag: Option<[f32; 2]>,
    canvas: [f32; 2],
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

pub fn register(r: &mut dyn Registrar) {
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_client(PayloadId(OPEN.into()), Some(Arc::new(OpenHandler)));
    reg.play_to_server(PayloadId(SAVE.into()), Arc::new(crate::dialogs::SaveDialog));
}

struct OpenHandler;

impl IPayloadHandler for OpenHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(host) = context.client_host() else { return Ok(()) };
        let Ok(v) = crate::serde_json::from_slice::<Value>(&payload.0) else { return Ok(()) };
        let id = v.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let name = v.get("name").and_then(Value::as_str).unwrap_or("mine").to_string();
        let ed = v.get("file").map(DialogEd::from_json).filter(|d| !d.nodes.is_empty()).unwrap_or_else(DialogEd::blank);
        crate::menu::yield_screen();
        let shown = host.mod_screen() == Some(SCREEN) || host.mod_screen_open(SCREEN);
        let e = Editor {
            id,
            ed,
            selected: 0,
            tab: 0,
            focus: None,
            save_as: name,
            drag: None,
            canvas: [0.0; 2],
            buttons: Vec::new(),
            status: "Pick a node to change it, drag it about. Right-click a picker goes back.".into(),
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
        if let F::SaveAs = f {
            return Some(&mut self.save_as);
        }
        let n = self.ed.nodes.get_mut(self.selected)?;
        Some(match f {
            F::Name => &mut n.name,
            F::Line(i) => n.lines.get_mut(i)?,
            F::Anim => &mut n.animation,
            F::More => &mut n.more,
            F::Act => &mut n.act,
            F::AText(i) => &mut n.answers.get_mut(i)?.text,
            F::AValue(i) => &mut n.answers.get_mut(i)?.cond.value,
            F::ARel(i) => &mut n.answers.get_mut(i)?.relation,
            F::AFeel(i) => &mut n.answers.get_mut(i)?.feel,
            F::AAct(i) => &mut n.answers.get_mut(i)?.act,
            F::CValue(i) => &mut n.checks.get_mut(i)?.cond.value,
            F::SaveAs => return None,
        })
    }

    /// The fields of the tab shown, in Tab order.
    fn order(&self) -> Vec<F> {
        let Some(n) = self.ed.nodes.get(self.selected) else { return vec![F::SaveAs] };
        let mut o = match (n.check, self.tab) {
            (true, _) => {
                let mut o = vec![F::Name];
                o.extend((0..n.checks.len()).map(F::CValue));
                o
            }
            (false, 1) => (0..n.answers.len()).flat_map(|i| [F::AText(i), F::AValue(i), F::ARel(i), F::AFeel(i), F::AAct(i)]).collect(),
            _ => {
                let mut o = vec![F::Name];
                o.extend((0..n.lines.len()).map(F::Line));
                o.extend([F::Anim, F::More, F::Act]);
                o
            }
        };
        o.push(F::SaveAs);
        o
    }

    fn save(&mut self) {
        match self.ed.to_json() {
            Ok(file) => {
                let name = crate::profile::make_id(self.save_as.trim());
                let body = json!({ "name": name, "file": file });
                self.outbox.push(crate::serde_json::to_vec(&body).unwrap_or_default());
                self.status = format!("Sent: felixdialog:{name}");
            }
            Err(e) => self.status = e,
        }
    }
}

/// The node after `g` (with the end first), for goto pickers.
fn cycle_goto(g: usize, by: i32, n: usize) -> usize {
    let at = if g == END { 0 } else { g + 1 };
    let next = (at as i64 + by as i64).rem_euclid(n as i64 + 1) as usize;
    if next == 0 { END } else { next - 1 }
}

fn cycle(i: usize, by: i32, n: usize) -> usize {
    if n == 0 { 0 } else { (i as i64 + by as i64).rem_euclid(n as i64) as usize }
}

fn act(e: &mut Editor, what: Do, by: i32) {
    let count = e.ed.nodes.len();
    let sel = e.selected;
    let Some(n) = e.ed.nodes.get_mut(sel) else { return };
    match what {
        Do::Field(f) => e.focus = Some(f),
        Do::Tab(t) => {
            e.tab = t;
            e.focus = None;
        }
        Do::Start => e.ed.start = sel,
        Do::Paged => n.paged = !n.paged,
        Do::AddLine => {
            if n.lines.len() < MAX_LINES {
                n.lines.push(String::new());
                e.focus = Some(F::Line(n.lines.len() - 1));
            }
        }
        Do::DelLine(i) => {
            if i < n.lines.len() {
                n.lines.remove(i);
                e.focus = None;
            }
        }
        Do::Next => n.next = cycle_goto(n.next, by, count),
        Do::AddAnswer => {
            if n.answers.len() < MAX_ANSWERS {
                n.answers.push(AnswerEd { goto: END, ..AnswerEd::default() });
                e.focus = Some(F::AText(n.answers.len() - 1));
            }
        }
        Do::DelAnswer(i) => {
            if i < n.answers.len() {
                n.answers.remove(i);
                e.focus = None;
            }
        }
        Do::ACond(i) => {
            if let Some(a) = n.answers.get_mut(i) {
                a.cond.kind = cycle(a.cond.kind, by, CONDS.len() - 1);
                a.cond.raw = None;
            }
        }
        Do::ANot(i) => {
            if let Some(a) = n.answers.get_mut(i) {
                a.cond.not = !a.cond.not;
            }
        }
        Do::AGoto(i) => {
            if let Some(a) = n.answers.get_mut(i) {
                a.goto = cycle_goto(a.goto, by, count);
            }
        }
        Do::AddCheck => {
            if n.checks.len() < MAX_CHECKS {
                n.checks.push(CheckEd { cond: CondEd::default(), goto: END });
            }
        }
        Do::DelCheck(i) => {
            if i < n.checks.len() && n.checks.len() > 1 {
                n.checks.remove(i);
                e.focus = None;
            }
        }
        Do::CCond(i) => {
            if let Some(c) = n.checks.get_mut(i) {
                c.cond.kind = cycle(c.cond.kind, by, CONDS.len() - 1);
                c.cond.raw = None;
            }
        }
        Do::CNot(i) => {
            if let Some(c) = n.checks.get_mut(i) {
                c.cond.not = !c.cond.not;
            }
        }
        Do::CGoto(i) => {
            if let Some(c) = n.checks.get_mut(i) {
                c.goto = cycle_goto(c.goto, by, count);
            }
        }
        Do::AddTalk | Do::AddCheckNode => {
            e.selected = e.ed.add_node(what == Do::AddCheckNode);
            e.tab = 0;
            e.focus = Some(F::Name);
        }
        Do::DelNode => {
            if count > 1 {
                e.ed.remove_node(sel);
                e.selected = sel.min(e.ed.nodes.len() - 1);
                e.focus = None;
            }
        }
        Do::Save => e.save(),
        Do::Close => e.close = true,
    }
}

// ---- drawing --------------------------------------------------------------------------------------------

fn line(draw: &mut dyn DrawContext, a: [f32; 2], b: [f32; 2], argb: u32) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let (nx, ny) = (-dy / len * 0.6, dx / len * 0.6);
    draw.quad([[a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]], argb);
}

fn centre(at: [f32; 2], c0: [f32; 2]) -> [f32; 2] {
    [c0[0] + at[0] + BOX[0] / 2.0, c0[1] + at[1] + BOX[1] / 2.0]
}

fn goto_name(e: &Editor, g: usize) -> String {
    if g == END { "(end)".into() } else { e.ed.nodes.get(g).map_or("?".into(), |n| n.name.clone()) }
}

fn draw_canvas(draw: &mut dyn DrawContext, e: &Editor, c0: [f32; 2]) {
    draw.fill([c0[0], c0[1], CANVAS[0], CANVAS[1]], 0xFF18_1C24);
    let sel = e.selected;
    for pass in 0..2 {
        for (i, n) in e.ed.nodes.iter().enumerate() {
            let mut ways: Vec<(String, usize, u32)> = Vec::new();
            for (k, a) in n.answers.iter().enumerate() {
                ways.push((crate::dialog::letter(k), a.goto, 0xFFFF_D86A));
            }
            for (k, c) in n.checks.iter().enumerate() {
                ways.push((format!("{}", k + 1), c.goto, 0xFF6A_B0FF));
            }
            if n.next != END {
                ways.push((if n.check { "else".into() } else { "on".into() }, n.next, 0xFF9A_E6A0));
            }
            for (label, to, colour) in ways {
                let Some(t) = e.ed.nodes.get(to) else { continue };
                if to == i {
                    continue;
                }
                let lit = i == sel || to == sel;
                if (pass == 0) == lit {
                    continue;
                }
                let (a, b) = (centre(n.at, c0), centre(t.at, c0));
                let colour = if lit { colour } else { 0x6090_90A0 };
                line(draw, a, b, colour);
                let p = [a[0] + (b[0] - a[0]) * 0.75, a[1] + (b[1] - a[1]) * 0.75];
                draw.fill([p[0] - 1.5, p[1] - 1.5, 3.0, 3.0], colour);
                if i == sel {
                    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0 - 4.0];
                    let w = draw.text_width(&label);
                    draw.fill([mid[0] - w / 2.0 - 1.0, mid[1] - 1.0, w + 2.0, 10.0], 0xC010_1018);
                    draw.text(&label, [mid[0] - w / 2.0, mid[1]], colour, false);
                }
            }
        }
    }
    for (i, n) in e.ed.nodes.iter().enumerate() {
        let r = [c0[0] + n.at[0], c0[1] + n.at[1], BOX[0], BOX[1]];
        let fill = match (i == sel, n.check) {
            (true, _) => 0xF050_4020,
            (false, true) => 0xE020_3858,
            (false, false) => 0xE030_3448,
        };
        draw.fill(r, fill);
        draw.fill([r[0], r[1] + r[3] - 1.0, r[2], 1.0], if i == e.ed.start { 0xFF9A_E6A0 } else { 0xFF60_6070 });
        let name = if i == e.ed.start { format!("*{}", n.name) } else if n.check { format!("?{}", n.name) } else { n.name.clone() };
        let shown = head(draw, &name, BOX[0] - 3.0);
        draw.text(&shown, [r[0] + 2.0, r[1] + 2.0], 0xFFFF_FFFF, false);
    }
    // What the colours mean.
    let y = c0[1] + CANVAS[1] - 9.0;
    let mut x = c0[0] + 2.0;
    for (t, colour) in [("answers", 0xFFFF_D86A), ("checks", 0xFF6A_B0FF), ("goes on", 0xFF9A_E6A0), ("*start ?check", 0xFF80_8090)] {
        draw.text(t, [x, y], colour, false);
        x += draw.text_width(t) + 6.0;
    }
}

#[allow(clippy::too_many_arguments)]
fn labelled(draw: &mut dyn DrawContext, b: &mut Vec<([f32; 4], Do)>, e: &Editor, label: &str, r: [f32; 4], f: F, value: &str, hint: &str, mouse: [f32; 2], blink: bool) {
    draw.text(label, [r[0] - draw.text_width(label) - 3.0, r[1] + 3.0], 0xFFA0_A0A0, false);
    hint_field(draw, b, r, value, hint, e.focus == Some(f), Do::Field(f), mouse, blink);
}

fn cond_name(c: &CondEd) -> String {
    CONDS.get(c.kind).map_or("?".into(), |k| k.0.to_string())
}

fn draw_panel(draw: &mut dyn DrawContext, b: &mut Vec<([f32; 4], Do)>, e: &Editor, at: [f32; 2], mouse: [f32; 2], blink: bool) {
    let [x, mut y] = at;
    let h = ROW - 2.0;
    let wide = PANEL;
    let Some(n) = e.ed.nodes.get(e.selected) else { return };
    let tabs: &[&str] = if n.check { &["Checks"] } else { &["Says", "Answers"] };
    let tab = if n.check { 0 } else { e.tab.min(1) };
    for (t, name) in tabs.iter().enumerate() {
        let label = if tab == t as u8 { format!("[{name}]") } else { name.to_string() };
        button(draw, b, [x + t as f32 * 62.0, y, 58.0, h], &label, Do::Tab(t as u8), mouse);
    }
    if e.ed.start == e.selected {
        draw.text("start", [x + wide - 28.0, y + 3.0], 0xFF9A_E6A0, false);
    } else {
        button(draw, b, [x + wide - 50.0, y, 50.0, h], "Make start", Do::Start, mouse);
    }
    y += ROW + 4.0;
    labelled(draw, b, e, "node", [x + 26.0, y, 90.0, h], F::Name, &n.name, "", mouse, blink);
    if n.check {
        draw.text("first that holds wins", [x + 120.0, y + 3.0], 0xFF70_7080, false);
        y += ROW + 2.0;
        draw.text("if                      leads to", [x, y], 0xFF70_7080, false);
        y += 10.0;
        for (i, c) in n.checks.iter().enumerate().take(MAX_CHECKS) {
            picker(draw, b, [x, y, 62.0, h], &cond_name(&c.cond), Do::CCond(i), mouse);
            let hint = CONDS.get(c.cond.kind).map_or("", |k| k.1);
            hint_field(draw, b, [x + 64.0, y, 78.0, h], &c.cond.value, hint, e.focus == Some(F::CValue(i)), Do::Field(F::CValue(i)), mouse, blink);
            button(draw, b, [x + 144.0, y, 14.0, h], if c.cond.not { "!" } else { "=" }, Do::CNot(i), mouse);
            picker(draw, b, [x + 160.0, y, 62.0, h], &goto_name(e, c.goto), Do::CGoto(i), mouse);
            button(draw, b, [x + wide - 12.0, y, 12.0, h], "x", Do::DelCheck(i), mouse);
            y += ROW;
        }
        if n.checks.len() < MAX_CHECKS {
            button(draw, b, [x, y, 56.0, h], "+ Check", Do::AddCheck, mouse);
        }
        y += ROW + 2.0;
        draw.text("else to", [x, y + 3.0], 0xFFA0_A0A0, false);
        picker(draw, b, [x + 40.0, y, 90.0, h], &goto_name(e, n.next), Do::Next, mouse);
        y += ROW + 2.0;
        draw.text("= holds, ! does not. answer: node=b", [x, y], 0xFF70_7080, false);
        return;
    }
    if tab == 0 {
        y += ROW;
        let mode = if n.paged { "a page each" } else { "all at once" };
        draw.text("lines:", [x, y + 3.0], 0xFFA0_A0A0, false);
        button(draw, b, [x + 30.0, y, 74.0, h], mode, Do::Paged, mouse);
        if n.lines.len() < MAX_LINES {
            button(draw, b, [x + 108.0, y, 50.0, h], "+ Line", Do::AddLine, mouse);
        }
        y += ROW;
        for (i, l) in n.lines.iter().enumerate().take(MAX_LINES) {
            draw.text(&format!("{}", i + 1), [x, y + 3.0], 0xFF70_7080, false);
            hint_field(draw, b, [x + 10.0, y, wide - 24.0, h], l, "what the NPC says (@player)", e.focus == Some(F::Line(i)), Do::Field(F::Line(i)), mouse, blink);
            button(draw, b, [x + wide - 12.0, y, 12.0, h], "x", Do::DelLine(i), mouse);
            y += ROW;
        }
        y += 2.0;
        labelled(draw, b, e, "plays", [x + 30.0, y, 100.0, h], F::Anim, &n.animation, "animation.felix.wave", mouse, blink);
        labelled(draw, b, e, "button", [x + 170.0, y, wide - 170.0, h], F::More, &n.more, "...", mouse, blink);
        y += ROW;
        labelled(draw, b, e, "does", [x + 30.0, y, wide - 30.0, h], F::Act, &n.act, "remember met; give minecraft:apple 2", mouse, blink);
        y += ROW;
        let then = if n.answers.is_empty() { "then" } else { "(answers)" };
        draw.text(then, [x, y + 3.0], 0xFFA0_A0A0, false);
        if n.answers.is_empty() {
            picker(draw, b, [x + 30.0, y, 90.0, h], &goto_name(e, n.next), Do::Next, mouse);
        } else {
            draw.text("its answers say where it goes", [x + 46.0, y + 3.0], 0xFF70_7080, false);
        }
        return;
    }
    // Answers: words; when shown; what they do unseen, and where they lead.
    y += AROW + 1.0;
    let ah = AROW - 1.0;
    for (i, a) in n.answers.iter().enumerate().take(MAX_ANSWERS) {
        draw.text(&format!("{})", crate::dialog::letter(i)), [x, y + 2.0], 0xFFFF_D86A, false);
        hint_field(draw, b, [x + 12.0, y, wide - 26.0, ah], &a.text, "what the player says", e.focus == Some(F::AText(i)), Do::Field(F::AText(i)), mouse, blink);
        button(draw, b, [x + wide - 12.0, y, 12.0, ah], "x", Do::DelAnswer(i), mouse);
        y += AROW;
        draw.text("if", [x + 2.0, y + 2.0], 0xFF70_7080, false);
        picker(draw, b, [x + 12.0, y, 62.0, ah], &cond_name(&a.cond), Do::ACond(i), mouse);
        let hint = CONDS.get(a.cond.kind).map_or("", |k| k.1);
        if a.cond.kind != 0 {
            hint_field(draw, b, [x + 76.0, y, wide - 92.0, ah], &a.cond.value, hint, e.focus == Some(F::AValue(i)), Do::Field(F::AValue(i)), mouse, blink);
            button(draw, b, [x + wide - 14.0, y, 14.0, ah], if a.cond.not { "!" } else { "=" }, Do::ANot(i), mouse);
        }
        y += AROW;
        hint_field(draw, b, [x + 12.0, y, 24.0, ah], &a.relation, "rel", e.focus == Some(F::ARel(i)), Do::Field(F::ARel(i)), mouse, blink);
        hint_field(draw, b, [x + 38.0, y, 40.0, ah], &a.feel, "feel", e.focus == Some(F::AFeel(i)), Do::Field(F::AFeel(i)), mouse, blink);
        hint_field(draw, b, [x + 80.0, y, wide - 144.0, ah], &a.act, "does", e.focus == Some(F::AAct(i)), Do::Field(F::AAct(i)), mouse, blink);
        picker(draw, b, [x + wide - 62.0, y, 62.0, ah], &goto_name(e, a.goto), Do::AGoto(i), mouse);
        y += AROW + 2.0;
    }
    if n.answers.len() < MAX_ANSWERS {
        button(draw, b, [x, y, 62.0, h], "+ Answer", Do::AddAnswer, mouse);
        draw.text("rel: +5 / -5, hidden", [x + 68.0, y + 3.0], 0xFF70_7080, false);
    }
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
    let title = if e.id.is_empty() { "New dialog".to_string() } else { format!("Dialog {}", e.id) };
    draw.text(&title, [x0, y0], 0xFFFF_D86A, false);
    let c0 = [x0, y0 + 12.0];
    e.canvas = c0;
    draw_canvas(draw, e, c0);
    let mut b = Vec::new();
    draw_panel(draw, &mut b, e, [x0 + CANVAS[0] + 8.0, y0 + 12.0], mouse, blink);
    let y = y0 + H - 26.0;
    draw.text("save as", [x0, y + 3.0], 0xFFA0_A0A0, false);
    let saving = e.save_as.clone();
    field(draw, &mut b, [x0 + 42.0, y, 76.0, 14.0], &saving, e.focus == Some(F::SaveAs), Do::Field(F::SaveAs), mouse, blink);
    button(draw, &mut b, [x0 + 122.0, y, 36.0, 14.0], "Save", Do::Save, mouse);
    button(draw, &mut b, [x0 + 166.0, y, 50.0, 14.0], "+ Talk", Do::AddTalk, mouse);
    button(draw, &mut b, [x0 + 220.0, y, 54.0, 14.0], "+ Check", Do::AddCheckNode, mouse);
    button(draw, &mut b, [x0 + 278.0, y, 50.0, 14.0], "- Node", Do::DelNode, mouse);
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
        return Flow::Cancel;
    }
    let p = [mouse[0] - e.canvas[0], mouse[1] - e.canvas[1]];
    match e.ed.node_at(p) {
        Some(i) => {
            if e.selected != i {
                e.tab = 0;
            }
            e.selected = i;
            e.focus = None;
            let at = e.ed.nodes[i].at;
            e.drag = Some([p[0] - at[0], p[1] - at[1]]);
        }
        None => e.focus = None,
    }
    Flow::Cancel
}

pub fn mouse_dragged(host: &dyn ClientHost, ev: &mut ScreenMouseDraggedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(e) = lock.as_mut() else { return Flow::Continue };
    if let Some(off) = e.drag {
        let p = [ev.screen.mouse[0] - e.canvas[0] - off[0], ev.screen.mouse[1] - e.canvas[1] - off[1]];
        if let Some(n) = e.ed.nodes.get_mut(e.selected) {
            n.at = [p[0].clamp(0.0, CANVAS[0] - BOX[0]).round(), p[1].clamp(0.0, CANVAS[1] - BOX[1] - 10.0).round()];
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
        e.drag = None;
    }
    Flow::Cancel
}

pub fn mouse_scrolled(host: &dyn ClientHost, mouse: [f32; 2], up: bool) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(e) = lock.as_mut() else { return Flow::Continue };
    if let Some(what) = e.buttons.iter().find(|(r, w)| inside(*r, mouse) && matches!(w, Do::Next | Do::ACond(_) | Do::AGoto(_) | Do::CCond(_) | Do::CGoto(_))).map(|(_, w)| *w) {
        act(e, what, if up { -1 } else { 1 });
    }
    Flow::Cancel
}

fn type_text(e: &mut Editor, text: &str) {
    let Some(f) = e.focus else { return };
    let Some(s) = e.text(f) else { return };
    for ch in text.chars().filter(|ch| !ch.is_control()) {
        if s.chars().count() >= 300 {
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
        "key.keyboard.s" if ctrl => e.save(),
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
        } else if !e.shown && host.mod_screen() == Some(SCREEN) {
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
