//! Bedrock animations (`.animation.json`, as Blockbench exports them) and their sampling.
//!
//! Read: `loop` (true, false, `"hold_on_last_frame"`), `animation_length`, per-bone
//! `rotation` / `position` / `scale` as a constant (number, `[x, y, z]`, Molang strings) or
//! keyframes (`"0.5": [..]`, `{ "pre": .., "post": .., "lerp_mode": "catmullrom" }`, and
//! GeckoLib's `{ "vector": .., "easing": .. }`), `sound_effects`, `particle_effects` and
//! `timeline`. Values come out in the space `geo` builds models in: rotations in radians
//! with x and y turned around, positions with x turned around, as GeckoLib applies them.
//! No mod API types here.

use crate::molang::{self, Context, Expr};
use crate::serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopMode {
    Once,
    Loop,
    Hold,
}

/// One axis' value: a constant, or Molang read each frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Num {
    Const(f32),
    Expr(Expr),
}

impl Num {
    fn get(&self, cx: &mut dyn Context) -> f32 {
        match self {
            Num::Const(v) => *v,
            Num::Expr(e) => molang::eval(e, cx),
        }
    }
}

pub type Vec3 = [Num; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lerp {
    Linear,
    Smooth,
    Step,
    Ease(Easing),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub time: f32,
    /// The value arriving at this keyframe.
    pub pre: Vec3,
    /// The value leaving it (the same as `pre` unless the file split them).
    pub post: Vec3,
    pub lerp: Lerp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub keyframes: Vec<Keyframe>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rotation,
    Position,
    Scale,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoneTrack {
    pub bone: String,
    pub rotation: Option<Channel>,
    pub position: Option<Channel>,
    pub scale: Option<Channel>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EffectKind {
    /// A sound event id.
    Sound(String),
    /// A particle type id, and the locator it comes out of (none: the entity's feet).
    Particle(String, Option<String>),
    /// Molang run on the entity's variables.
    Script(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub time: f32,
    pub kind: EffectKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub name: String,
    pub length: f32,
    pub mode: LoopMode,
    pub bones: Vec<BoneTrack>,
    /// Sorted by time.
    pub effects: Vec<Effect>,
}

/// One bone's change from its rest pose at a moment.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct BonePose {
    /// Radians, added to the rest rotation (z, then y, then x).
    pub rotation: Option<[f32; 3]>,
    /// Texels, added to the bone's place.
    pub position: Option<[f32; 3]>,
    /// Multiplies the bone's size.
    pub scale: Option<[f32; 3]>,
}

impl Animation {
    /// The time in the animation `elapsed` seconds after it started, by its loop mode, and
    /// whether it has finished (a play-once animation past its end).
    pub fn local_time(&self, elapsed: f32) -> (f32, bool) {
        let elapsed = elapsed.max(0.0);
        match self.mode {
            LoopMode::Loop if self.length > 0.0 => (elapsed % self.length, false),
            LoopMode::Loop => (0.0, false),
            LoopMode::Hold => (elapsed.min(self.length), false),
            LoopMode::Once => (elapsed.min(self.length), elapsed > self.length),
        }
    }

    /// Every bone's pose at `time` (seconds into the animation). `cx` answers Molang;
    /// `query.anim_time` should be `time`.
    pub fn sample(&self, time: f32, cx: &mut dyn Context) -> Vec<(String, BonePose)> {
        self.bones
            .iter()
            .map(|t| {
                let pose = BonePose {
                    rotation: t.rotation.as_ref().map(|c| convert(Kind::Rotation, c.sample(time, cx))),
                    position: t.position.as_ref().map(|c| convert(Kind::Position, c.sample(time, cx))),
                    scale: t.scale.as_ref().map(|c| c.sample(time, cx)),
                };
                (t.bone.clone(), pose)
            })
            .collect()
    }

    /// The effects whose time is in `(from, to]`, wrapping round for looping animations.
    pub fn effects_between(&self, from: f32, to: f32) -> Vec<&Effect> {
        if to >= from {
            self.effects.iter().filter(|e| e.time > from && e.time <= to).collect()
        } else {
            // looped past the end
            self.effects.iter().filter(|e| e.time > from || e.time <= to).collect()
        }
    }
}

fn convert(kind: Kind, v: [f32; 3]) -> [f32; 3] {
    match kind {
        Kind::Rotation => [(-v[0]).to_radians(), (-v[1]).to_radians(), v[2].to_radians()],
        Kind::Position => [-v[0], v[1], v[2]],
        Kind::Scale => v,
    }
}

impl Channel {
    pub fn sample(&self, t: f32, cx: &mut dyn Context) -> [f32; 3] {
        let k = &self.keyframes;
        let get = |v: &Vec3, cx: &mut dyn Context| [v[0].get(cx), v[1].get(cx), v[2].get(cx)];
        if k.is_empty() {
            return [0.0; 3];
        }
        if t <= k[0].time {
            return get(&k[0].pre, cx);
        }
        let Some(i) = k.windows(2).position(|w| t >= w[0].time && t < w[1].time) else {
            return get(&k[k.len() - 1].post, cx);
        };
        let (a, b) = (&k[i], &k[i + 1]);
        let span = b.time - a.time;
        let x = if span > 0.0 { (t - a.time) / span } else { 1.0 };
        let from = get(&a.post, cx);
        let to = get(&b.pre, cx);
        // As Blockbench: smooth if either end is smooth, step from a step keyframe.
        let lerp = if a.lerp == Lerp::Smooth || b.lerp == Lerp::Smooth {
            Lerp::Smooth
        } else if a.lerp == Lerp::Step {
            Lerp::Step
        } else if let Lerp::Ease(e) = b.lerp {
            Lerp::Ease(e)
        } else {
            Lerp::Linear
        };
        match lerp {
            Lerp::Step => from,
            Lerp::Linear => lerp3(from, to, x),
            Lerp::Ease(e) => lerp3(from, to, e.apply(x)),
            Lerp::Smooth => {
                let before = if i > 0 { get(&k[i - 1].post, cx) } else { from };
                let after = if i + 2 < k.len() { get(&k[i + 2].pre, cx) } else { to };
                let mut out = [0.0; 3];
                for j in 0..3 {
                    out[j] = catmull_rom(before[j], from[j], to[j], after[j], x);
                }
                out
            }
        }
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], x: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * x, a[1] + (b[1] - a[1]) * x, a[2] + (b[2] - a[2]) * x]
}

fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let (t2, t3) = (t * t, t * t * t);
    0.5 * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

/// GeckoLib's easing names (`easeInOutSine` ...).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    In(Curve),
    Out(Curve),
    InOut(Curve),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Sine,
    Quad,
    Cubic,
    Quart,
    Quint,
    Expo,
    Circ,
    Back,
    Elastic,
    Bounce,
}

impl Curve {
    fn ease_in(self, t: f32) -> f32 {
        use std::f32::consts::PI;
        match self {
            Curve::Sine => 1.0 - (t * PI / 2.0).cos(),
            Curve::Quad => t * t,
            Curve::Cubic => t * t * t,
            Curve::Quart => t.powi(4),
            Curve::Quint => t.powi(5),
            Curve::Expo => {
                if t == 0.0 {
                    0.0
                } else {
                    2f32.powf(10.0 * t - 10.0)
                }
            }
            Curve::Circ => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
            Curve::Back => 2.70158 * t * t * t - 1.70158 * t * t,
            Curve::Elastic => {
                if t == 0.0 || t == 1.0 {
                    t
                } else {
                    -(2f32.powf(10.0 * t - 10.0)) * ((t * 10.0 - 10.75) * (2.0 * PI / 3.0)).sin()
                }
            }
            Curve::Bounce => 1.0 - bounce_out(1.0 - t),
        }
    }
}

fn bounce_out(t: f32) -> f32 {
    let (n, d) = (7.5625, 2.75);
    if t < 1.0 / d {
        n * t * t
    } else if t < 2.0 / d {
        let t = t - 1.5 / d;
        n * t * t + 0.75
    } else if t < 2.5 / d {
        let t = t - 2.25 / d;
        n * t * t + 0.9375
    } else {
        let t = t - 2.625 / d;
        n * t * t + 0.984375
    }
}

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        match self {
            Easing::In(c) => c.ease_in(t),
            Easing::Out(c) => 1.0 - c.ease_in(1.0 - t),
            Easing::InOut(c) => {
                if t < 0.5 {
                    c.ease_in(t * 2.0) / 2.0
                } else {
                    1.0 - c.ease_in((1.0 - t) * 2.0) / 2.0
                }
            }
        }
    }

    fn parse(name: &str) -> Option<Easing> {
        let n = name.to_ascii_lowercase();
        let (wrap, rest): (fn(Curve) -> Easing, &str) = if let Some(r) = n.strip_prefix("easeinout") {
            (Easing::InOut, r)
        } else if let Some(r) = n.strip_prefix("easein") {
            (Easing::In, r)
        } else if let Some(r) = n.strip_prefix("easeout") {
            (Easing::Out, r)
        } else {
            return None;
        };
        let curve = match rest {
            "sine" => Curve::Sine,
            "quad" => Curve::Quad,
            "cubic" => Curve::Cubic,
            "quart" => Curve::Quart,
            "quint" => Curve::Quint,
            "expo" => Curve::Expo,
            "circ" => Curve::Circ,
            "back" => Curve::Back,
            "elastic" => Curve::Elastic,
            "bounce" => Curve::Bounce,
            _ => return None,
        };
        Some(wrap(curve))
    }
}

