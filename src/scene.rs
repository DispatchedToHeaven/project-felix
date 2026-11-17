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

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// Seconds.
    pub length: f32,
    pub actor_animation: Option<String>,
    pub props: Vec<Prop>,
    pub end: Vec<Action>,
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
    Ok(Scene { length, actor_animation, props, end })
}

/// Where a prop's feet are, in blocks from the NPC's feet in the frame the renderer draws the
/// NPC in (Bedrock's x turned around, the front towards -z): right is +x, forward is -z.
pub fn prop_place(offset: [f32; 3]) -> [f32; 3] {
    [offset[0], offset[1], -offset[2]]
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
