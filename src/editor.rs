//! The animation editor, in the world: the NPC in front of the player plays what is being made,
//! the mod's screen lays its panels round the edges (bones, model and texture on the left; values
//! and effects on the right; the timeline below; the toolbar above) and leaves the middle clear.
//!
//! Values are as Blockbench shows them: rotation in degrees, position in texels, scale. Changing
//! a value sets a keyframe at the current time. Sounds and particles go on the timeline at the
//! current time, and play in the preview. Space plays, K sets a keyframe, Delete removes it, the
//! arrows step through time and bones, Ctrl+Z undoes, Ctrl+S saves. Saved files go to
//! `horizon-mod-data/projectfelix/animations/<name>.animation.json`, open in Blockbench, and play
//! in the game at once (`/felix animate`). Models of the player's own go in
//! `horizon-mod-data/projectfelix/models/<name>.geo.json` (the model `user:<name>`).
use crate::anim::{self, Animation, EffectKind, LoopMode};
use crate::anim_edit::{EditAnim, SNAP, snap};
use crate::companion::{DEFAULT_MODEL, Look};
use crate::{animations, models, runtime};
use mod_api::{
    ClientHost, ConfigPath, DrawContext, Flow, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId,
    ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenMouseButtonReleasedPreEvent,
    ScreenMouseDraggedPreEvent, ScreenMouseScrolledPreEvent, ScreenRenderPostEvent, SoundPlay,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

const SCREEN: &str = "projectfelix";
const FOLDER: &str = "animations";
const MODELS: &str = "models";
/// Client to server: put the editor's model and texture on the NPC (JSON: npc, model, texture).
pub const SET_LOOK: &str = "projectfelix:set_look";
const BAR: f32 = 16.0;
const LEFT_W: f32 = 84.0;
const RIGHT_W: f32 = 96.0;
const TIMELINE_H: f32 = 66.0;
const ROW: f32 = 10.0;
const UNDO: usize = 60;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Do {
    Play,
    Shorter,
    Longer,
    Mode,
    Save,
    Close,
    Undo,
    Turn(f32),
    Bone(usize),
    Channel(usize),
    Nudge(usize, f32),
    Value(usize),
    Key,
    Delete,
    Ease,
    Timeline,
    Sound,
    Particle,
    Locator,
    AddSound,
    AddParticle,
    DelEffects,
    Model,
    Texture,
    UseOnNpc,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Value { axis: usize, x0: f32, v0: [f32; 3] },
    Time,
}

struct Editor {
    npc: u32,
    anim: EditAnim,
    /// `anim` built for playing, after each change.
    built: Arc<Animation>,
    undo: Vec<EditAnim>,
    model: String,
    texture: String,
    bones: Vec<String>,
    locators: Vec<String>,
    bone: usize,
    channel: usize,
    time: f32,
    /// Playing since this clock (ticks), from `time`.
    playing: Option<(f64, f32)>,
    /// Effects fired up to this time while playing.
    fired: Option<f32>,
    turn: f32,
    scroll: usize,
    sounds: Vec<String>,
    sound: usize,
    particle: usize,
    locator: usize,
    buttons: Vec<([f32; 4], Do)>,
    drag: Option<Drag>,
    status: String,
    shown: bool,
    close: bool,
    timeline: [f32; 4],
    outbox: Vec<(&'static str, Vec<u8>)>,
}

static EDITOR: LazyLock<Mutex<Option<Editor>>> = LazyLock::new(Default::default);
/// Animations being edited, by name, kept when the editor closes.
static DRAFTS: LazyLock<Mutex<HashMap<String, EditAnim>>> = LazyLock::new(Default::default);

pub fn active() -> bool {
    EDITOR.lock().is_ok_and(|e| e.is_some())
}

/// What the NPC being edited shows.
pub struct Preview {
    pub anim: Arc<Animation>,
    pub time: f32,
    pub turn: f32,
    pub model: String,
    pub texture: String,
}

/// The preview for NPC `id`, while the editor is open on it.
pub fn preview(id: u32, clock: f64) -> Option<Preview> {
    let e = EDITOR.lock().ok()?;
    let e = e.as_ref().filter(|e| e.npc == id)?;
    Some(Preview { anim: e.built.clone(), time: e.now(clock), turn: e.turn, model: e.model.clone(), texture: e.texture.clone() })
}

impl Editor {
    fn now(&self, clock: f64) -> f32 {
        match self.playing {
            Some((since, from)) => {
                let t = from + ((clock - since) / 20.0) as f32;
                if self.anim.length > 0.0 { t.rem_euclid(self.anim.length) } else { 0.0 }
            }
            None => self.time,
        }
    }
    fn bone_name(&self) -> String {
        self.bones.get(self.bone).cloned().unwrap_or_default()
    }
    fn values(&self) -> [f32; 3] {
        self.anim.value_at(&self.bone_name(), self.channel, self.time)
    }
    fn set_values(&mut self, v: [f32; 3]) {
        let bone = self.bone_name();
        self.anim.set_key(&bone, self.channel, self.time, v);
    }
    fn step(&self) -> f32 {
        match self.channel {
            0 => 5.0,
            1 => 0.5,
            _ => 0.05,
        }
    }
    /// Keeps the animation as it is now, for Ctrl+Z.
    fn remember(&mut self) {
        self.undo.push(self.anim.clone());
        if self.undo.len() > UNDO {
            self.undo.remove(0);
        }
    }
    fn rebuild(&mut self) {
        self.built = Arc::new(self.anim.to_animation());
    }
    fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
        let m = models::get(model);
        self.bones = m.as_ref().map(|m| m.geo.bones.iter().map(|b| b.name.clone()).collect()).unwrap_or_default();
        self.locators = m.map(|m| m.geo.bones.iter().flat_map(|b| b.locators.iter().map(|l| l.0.clone())).collect()).unwrap_or_default();
        self.bone = self.bone.min(self.bones.len().saturating_sub(1));
        self.locator = 0;
    }
    fn locator_name(&self) -> Option<String> {
        // 0 is the NPC's feet; locators follow.
        self.locator.checked_sub(1).and_then(|i| self.locators.get(i).cloned())
    }
}

pub(crate) fn read_folder(host: &dyn ClientHost, folder: &str, mut each: impl FnMut(&str, Vec<u8>)) {
    let files = host.data_files();
    let Ok(dir) = ConfigPath::new(folder) else { return };
    let mut after: Option<String> = None;
    loop {
        let Ok(page) = files.list(&dir, after.as_deref(), 256) else { break };
        if page.is_empty() {
            break;
        }
        for entry in &page {
            if let Ok(path) = ConfigPath::new(&format!("{folder}/{}", entry.name))
                && let Ok(bytes) = files.read(&path)
            {
                each(&entry.name, bytes);
            }
        }
        after = page.last().map(|e| e.name.clone());
    }
}

/// The player's saved animations and own models, read once a session: they play and draw in the
/// game like any other.
pub fn load_saved(host: &dyn ClientHost) {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| {
        read_folder(host, FOLDER, |name, bytes| match anim::parse(&bytes) {
            Ok(list) => list.into_iter().for_each(animations::insert_saved),
            Err(e) => eprintln!("[projectfelix] saved animation {name} not loaded: {e}"),
        });
        read_folder(host, MODELS, |name, bytes| {
            let stem = name.strip_suffix(".geo.json").or_else(|| name.strip_suffix(".json")).unwrap_or(name);
            match models::load(&bytes) {
                Ok(m) => models::insert_saved(stem, m),
                Err(e) => eprintln!("[projectfelix] model {name} not loaded: {e}"),
            }
        });
    });
}

