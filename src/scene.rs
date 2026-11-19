//! Scenes: a prepared moment an NPC plays out, with props that appear only for it.
//!
//! A file is `data/<namespace>/felix_scenes/<name>.json`, the scene `<namespace>:<name>`:
//!
//! ```json
//! { "length": 10,
//!   "actor": { "animation": "animation.felix.play_chess" },
//!   "props": [ { "model": "projectfelix:chess_table", "texture": "projectfelix:entity/chess_table",
//!                "offset": [0, 0, 1.2], "turn": 0, "animation": "animation.chess_table.scholars_mate" } ],
//!   "end": [ { "command": "say Checkmate!" } ] }
//! ```
//!
//! `offset` is in blocks from the NPC's feet: right, up and forward as the NPC faces at the start.
//! `turn` turns the prop about its own feet, in degrees. The NPC holds still while the scene
//! plays, facing the player who started it; `end` runs the dialog actions when it is over.
//!
//! More NPCs may play in it: `cast` names their roles, where each stands (`offset`, as props) and
//! which way it faces (`turn`: degrees from the way the lead faces; 180 faces it back). The NPCs
//! nearest the lead take them. `timeline` says, from the start, who (`lead` or a role) says what
//! and plays which animation:
//!
//! ```json
//! { "cast": [ { "role": "bob", "offset": [0, 0, 2], "turn": 180 } ],
//!   "timeline": [ { "at": 0.5, "who": "lead", "say": "Morning, @player!", "animation": "animation.felix.wave" },
//!                 { "at": 2, "who": "bob", "say": "Is it?" } ] }
//! ```
//! No mod API types here.

use crate::dialog::{self, Action};
use crate::serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    pub model: String,
    pub texture: String,
    pub offset: [f32; 3],
    pub turn: f32,
    pub animation: Option<String>,
}

/// A part for another NPC.
#[derive(Clone, Debug, PartialEq)]
pub struct Role {
    pub name: String,
    pub offset: [f32; 3],
    pub turn: f32,
}

/// What one of them does when.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    /// Seconds from the start.
    pub at: f32,
    /// `lead`, or a role's name.
    pub who: String,
    pub say: Option<String>,
    pub animation: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// Seconds.
    pub length: f32,
    pub actor_animation: Option<String>,
    pub props: Vec<Prop>,
    pub end: Vec<Action>,
    pub cast: Vec<Role>,
    pub timeline: Vec<Cue>,
}

fn three(v: Option<&Value>) -> [f32; 3] {
    let mut out = [0.0; 3];
    if let Some(a) = v.and_then(Value::as_array) {
        for (o, x) in out.iter_mut().zip(a) {
            *o = x.as_f64().unwrap_or(0.0) as f32;
        }
    }
    out
}

pub fn parse(bytes: &[u8]) -> Result<Scene, String> {
    let v: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    let length = v.get("length").and_then(Value::as_f64).ok_or("no \"length\"")? as f32;
    if !(length > 0.0 && length <= 600.0) {
        return Err("length must be over 0 and at most 600 seconds".into());
    }
    let actor_animation = v.get("actor").and_then(|a| a.get("animation")).and_then(Value::as_str).map(str::to_string);
    let mut props = Vec::new();
    if let Some(list) = v.get("props").and_then(Value::as_array) {
        for (i, p) in list.iter().enumerate() {
            let model = p.get("model").and_then(Value::as_str).ok_or_else(|| format!("prop {i} has no model"))?.to_string();
            let texture = match p.get("texture").and_then(Value::as_str) {
                Some(t) => t.to_string(),
                None => {
                    let (ns, name) = model.split_once(':').unwrap_or(("minecraft", &model));
                    format!("{ns}:entity/{name}")
                }
            };
            let mut offset = [0.0; 3];
            if let Some(a) = p.get("offset").and_then(Value::as_array) {
                for (o, x) in offset.iter_mut().zip(a) {
                    *o = x.as_f64().unwrap_or(0.0) as f32;
                }
            }
            props.push(Prop {
                model,
                texture,
                offset,
                turn: p.get("turn").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                animation: p.get("animation").and_then(Value::as_str).map(str::to_string),
            });
        }
    }
    let end = dialog::read_actions(v.get("end")).map_err(|e| format!("end: {e}"))?;
    let mut cast = Vec::new();
    for (i, r) in v.get("cast").and_then(Value::as_array).cloned().unwrap_or_default().iter().enumerate() {
        let name = r.get("role").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or(format!("cast {}: no role", i + 1))?.to_string();
        if name == "lead" || cast.iter().any(|x: &Role| x.name == name) {
            return Err(format!("cast {}: the role {name} is taken", i + 1));
        }
        cast.push(Role { name, offset: three(r.get("offset")), turn: r.get("turn").and_then(Value::as_f64).unwrap_or(180.0) as f32 });
    }
    let mut timeline = Vec::new();
    for (i, c) in v.get("timeline").and_then(Value::as_array).cloned().unwrap_or_default().iter().enumerate() {
        let at = c.get("at").and_then(Value::as_f64).ok_or(format!("timeline {}: no \"at\"", i + 1))? as f32;
        let who = c.get("who").and_then(Value::as_str).unwrap_or("lead").to_string();
        if who != "lead" && !cast.iter().any(|r| r.name == who) {
            return Err(format!("timeline {}: nobody plays {who}", i + 1));
        }
        let s = |k: &str| c.get(k).and_then(Value::as_str).map(str::to_string).filter(|x| !x.trim().is_empty());
        timeline.push(Cue { at: at.clamp(0.0, length), who, say: s("say"), animation: s("animation") });
    }
    timeline.sort_by(|a, b| a.at.total_cmp(&b.at));
    Ok(Scene { length, actor_animation, props, end, cast, timeline })
}

