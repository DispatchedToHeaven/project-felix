//! The character editor, in the world (`/felix character edit <id>`): the moods as boxes on a
//! canvas, dragged where they read best, with lines for the ways between them; beside it, the mood
//! picked (what it does, says and plays), its ways (on what event, to which mood, if the relation
//! is below or above what), and the whole character (its start, relations, forgiving, bonds).
//! Save sends it to the server, which keeps it as `felixchar:<name>` for NPCs to have.
use crate::char_edit::{BOX, CANVAS, CharEd, WayEd};
use crate::creator::{button, field, head, hint_field, inside, picker};
use crate::serde_json::{Value, json};
use mod_api::{
    ClientHost, DrawContext, Flow, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId, Registrar,
    ScreenCharTypedPreEvent, ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenMouseDraggedPreEvent,
    ScreenRenderPostEvent, SoundPlay,
};
use std::sync::{Arc, LazyLock, Mutex};

/// Server to client: open the editor (JSON: id, name, file).
pub const OPEN: &str = "projectfelix:char_open";
/// Client to server: a character to keep (JSON: name, file).
pub const SAVE: &str = "projectfelix:char_save";
/// Server to client: the characters made in the game, for the NPC maker's picker.
pub const LIST: &str = "projectfelix:char_list";

const SCREEN: &str = "projectfelix";
const W: f32 = 404.0;
const H: f32 = 226.0;
const ROW: f32 = 14.0;
const MAX_WAYS: usize = 9;
const MAX_RELATIONS: usize = 6;

#[derive(Clone, Copy, PartialEq, Debug)]
enum F {
    Name,
    Obey,
    Speed,
    Delay,
    Say,
    Refuse,
    Grumble,
    Chatter,
    Animation,
    Particle,
    After,
    On(usize),
    Cond(usize),
    FriendAt,
    HateAt,
    Owner,
    Forgive,
    Bond,
    BondMax,
    Shun,
    RelEvent(usize),
    RelDelta(usize),
    SaveAs,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Do {
    Field(F),
    Tab(u8),
    Start,
    AfterTo,
    WayTo(usize),
    DelWay(usize),
    AddWay,
    StartPick,
    DelRel(usize),
    AddRel,
    AddMood,
    DelMood,
    Save,
    Close,
}

struct Editor {
    id: String,
    ed: CharEd,
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
static KNOWN: LazyLock<Mutex<Vec<String>>> = LazyLock::new(Default::default);
/// The dialogs and rules made in the game, as the server last said.
static KNOWN_SETS: LazyLock<Mutex<(Vec<String>, Vec<String>)>> = LazyLock::new(Default::default);

/// The dialogs and rules made in the game (for the maker's pickers).
pub fn known_sets() -> (Vec<String>, Vec<String>) {
    KNOWN_SETS.lock().map(|k| k.clone()).unwrap_or_default()
}

pub fn active() -> bool {
    EDITOR.lock().is_ok_and(|e| e.is_some())
}

/// The characters made in the game, as the server last said.
pub fn known() -> Vec<String> {
    KNOWN.lock().map(|k| k.clone()).unwrap_or_default()
}

pub fn register(r: &mut dyn Registrar) {
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_client(PayloadId(OPEN.into()), Some(Arc::new(OpenHandler)));
    reg.play_to_client(PayloadId(LIST.into()), Some(Arc::new(ListHandler)));
    reg.play_to_server(PayloadId(SAVE.into()), Arc::new(crate::heart::SaveCharacter));
}

struct ListHandler;

impl IPayloadHandler for ListHandler {
    fn handle(&self, payload: PayloadBytes, _context: &mut dyn IPayloadContext) -> Result<(), String> {
        let v = crate::serde_json::from_slice::<Value>(&payload.0).unwrap_or(Value::Null);
        let strings = |v: Option<&Value>| -> Vec<String> { v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default() };
        let list = if v.is_array() { strings(Some(&v)) } else { strings(v.get("characters")) };
        if let Ok(mut k) = KNOWN.lock() {
            *k = list;
        }
        if let Ok(mut k) = KNOWN_SETS.lock() {
            *k = (strings(v.get("dialogs")), strings(v.get("rules")));
        }
        Ok(())
    }
}

struct OpenHandler;

impl IPayloadHandler for OpenHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(host) = context.client_host() else { return Ok(()) };
        let Ok(v) = crate::serde_json::from_slice::<Value>(&payload.0) else { return Ok(()) };
        let id = v.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let name = v.get("name").and_then(Value::as_str).unwrap_or("mine").to_string();
        let ed = v.get("file").map(CharEd::from_json).unwrap_or_else(CharEd::blank);
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
            status: "Drag the moods; pick one to change it. Save keeps it as felixchar:<name>.".into(),
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
        let ed = &mut self.ed;
        match f {
            F::FriendAt => return Some(&mut ed.friend_at),
            F::HateAt => return Some(&mut ed.hate_at),
            F::Owner => return Some(&mut ed.owner),
            F::Forgive => return Some(&mut ed.forgive),
            F::Bond => return Some(&mut ed.bond),
            F::BondMax => return Some(&mut ed.bond_max),
            F::Shun => return Some(&mut ed.shun),
            F::RelEvent(i) => return ed.relations.get_mut(i).map(|r| &mut r.0),
            F::RelDelta(i) => return ed.relations.get_mut(i).map(|r| &mut r.1),
            _ => {}
        }
        let m = ed.moods.get_mut(self.selected)?;
        Some(match f {
            F::Name => &mut m.name,
            F::Obey => &mut m.obey,
            F::Speed => &mut m.speed,
            F::Delay => &mut m.delay,
            F::Say => &mut m.say,
            F::Refuse => &mut m.refuse,
            F::Grumble => &mut m.grumble,
            F::Chatter => &mut m.chatter,
            F::Animation => &mut m.animation,
            F::Particle => &mut m.particle,
            F::After => &mut m.after,
            F::On(i) => &mut m.ways.get_mut(i)?.on,
            F::Cond(i) => &mut m.ways.get_mut(i)?.cond,
            _ => return None,
        })
    }