/// `projectfelix:edit` from the server: open the editor on an NPC with an animation name.
pub struct OpenHandler;

impl IPayloadHandler for OpenHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(v) = crate::serde_json::from_slice::<crate::serde_json::Value>(&payload.0) else { return Ok(()) };
        let npc = v.get("npc").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("animation.felix.custom").to_string();
        let Ok(host) = context.client_host() else { return Ok(()) };
        load_saved(&*host);
        let look = runtime::look(npc).unwrap_or_else(|| Look::of_model(DEFAULT_MODEL));
        let anim = DRAFTS
            .lock()
            .ok()
            .and_then(|d| d.get(&name).cloned())
            .or_else(|| animations::get(&name).map(|a| EditAnim::from_animation(&a)))
            .unwrap_or_else(|| EditAnim::new(&name));
        let mut sounds: Vec<String> = host.sound_events().into_iter().map(|i| i.to_string()).collect();
        sounds.sort();
        let sound = sounds.iter().position(|s| s == "minecraft:entity.villager.yes").unwrap_or(0);
        crate::menu::yield_screen();
        let open = host.mod_screen() == Some(SCREEN) || host.mod_screen_open(SCREEN);
        let mut ed = Editor {
            npc,
            built: Arc::new(anim.to_animation()),
            anim,
            undo: Vec::new(),
            model: String::new(),
            texture: look.texture.clone(),
            bones: Vec::new(),
            locators: Vec::new(),
            bone: 0,
            channel: 0,
            time: 0.0,
            playing: None,
            fired: None,
            turn: 0.0,
            scroll: 0,
            sounds,
            sound,
            particle: runtime::PARTICLES.iter().position(|p| *p == "minecraft:heart").unwrap_or(0),
            locator: 0,
            buttons: Vec::new(),
            drag: None,
            status: format!("Editing {name}"),
            shown: open,
            close: false,
            timeline: [0.0; 4],
            outbox: Vec::new(),
        };
        ed.set_model(&look.model);
        if let Ok(mut e) = EDITOR.lock() {
            *e = Some(ed);
        }
        Ok(())
    }
}