/// Reads every animation in a `.animation.json` file.
pub fn parse(bytes: &[u8]) -> Result<Vec<Animation>, String> {
    let root: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    let anims = root.get("animations").and_then(Value::as_object).ok_or("no \"animations\" object")?;
    let mut out = Vec::new();
    for (name, a) in anims {
        let a = a.as_object().ok_or_else(|| format!("{name} is not an object"))?;
        out.push(read_animation(name, a).map_err(|e| format!("{name}: {e}"))?);
    }
    Ok(out)
}

fn read_animation(name: &str, a: &Map<String, Value>) -> Result<Animation, String> {
    let mode = match a.get("loop") {
        Some(Value::Bool(true)) => LoopMode::Loop,
        Some(Value::String(s)) if s == "hold_on_last_frame" => LoopMode::Hold,
        Some(Value::String(s)) if s == "true" => LoopMode::Loop,
        _ => LoopMode::Once,
    };
    let mut bones = Vec::new();
    let mut last_key = 0.0f32;
    if let Some(b) = a.get("bones").and_then(Value::as_object) {
        for (bone, t) in b {
            let t = t.as_object().ok_or_else(|| format!("bone {bone} is not an object"))?;
            let mut channel = |key: &str, kind: Kind| -> Result<Option<Channel>, String> {
                match t.get(key) {
                    None => Ok(None),
                    Some(v) => {
                        let c = read_channel(v, kind).map_err(|e| format!("bone {bone} {key}: {e}"))?;
                        if let Some(k) = c.keyframes.last() {
                            last_key = last_key.max(k.time);
                        }
                        Ok(Some(c))
                    }
                }
            };
            let rotation = channel("rotation", Kind::Rotation)?;
            let position = channel("position", Kind::Position)?;
            let scale = channel("scale", Kind::Scale)?;
            bones.push(BoneTrack { bone: bone.clone(), rotation, position, scale });
        }
    }
    let mut effects = Vec::new();
    for (key, field) in [("sound_effects", 0), ("particle_effects", 1), ("timeline", 2)] {
        let Some(map) = a.get(key).and_then(Value::as_object) else { continue };
        for (time, v) in map {
            let time: f32 = time.trim().parse().map_err(|_| format!("{key}: bad time {time}"))?;
            last_key = last_key.max(time);
            let items: Vec<&Value> = match v {
                Value::Array(list) => list.iter().collect(),
                other => vec![other],
            };
            for item in items {
                let kind = match field {
                    0 => item.get("effect").and_then(Value::as_str).map(|s| EffectKind::Sound(namespaced(s))),
                    1 => item.get("effect").and_then(Value::as_str).map(|s| {
                        let locator = item.get("locator").and_then(Value::as_str).map(str::to_string);
                        EffectKind::Particle(namespaced(s), locator)
                    }),
                    _ => item.as_str().map(|s| molang::parse(s).map(EffectKind::Script)).transpose()?,
                };
                if let Some(kind) = kind {
                    effects.push(Effect { time, kind });
                }
            }
        }
    }
    effects.sort_by(|a, b| a.time.total_cmp(&b.time));
    let length = a.get("animation_length").and_then(Value::as_f64).map(|x| x as f32).unwrap_or(last_key);
    Ok(Animation { name: name.to_string(), length, mode, bones, effects })
}

