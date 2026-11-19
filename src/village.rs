//! A village as a file: the NPCs around a place, and everything made in the game they need. No
//! mod API types here.
//!
//! `<name>.felixvillage` is a zip: `village.json`, and `npcs/<id>.felixnpc`, `characters/<name>.json`,
//! `scenes/<name>.json` for the NPC files, characters and scenes made in the game it uses.
//! `village.json`:
//!
//! ```json
//! { "format": 1, "name": "Oakvale",
//!   "npcs": [ { "name": "Ann", "at": [2, 0, -3], "yaw": 90, "model": "projectfelix:felix",
//!               "texture": "projectfelix:entity/felix", "dialog": "projectfelix:companion",
//!               "routine": "projectfelix:villager", "shop": "projectfelix:baker",
//!               "quests": "projectfelix:festival_1", "character": "felixchar:grumpy",
//!               "rules": null, "places": { "home": [0, 0, 0] }, "tags": ["felix_guest"],
//!               "level": 3, "orders": "live", "voice": "bell",
//!               "job": "farm" } ] }
//! ```
//!
//! Places are from where the village was saved (and is put back): its NPCs' feet and places
//! move with it.

use crate::serde_json::{Map, Value, json};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NpcSpec {
    pub name: String,
    pub at: [f64; 3],
    pub yaw: f32,
    pub model: String,
    pub texture: String,
    pub dialog: Option<String>,
    pub rules: Option<String>,
    pub routine: Option<String>,
    pub shop: Option<String>,
    pub quests: Option<String>,
    pub character: Option<String>,
    pub places: Vec<(String, [f64; 3])>,
    pub tags: Vec<String>,
    pub level: u32,
    pub orders: String,
    /// Its own voice (see `voice`), if it has one.
    pub voice: Option<String>,
    /// Its job (see `job`), if it has one.
    pub job: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Village {
    pub name: String,
    pub npcs: Vec<NpcSpec>,
}

fn point(v: Option<&Value>) -> Option<[f64; 3]> {
    let a = v?.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}

fn round(p: [f64; 3]) -> Value {
    json!(p.map(|x| (x * 100.0).round() / 100.0))
}

impl NpcSpec {
    /// The NPC file it was made from, if one (`felixnpc:<id>`).
    pub fn profile(&self) -> Option<&str> {
        self.dialog.as_deref()?.strip_prefix("felixnpc:")
    }
}

impl Village {
    pub fn to_json(&self) -> Value {
        let npcs: Vec<Value> = self
            .npcs
            .iter()
            .map(|n| {
                let mut places = Map::new();
                for (k, p) in &n.places {
                    places.insert(k.clone(), round(*p));
                }
                json!({
                    "name": n.name, "at": round(n.at), "yaw": n.yaw.round(), "model": n.model, "texture": n.texture,
                    "dialog": n.dialog, "rules": n.rules, "routine": n.routine, "shop": n.shop, "quests": n.quests,
                    "character": n.character, "places": Value::Object(places), "tags": n.tags, "level": n.level, "orders": n.orders, "voice": n.voice, "job": n.job,
                })
            })
            .collect();
        json!({ "format": 1, "name": self.name, "npcs": npcs })
    }

    pub fn parse(bytes: &[u8]) -> Result<Village, String> {
        let v: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
        Village::read(&v)
    }

    pub fn read(v: &Value) -> Result<Village, String> {
        let list = v.get("npcs").and_then(Value::as_array).ok_or("no \"npcs\"")?;
        let mut npcs = Vec::new();
        for (i, n) in list.iter().enumerate() {
            let s = |k: &str| n.get(k).and_then(Value::as_str).map(str::to_string).filter(|x| !x.is_empty());
            let at = point(n.get("at")).ok_or(format!("NPC {}: no \"at\"", i + 1))?;
            let places = n
                .get("places")
                .and_then(Value::as_object)
                .map(|m| m.iter().filter_map(|(k, p)| Some((k.clone(), point(Some(p))?))).collect())
                .unwrap_or_default();
            npcs.push(NpcSpec {
                name: s("name").unwrap_or_else(|| "NPC".into()),
                at,
                yaw: n.get("yaw").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                model: s("model").unwrap_or_else(|| "projectfelix:felix".into()),
                texture: s("texture").unwrap_or_else(|| "projectfelix:entity/felix".into()),
                dialog: s("dialog"),
                rules: s("rules"),
                routine: s("routine"),
                shop: s("shop"),
                quests: s("quests"),
                character: s("character"),
                places,
                tags: n.get("tags").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
                level: n.get("level").and_then(Value::as_u64).unwrap_or(0).min(20) as u32,
                orders: s("orders").unwrap_or_else(|| "wait".into()),
                voice: s("voice"),
                job: s("job"),
            });
        }
        Ok(Village { name: v.get("name").and_then(Value::as_str).unwrap_or("village").to_string(), npcs })
    }

    /// The rules made in the game it needs (`felixrules:…`).
    pub fn made_rules(&self) -> Vec<String> {
        let mut out: Vec<String> = self.npcs.iter().filter_map(|n| n.rules.clone()).filter(|d| d.starts_with("felixrules:")).collect();
        out.sort();
        out.dedup();
        out
    }

    /// The dialogs made in the game it needs (`felixdialog:…`).
    pub fn made_dialogs(&self) -> Vec<String> {
        let mut out: Vec<String> = self.npcs.iter().filter_map(|n| n.dialog.clone()).filter(|d| d.starts_with("felixdialog:")).collect();
        out.sort();
        out.dedup();
        out
    }

    /// The characters and scenes made in the game it needs (`felixchar:…`, `felixscene:…`).
    pub fn made_characters(&self) -> Vec<String> {
        let mut out: Vec<String> = self.npcs.iter().filter_map(|n| n.character.clone()).filter(|c| c.starts_with("felixchar:")).collect();
        out.sort();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_village_goes_round() {
        let v = Village {
            name: "Oakvale".into(),
            npcs: vec![
                NpcSpec {
                    name: "Ann".into(),
                    at: [2.0, 0.0, -3.004],
                    yaw: 90.2,
                    model: "projectfelix:felix".into(),
                    texture: "felixnpc:ann".into(),
                    dialog: Some("felixnpc:ann".into()),
                    routine: Some("projectfelix:villager".into()),
                    character: Some("felixchar:grumpy".into()),
                    places: vec![("home".into(), [1.0, 0.0, 1.0])],
                    tags: vec!["felix_guest".into()],
                    level: 3,
                    orders: "live".into(),
                    ..NpcSpec::default()
                },
                NpcSpec { name: "Bob".into(), model: "projectfelix:felix".into(), texture: "x".into(), orders: "wait".into(), ..NpcSpec::default() },
            ],
        };
        let back = Village::read(&v.to_json()).unwrap();
        assert_eq!(back.npcs[0].at, [2.0, 0.0, -3.0]);
        assert_eq!(back.npcs[0].yaw, 90.0);
        assert_eq!(back.npcs[0].profile(), Some("ann"));
        assert_eq!(back.npcs[0].places, v.npcs[0].places);
        assert_eq!(back.npcs[1].dialog, None);
        assert_eq!(back.made_characters(), vec!["felixchar:grumpy".to_string()]);
        assert!(Village::parse(br#"{ "npcs": [ { "name": "X" } ] }"#).is_err());
    }
}
