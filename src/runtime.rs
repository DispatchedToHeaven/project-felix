//! Playing animations on the client: each NPC's idle or walk animation, the action the server
//! synced to it, the blend between them, and the sounds, particles and scripts on their timelines.
//!
//! The renderer asks for a pose every frame ([`pose`]); the client tick ([`tick`]) reads the
//! synced actions and fires the effects the animations passed since the last tick.
use crate::anim::{Animation, BonePose, EffectKind, LoopMode};
use crate::molang::{self, Context, MapContext};
use crate::companion::{LOOK, Look, ORDERS, Orders};
use crate::scene::{self, Scene};
use crate::games::{TABLE, Table};
use crate::matches::{Court, SPORT};
use crate::sports::Match;
use crate::scenes::{Playing, SCENE};
use crate::sync::{ACTION, Action};
use crate::{animations, models};
use mod_api::{
    AttachmentHolder, AttachmentQueries, ClientHost, LivingRenderState, ParticleKey, ParticleOptions, Registrar,
    SoundPlay, SoundSource,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

/// Ticks a change of the idle/walk animation blends over.
const BASE_BLEND: f64 = 4.0;
/// Ticks an action fades in, and out at the end of a play-once action.
const ACTION_FADE: f64 = 3.0;
/// Walk speed above which the walk animation plays.
const WALKING: f32 = 0.05;

/// A bone's full change from rest: rotation (radians), position (texels), scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Full {
    pub rotation: [f32; 3],
    pub position: [f32; 3],
    pub scale: [f32; 3],
}

impl Default for Full {
    fn default() -> Self {
        Full { rotation: [0.0; 3], position: [0.0; 3], scale: [1.0; 3] }
    }
}

fn mix(a: [f32; 3], b: [f32; 3], w: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w, a[2] + (b[2] - a[2]) * w]
}

struct Layer {
    animation: Arc<Animation>,
    /// Clock (ticks) it started at.
    start: f64,
    /// The animation time effects were fired up to; none until the first tick.
    fired_to: Option<f32>,
}

impl Layer {
    fn new(animation: Arc<Animation>, start: f64) -> Self {
        Layer { animation, start, fired_to: None }
    }
    fn elapsed(&self, now: f64) -> f32 {
        ((now - self.start) / 20.0) as f32
    }
}

#[derive(Default)]
struct Npc {
    model: String,
    /// The model's own walk and idle animations' names.
    walk_idle: [String; 2],
    base: Option<Layer>,
    /// The base animation it is blending away from, and when the blend began.
    leaving: Option<(Layer, f64)>,
    action: Option<Layer>,
    vars: MapContext,
    body_rot: f32,
    walk_speed: f32,
    head: [f32; 2],
    age: f32,
    last_frame: f64,
}

#[derive(Default)]
struct State {
    clock: f64,
    npcs: HashMap<u32, Npc>,
    /// Synced actions by entity id, as the last client tick read them.
    actions: HashMap<u32, Action>,
    /// Synced orders and looks, likewise.
    orders: HashMap<u32, Orders>,
    looks: HashMap<u32, Look>,
    /// Scenes playing, by NPC.
    scenes: HashMap<u32, Active>,
    /// Game tables, by NPC.
    tables: HashMap<u32, Table>,
    /// Ball game courts, by NPC, with the match read from each.
    courts: HashMap<u32, (Court, Option<Arc<Match>>)>,
}

/// NPC `id`'s ball game: which way its court faces, and the match.
pub fn court_of(id: u32) -> Option<(f32, Arc<Match>)> {
    let s = STATE.lock().ok()?;
    let (c, m) = s.courts.get(&id)?;
    Some((c.yaw, m.clone()?))
}

/// The NPC playing a ball game with the player whose UUID is `player` (32 hex digits).
pub fn court_player(player: &str) -> Option<u32> {
    STATE.lock().ok()?.courts.iter().find(|(_, (c, _))| c.player == player).map(|(id, _)| *id)
}

/// The client's clock as the renderer last saw it: game time in ticks, with the partial tick.
pub fn clock() -> f64 {
    STATE.lock().map(|s| s.clock).unwrap_or(0.0)
}

