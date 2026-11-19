//! Building plans: what a builder NPC builds, layer by layer. No mod API types here.
//!
//! A plan is `data/<namespace>/felix_plans/<name>.json`, or one saved in the game from what
//! stands in the world (`/felix plan save`, `felixplan:<name>`):
//!
//! ```json
//! { "name": "fence", "palette": { "F": "minecraft:oak_fence", ".": "minecraft:air" },
//!   "layers": [ [ "FFF", "F.F", "FFF" ] ] }
//! ```
//!
//! `layers` go from the bottom up; in each, rows go away from the builder's site (south), and
//! letters east; `sink` sets it that many blocks into the ground. A space is left as it is; a letter of `minecraft:air` is cleared. A block state may
//! have properties (`minecraft:oak_door[half=lower,facing=south]`); they turn with the plan.
//! The upper half of a door and the head of a bed come with their other half: no item for them.

use crate::serde_json::{Map, Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub name: String,
    /// How far below the site it starts (1: its bottom layer replaces the ground).
    pub sink: i32,
    pub palette: BTreeMap<char, String>,
    pub layers: Vec<Vec<String>>,
}

/// One block of a plan: where (from its corner: east, up, south), and what (None: cleared).
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub at: [i32; 3],
    pub state: Option<String>,
}

pub fn parse(bytes: &[u8]) -> Result<Plan, String> {
    let v: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    read(&v)
}

pub fn read(v: &Value) -> Result<Plan, String> {
    let pal = v.get("palette").and_then(Value::as_object).ok_or("no \"palette\"")?;
    let mut palette = BTreeMap::new();
    for (k, s) in pal {
        let mut chars = k.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else { return Err(format!("palette key {k:?} is not one letter")) };
        if c == ' ' {
            return Err("a space is \"leave it\": it can't be in the palette".into());
        }
        let s = s.as_str().ok_or(format!("palette {k}: a block state"))?;
        if !s.contains(':') {
            return Err(format!("palette {k}: {s} is not a block id"));
        }
        palette.insert(c, s.to_string());
    }
    let layers: Vec<Vec<String>> = v
        .get("layers")
        .and_then(Value::as_array)
        .ok_or("no \"layers\"")?
        .iter()
        .map(|l| l.as_array().map(|rows| rows.iter().filter_map(Value::as_str).map(str::to_string).collect()).ok_or("a layer is a list of rows"))
        .collect::<Result<_, _>>()?;
    for (y, l) in layers.iter().enumerate() {
        for row in l {
            for c in row.chars().filter(|c| *c != ' ') {
                if !palette.contains_key(&c) {
                    return Err(format!("layer {}: {c} is not in the palette", y + 1));
                }
            }
        }
    }
    if layers.is_empty() {
        return Err("no layers".into());
    }
    Ok(Plan { name: v.get("name").and_then(Value::as_str).unwrap_or("plan").to_string(), sink: v.get("sink").and_then(Value::as_i64).unwrap_or(0).clamp(0, 16) as i32, palette, layers })
}

impl Plan {
    /// Its size: east, up, south.
    pub fn size(&self) -> [i32; 3] {
        let w = self.layers.iter().flat_map(|l| l.iter().map(|r| r.chars().count())).max().unwrap_or(0) as i32;
        let d = self.layers.iter().map(Vec::len).max().unwrap_or(0) as i32;
        [w, self.layers.len() as i32, d]
    }

    /// Its blocks in the order they are built: from the bottom up, row by row.
    pub fn cells(&self) -> Vec<Cell> {
        let mut out = Vec::new();
        for (y, l) in self.layers.iter().enumerate() {
            for (z, row) in l.iter().enumerate() {
                for (x, c) in row.chars().enumerate() {
                    if c == ' ' {
                        continue;
                    }
                    let state = self.palette.get(&c).cloned().filter(|s| s != "minecraft:air");
                    out.push(Cell { at: [x as i32, y as i32, z as i32], state });
                }
            }
        }
        // Clearing first in each layer, then the blocks.
        out.sort_by_key(|c| (c.at[1], c.state.is_some()));
        out
    }

    pub fn to_json(&self) -> Value {
        let mut pal = Map::new();
        for (c, s) in &self.palette {
            pal.insert(c.to_string(), json!(s));
        }
        json!({ "name": self.name, "sink": self.sink, "palette": Value::Object(pal), "layers": self.layers })
    }

    /// A plan of what stands in a box: `blocks[y][z][x]` block states (air too).
    pub fn capture(name: &str, blocks: &[Vec<Vec<String>>]) -> Result<Plan, String> {
        const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789#$%&*+-=?@^~";
        let mut palette: BTreeMap<char, String> = BTreeMap::new();
        palette.insert('.', "minecraft:air".into());
        let mut letter_of: BTreeMap<String, char> = BTreeMap::new();
        letter_of.insert("minecraft:air".into(), '.');
        let mut letters = LETTERS.chars();
        let mut layers = Vec::new();
        for l in blocks {
            let mut rows = Vec::new();
            for r in l {
                let mut row = String::new();
                for s in r {
                    let c = match letter_of.get(s) {
                        Some(c) => *c,
                        None => {
                            let c = letters.next().ok_or("more kinds of blocks than a plan holds (75)")?;
                            letter_of.insert(s.clone(), c);
                            palette.insert(c, s.clone());
                            c
                        }
                    };
                    row.push(c);
                }
                rows.push(row);
            }
            layers.push(rows);
        }
        Ok(Plan { name: name.to_string(), sink: 0, palette, layers })
    }
}