fn namespaced(s: &str) -> String {
    if s.contains(':') { s.to_string() } else { format!("minecraft:{s}") }
}

fn read_channel(v: &Value, kind: Kind) -> Result<Channel, String> {
    // A map of times is keyframes; anything else is one constant keyframe.
    if let Value::Object(m) = v {
        let is_keyframe_map = m.keys().all(|k| k.trim().parse::<f32>().is_ok()) && !m.is_empty();
        if is_keyframe_map {
            let mut keyframes = Vec::new();
            for (time, kv) in m {
                let time: f32 = time.trim().parse().unwrap_or(0.0);
                keyframes.push(read_keyframe(time, kv, kind)?);
            }
            keyframes.sort_by(|a, b| a.time.total_cmp(&b.time));
            return Ok(Channel { keyframes });
        }
    }
    Ok(Channel { keyframes: vec![read_keyframe(0.0, v, kind)?] })
}

fn read_keyframe(time: f32, v: &Value, kind: Kind) -> Result<Keyframe, String> {
    if let Value::Object(m) = v {
        let lerp = match m.get("lerp_mode").or_else(|| m.get("easing")).and_then(Value::as_str) {
            Some("catmullrom") => Lerp::Smooth,
            Some("step") => Lerp::Step,
            Some(name) => Easing::parse(name).map(Lerp::Ease).unwrap_or(Lerp::Linear),
            None => Lerp::Linear,
        };
        let vector = m.get("vector").map(|x| vec3(x, kind)).transpose()?;
        let pre = m.get("pre").map(|x| vec3(x, kind)).transpose()?;
        let post = m.get("post").map(|x| vec3(x, kind)).transpose()?;
        let (pre, post) = match (pre, post, vector) {
            (Some(a), Some(b), _) => (a, b),
            (Some(a), None, v) => (a.clone(), v.unwrap_or(a)),
            (None, Some(b), v) => (v.unwrap_or_else(|| b.clone()), b),
            (None, None, Some(v)) => (v.clone(), v),
            (None, None, None) => return Err(format!("keyframe at {time} has no value")),
        };
        return Ok(Keyframe { time, pre, post, lerp });
    }
    let value = vec3(v, kind)?;
    Ok(Keyframe { time, pre: value.clone(), post: value, lerp: Lerp::Linear })
}

