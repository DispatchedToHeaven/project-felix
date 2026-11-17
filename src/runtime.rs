//! Playing animations on the client: each NPC's idle or walk animation, the action the server
//! synced to it, the blend between them, and the sounds, particles and scripts on their timelines.
//!
//! The renderer asks for a pose every frame ([`pose`]); the client tick ([`tick`]) reads the
//! synced actions and fires the effects the animations passed since the last tick.
use crate::anim::{Animation, BonePose, EffectKind, LoopMode};
use crate::molang::{self, MapContext};
use crate::companion::{LOOK, Look, ORDERS, Orders};
use crate::scene::{self, Scene};
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
    full_of(a.sample(t, &mut cx)).into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect()
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

/// An NPC's look, if the server gave it one.
pub fn look(id: u32) -> Option<Look> {
    STATE.lock().ok()?.looks.get(&id).cloned()
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// The particles an animation may name: asked for at registration, as the API wants.
const PARTICLES: &[&str] = &[
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

fn sample(npc: &Npc, layer: &Layer, now: f64) -> Vec<(String, BonePose)> {
    let (t, _) = layer.animation.local_time(layer.elapsed(now));
    let mut cx = context(npc, t);
    layer.animation.sample(t, &mut cx)
}

fn full_of(poses: Vec<(String, BonePose)>) -> HashMap<String, Full> {
    poses
        .into_iter()
        .map(|(bone, p)| {
            let d = Full::default();
            (bone, Full { rotation: p.rotation.unwrap_or(d.rotation), position: p.position.unwrap_or(d.position), scale: p.scale.unwrap_or(d.scale) })
        })
        .collect()
}

/// Every animated bone's change from rest, by lowercase bone name.
fn merged(npc: &Npc, now: f64) -> HashMap<String, Full> {
    let mut out = HashMap::new();
    if let Some(base) = &npc.base {
        out = full_of(sample(npc, base, now));
    }
    if let Some((old, since)) = &npc.leaving {
        let w = ((now - since) / BASE_BLEND).clamp(0.0, 1.0) as f32;
        let from = full_of(sample(npc, old, now));
        let names: Vec<String> = from.keys().chain(out.keys()).cloned().collect();
        let mut blended = HashMap::new();
        for name in names {
            let a = from.get(&name).copied().unwrap_or_default();
            let b = out.get(&name).copied().unwrap_or_default();
            blended.insert(
                name,
                Full { rotation: mix(a.rotation, b.rotation, w), position: mix(a.position, b.position, w), scale: mix(a.scale, b.scale, w) },
            );
        }
        out = blended;
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
            let base = out.entry(bone).or_default();
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
    out.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect()
}

/// Brings `npc`'s layers up to `now`: the base by how fast it walks, the action from the server.
fn update_layers(npc: &mut Npc, id: u32, actions: &HashMap<u32, Action>, scenes: &HashMap<u32, Active>, now: f64) {
    let prefix = prefix(&npc.model);
    let wanted = if npc.walk_speed > WALKING { "walk" } else { "idle" };
    let wanted = animations::get(&format!("{prefix}{wanted}")).or_else(|| animations::get(&format!("{prefix}idle")));
    let current = npc.base.as_ref().map(|l| l.animation.name.clone());
    if wanted.as_ref().map(|a| &a.name) != current.as_ref() {
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
    let npc = npcs.entry(id).or_default();
    npc.model = model.to_string();
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
    let mut fire = Vec::new();
    {
        let Ok(mut state) = STATE.lock() else { return };
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
        state.npcs.retain(|_, n| now - n.last_frame < 100.0);
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
        let Some(entity) = host.entity(id) else { continue };
        let Some(at) = entity.position(&*host) else { continue };
        let at = [at[0] + shift[0] as f64, at[1] + shift[1] as f64, at[2] + shift[2] as f64];
        match kind {
            EffectKind::Sound(sound) => {
                host.play_local_sound(&SoundPlay::at(&sound, SoundSource::Neutral, at), 0);
            }
            EffectKind::Particle(particle, locator) => {
                let Some(key) = PARTICLE_KEYS.get().and_then(|k| k.get(particle.as_str())) else {
                    eprintln!("[projectfelix] particle {particle} is not one an animation can use");
                    continue;
                };
                let offset = locator.and_then(|l| locator_offset(&model, &l, &pose, body_rot)).unwrap_or([0.0; 3]);
                let p = [at[0] + offset[0] as f64, at[1] + offset[1] as f64, at[2] + offset[2] as f64];
                host.add_particle(&ParticleOptions::simple(*key), p, [0.0; 3]);
            }
            EffectKind::Script(_) => {}
        }
    }
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