/// The game table at NPC `id`, if a game is set up there.
pub fn table_of(id: u32) -> Option<Table> {
    STATE.lock().ok()?.tables.get(&id).cloned()
}

/// A scene playing on an NPC, its file read, and how far each prop's effects have been fired.
struct Active {
    playing: Playing,
    scene: Arc<Scene>,
    props_fired: Vec<Option<f32>>,
}

impl Active {
    fn on_at(&self, now: f64) -> bool {
        now >= self.playing.start as f64 && now < self.playing.start as f64 + self.scene.length as f64 * 20.0
    }
}

/// The scene NPC `id` plays at `now`: the way it faces, the scene, and when it started.
pub fn scene_of(id: u32, now: f64) -> Option<(f32, Arc<Scene>, f64)> {
    let s = STATE.lock().ok()?;
    let a = s.scenes.get(&id).filter(|a| a.on_at(now))?;
    Some((a.playing.yaw, a.scene.clone(), a.playing.start as f64))
}

/// A prop's bones at `t` seconds into its animation `name`.
pub fn prop_pose(name: &str, t: f32) -> HashMap<String, Full> {
    let Some(a) = animations::get(name) else { return HashMap::new() };
    let (t, _) = a.local_time(t);
    let mut cx = MapContext::default();
    cx.queries.insert("anim_time".into(), t);
    let mut out = HashMap::new();
    add(&mut out, a.sample(t, &mut cx), 1.0);
    out
}

/// The scene actor's animation, as a synced action from the scene's start, while it plays.
fn scene_action(scenes: &HashMap<u32, Active>, id: u32, now: f64) -> Option<Action> {
    let a = scenes.get(&id).filter(|a| a.on_at(now))?;
    Some(Action { animation: a.scene.actor_animation.clone()?, start: a.playing.start })
}

/// The NPCs drawn in the last five seconds.
pub fn visible_npcs() -> Vec<u32> {
    STATE.lock().map(|s| s.npcs.keys().copied().collect()).unwrap_or_default()
}

/// An NPC's orders, as the server last synced them.
pub fn orders(id: u32) -> Option<Orders> {
    STATE.lock().ok()?.orders.get(&id).copied()
}

/// The NPCs the client knows of (whose orders it has).
pub fn npc_ids() -> Vec<u32> {
    STATE.lock().map(|s| s.orders.keys().copied().collect()).unwrap_or_default()
}

/// An NPC's look, if the server gave it one.
pub fn look(id: u32) -> Option<Look> {
    STATE.lock().ok()?.looks.get(&id).cloned()
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// The particles an animation may name: asked for at registration, as the API wants.
pub const PARTICLES: &[&str] = &[
    "minecraft:angry_villager", "minecraft:bubble", "minecraft:bubble_pop", "minecraft:cherry_leaves",
    "minecraft:cloud", "minecraft:composter", "minecraft:crit", "minecraft:damage_indicator", "minecraft:dolphin",
    "minecraft:dripping_water", "minecraft:effect", "minecraft:electric_spark", "minecraft:enchant",
    "minecraft:enchanted_hit", "minecraft:end_rod", "minecraft:explosion", "minecraft:firework", "minecraft:flame",
    "minecraft:glow", "minecraft:happy_villager", "minecraft:heart", "minecraft:instant_effect", "minecraft:lava",
    "minecraft:note", "minecraft:poof", "minecraft:portal", "minecraft:scrape", "minecraft:smoke",
    "minecraft:large_smoke", "minecraft:small_flame", "minecraft:snowflake", "minecraft:soul",
    "minecraft:soul_fire_flame", "minecraft:splash", "minecraft:sweep_attack", "minecraft:totem_of_undying",
    "minecraft:wax_off", "minecraft:wax_on", "minecraft:witch", "minecraft:white_ash",
];

static PARTICLE_KEYS: OnceLock<HashMap<&'static str, ParticleKey>> = OnceLock::new();

pub fn register(r: &mut dyn Registrar) {
    let keys = PARTICLES.iter().map(|id| (*id, r.particle_key(id))).collect();
    let _ = PARTICLE_KEYS.set(keys);
}

/// `projectfelix:felix` → `animation.felix.`: the names a model's own animations start with.
fn prefix(model: &str) -> String {
    format!("animation.{}.", model.rsplit(':').next().unwrap_or(model))
}

fn context(npc: &Npc, anim_time: f32) -> MapContext {
    let mut cx = npc.vars.clone();
    let q = &mut cx.queries;
    q.insert("anim_time".into(), anim_time);
    q.insert("life_time".into(), npc.age / 20.0);
    q.insert("modified_move_speed".into(), npc.walk_speed);
    q.insert("ground_speed".into(), npc.walk_speed * 4.0);
    q.insert("is_moving".into(), if npc.walk_speed > WALKING { 1.0 } else { 0.0 });
    q.insert("is_on_ground".into(), 1.0);
    q.insert("head_y_rotation".into(), npc.head[0]);
    q.insert("head_x_rotation".into(), npc.head[1]);
    cx
}

/// What an animation reads while it is drawn: the NPC's queries and variables, read where they
/// are (what it sets lasts the frame).
struct Frame<'a> {
    npc: &'a Npc,
    anim_time: f32,
    set: HashMap<String, f32>,
    seed: u32,
}

