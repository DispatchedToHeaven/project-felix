//! An animation as the editor holds it: keyframes per bone and channel, values as Blockbench
//! shows them (rotation in degrees, position in texels, scale), and its way to and from a Bedrock
//! `.animation.json` that Blockbench opens. No mod API types here.

use crate::anim::{self, Animation, BoneTrack, Channel, Effect, Keyframe, Lerp, LoopMode, Num};
use crate::molang::{self, Expr, MapContext};
use crate::serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ease {
    Linear,
    Smooth,
    /// Holds the value until the next keyframe.
    Step,
}

impl Ease {
    pub fn next(self) -> Ease {
        match self {
            Ease::Linear => Ease::Smooth,
            Ease::Smooth => Ease::Step,
            Ease::Step => Ease::Linear,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Ease::Linear => "Linear",
            Ease::Smooth => "Smooth",
            Ease::Step => "Step",
        }
    }
}

/// Rotation, position, scale.
pub const CHANNELS: [&str; 3] = ["rotation", "position", "scale"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub time: f32,
    pub value: [f32; 3],
    pub ease: Ease,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Track {
    pub bone: String,
    /// Keyframes by channel (rotation, position, scale), each sorted by time.
    pub channels: [Vec<Key>; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditAnim {
    pub name: String,
    pub length: f32,
    pub mode: LoopMode,
    pub tracks: Vec<Track>,
    /// Sounds, particles and timeline scripts.
    pub effects: Vec<Effect>,
    /// Keyframes whose values are Molang in the file, kept as they came until changed here.
    pub molang: Vec<Formula>,
}

/// A keyframe's Molang values (by axis; none: a plain number).
#[derive(Clone, Debug, PartialEq)]
pub struct Formula {
    pub bone: String,
    pub channel: usize,
    pub time: f32,
    pub axes: [Option<Expr>; 3],
}

/// Times snap to this (seconds).
pub const SNAP: f32 = 0.05;

pub fn snap(t: f32) -> f32 {
    (t / SNAP).round() * SNAP
}

fn same(a: f32, b: f32) -> bool {
    (a - b).abs() < SNAP / 2.0
}

impl EditAnim {
    pub fn new(name: &str) -> EditAnim {
        EditAnim { name: name.to_string(), length: 2.0, mode: LoopMode::Loop, tracks: Vec::new(), effects: Vec::new(), molang: Vec::new() }
    }

    pub fn track(&self, bone: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.bone.eq_ignore_ascii_case(bone))
    }

    fn track_mut(&mut self, bone: &str) -> &mut Track {
        if let Some(i) = self.tracks.iter().position(|t| t.bone.eq_ignore_ascii_case(bone)) {
            return &mut self.tracks[i];
        }
        self.tracks.push(Track { bone: bone.to_string(), ..Default::default() });
        self.tracks.last_mut().unwrap()
    }

    /// The keyframe of `bone`'s channel at `time`, if there is one.
    pub fn key_at(&self, bone: &str, channel: usize, time: f32) -> Option<&Key> {
        self.track(bone)?.channels[channel].iter().find(|k| same(k.time, time))
    }

    /// The Molang of `bone`'s keyframe at `time` in `channel`, if the file had any.
    pub fn formula(&self, bone: &str, channel: usize, time: f32) -> Option<&[Option<Expr>; 3]> {
        self.molang.iter().find(|f| f.channel == channel && same(f.time, time) && f.bone.eq_ignore_ascii_case(bone)).map(|f| &f.axes)
    }

    fn drop_formula(&mut self, bone: &str, channel: usize, time: f32) {
        self.molang.retain(|f| !(f.channel == channel && same(f.time, time) && f.bone.eq_ignore_ascii_case(bone)));
    }

    /// Sets (or adds) the keyframe at `time`, keeping its ease if it existed.
    pub fn set_key(&mut self, bone: &str, channel: usize, time: f32, value: [f32; 3]) {
        let time = snap(time).clamp(0.0, self.length);
        self.drop_formula(bone, channel, time);
        let keys = &mut self.track_mut(bone).channels[channel];
        match keys.iter_mut().find(|k| same(k.time, time)) {
            Some(k) => k.value = value,
            None => {
                keys.push(Key { time, value, ease: Ease::Linear });
                keys.sort_by(|a, b| a.time.total_cmp(&b.time));
            }
        }
    }

    pub fn remove_key(&mut self, bone: &str, channel: usize, time: f32) -> bool {
        self.drop_formula(bone, channel, time);
        let keys = &mut self.track_mut(bone).channels[channel];
        let before = keys.len();
        keys.retain(|k| !same(k.time, time));
        before != keys.len()
    }

    pub fn set_ease(&mut self, bone: &str, channel: usize, time: f32, ease: Ease) {
        if let Some(k) = self.track_mut(bone).channels[channel].iter_mut().find(|k| same(k.time, time)) {
            k.ease = ease;
        }
    }

    /// Adds a sound or a particle at `time`.
    pub fn add_effect(&mut self, time: f32, kind: anim::EffectKind) {
        let time = snap(time).clamp(0.0, self.length);
        self.effects.push(Effect { time, kind });
        self.effects.sort_by(|a, b| a.time.total_cmp(&b.time));
    }

    /// Removes every sound, particle and script at `time`; true if there were any.
    pub fn remove_effects_at(&mut self, time: f32) -> bool {
        let before = self.effects.len();
        self.effects.retain(|e| !same(e.time, time));
        before != self.effects.len()
    }

    /// The effects at `time`, as short text for the editor.
    pub fn effects_at(&self, time: f32) -> Vec<String> {
        self.effects
            .iter()
            .filter(|e| same(e.time, time))
            .filter_map(|e| match &e.kind {
                anim::EffectKind::Sound(s) => Some(format!("sound {}", s.trim_start_matches("minecraft:"))),
                anim::EffectKind::Particle(p, l) => Some(format!("particle {}{}", p.trim_start_matches("minecraft:"), l.as_ref().map(|l| format!(" @{l}")).unwrap_or_default())),
                anim::EffectKind::Script(e) => Some(format!("script {}", molang::show(e))),
            })
            .collect()
    }

    /// Every keyframe time of `bone` (any channel), for the timeline.
    pub fn times(&self, bone: &str) -> Vec<f32> {
        let mut t: Vec<f32> = self.track(bone).map(|t| t.channels.iter().flatten().map(|k| k.time).collect()).unwrap_or_default();
        t.sort_by(f32::total_cmp);
        t.dedup_by(|a, b| same(*a, *b));
        t
    }

    /// `keys` to play, with the Molang of `formulas` (this bone and channel's) where the file had it.
    fn channel(keys: &[Key], formulas: &[&Formula]) -> Channel {
        let num = |v: [f32; 3], t: f32| -> [Num; 3] {
            let f = formulas.iter().find(|f| same(f.time, t)).map(|f| &f.axes);
            std::array::from_fn(|i| match f.and_then(|f| f[i].clone()) {
                Some(e) => Num::Expr(e),
                None => Num::Const(v[i]),
            })
        };
        let keyframes = keys
            .iter()
            .enumerate()
            .map(|(j, k)| {
                let lerp = match k.ease {
                    Ease::Linear => Lerp::Linear,
                    Ease::Smooth => Lerp::Smooth,
                    Ease::Step => Lerp::Step,
                };
                // After a step the next keyframe arrives with the step's value, as the file has it.
                let pre = if j > 0 && keys[j - 1].ease == Ease::Step { num(keys[j - 1].value, keys[j - 1].time) } else { num(k.value, k.time) };
                Keyframe { time: k.time, pre, post: num(k.value, k.time), lerp }
            })
            .collect();
        Channel { keyframes }
    }

    /// `bone`'s channel at `time`, as Blockbench shows it; the rest value with no keyframes.
    pub fn value_at(&self, bone: &str, channel: usize, time: f32) -> [f32; 3] {
        let rest = if channel == 2 { [1.0; 3] } else { [0.0; 3] };
        match self.track(bone).map(|t| &t.channels[channel]) {
            Some(keys) if !keys.is_empty() => Self::channel(keys, &[]).sample(time, &mut MapContext::default()),
            _ => rest,
        }
    }

    /// The animation to play: what the runtime samples.
    pub fn to_animation(&self) -> Animation {
        let bones = self
            .tracks
            .iter()
            .filter(|t| t.channels.iter().any(|c| !c.is_empty()))
            .map(|t| {
                let ch = |i: usize| {
                    let f: Vec<&Formula> = self.molang.iter().filter(|f| f.channel == i && f.bone.eq_ignore_ascii_case(&t.bone)).collect();
                    (!t.channels[i].is_empty()).then(|| Self::channel(&t.channels[i], &f))
                };
                BoneTrack { bone: t.bone.clone(), rotation: ch(0), position: ch(1), scale: ch(2) }
            })
            .collect();
        Animation { name: self.name.clone(), length: self.length, mode: self.mode, bones, effects: self.effects.clone() }
    }

    /// The `.animation.json` Blockbench opens: step keyframes become the next keyframe's
    /// `pre`/`post`, smooth ones `lerp_mode: catmullrom`.
    pub fn to_bedrock(&self) -> String {
        let time = |t: f32| format!("{}", (t * 100.0).round() / 100.0);
        let num = |v: f32| json!((v * 1000.0).round() / 1000.0);
        // A keyframe's values; Molang as the file had it.
        let vec = |v: [f32; 3], f: Option<&[Option<Expr>; 3]>| {
            let axis = |i: usize| match f.and_then(|f| f[i].as_ref()) {
                Some(e) => json!(molang::show(e)),
                None => num(v[i]),
            };
            json!([axis(0), axis(1), axis(2)])
        };
        let mut bones = Map::new();
        for t in &self.tracks {
            let mut b = Map::new();
            for (i, keys) in t.channels.iter().enumerate() {
                if keys.is_empty() {
                    continue;
                }
                let mut frames = Map::new();
                for (j, k) in keys.iter().enumerate() {
                    let prev_step = j > 0 && keys[j - 1].ease == Ease::Step;
                    let here = self.formula(&t.bone, i, k.time);
                    let v = if prev_step {
                        let mut m = Map::new();
                        m.insert("pre".into(), vec(keys[j - 1].value, self.formula(&t.bone, i, keys[j - 1].time)));
                        m.insert("post".into(), vec(k.value, here));
                        if k.ease == Ease::Smooth {
                            m.insert("lerp_mode".into(), json!("catmullrom"));
                        }
                        Value::Object(m)
                    } else if k.ease == Ease::Smooth {
                        json!({ "post": vec(k.value, here), "lerp_mode": "catmullrom" })
                    } else {
                        vec(k.value, here)
                    };
                    frames.insert(time(k.time), v);
                }
                b.insert(CHANNELS[i].into(), Value::Object(frames));
            }
            if !b.is_empty() {
                bones.insert(t.bone.clone(), Value::Object(b));
            }
        }
        let mut a = Map::new();
        a.insert(
            "loop".into(),
            match self.mode {
                LoopMode::Loop => json!(true),
                LoopMode::Hold => json!("hold_on_last_frame"),
                LoopMode::Once => json!(false),
            },
        );
        a.insert("animation_length".into(), num(self.length));
        a.insert("bones".into(), Value::Object(bones));
        // Several at one time are a list.
        fn put(map: &mut Map<String, Value>, key: String, v: Value) {
            match map.remove(&key) {
                None => {
                    map.insert(key, v);
                }
                Some(Value::Array(mut list)) => {
                    list.push(v);
                    map.insert(key, Value::Array(list));
                }
                Some(one) => {
                    map.insert(key, json!([one, v]));
                }
            }
        }
        let mut sounds = Map::new();
        let mut particles = Map::new();
        let mut timeline = Map::new();
        for e in &self.effects {
            match &e.kind {
                anim::EffectKind::Sound(s) => {
                    put(&mut sounds, time(e.time), json!({ "effect": s }));
                }
                anim::EffectKind::Particle(p, l) => {
                    let mut m = Map::new();
                    m.insert("effect".into(), json!(p));
                    if let Some(l) = l {
                        m.insert("locator".into(), json!(l));
                    }
                    put(&mut particles, time(e.time), Value::Object(m));
                }
                anim::EffectKind::Script(s) => {
                    put(&mut timeline, time(e.time), json!(molang::show(s)));
                }
            }
        }
        if !sounds.is_empty() {
            a.insert("sound_effects".into(), Value::Object(sounds));
        }
        if !particles.is_empty() {
            a.insert("particle_effects".into(), Value::Object(particles));
        }
        if !timeline.is_empty() {
            a.insert("timeline".into(), Value::Object(timeline));
        }
        let mut anims = Map::new();
        anims.insert(self.name.clone(), Value::Object(a));
        let root = json!({ "format_version": "1.8.0", "animations": anims });
        crate::serde_json::to_string_pretty(&root).unwrap_or_default()
    }

    /// An animation read from a file, made editable: Molang values show as read at time 0 and are
    /// kept (until their keyframe is changed), easings
    /// other than smooth become linear, and a keyframe whose `pre` differs from the one before
    /// makes that one a step.
    pub fn from_animation(a: &Animation) -> EditAnim {
        let mut cx = MapContext::default();
        let mut get = |v: &[Num; 3]| {
            let mut out = [0.0; 3];
            for (o, n) in out.iter_mut().zip(v) {
                *o = match n {
                    Num::Const(c) => *c,
                    Num::Expr(e) => crate::molang::eval(e, &mut cx),
                };
            }
            out
        };
        let mut tracks = Vec::new();
        let mut molang = Vec::new();
        for t in &a.bones {
            let mut track = Track { bone: t.bone.clone(), ..Default::default() };
            for (i, c) in [&t.rotation, &t.position, &t.scale].into_iter().enumerate() {
                let Some(c) = c else { continue };
                let mut keys: Vec<Key> = Vec::new();
                for k in &c.keyframes {
                    let pre = get(&k.pre);
                    let post = get(&k.post);
                    if let Some(prev) = keys.last_mut()
                        && pre != post
                        && prev.value == pre
                    {
                        prev.ease = Ease::Step;
                    }
                    let ease = if k.lerp == Lerp::Smooth { Ease::Smooth } else { Ease::Linear };
                    keys.push(Key { time: k.time, value: post, ease });
                    if k.post.iter().any(|n| matches!(n, Num::Expr(_))) {
                        let axes = std::array::from_fn(|j| match &k.post[j] {
                            Num::Expr(e) => Some(e.clone()),
                            Num::Const(_) => None,
                        });
                        molang.push(Formula { bone: t.bone.clone(), channel: i, time: k.time, axes });
                    }
                }
                track.channels[i] = keys;
            }
            tracks.push(track);
        }
        EditAnim { name: a.name.clone(), length: a.length.max(SNAP), mode: a.mode, tracks, effects: a.effects.clone(), molang }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_at_one_time_are_all_written() {
        let mut a = EditAnim::new("animation.felix.test");
        a.length = 1.0;
        a.add_effect(0.5, anim::EffectKind::Sound("minecraft:a".into()));
        a.add_effect(0.5, anim::EffectKind::Sound("minecraft:b".into()));
        let text = a.to_bedrock();
        assert!(text.contains("minecraft:a") && text.contains("minecraft:b"), "{text}");
        let back = anim::parse(text.as_bytes()).unwrap();
        assert_eq!(back[0].effects.len(), 2);
    }

    #[test]
    fn molang_and_scripts_are_kept() {
        let file = r#"{"animations":{"animation.t.m":{"loop":true,"animation_length":1,
            "bones":{"head":{"rotation":{"0":["math.sin(q.anim_time * 360) * 10", 5, 0],"1":[0,0,0]}}},
            "timeline":{"0.5":"v.done = 1;"}}}}"#;
        let a = EditAnim::from_animation(&anim::parse(file.as_bytes()).unwrap()[0]);
        assert_eq!(a.effects_at(0.5), vec!["script v.done = 1;"]);
        let back = anim::parse(a.to_bedrock().as_bytes()).unwrap();
        assert_eq!(back, anim::parse(file.as_bytes()).unwrap());
        // The preview plays the Molang too.
        let played = a.to_animation().bones[0].rotation.as_ref().unwrap().keyframes[0].post[0].clone();
        assert!(matches!(played, Num::Expr(_)));
        // A keyframe changed here is a number from then on.
        let mut b = a.clone();
        b.set_key("head", 0, 0.0, [1.0, 2.0, 3.0]);
        assert!(b.formula("head", 0, 0.0).is_none() && !b.to_bedrock().contains("math"));
        assert!(b.remove_effects_at(0.5) && !b.to_bedrock().contains("timeline"));
    }

    #[test]
    fn keys_and_values() {
        let mut a = EditAnim::new("animation.t.test");
        a.set_key("rightArm", 0, 0.0, [0.0; 3]);
        a.set_key("rightArm", 0, 1.02, [90.0, 0.0, 0.0]);
        assert_eq!(a.track("rightarm").unwrap().channels[0].len(), 2);
        assert_eq!(a.key_at("rightArm", 0, 1.0).unwrap().value[0], 90.0);
        let mid = a.value_at("rightArm", 0, 0.5);
        assert!((mid[0] - 45.0).abs() < 1e-3, "{mid:?}");
        assert_eq!(a.value_at("head", 2, 0.5), [1.0; 3]);
        a.set_ease("rightArm", 0, 0.0, Ease::Step);
        assert_eq!(a.value_at("rightArm", 0, 0.9)[0], 0.0);
        assert!(a.remove_key("rightArm", 0, 1.0));
        assert_eq!(a.times("rightArm"), vec![0.0]);
    }

    #[test]
    fn effects_round_trip() {
        let mut a = EditAnim::new("animation.t.fx");
        a.add_effect(0.52, anim::EffectKind::Sound("minecraft:entity.villager.yes".into()));
        a.add_effect(1.0, anim::EffectKind::Particle("minecraft:heart".into(), Some("hand".into())));
        assert_eq!(a.effects_at(0.5), vec!["sound entity.villager.yes"]);
        let back = EditAnim::from_animation(&anim::parse(a.to_bedrock().as_bytes()).unwrap()[0]);
        assert_eq!(back.effects_at(1.0), vec!["particle heart @hand"]);
        assert!(a.remove_effects_at(0.5));
        assert!(a.effects_at(0.5).is_empty());
    }

    #[test]
    fn bedrock_round_trip() {
        let mut a = EditAnim::new("animation.t.round");
        a.length = 1.5;
        a.mode = LoopMode::Hold;
        a.set_key("head", 0, 0.0, [0.0, 10.0, 0.0]);
        a.set_key("head", 0, 0.5, [20.0, 10.0, 0.0]);
        a.set_key("head", 0, 1.0, [0.0, 0.0, 5.0]);
        a.set_ease("head", 0, 0.5, Ease::Step);
        a.set_ease("head", 0, 0.0, Ease::Smooth);
        a.set_key("body", 1, 0.25, [0.0, 2.0, 0.0]);
        let text = a.to_bedrock();
        let parsed = anim::parse(text.as_bytes()).unwrap();
        let back = EditAnim::from_animation(&parsed[0]);
        assert_eq!(back.length, 1.5);
        assert_eq!(back.mode, LoopMode::Hold);
        let head = &back.track("head").unwrap().channels[0];
        assert_eq!(head.iter().map(|k| k.ease).collect::<Vec<_>>(), [Ease::Smooth, Ease::Step, Ease::Linear]);
        // The game plays the file as the editor showed it.
        for t in [0.2f32, 0.7, 0.99] {
            let mut cx = MapContext::default();
            let played = Channel::sample(&parsed[0].bones.iter().find(|b| b.bone == "head").unwrap().rotation.clone().unwrap(), t, &mut cx);
            let shown = a.value_at("head", 0, t);
            for i in 0..3 {
                assert!((played[i] - shown[i]).abs() < 1e-3, "t {t}: {played:?} vs {shown:?}");
            }
        }
    }
}