fn vec3(v: &Value, kind: Kind) -> Result<Vec3, String> {
    let num = |x: &Value| -> Result<Num, String> {
        match x {
            Value::Number(n) => Ok(Num::Const(n.as_f64().unwrap_or(0.0) as f32)),
            Value::String(s) => {
                let e = molang::parse(s)?;
                Ok(match e {
                    Expr::Num(n) => Num::Const(n),
                    e => Num::Expr(e),
                })
            }
            Value::Bool(b) => Ok(Num::Const(if *b { 1.0 } else { 0.0 })),
            _ => Err(format!("{x} is not a number")),
        }
    };
    match v {
        Value::Array(a) if a.len() == 3 => Ok([num(&a[0])?, num(&a[1])?, num(&a[2])?]),
        Value::Array(a) if a.len() == 1 => {
            let n = num(&a[0])?;
            Ok([n.clone(), n.clone(), n])
        }
        // A single value: every axis for scale, as Bedrock reads it; for the others too.
        Value::Number(_) | Value::String(_) => {
            let n = num(v)?;
            let _ = kind;
            Ok([n.clone(), n.clone(), n])
        }
        other => Err(format!("{other} is not [x, y, z]")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::molang::MapContext;

    fn one(src: &str) -> Animation {
        parse(src.as_bytes()).unwrap().remove(0)
    }

    #[test]
    fn linear_and_signs() {
        let a = one(r#"{"format_version":"1.8.0","animations":{"animation.t.wave":{"animation_length":1,
            "bones":{"arm":{"rotation":{"0":[0,0,0],"1":[90,40,20]},"position":[2,3,4]}}}}}"#);
        assert_eq!(a.mode, LoopMode::Once);
        let mut cx = MapContext::default();
        let s = a.sample(0.5, &mut cx);
        let r = s[0].1.rotation.unwrap();
        assert!((r[0] + 45f32.to_radians()).abs() < 1e-5);
        assert!((r[1] + 20f32.to_radians()).abs() < 1e-5);
        assert!((r[2] - 10f32.to_radians()).abs() < 1e-5);
        assert_eq!(s[0].1.position.unwrap(), [-2.0, 3.0, 4.0]);
        assert_eq!(a.local_time(2.0), (1.0, true));
    }

    #[test]
    fn step_via_pre_post_and_smooth() {
        let a = one(r#"{"animations":{"a":{"loop":true,"bones":{"b":{
            "position":{"0":[0,0,0],"1":{"pre":[0,0,0],"post":[0,10,0]},"2":[0,10,0]},
            "scale":{"0":{"post":[1,1,1],"lerp_mode":"catmullrom"},"1":{"post":[2,2,2],"lerp_mode":"catmullrom"},"2":[1,1,1]}}}}}}"#);
        assert_eq!(a.mode, LoopMode::Loop);
        assert_eq!(a.length, 2.0);
        let mut cx = MapContext::default();
        assert_eq!(a.bones[0].position.as_ref().unwrap().sample(0.99, &mut cx)[1], 0.0);
        assert_eq!(a.bones[0].position.as_ref().unwrap().sample(1.0, &mut cx)[1], 10.0);
        let mid = a.bones[0].scale.as_ref().unwrap().sample(0.5, &mut cx)[0];
        assert!(mid > 1.4 && mid < 1.7, "{mid}");
        assert_eq!(a.local_time(2.5).0, 0.5);
    }

    #[test]
    fn molang_values_and_effects() {
        let a = one(r#"{"animations":{"animation.t.idle":{"loop":"hold_on_last_frame","animation_length":2,
            "bones":{"head":{"rotation":["math.sin(q.anim_time * 90) * 10", 0, 0]}},
            "sound_effects":{"0.5":{"effect":"entity.villager.yes"}},
            "particle_effects":{"1.0":[{"effect":"minecraft:heart","locator":"top"}]},
            "timeline":{"1.5":"v.done = 1;"}}}}"#);
        assert_eq!(a.mode, LoopMode::Hold);
        let mut cx = MapContext::default();
        cx.queries.insert("anim_time".into(), 1.0);
        let r = a.sample(1.0, &mut cx)[0].1.rotation.unwrap();
        assert!((r[0] + 10f32.to_radians()).abs() < 1e-5);
        assert_eq!(a.effects.len(), 3);
        assert_eq!(a.effects[0].kind, EffectKind::Sound("minecraft:entity.villager.yes".into()));
        assert_eq!(a.effects_between(0.4, 1.0).len(), 2);
        assert_eq!(a.effects_between(1.9, 0.6).len(), 1);
    }

    #[test]
    fn easing() {
        let a = one(r#"{"animations":{"a":{"bones":{"b":{"position":{"0":{"vector":[0,0,0]},"1":{"vector":[0,10,0],"easing":"easeInQuad"}}}}}}}"#);
        let mut cx = MapContext::default();
        let y = a.bones[0].position.as_ref().unwrap().sample(0.5, &mut cx)[1];
        assert!((y - 2.5).abs() < 1e-4, "{y}");
        assert!((Easing::InOut(Curve::Sine).apply(0.5) - 0.5).abs() < 1e-5);
        assert!((Easing::Out(Curve::Bounce).apply(1.0) - 1.0).abs() < 1e-5);
    }
}