impl Context for Frame<'_> {
    fn query(&self, name: &str) -> f32 {
        let n = self.npc;
        match name {
            "anim_time" => self.anim_time,
            "life_time" => n.age / 20.0,
            "modified_move_speed" => n.walk_speed,
            "ground_speed" => n.walk_speed * 4.0,
            "is_moving" => f32::from(u8::from(n.walk_speed > WALKING)),
            "is_on_ground" => 1.0,
            "head_y_rotation" => n.head[0],
            "head_x_rotation" => n.head[1],
            other => n.vars.query(other),
        }
    }
    fn var(&self, name: &str) -> f32 {
        self.set.get(name).copied().unwrap_or_else(|| self.npc.vars.var(name))
    }
    fn set_var(&mut self, name: &str, value: f32) {
        self.set.insert(name.to_string(), value);
    }
    fn random(&mut self) -> f32 {
        let mut x = self.seed.wrapping_add(0x9E37_79B9);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
}

fn sample<'a>(npc: &Npc, layer: &'a Layer, now: f64) -> Vec<(&'a str, BonePose)> {
    let (t, _) = layer.animation.local_time(layer.elapsed(now));
    let mut cx = Frame { npc, anim_time: t, set: HashMap::new(), seed: npc.vars.seed };
    layer.animation.sample(t, &mut cx)
}

/// Adds `w` of each bone's change in `poses` to `out` (by lowercase bone name).
fn add(out: &mut HashMap<String, Full>, poses: Vec<(&str, BonePose)>, w: f32) {
    let d = Full::default();
    for (bone, p) in poses {
        let (r, at, s) = (p.rotation.unwrap_or(d.rotation), p.position.unwrap_or(d.position), p.scale.unwrap_or(d.scale));
        let f = out.entry(bone.to_ascii_lowercase()).or_default();
        for i in 0..3 {
            f.rotation[i] += (r[i] - d.rotation[i]) * w;
            f.position[i] += (at[i] - d.position[i]) * w;
            f.scale[i] += (s[i] - d.scale[i]) * w;
        }
    }
}

/// Every animated bone's change from rest, by lowercase bone name.
fn merged(npc: &Npc, now: f64) -> HashMap<String, Full> {
    let mut out = HashMap::new();
    // The base, blended from the one it is leaving.
    let leaving = npc.leaving.as_ref().map(|(old, since)| (old, ((now - since) / BASE_BLEND).clamp(0.0, 1.0) as f32));
    if let Some((old, w)) = leaving {
        add(&mut out, sample(npc, old, now), 1.0 - w);
    }
    if let Some(base) = &npc.base {
        add(&mut out, sample(npc, base, now), leaving.map_or(1.0, |(_, w)| w));
    }
    if let Some(action) = &npc.action {
        let a = &action.animation;
        let elapsed = now - action.start;
        let mut w = (elapsed / ACTION_FADE).clamp(0.0, 1.0);
        if a.mode == LoopMode::Once {
            let left = a.length as f64 * 20.0 - elapsed;
            w = w.min((left / ACTION_FADE).clamp(0.0, 1.0));
        }
        let w = w as f32;
        for (bone, p) in sample(npc, action, now) {
            let base = out.entry(bone.to_ascii_lowercase()).or_default();
            if let Some(r) = p.rotation {
                base.rotation = mix(base.rotation, r, w);
            }
            if let Some(v) = p.position {
                base.position = mix(base.position, v, w);
            }
            if let Some(s) = p.scale {
                base.scale = mix(base.scale, s, w);
            }
        }
    }
    out
}