    /// The fields of the tab shown, in Tab order.
    fn order(&self) -> Vec<F> {
        let mut o = match self.tab {
            0 => vec![F::Name, F::Obey, F::Speed, F::Delay, F::Say, F::Refuse, F::Grumble, F::Chatter, F::Animation, F::Particle],
            1 => {
                let mut o = vec![F::After];
                let n = self.ed.moods.get(self.selected).map_or(0, |m| m.ways.len().min(MAX_WAYS));
                for i in 0..n {
                    o.extend([F::On(i), F::Cond(i)]);
                }
                o
            }
            _ => {
                let mut o = vec![F::FriendAt, F::HateAt, F::Owner, F::Forgive, F::Bond, F::BondMax, F::Shun];
                for i in 0..self.ed.relations.len().min(MAX_RELATIONS) {
                    o.extend([F::RelEvent(i), F::RelDelta(i)]);
                }
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
                self.status = format!("Sent: felixchar:{name}");
            }
            Err(e) => self.status = e,
        }
    }
}

fn cycle(i: usize, by: i32, n: usize) -> usize {
    if n == 0 { 0 } else { (i as i64 + by as i64).rem_euclid(n as i64) as usize }
}

fn act(e: &mut Editor, what: Do, by: i32) {
    let n = e.ed.moods.len();
    match what {
        Do::Field(f) => e.focus = Some(f),
        Do::Tab(t) => {
            e.tab = t;
            e.focus = None;
        }
        Do::Start => e.ed.start = e.selected,
        Do::StartPick => e.ed.start = cycle(e.ed.start, by, n),
        Do::AfterTo => {
            if let Some(m) = e.ed.moods.get_mut(e.selected) {
                m.after_to = cycle(m.after_to, by, n);
            }
        }
        Do::WayTo(i) => {
            if let Some(w) = e.ed.moods.get_mut(e.selected).and_then(|m| m.ways.get_mut(i)) {
                w.to = cycle(w.to, by, n);
            }
        }
        Do::DelWay(i) => {
            if let Some(m) = e.ed.moods.get_mut(e.selected)
                && i < m.ways.len()
            {
                m.ways.remove(i);
                e.focus = None;
            }
        }
        Do::AddWay => {
            let sel = e.selected;
            if let Some(m) = e.ed.moods.get_mut(sel)
                && m.ways.len() < MAX_WAYS
            {
                let to = if n > 1 { (sel + 1) % n } else { 0 };
                m.ways.push(WayEd { on: "gift".into(), to, cond: String::new() });
                e.focus = Some(F::On(m.ways.len() - 1));
            }
        }
        Do::DelRel(i) => {
            if i < e.ed.relations.len() {
                e.ed.relations.remove(i);
                e.focus = None;
            }
        }
        Do::AddRel => {
            if e.ed.relations.len() < MAX_RELATIONS {
                e.ed.relations.push(("praise".into(), "3".into()));
                e.focus = Some(F::RelEvent(e.ed.relations.len() - 1));
            }
        }
        Do::AddMood => {
            e.selected = e.ed.add_mood();
            e.tab = 0;
            e.focus = Some(F::Name);
        }
        Do::DelMood => {
            if n > 1 {
                e.ed.remove_mood(e.selected);
                e.selected = e.selected.min(e.ed.moods.len() - 1);
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

fn draw_canvas(draw: &mut dyn DrawContext, e: &Editor, c0: [f32; 2]) {
    draw.fill([c0[0], c0[1], CANVAS[0], CANVAS[1]], 0xFF18_1C24);
    let sel = e.selected;
    // The ways: grey; from the picked mood yellow, to it blue, its labels written.
    for pass in 0..2 {
        for (i, m) in e.ed.moods.iter().enumerate() {
            let mut ways: Vec<(String, usize, bool)> = m.ways.iter().map(|w| (w.on.clone(), w.to, false)).collect();
            if !m.after.trim().is_empty() {
                ways.push((format!("after {}s", m.after.trim()), m.after_to, true));
            }
            let mut seen: Vec<usize> = Vec::new();
            for (label, to, timed) in ways {
                let Some(t) = e.ed.moods.get(to) else { continue };
                if to == i {
                    continue;
                }
                let lit = i == sel || to == sel;
                if (pass == 0) == lit {
                    continue;
                }
                let (a, b) = (centre(m.at, c0), centre(t.at, c0));
                let colour = if i == sel {
                    if timed { 0xFF9A_E6A0 } else { 0xFFFF_D86A }
                } else if to == sel {
                    0xFF6A_B0FF
                } else {
                    0x6090_90A0
                };
                line(draw, a, b, colour);
                // An arrowhead near the box it goes to.
                let p = [a[0] + (b[0] - a[0]) * 0.72, a[1] + (b[1] - a[1]) * 0.72];
                draw.fill([p[0] - 1.5, p[1] - 1.5, 3.0, 3.0], colour);
                if i == sel {
                    let k = seen.iter().filter(|x| **x == to).count();
                    seen.push(to);
                    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0 - 4.0 + k as f32 * 9.0];
                    let w = draw.text_width(&label);
                    draw.fill([mid[0] - w / 2.0 - 1.0, mid[1] - 1.0, w + 2.0, 10.0], 0xC010_1018);
                    draw.text(&label, [mid[0] - w / 2.0, mid[1]], colour, false);
                }
            }
        }
    }
    for (i, m) in e.ed.moods.iter().enumerate() {
        let r = [c0[0] + m.at[0], c0[1] + m.at[1], BOX[0], BOX[1]];
        let fill = if i == sel { 0xF050_4020 } else { 0xE030_3448 };
        draw.fill(r, fill);
        draw.fill([r[0], r[1] + r[3] - 1.0, r[2], 1.0], if i == e.ed.start { 0xFF9A_E6A0 } else { 0xFF60_6070 });
        let name = if i == e.ed.start { format!("*{}", m.name) } else { m.name.clone() };
        let shown = head(draw, &name, BOX[0] - 4.0);
        draw.text(&shown, [r[0] + 2.0, r[1] + 3.0], 0xFFFF_FFFF, false);
    }
}

#[allow(clippy::too_many_arguments)]
fn labelled(draw: &mut dyn DrawContext, b: &mut Vec<([f32; 4], Do)>, e: &Editor, label: &str, r: [f32; 4], f: F, value: &str, hint: &str, mouse: [f32; 2], blink: bool) {
    draw.text(label, [r[0] - draw.text_width(label) - 3.0, r[1] + 3.0], 0xFFA0_A0A0, false);
    hint_field(draw, b, r, value, hint, e.focus == Some(f), Do::Field(f), mouse, blink);
}

fn draw_panel(draw: &mut dyn DrawContext, b: &mut Vec<([f32; 4], Do)>, e: &Editor, at: [f32; 2], mouse: [f32; 2], blink: bool) {
    let [x, mut y] = at;
    let h = ROW - 2.0;
    let wide = 178.0;
    for (t, name) in ["Mood", "Ways", "All"].iter().enumerate() {
        let label = if e.tab == t as u8 { format!("[{name}]") } else { name.to_string() };
        button(draw, b, [x + t as f32 * 60.0, y, 56.0, h], &label, Do::Tab(t as u8), mouse);
    }
    y += ROW + 4.0;
    let names: Vec<String> = e.ed.moods.iter().map(|m| m.name.clone()).collect();
    let name_of = |i: usize| names.get(i).cloned().unwrap_or_default();
    let Some(m) = e.ed.moods.get(e.selected) else { return };
    match e.tab {
        0 => {
            let fx = x + 44.0;
            let fw = wide - 44.0;
            labelled(draw, b, e, "name", [fx, y, fw - 50.0, h], F::Name, &m.name, "", mouse, blink);
            if e.ed.start == e.selected {
                draw.text("start", [x + wide - 44.0, y + 3.0], 0xFF9A_E6A0, false);
            } else {
                button(draw, b, [x + wide - 46.0, y, 46.0, h], "Start", Do::Start, mouse);
            }
            y += ROW;
            labelled(draw, b, e, "obeys", [fx, y, 26.0, h], F::Obey, &m.obey, "1", mouse, blink);
            labelled(draw, b, e, "speed", [fx + 60.0, y, 26.0, h], F::Speed, &m.speed, "1", mouse, blink);
            labelled(draw, b, e, "wait", [fx + 108.0, y, 26.0, h], F::Delay, &m.delay, "0", mouse, blink);
            y += ROW;
            for (label, f, v, hint) in [
                ("says", F::Say, &m.say, "on coming in: a | b"),
                ("refuses", F::Refuse, &m.refuse, "when it won't obey"),
                ("grumbles", F::Grumble, &m.grumble, "when it obeys slowly"),
                ("chats", F::Chatter, &m.chatter, "to friends: @listener"),
                ("plays", F::Animation, &m.animation, "animation.felix.sad"),
                ("shows", F::Particle, &m.particle, "minecraft:heart"),
            ] {
                labelled(draw, b, e, label, [fx, y, fw, h], f, v, hint, mouse, blink);
                y += ROW;
            }
            draw.text("obeys 0-1, speed x, wait s", [x, y + 2.0], 0xFF70_7080, false);
        }
        1 => {
            labelled(draw, b, e, "after", [x + 30.0, y, 30.0, h], F::After, &m.after, "s", mouse, blink);
            draw.text("s to", [x + 63.0, y + 3.0], 0xFFA0_A0A0, false);
            picker(draw, b, [x + 86.0, y, wide - 86.0, h], &name_of(m.after_to), Do::AfterTo, mouse);
            y += ROW + 2.0;
            draw.text("on event      to mood     if rel.", [x, y], 0xFF70_7080, false);
            y += 10.0;
            for (i, w) in m.ways.iter().enumerate().take(MAX_WAYS) {
                hint_field(draw, b, [x, y, 64.0, h], &w.on, "event", e.focus == Some(F::On(i)), Do::Field(F::On(i)), mouse, blink);
                picker(draw, b, [x + 66.0, y, 64.0, h], &name_of(w.to), Do::WayTo(i), mouse);
                hint_field(draw, b, [x + 132.0, y, 30.0, h], &w.cond, "<n", e.focus == Some(F::Cond(i)), Do::Field(F::Cond(i)), mouse, blink);
                button(draw, b, [x + wide - 12.0, y, 12.0, h], "x", Do::DelWay(i), mouse);
                y += ROW;
            }
            if m.ways.len() < MAX_WAYS {
                button(draw, b, [x, y, 56.0, h], "+ Way", Do::AddWay, mouse);
            }
        }
        _ => {
            draw.text("start", [x, y + 3.0], 0xFFA0_A0A0, false);
            picker(draw, b, [x + 44.0, y, wide - 44.0, h], &name_of(e.ed.start), Do::StartPick, mouse);
            y += ROW;
            labelled(draw, b, e, "friend", [x + 32.0, y, 26.0, h], F::FriendAt, &e.ed.friend_at, "30", mouse, blink);
            labelled(draw, b, e, "hate", [x + 86.0, y, 26.0, h], F::HateAt, &e.ed.hate_at, "-50", mouse, blink);
            labelled(draw, b, e, "owner", [x + 148.0, y, 30.0, h], F::Owner, &e.ed.owner, "50", mouse, blink);
            y += ROW;
            labelled(draw, b, e, "forgive", [x + 38.0, y, 22.0, h], F::Forgive, &e.ed.forgive, "2", mouse, blink);
            labelled(draw, b, e, "bond", [x + 88.0, y, 22.0, h], F::Bond, &e.ed.bond, "1", mouse, blink);
            labelled(draw, b, e, "max", [x + 136.0, y, 42.0, h], F::BondMax, &e.ed.bond_max, "60", mouse, blink);
            y += ROW;
            labelled(draw, b, e, "shuns", [x + 32.0, y, wide - 32.0, h], F::Shun, &e.ed.shun, "to one it hates: a | b", mouse, blink);
            y += ROW + 2.0;
            draw.text("event moves the relation by", [x, y], 0xFF70_7080, false);
            y += 10.0;
            for (i, (event, delta)) in e.ed.relations.iter().enumerate().take(MAX_RELATIONS) {
                hint_field(draw, b, [x, y, 110.0, h], event, "event", e.focus == Some(F::RelEvent(i)), Do::Field(F::RelEvent(i)), mouse, blink);
                hint_field(draw, b, [x + 114.0, y, 46.0, h], delta, "+/-", e.focus == Some(F::RelDelta(i)), Do::Field(F::RelDelta(i)), mouse, blink);
                button(draw, b, [x + wide - 12.0, y, 12.0, h], "x", Do::DelRel(i), mouse);
                y += ROW;
            }
            if e.ed.relations.len() < MAX_RELATIONS {
                button(draw, b, [x, y, 70.0, h], "+ Relation", Do::AddRel, mouse);
            }
        }
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
    let title = if e.id.is_empty() { "New character".to_string() } else { format!("Character {}", e.id) };
    draw.text(&title, [x0, y0], 0xFFFF_D86A, false);
    let c0 = [x0, y0 + 12.0];
    e.canvas = c0;
    draw_canvas(draw, e, c0);
    let mut b = Vec::new();
    draw_panel(draw, &mut b, e, [x0 + 226.0, y0 + 12.0], mouse, blink);
    let y = y0 + H - 30.0;
    draw.text("save as", [x0, y + 3.0], 0xFFA0_A0A0, false);
    let saving = e.save_as.clone();
    field(draw, &mut b, [x0 + 38.0, y, 96.0, 14.0], &saving, e.focus == Some(F::SaveAs), Do::Field(F::SaveAs), mouse, blink);
    button(draw, &mut b, [x0 + 138.0, y, 44.0, 14.0], "Save", Do::Save, mouse);
    button(draw, &mut b, [x0 + 190.0, y, 52.0, 14.0], "+ Mood", Do::AddMood, mouse);
    button(draw, &mut b, [x0 + 246.0, y, 52.0, 14.0], "- Mood", Do::DelMood, mouse);
    button(draw, &mut b, [x0 + W - 50.0, y, 50.0, 14.0], "Close", Do::Close, mouse);
    let status = crate::creator::tail(draw, &e.status, W);
    draw.text(&status, [x0, y + 18.0], 0xFFC0_C0C8, false);
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
    match e.ed.mood_at(p) {
        Some(i) => {
            e.selected = i;
            e.focus = None;
            let at = e.ed.moods[i].at;
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
        if let Some(m) = e.ed.moods.get_mut(e.selected) {
            m.at = [p[0].clamp(0.0, CANVAS[0] - BOX[0]).round(), p[1].clamp(0.0, CANVAS[1] - BOX[1]).round()];
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
    if let Some(what) = e.buttons.iter().find(|(r, w)| inside(*r, mouse) && matches!(w, Do::AfterTo | Do::WayTo(_) | Do::StartPick)).map(|(_, w)| *w) {
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