/// The client tick: the preview's sounds and particles while it plays, the editor's messages to
/// the server, and forgetting the editor once its screen is closed (the draft stays).
pub fn tick(host: &mut dyn ClientHost) {
    load_saved(&*host);
    let mut fire = Vec::new();
    let (close, out) = {
        let Ok(mut e) = EDITOR.lock() else { return };
        let Some(ed) = e.as_mut() else { return };
        if ed.playing.is_some() {
            let t = ed.now(runtime::clock());
            let from = ed.fired.unwrap_or(-1.0);
            if from != t {
                for fx in ed.built.effects_between(from, t) {
                    fire.push(fx.kind.clone());
                }
            }
            ed.fired = Some(t);
        } else {
            ed.fired = None;
        }
        let pose = runtime::anim_pose(&ed.built, ed.now(runtime::clock()));
        let fire: Vec<_> = fire.into_iter().map(|k| (ed.npc, ed.model.clone(), runtime::body_rot(ed.npc), pose.clone(), k)).collect();
        let out = std::mem::take(&mut ed.outbox);
        let gone = ed.close || (ed.shown && host.mod_screen() != Some(SCREEN));
        let close = ed.close;
        if gone {
            if let Ok(mut d) = DRAFTS.lock() {
                d.insert(ed.anim.name.clone(), ed.anim.clone());
            }
            *e = None;
        }
        drop(e);
        for (id, model, body, pose, kind) in fire {
            runtime::play_effect(host, id, &model, body, &pose, &kind, [0.0; 3]);
        }
        (gone && close, out)
    };
    for (name, bytes) in out {
        let _ = host.send_payload_to_server(&PayloadId(name.into()), PayloadBytes(bytes));
    }
    if close {
        host.mod_screen_close(SCREEN);
    }
}

fn save(host: &dyn ClientHost, ed: &mut Editor) {
    let text = ed.anim.to_bedrock();
    let file = format!("{FOLDER}/{}.animation.json", ed.anim.name);
    let result = ConfigPath::new(&file).map_err(|e| format!("{e:?}")).and_then(|p| host.data_files().write(&p, text.as_bytes()).map_err(|e| format!("{e:?}")));
    ed.status = match result {
        Ok(()) => {
            animations::insert_saved(ed.anim.to_animation());
            format!("Saved: horizon-mod-data/projectfelix/{file}")
        }
        Err(e) => format!("Not saved: {e}"),
    };
}

// ---- drawing --------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn button(draw: &mut dyn DrawContext, buttons: &mut Vec<([f32; 4], Do)>, r: [f32; 4], label: &str, what: Do, on: bool, mouse: [f32; 2]) {
    let hover = inside(r, mouse);
    draw.fill(r, if on { 0xE040_7040 } else if hover { 0xE050_5868 } else { 0xD028_2C38 });
    draw.fill([r[0], r[1] + r[3] - 1.0, r[2], 1.0], 0xFF10_1018);
    let label = fit(draw, label, r[2] - 4.0);
    let tw = draw.text_width(&label);
    draw.text(&label, [r[0] + (r[2] - tw) / 2.0, r[1] + (r[3] - 8.0) / 2.0], 0xFFF0_F0F0, false);
    buttons.push((r, what));
}