/// Brings `npc`'s layers up to `now`: the base by how fast it walks, the action from the server.
fn update_layers(npc: &mut Npc, id: u32, actions: &HashMap<u32, Action>, scenes: &HashMap<u32, Active>, now: f64) {
    if npc.walk_idle[1].is_empty() {
        let prefix = prefix(&npc.model);
        npc.walk_idle = [format!("{prefix}walk"), format!("{prefix}idle")];
    }
    let wanted = if npc.walk_speed > WALKING { &npc.walk_idle[0] } else { &npc.walk_idle[1] };
    let wanted = animations::get(wanted).or_else(|| animations::get(&npc.walk_idle[1]));
    let changed = wanted.as_ref().map(|a| &a.name) != npc.base.as_ref().map(|l| &l.animation.name);
    if changed {
        let old = npc.base.take();
        npc.base = wanted.map(|a| Layer::new(a, now));
        npc.leaving = old.map(|l| (l, now));
    }
    if npc.leaving.as_ref().is_some_and(|(_, since)| now - since >= BASE_BLEND) {
        npc.leaving = None;
    }
    let from_scene = scene_action(scenes, id, now);
    let synced = from_scene.as_ref().or_else(|| actions.get(&id));
    let same = match (&npc.action, synced) {
        (Some(l), Some(a)) => l.animation.name == a.animation && l.start == a.start as f64,
        (None, None) => true,
        _ => false,
    };
    if !same {
        npc.action = synced.and_then(|a| animations::get(&a.animation).map(|anim| Layer::new(anim, a.start as f64)));
    }
    // A play-once action that has ended gives the base its bones back.
    if npc.action.as_ref().is_some_and(|l| l.animation.mode == LoopMode::Once && l.elapsed(now) > l.animation.length + 0.5) {
        npc.action = None;
    }
}

/// The renderer's call, each frame: the bones' changes from rest for entity `id` drawn with
/// `model`, at clock `now` (game time in ticks, with the partial tick).
pub fn pose(id: u32, model: &str, living: &LivingRenderState, age: f32, now: f64) -> HashMap<String, Full> {
    let Ok(mut state) = STATE.lock() else { return HashMap::new() };
    state.clock = now;
    let State { npcs, actions, scenes, .. } = &mut *state;
    // The editor's preview: only the animation being made, at the editor's time.
    if let Some(p) = crate::editor::preview(id, now) {
        npcs.entry(id).or_default().body_rot = living.body_rot;
        npcs.entry(id).or_default().last_frame = now;
        return anim_pose_at(&p.anim, p.time);
    }
    let npc = npcs.entry(id).or_default();
    if npc.model != model {
        npc.model = model.to_string();
        npc.walk_idle = Default::default();
    }
    npc.body_rot = living.body_rot;
    npc.walk_speed = living.walk_animation_speed;
    npc.head = [living.y_rot, living.x_rot];
    npc.age = age;
    npc.last_frame = now;
    update_layers(npc, id, actions, scenes, now);
    merged(npc, now)
}