/// Where a plan's cell `at` goes, turned `rot` quarter turns clockwise (seen from above) about
/// its corner.
pub fn turn(at: [i32; 3], rot: u8) -> [i32; 3] {
    let [x, y, z] = at;
    match rot % 4 {
        0 => [x, y, z],
        1 => [-z, y, x],
        2 => [-x, y, -z],
        _ => [z, y, -x],
    }
}

/// A block state turned `rot` quarter turns clockwise: its `facing`, `axis`, `rotation`.
pub fn turn_state(state: &str, rot: u8) -> String {
    let rot = rot % 4;
    if rot == 0 {
        return state.to_string();
    }
    let Some((id, props)) = state.split_once('[') else { return state.to_string() };
    let props = props.trim_end_matches(']');
    let dirs = ["north", "east", "south", "west"];
    let turned: Vec<String> = props
        .split(',')
        .map(|p| {
            let Some((k, v)) = p.split_once('=') else { return p.to_string() };
            let v2 = match k {
                "facing" | "horizontal_facing" => dirs.iter().position(|d| *d == v).map_or(v.to_string(), |i| dirs[(i + rot as usize) % 4].to_string()),
                "axis" if rot % 2 == 1 => match v {
                    "x" => "z".into(),
                    "z" => "x".into(),
                    o => o.to_string(),
                },
                "rotation" => v.parse::<u8>().map_or(v.to_string(), |r| ((r + rot * 4) % 16).to_string()),
                _ => v.to_string(),
            };
            format!("{k}={v2}")
        })
        .collect();
    format!("{id}[{}]", turned.join(","))
}

/// The item a block state takes to place (none: it comes with another half, or nothing does).
pub fn item_for(state: &str) -> Option<String> {
    let (id, props) = state.split_once('[').unwrap_or((state, ""));
    if props.contains("half=upper") || props.contains("part=head") || id == "minecraft:air" {
        return None;
    }
    Some(match id {
        "minecraft:wall_torch" => "minecraft:torch".into(),
        "minecraft:redstone_wall_torch" => "minecraft:redstone_torch".into(),
        "minecraft:dirt_path" | "minecraft:farmland" => "minecraft:dirt".into(),
        "minecraft:redstone_wire" => "minecraft:redstone".into(),
        "minecraft:water" => "minecraft:water_bucket".into(),
        "minecraft:fire" | "minecraft:piston_head" | "minecraft:moving_piston" => return None,
        other if other.ends_with("_wall_sign") => other.replace("_wall_sign", "_sign"),
        other if other.ends_with("_wall_banner") => other.replace("_wall_banner", "_banner"),
        other => other.to_string(),
    })
}

/// The block id of a state (no properties).
pub fn block_id(state: &str) -> &str {
    state.split_once('[').map_or(state, |s| s.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_read_and_turn() {
        let p = parse(br#"{ "name": "hut", "palette": { "P": "minecraft:oak_planks", ".": "minecraft:air", "D": "minecraft:oak_door[facing=south,half=lower]", "d": "minecraft:oak_door[facing=south,half=upper]" },
            "layers": [ [ "PPP", "PPP" ], [ "P.D", "P P" ], [ ". d" ] ] }"#).unwrap();
        assert_eq!(p.size(), [3, 3, 2]);
        let cells = p.cells();
        assert_eq!(cells.len(), 6 + 5 + 2);
        assert!(cells[6].state.is_none() && cells[6].at == [1, 1, 0]);
        assert_eq!(item_for("minecraft:oak_door[facing=south,half=upper]"), None);
        assert_eq!(item_for("minecraft:oak_door[facing=south,half=lower]").as_deref(), Some("minecraft:oak_door"));
        assert_eq!(item_for("minecraft:wall_torch[facing=east]").as_deref(), Some("minecraft:torch"));
        assert_eq!(turn([2, 0, 1], 1), [-1, 0, 2]);
        assert_eq!(turn(turn([2, 0, 1], 1), 3), [2, 0, 1]);
        assert_eq!(turn_state("minecraft:oak_door[facing=south,half=lower]", 1), "minecraft:oak_door[facing=west,half=lower]");
        assert_eq!(turn_state("minecraft:oak_log[axis=x]", 1), "minecraft:oak_log[axis=z]");
        assert!(parse(br#"{ "palette": { "A": "stone" }, "layers": [ [ "A" ] ] }"#).is_err());
        assert!(parse(br#"{ "palette": { "A": "minecraft:stone" }, "layers": [ [ "AB" ] ] }"#).is_err());
        let blocks = vec![vec![vec!["minecraft:stone".to_string(), "minecraft:air".to_string()]]];
        let c = Plan::capture("x", &blocks).unwrap();
        assert_eq!(c.layers, vec![vec!["A.".to_string()]]);
        assert_eq!(read(&c.to_json()).unwrap(), c);
        assert!(parse(include_bytes!("../data/projectfelix/felix_plans/cabin.json")).is_ok());
    }
}