/// A picker: a field showing the current choice; a click takes the next, the wheel moves through.
fn picker(draw: &mut dyn DrawContext, buttons: &mut Vec<([f32; 4], Do)>, r: [f32; 4], label: &str, value: &str, what: Do, mouse: [f32; 2]) {
    draw.fill(r, if inside(r, mouse) { 0xE040_4458 } else { 0xC030_3040 });
    draw.text(label, [r[0] + 2.0, r[1] + 2.0], 0xFFA0_A0A0, false);
    let lw = draw.text_width(label) + 5.0;
    let v = fit(draw, value, r[2] - lw - 2.0);
    draw.text(&v, [r[0] + lw, r[1] + 2.0], 0xFFFF_FFFF, false);
    buttons.push((r, what));
}

/// `text` cut to `width`, with an ellipsis, keeping its end (where names differ).
fn fit(draw: &dyn DrawContext, text: &str, width: f32) -> String {
    if draw.text_width(text) <= width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    for start in 0..chars.len() {
        let s: String = std::iter::once('…').chain(chars[start..].iter().copied()).collect();
        if draw.text_width(&s) <= width {
            return s;
        }
    }
    String::new()
}

fn inside(r: [f32; 4], at: [f32; 2]) -> bool {
    at[0] >= r[0] && at[0] < r[0] + r[2] && at[1] >= r[1] && at[1] < r[1] + r[3]
}