/// The client tick: reads the synced actions and fires the effects each NPC's animations have
/// passed since the last tick.
pub fn tick(host: &mut dyn ClientHost) {
    let mut actions = HashMap::new();
    if let Ok(query) = host.query(&ACTION) {
        for row in query.iter().flatten() {
            if let (AttachmentHolder::Entity(e), Some(a)) = (row.holder, row.value) {
                actions.insert(e.id(), a.clone());
            }
        }
    }
    let mut orders = HashMap::new();
    if let Ok(query) = host.query(&ORDERS) {
        for row in query.iter().flatten() {
            if let AttachmentHolder::Entity(e) = row.holder {
                orders.insert(e.id(), *row.value);
            }
        }
    }
    let mut looks = HashMap::new();
    if let Ok(query) = host.query(&LOOK) {
        for row in query.iter().flatten() {
            if let (AttachmentHolder::Entity(e), Some(l)) = (row.holder, row.value) {
                looks.insert(e.id(), l.clone());
            }
        }
    }
    let mut playing = HashMap::new();
    if let Ok(query) = host.query(&SCENE) {
        for row in query.iter().flatten() {
            if let (AttachmentHolder::Entity(e), Some(p)) = (row.holder, row.value) {
                playing.insert(e.id(), p.clone());
            }
        }
    }
    let mut tables = HashMap::new();
    if let Ok(query) = host.query(&TABLE) {
        for row in query.iter().flatten() {
            if let (AttachmentHolder::Entity(e), Some(t)) = (row.holder, row.value) {
                tables.insert(e.id(), t.clone());
            }
        }
    }
    let mut courts = HashMap::new();
    if let Ok(query) = host.query(&SPORT) {
        for row in query.iter().flatten() {
            if let (AttachmentHolder::Entity(e), Some(c)) = (row.holder, row.value) {
                courts.insert(e.id(), c.clone());
            }
        }
    }
    let mut fire = Vec::new();
    {
        let Ok(mut state) = STATE.lock() else { return };
        state.tables = tables;
        // A match is read once, again only when it changes.
        let old = std::mem::take(&mut state.courts);
        state.courts = courts
            .into_iter()
            .map(|(id, c): (u32, Court)| {
                let m = match old.get(&id) {
                    Some((o, m)) if o.json == c.json => m.clone(),
                    _ => crate::serde_json::from_str(&c.json).ok().and_then(|v| Match::from_json(&v)).map(Arc::new),
                };
                (id, (c, m))
            })
            .collect();
        let mut scenes = std::mem::take(&mut state.scenes);
        scenes.retain(|id, a| playing.get(id) == Some(&a.playing));
        for (id, p) in playing {
            if scenes.contains_key(&id) {
                continue;
            }
            match scene::parse(p.json.as_bytes()) {
                Ok(s) => {
                    let props_fired = vec![None; s.props.len()];
                    scenes.insert(id, Active { playing: p, scene: Arc::new(s), props_fired });
                }
                Err(e) => eprintln!("[projectfelix] scene {} not shown: {e}", p.id),
            }
        }
        state.scenes = scenes;
        let now = state.clock;
        state.actions = actions;
        state.orders = orders;
        state.looks = looks;
        // NPCs not drawn for five seconds are gone or out of sight.
        state.npcs.retain(|_, n| (now - n.last_frame).abs() < 100.0);
        let State { npcs, actions, scenes, .. } = &mut *state;
        for (&id, npc) in npcs.iter_mut() {
            update_layers(npc, id, actions, scenes, now);
            // The props' own effects (a piece set down), from the NPC's feet plus the prop's place.
            if let Some(a) = scenes.get_mut(&id).filter(|a| a.on_at(now)) {
                let t = ((now - a.playing.start as f64) / 20.0) as f32;
                for (i, prop) in a.scene.props.iter().enumerate() {
                    let Some(anim) = prop.animation.as_ref().and_then(|n| animations::get(n)) else { continue };
                    let (to, _) = anim.local_time(t);
                    let from = a.props_fired[i].unwrap_or(-1.0);
                    if from != to {
                        let p = scene::prop_place(prop.offset);
                        let r = (180.0 - a.playing.yaw).to_radians();
                        let (s, c) = r.sin_cos();
                        let off = [p[0] * c + p[2] * s, p[1], -p[0] * s + p[2] * c];
                        for e in anim.effects_between(from, to) {
                            if !matches!(e.kind, EffectKind::Script(_)) {
                                fire.push((id, String::new(), 0.0, HashMap::new(), e.kind.clone(), off));
                            }
                        }
                    }
                    a.props_fired[i] = Some(to);
                }
            }
            let mut effects = Vec::new();
            for layer in [npc.base.as_mut(), npc.action.as_mut()].into_iter().flatten() {
                let (to, _) = layer.animation.local_time(layer.elapsed(now));
                let from = layer.fired_to.unwrap_or(-1.0);
                if from != to {
                    for e in layer.animation.effects_between(from, to) {
                        effects.push(e.kind.clone());
                    }
                }
                layer.fired_to = Some(to);
            }
            if effects.is_empty() {
                continue;
            }
            let pose = merged(npc, now);
            for kind in effects {
                match kind {
                    EffectKind::Script(expr) => {
                        let mut cx = context(npc, 0.0);
                        molang::eval(&expr, &mut cx);
                        npc.vars.vars = cx.vars;
                    }
                    other => fire.push((id, npc.model.clone(), npc.body_rot, pose.clone(), other, [0.0; 3])),
                }
            }
        }
    }
    for (id, model, body_rot, pose, kind, shift) in fire {
        play_effect(host, id, &model, body_rot, &pose, &kind, shift);
    }
}