/// Where a prop's feet are, in blocks from the NPC's feet in the frame the renderer draws the
/// NPC in (Bedrock's x turned around, the front towards -z): right is +x, forward is -z.
pub fn prop_place(offset: [f32; 3]) -> [f32; 3] {
    [offset[0], offset[1], -offset[2]]
}

/// Where a role stands in the world: `offset` (right, up, forward) from the lead at `at` facing
/// `yaw` (Minecraft's degrees).
pub fn place_of(at: [f64; 3], yaw: f32, offset: [f32; 3]) -> [f64; 3] {
    let r = (yaw as f64).to_radians();
    let (fx, fz) = (-r.sin(), r.cos());
    let (rx, rz) = (-r.cos(), -r.sin());
    let [x, y, z] = offset.map(f64::from);
    [at[0] + rx * x + fx * z, at[1] + y, at[2] + rz * x + fz * z]
}

/// The yaw (degrees, Minecraft's: 0 faces south, 90 west) from `from` towards `to`.
pub fn yaw_towards(from: [f64; 3], to: [f64; 3]) -> f32 {
    let (dx, dz) = (to[0] - from[0], to[2] - from[2]);
    (-dx.atan2(dz)).to_degrees() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_scene() {
        let s = parse(br#"{ "length": 10, "actor": { "animation": "animation.felix.play" },
            "props": [ { "model": "projectfelix:chess_table", "offset": [0, 0, 1.2], "turn": 90, "animation": "animation.t.game" } ],
            "end": [ { "command": "say done" }, { "give": "minecraft:apple" } ] }"#)
        .unwrap();
        assert_eq!(s.length, 10.0);
        assert_eq!(s.actor_animation.as_deref(), Some("animation.felix.play"));
        assert_eq!(s.props[0].texture, "projectfelix:entity/chess_table");
        assert_eq!(s.props[0].offset, [0.0, 0.0, 1.2]);
        assert_eq!(s.end.len(), 2);
        assert_eq!(prop_place([1.0, 0.0, 2.0]), [1.0, 0.0, -2.0]);
    }

    #[test]
    fn a_cast_and_a_timeline() {
        let s = parse(br#"{ "length": 6, "cast": [ { "role": "bob", "offset": [1, 0, 2] } ],
            "timeline": [ { "at": 3, "who": "bob", "say": "Hi" }, { "at": 1, "say": "Hello", "animation": "a.w" }, { "at": 99, "who": "lead" } ] }"#)
        .unwrap();
        assert_eq!(s.cast[0].turn, 180.0);
        assert_eq!(s.timeline[0].who, "lead");
        assert_eq!(s.timeline[1].say.as_deref(), Some("Hi"));
        assert_eq!(s.timeline[2].at, 6.0, "within the scene");
        assert!(parse(br#"{ "length": 6, "timeline": [ { "at": 1, "who": "ann" } ] }"#).unwrap_err().contains("ann"));
        assert!(parse(br#"{ "length": 6, "cast": [ { "role": "lead" } ] }"#).is_err());
        // Facing south (yaw 0): forward is +z, right is -x.
        let p = place_of([10.0, 64.0, 10.0], 0.0, [1.0, 0.0, 2.0]);
        assert!((p[0] - 9.0).abs() < 1e-9 && (p[2] - 12.0).abs() < 1e-9, "{p:?}");
        let samples: [&[u8]; 2] = [include_bytes!("../data/projectfelix/felix_scenes/chess.json"), include_bytes!("../data/projectfelix/felix_scenes/greeting.json")];
        for f in samples {
            parse(f).unwrap();
        }
    }

    #[test]
    fn refuses_bad_scenes() {
        assert!(parse(br#"{ "props": [] }"#).is_err());
        assert!(parse(br#"{ "length": 0 }"#).is_err());
        assert!(parse(br#"{ "length": 5, "props": [ { "texture": "x" } ] }"#).is_err());
    }

    #[test]
    fn yaw() {
        // Facing south (+z) is 0, west (-x) is 90, north is 180 or -180, east is -90.
        assert!((yaw_towards([0.0; 3], [0.0, 0.0, 5.0]) - 0.0).abs() < 1e-4);
        assert!((yaw_towards([0.0; 3], [-5.0, 0.0, 0.0]) - 90.0).abs() < 1e-4);
        assert!((yaw_towards([0.0; 3], [5.0, 0.0, 0.0]) + 90.0).abs() < 1e-4);
    }
}