fn fmt(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn short(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}

/// `screen_render_post`: the editor's panels.
pub fn render(host: &dyn ClientHost, e: &mut ScreenRenderPostEvent<'_>) {
    if host.mod_screen() != Some(SCREEN) {
        return;
    }
    let Ok(mut lock) = EDITOR.lock() else { return };
    let Some(ed) = lock.as_mut() else { return };
    let clock = runtime::clock();
    if ed.playing.is_some() {
        ed.time = ed.now(clock);
    }
    let draw = &mut *e.draw;
    let mouse = e.screen.mouse;
    let [w, h] = draw.size();
    let mut buttons = Vec::new();

    // Toolbar, right to left; the length is a label between its - and +.
    draw.fill([0.0, 0.0, w, BAR], 0xE010_1018);
    draw.text(&ed.anim.name, [4.0, 4.0], 0xFFFF_D86A, false);
    let mode = match ed.anim.mode {
        LoopMode::Loop => "Loop",
        LoopMode::Once => "Once",
        LoopMode::Hold => "Hold",
    };
    let len = format!("{}s", fmt(ed.anim.length));
    let play = if ed.playing.is_some() { "Pause" } else { "Play" };
    let tools: [(f32, &str, Option<Do>, bool); 11] = [
        (32.0, "Close", Some(Do::Close), false),
        (28.0, "Save", Some(Do::Save), false),
        (28.0, "Undo", Some(Do::Undo), false),
        (12.0, ">", Some(Do::Turn(45.0)), false),
        (12.0, "<", Some(Do::Turn(-45.0)), false),
        (28.0, mode, Some(Do::Mode), false),
        (12.0, "+", Some(Do::Longer), false),
        (24.0, &len, None, false),
        (12.0, "-", Some(Do::Shorter), false),
        (30.0, play, Some(Do::Play), ed.playing.is_some()),
        (0.0, "", None, false),
    ];
    let mut x = w - 2.0;
    for (width, label, what, on) in tools {
        if width == 0.0 {
            continue;
        }
        x -= width + 2.0;
        match what {
            Some(what) => button(draw, &mut buttons, [x, 1.0, width, BAR - 2.0], label, what, on, mouse),
            None => {
                let tw = draw.text_width(label);
                draw.text(label, [x + (width - tw) / 2.0, 4.0], 0xFFE0_E0E0, false);
            }
        }
    }
    let time = format!("{} s", fmt(ed.time));
    draw.text(&time, [x - draw.text_width(&time) - 6.0, 4.0], 0xFFE0_E0E0, false);

    // Left: bones, then the model, the texture and putting them on the NPC.
    let top = BAR + 2.0;
    let panel_h = h - top - TIMELINE_H - 4.0;
    draw.fill([2.0, top, LEFT_W, panel_h], 0xD810_1018);
    draw.text("Bones", [6.0, top + 2.0], 0xFFA0_A0A0, false);
    let foot = 42.0;
    let rows = ((panel_h - 14.0 - foot) / ROW).max(1.0) as usize;
    ed.scroll = ed.scroll.min(ed.bones.len().saturating_sub(rows));
    let bone_now = ed.bone_name();
    for (i, name) in ed.bones.iter().enumerate().skip(ed.scroll).take(rows) {
        let r = [2.0, top + 12.0 + (i - ed.scroll) as f32 * ROW, LEFT_W, ROW];
        let keyed = ed.anim.track(name).is_some_and(|t| t.channels.iter().any(|c| !c.is_empty()));
        if i == ed.bone {
            draw.fill(r, 0xC040_6090);
        } else if inside(r, mouse) {
            draw.fill(r, 0x60FF_FFFF);
        }
        let label = fit(draw, &if keyed { format!("* {name}") } else { name.clone() }, LEFT_W - 6.0);
        draw.text(&label, [r[0] + 4.0, r[1] + 1.0], 0xFFF0_F0F0, false);
        buttons.push((r, Do::Bone(i)));
    }
    let fy = top + panel_h - foot;
    picker(draw, &mut buttons, [4.0, fy, LEFT_W - 4.0, 12.0], "Model", &ed.model.clone(), Do::Model, mouse);
    picker(draw, &mut buttons, [4.0, fy + 14.0, LEFT_W - 4.0, 12.0], "Tex", &ed.texture.clone(), Do::Texture, mouse);
    button(draw, &mut buttons, [4.0, fy + 28.0, LEFT_W - 4.0, 12.0], "Use on NPC", Do::UseOnNpc, false, mouse);

    // Right: values, keyframe buttons, effects.
    let rx = w - RIGHT_W - 2.0;
    draw.fill([rx, top, RIGHT_W, panel_h], 0xD810_1018);
    let name = fit(draw, &bone_now, RIGHT_W - 6.0);
    draw.text(&name, [rx + 4.0, top + 2.0], 0xFFFF_D86A, false);
    for (i, label) in ["Rot", "Pos", "Scale"].into_iter().enumerate() {
        button(draw, &mut buttons, [rx + 2.0 + i as f32 * 31.0, top + 11.0, 30.0, 12.0], label, Do::Channel(i), ed.channel == i, mouse);
    }
    let v = ed.values();
    let key_here = ed.anim.key_at(&bone_now, ed.channel, ed.time).copied();
    for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
        let y = top + 25.0 + axis as f32 * 13.0;
        draw.text(label, [rx + 4.0, y + 2.0], [0xFFFF_6060, 0xFF60_FF60, 0xFF60_A0FF][axis], false);
        let field = [rx + 14.0, y, 46.0, 12.0];
        draw.fill(field, if key_here.is_some() { 0xC060_5020 } else { 0xC030_3040 });
        draw.text(&fmt(v[axis]), [field[0] + 3.0, y + 2.0], 0xFFFF_FFFF, false);
        buttons.push((field, Do::Value(axis)));
        let step = ed.step();
        button(draw, &mut buttons, [rx + 62.0, y, 14.0, 12.0], "-", Do::Nudge(axis, -step), false, mouse);
        button(draw, &mut buttons, [rx + 78.0, y, 14.0, 12.0], "+", Do::Nudge(axis, step), false, mouse);
    }
    let y = top + 65.0;
    button(draw, &mut buttons, [rx + 2.0, y, 44.0, 12.0], "Key (K)", Do::Key, key_here.is_some(), mouse);
    button(draw, &mut buttons, [rx + 48.0, y, 44.0, 12.0], "Delete", Do::Delete, false, mouse);
    let ease = key_here.map(|k| k.ease.name()).unwrap_or("-");
    button(draw, &mut buttons, [rx + 2.0, y + 13.0, 90.0, 12.0], &format!("Ease: {ease}"), Do::Ease, false, mouse);
    let y = top + 92.0;
    let sound = ed.sounds.get(ed.sound).map(|s| short(s).to_string()).unwrap_or_else(|| "-".into());
    picker(draw, &mut buttons, [rx + 2.0, y, 90.0, 12.0], "Snd", &sound, Do::Sound, mouse);
    button(draw, &mut buttons, [rx + 2.0, y + 13.0, 44.0, 12.0], "+ Sound", Do::AddSound, false, mouse);
    button(draw, &mut buttons, [rx + 48.0, y + 13.0, 44.0, 12.0], "Del FX", Do::DelEffects, false, mouse);
    let particle = short(runtime::PARTICLES[ed.particle % runtime::PARTICLES.len()]).to_string();
    picker(draw, &mut buttons, [rx + 2.0, y + 27.0, 90.0, 12.0], "Fx", &particle, Do::Particle, mouse);
    let at = ed.locator_name().unwrap_or_else(|| "feet".into());
    picker(draw, &mut buttons, [rx + 2.0, y + 40.0, 56.0, 12.0], "At", &at, Do::Locator, mouse);
    button(draw, &mut buttons, [rx + 60.0, y + 40.0, 32.0, 12.0], "+ Fx", Do::AddParticle, false, mouse);

    // Timeline: ruler, a row a channel of the selected bone, the effects row, the playhead.
    let tl = [LEFT_W + 6.0, h - TIMELINE_H - 2.0, w - LEFT_W - RIGHT_W - 12.0, TIMELINE_H];
    ed.timeline = tl;
    draw.fill(tl, 0xEC10_1018);
    let len = ed.anim.length.max(SNAP);
    let tx = |t: f32| tl[0] + 4.0 + (tl[2] - 8.0) * (t / len);
    let mut t = 0.0;
    while t <= len + 1e-4 {
        let whole = (t - t.round()).abs() < 1e-4;
        draw.fill([tx(t), tl[1] + 2.0, 1.0, if whole { 6.0 } else { 3.0 }], 0xFFA0_A0A0);
        if whole {
            draw.text(&format!("{}", t.round() as i32), [tx(t) + 2.0, tl[1] + 1.0], 0xFFA0_A0A0, false);
        }
        t += 0.25;
    }
    for (c, label) in ["Rot", "Pos", "Scale", "FX"].into_iter().enumerate() {
        let ry = tl[1] + 11.0 + c as f32 * 12.0;
        draw.fill([tl[0] + 2.0, ry, tl[2] - 4.0, 11.0], if c == ed.channel { 0x60FF_FFFF } else { 0x30FF_FFFF });
        draw.text(label, [tl[0] + 4.0, ry + 2.0], 0xFFC0_C0C0, false);
        if c < 3 {
            if let Some(track) = ed.anim.track(&bone_now) {
                for k in &track.channels[c] {
                    let kx = tx(k.time);
                    draw.quad([[kx, ry + 1.0], [kx + 4.5, ry + 5.5], [kx, ry + 10.0], [kx - 4.5, ry + 5.5]], 0xFFFF_D040);
                }
            }
        } else {
            for fx in &ed.anim.effects {
                let color = if matches!(fx.kind, EffectKind::Sound(_)) { 0xFF60_C0FF } else { 0xFFFF_70C0 };
                draw.fill([tx(fx.time) - 2.0, ry + 3.0, 4.0, 5.0], color);
            }
        }
    }
    draw.fill([tx(ed.time) - 0.5, tl[1] + 2.0, 1.5, tl[3] - 14.0], 0xFFFF_4040);
    buttons.push((tl, Do::Timeline));
    let here = ed.anim.effects_at(ed.time);
    let line = if !ed.status.is_empty() {
        ed.status.clone()
    } else if !here.is_empty() {
        format!("Here: {}", here.join(", "))
    } else {
        String::new()
    };
    let line = fit(draw, &line, tl[2] - 8.0);
    draw.text(&line, [tl[0] + 4.0, tl[1] + tl[3] - 10.0], 0xFF90_E090, false);
    ed.buttons = buttons;
}