/// Plays a sound or shows a particle of an animation on NPC `id` (drawn with `model`, turned
/// by `body_rot`, posed by `pose`), `shift` blocks from its feet.
pub fn play_effect(host: &mut dyn ClientHost, id: u32, model: &str, body_rot: f32, pose: &HashMap<String, Full>, kind: &EffectKind, shift: [f32; 3]) {
    let Some(entity) = host.entity(id) else { return };
    let Some(at) = entity.position(&*host) else { return };
    let at = [at[0] + shift[0] as f64, at[1] + shift[1] as f64, at[2] + shift[2] as f64];
    match kind {
        EffectKind::Sound(sound) => {
            host.play_local_sound(&SoundPlay::at(sound, SoundSource::Neutral, at), 0);
        }
        EffectKind::Particle(particle, locator) => {
            let Some(key) = PARTICLE_KEYS.get().and_then(|k| k.get(particle.as_str())) else {
                eprintln!("[projectfelix] particle {particle} is not one an animation can use");
                return;
            };
            let offset = locator.as_ref().and_then(|l| locator_offset(model, l, pose, body_rot)).unwrap_or([0.0; 3]);
            let p = [at[0] + offset[0] as f64, at[1] + offset[1] as f64, at[2] + offset[2] as f64];
            host.add_particle(&ParticleOptions::simple(*key), p, [0.0; 3]);
        }
        EffectKind::Script(_) => {}
    }
}

/// The bones of `anim` at `t` seconds, by lowercase name.
/// The player left the world: nothing is known of its NPCs.
pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

pub fn anim_pose(anim: &Animation, t: f32) -> HashMap<String, Full> {
    let (t, _) = anim.local_time(t);
    anim_pose_at(anim, t)
}

/// The pose of `anim` at `t` seconds exactly (its end is its end, not its start again): what the
/// editor shows.
pub fn anim_pose_at(anim: &Animation, t: f32) -> HashMap<String, Full> {
    let t = t.clamp(0.0, anim.length.max(0.0));
    let mut cx = MapContext::default();
    cx.queries.insert("anim_time".into(), t);
    let mut out = HashMap::new();
    add(&mut out, anim.sample(t, &mut cx), 1.0);
    out
}

/// How NPC `id`'s body was turned when last drawn.
pub fn body_rot(id: u32) -> f32 {
    STATE.lock().ok().and_then(|s| s.npcs.get(&id).map(|n| n.body_rot)).unwrap_or(0.0)
}

/// Where locator `name` is now, in blocks from the entity's feet, the model turned as the renderer turns it.
fn locator_offset(model: &str, name: &str, pose: &HashMap<String, Full>, body_rot: f32) -> Option<[f32; 3]> {
    let m = models::get(model)?;
    let geo = &m.geo;
    let (bone, at) = geo.locator(name)?;
    let by_index = |i: usize| {
        let f = pose.get(&geo.bones[i].name.to_ascii_lowercase()).copied().unwrap_or_default();
        (f.rotation, f.position, f.scale)
    };
    let p = geo.posed_point(bone, at, &by_index);
    let (x, y, z) = (p[0] / 16.0, p[1] / 16.0, p[2] / 16.0);
    let a = (180.0 - body_rot).to_radians();
    let (s, c) = a.sin_cos();
    Some([x * c + z * s, y, -x * s + z * c])
}