// ---- input ----------------------------------------------------------------------------------------

/// What a click (`by` 1) or the wheel (`by` up or down) does to a picker.
fn cycle(i: usize, by: i32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (i as i64 + by as i64).rem_euclid(n as i64) as usize
}

fn pick(host: &dyn ClientHost, ed: &mut Editor, what: Do, by: i32) {
    match what {
        Do::Sound => {
            ed.sound = cycle(ed.sound, by, ed.sounds.len());
            if let Some(s) = ed.sounds.get(ed.sound) {
                host.play_local_sound(&SoundPlay::ui(s), 0);
            }
        }
        Do::Particle => ed.particle = cycle(ed.particle, by, runtime::PARTICLES.len()),
        Do::Locator => ed.locator = cycle(ed.locator, by, ed.locators.len() + 1),
        Do::Model => {
            let all = models::list();
            let at = all.iter().position(|m| *m == ed.model).unwrap_or(0);
            if let Some(next) = all.get(cycle(at, by, all.len())).cloned() {
                ed.set_model(&next);
                ed.texture = Look::of_model(&next).texture;
            }
        }
        Do::Texture => {
            let all = models::textures();
            let at = all.iter().position(|t| *t == ed.texture).unwrap_or(0);
            if let Some(next) = all.get(cycle(at, by, all.len())).cloned() {
                ed.texture = next;
            }
        }
        _ => {}
    }
}

fn act(host: &dyn ClientHost, ed: &mut Editor, what: Do, mouse: [f32; 2]) {
    let clock = runtime::clock();
    let changes = matches!(
        what,
        Do::Shorter | Do::Longer | Do::Mode | Do::Nudge(..) | Do::Value(_) | Do::Key | Do::Delete | Do::Ease | Do::AddSound | Do::AddParticle | Do::DelEffects
    );
    if changes {
        ed.remember();
    }
    match what {
        Do::Play => {
            ed.playing = match ed.playing {
                Some(_) => {
                    ed.time = snap(ed.now(clock));
                    None
                }
                None => Some((clock, ed.time)),
            }
        }
        Do::Shorter => ed.anim.length = (ed.anim.length - 0.25).max(0.25),
        Do::Longer => ed.anim.length = (ed.anim.length + 0.25).min(60.0),
        Do::Mode => {
            ed.anim.mode = match ed.anim.mode {
                LoopMode::Loop => LoopMode::Once,
                LoopMode::Once => LoopMode::Hold,
                LoopMode::Hold => LoopMode::Loop,
            }
        }
        Do::Save => save(host, ed),
        Do::Close => ed.close = true,
        Do::Undo => match ed.undo.pop() {
            Some(a) => ed.anim = a,
            None => ed.status = "Nothing to undo".into(),
        },
        Do::Turn(by) => ed.turn = (ed.turn + by).rem_euclid(360.0),
        Do::Bone(i) => ed.bone = i,
        Do::Channel(c) => ed.channel = c,
        Do::Nudge(axis, by) => {
            let mut v = ed.values();
            v[axis] += by;
            ed.set_values(v);
        }
        Do::Value(axis) => ed.drag = Some(Drag::Value { axis, x0: mouse[0], v0: ed.values() }),
        Do::Key => {
            let v = ed.values();
            ed.set_values(v);
        }
        Do::Delete => {
            let bone = ed.bone_name();
            ed.anim.remove_key(&bone, ed.channel, ed.time);
        }
        Do::Ease => {
            let bone = ed.bone_name();
            if let Some(k) = ed.anim.key_at(&bone, ed.channel, ed.time).copied() {
                ed.anim.set_ease(&bone, ed.channel, ed.time, k.ease.next());
            }
        }
        Do::Timeline => {
            ed.playing = None;
            ed.drag = Some(Drag::Time);
            time_at(ed, mouse);
        }
        Do::Sound | Do::Particle | Do::Locator | Do::Model | Do::Texture => pick(host, ed, what, 1),
        Do::AddSound => {
            if let Some(s) = ed.sounds.get(ed.sound).cloned() {
                ed.anim.add_effect(ed.time, EffectKind::Sound(s));
            }
        }
        Do::AddParticle => {
            let p = runtime::PARTICLES[ed.particle % runtime::PARTICLES.len()].to_string();
            let at = ed.locator_name();
            ed.anim.add_effect(ed.time, EffectKind::Particle(p, at));
        }
        Do::DelEffects => {
            if !ed.anim.remove_effects_at(ed.time) {
                ed.undo.pop();
            }
        }
        Do::UseOnNpc => {
            let body = crate::serde_json::json!({ "npc": ed.npc, "model": ed.model, "texture": ed.texture }).to_string();
            ed.outbox.push((SET_LOOK, body.into_bytes()));
            ed.status = format!("Asked to put {} on the NPC", ed.model);
        }
    }
    if changes || matches!(what, Do::Undo) {
        ed.rebuild();
    }
    if !matches!(what, Do::Save | Do::UseOnNpc | Do::Undo) {
        ed.status.clear();
    }
}

fn time_at(ed: &mut Editor, mouse: [f32; 2]) {
    let tl = ed.timeline;
    let f = ((mouse[0] - tl[0] - 4.0) / (tl[2] - 8.0)).clamp(0.0, 1.0);
    ed.time = snap(f * ed.anim.length);
    // A click on a channel row picks that channel.
    let row = ((mouse[1] - tl[1] - 11.0) / 12.0).floor();
    if (0.0..3.0).contains(&row) {
        ed.channel = row as usize;
    }
}

pub fn mouse_pressed(host: &dyn ClientHost, e: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) || e.button != 0 {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(ed) = lock.as_mut() else { return Flow::Continue };
    let mouse = e.screen.mouse;
    if let Some((_, what)) = ed.buttons.iter().rev().find(|(r, _)| inside(*r, mouse)).copied() {
        if !matches!(what, Do::Timeline | Do::Value(_) | Do::Sound) {
            host.play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0);
        }
        act(host, ed, what, mouse);
    }
    Flow::Cancel
}

pub fn mouse_dragged(host: &dyn ClientHost, e: &mut ScreenMouseDraggedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(ed) = lock.as_mut() else { return Flow::Continue };
    let mouse = e.screen.mouse;
    match ed.drag {
        Some(Drag::Value { axis, x0, v0 }) => {
            // A step for every four pixels.
            let mut v = v0;
            v[axis] = v0[axis] + ((mouse[0] - x0) / 4.0).round() * ed.step();
            ed.set_values(v);
            ed.rebuild();
        }
        Some(Drag::Time) => time_at(ed, mouse),
        None => {}
    }
    Flow::Cancel
}

pub fn mouse_released(host: &dyn ClientHost, _e: &mut ScreenMouseButtonReleasedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    if let Ok(mut lock) = EDITOR.lock()
        && let Some(ed) = lock.as_mut()
    {
        ed.drag = None;
        return Flow::Cancel;
    }
    Flow::Continue
}

pub fn mouse_scrolled(host: &dyn ClientHost, e: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(ed) = lock.as_mut() else { return Flow::Continue };
    let up = e.scroll[1] > 0.0;
    let mouse = e.screen.mouse;
    // Over a picker: move through it (Shift: twenty at a time).
    if let Some((_, what)) = ed.buttons.iter().find(|(r, w)| inside(*r, mouse) && matches!(w, Do::Sound | Do::Particle | Do::Locator | Do::Model | Do::Texture)).copied() {
        let by = if e.modifiers & 1 != 0 { 20 } else { 1 };
        pick(host, ed, what, if up { -by } else { by });
        return Flow::Cancel;
    }
    if mouse[0] < LEFT_W + 4.0 {
        ed.scroll = if up { ed.scroll.saturating_sub(1) } else { ed.scroll + 1 };
    } else {
        ed.playing = None;
        ed.time = snap((ed.time + if up { SNAP } else { -SNAP }).clamp(0.0, ed.anim.length));
    }
    Flow::Cancel
}

pub fn key_pressed(host: &dyn ClientHost, e: &mut ScreenKeyPressedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = EDITOR.lock() else { return Flow::Continue };
    let Some(ed) = lock.as_mut() else { return Flow::Continue };
    let ctrl = e.modifiers & 2 != 0;
    let what = match e.key {
        "key.keyboard.space" => Some(Do::Play),
        "key.keyboard.k" => Some(Do::Key),
        "key.keyboard.delete" => Some(Do::Delete),
        "key.keyboard.s" if ctrl => Some(Do::Save),
        "key.keyboard.z" if ctrl => Some(Do::Undo),
        "key.keyboard.left" | "key.keyboard.right" => {
            ed.playing = None;
            let by = if e.key.ends_with("left") { -SNAP } else { SNAP };
            ed.time = snap((ed.time + by).clamp(0.0, ed.anim.length));
            return Flow::Cancel;
        }
        "key.keyboard.up" | "key.keyboard.down" => {
            let n = ed.bones.len().max(1);
            ed.bone = if e.key.ends_with("up") { (ed.bone + n - 1) % n } else { (ed.bone + 1) % n };
            return Flow::Cancel;
        }
        _ => None,
    };
    match what {
        Some(w) => {
            act(host, ed, w, e.screen.mouse);
            Flow::Cancel
        }
        // Escape and the rest go to the screen (Escape closes it).
        None => Flow::Continue,
    }
}
