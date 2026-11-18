//! An NPC made in the game: its name, model, skin, what it says and its rules, and its file,
//! `<id>.felixnpc` (a zip: `npc.json`, `skin.png`, `dialog.json`, `rules.json` when it has rules,
//! and `model.geo.json` when it brings its own model). No mod API types here.
//!
//! `npc.json`: `{ "format": 1, "id", "name", "model", "greeting", "topics": [{ "ask", "answer" }] }`.
//! The dialog is built from the greeting and the topics, with a companion's orders for its owner,
//! unless the file brings a `dialog.json` of its own (`"own_dialog": true`).

use crate::serde_json::{Value, json};
use crate::zipfile::{self, Entry};

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub model: String,
    pub greeting: String,
    /// What the player can ask, and what the NPC answers.
    pub topics: Vec<(String, String)>,
    /// A dialog file of its own, instead of the one built from the topics.
    pub own_dialog: Option<String>,
    /// The skin, a PNG.
    pub skin: Vec<u8>,
    /// A model of its own (`.geo.json`), drawn as `npc:<id>`.
    pub model_file: Option<Vec<u8>>,
    /// Its rules (`rules.json`, see `rules`).
    pub rules: Option<String>,
}

/// An id from a name: lower case letters, digits and `_`, at most 32.
pub fn make_id(name: &str) -> String {
    let mut id: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    id.truncate(32);
    if id.is_empty() { "npc".into() } else { id }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl Profile {
    pub fn new(name: &str) -> Profile {
        Profile {
            id: make_id(name),
            name: name.to_string(),
            model: crate::companion_default_model().to_string(),
            greeting: "Hello, @player!".into(),
            topics: Vec::new(),
            own_dialog: None,
            skin: Vec::new(),
            model_file: None,
            rules: None,
        }
    }

    /// The model it is drawn with: its own if it brings one.
    pub fn model_id(&self) -> String {
        if self.model_file.is_some() { format!("npc:{}", self.id) } else { self.model.clone() }
    }

    /// Its dialog file: its own, or one built from the greeting and the topics.
    pub fn dialog_json(&self) -> String {
        if let Some(d) = &self.own_dialog {
            return d.clone();
        }
        let mut options = vec![
            json!({ "text": "Follow me.", "if": { "all": [ { "owner": true }, { "not": { "mode": "follow" } } ] }, "actions": [ { "order": "follow" } ] }),
            json!({ "text": "Wait here.", "if": { "all": [ { "owner": true }, { "not": { "mode": "wait" } } ] }, "actions": [ { "order": "wait" } ] }),
            json!({ "text": "Guard this place.", "if": { "all": [ { "owner": true }, { "not": { "mode": "guard" } } ] }, "actions": [ { "order": "guard" } ] }),
        ];
        let mut nodes = serde_json_map();
        for (i, (ask, answer)) in self.topics.iter().enumerate() {
            options.push(json!({ "text": ask, "goto": format!("topic{i}") }));
            nodes.insert(
                format!("topic{i}"),
                json!({ "text": answer, "options": [ { "text": "Something else...", "goto": "hello" }, { "text": "Bye." } ] }),
            );
        }
        options.push(json!({ "text": "Bye." }));
        nodes.insert("hello".into(), json!({ "text": self.greeting, "options": options }));
        crate::serde_json::to_string_pretty(&json!({ "start": "hello", "nodes": Value::Object(nodes) })).unwrap_or_default()
    }

    /// Its `.felixnpc` file.
    pub fn pack(&self, crc32: &dyn Fn(&[u8]) -> u32) -> Vec<u8> {
        let topics: Vec<Value> = self.topics.iter().map(|(a, b)| json!({ "ask": a, "answer": b })).collect();
        let head = json!({
            "format": 1,
            "id": self.id,
            "name": self.name,
            "model": self.model,
            "greeting": self.greeting,
            "topics": topics,
            "own_dialog": self.own_dialog.is_some(),
        });
        let mut entries = vec![
            Entry { name: "npc.json".into(), data: crate::serde_json::to_vec_pretty(&head).unwrap_or_default() },
            Entry { name: "dialog.json".into(), data: self.dialog_json().into_bytes() },
        ];
        if !self.skin.is_empty() {
            entries.push(Entry { name: "skin.png".into(), data: self.skin.clone() });
        }
        if let Some(m) = &self.model_file {
            entries.push(Entry { name: "model.geo.json".into(), data: m.clone() });
        }
        if let Some(r) = &self.rules {
            entries.push(Entry { name: "rules.json".into(), data: r.clone().into_bytes() });
        }
        zipfile::write(&entries, crc32)
    }

    /// A `.felixnpc` (or any zip with the same files) read back.
    pub fn unpack(bytes: &[u8], inflate: &dyn Fn(&[u8]) -> Result<Vec<u8>, String>) -> Result<Profile, String> {
        let entries = zipfile::read(bytes, inflate, 4 << 20)?;
        // Files may sit in one folder inside the zip (as a zipped folder has them).
        let get = |name: &str| entries.iter().find(|e| e.name == name || e.name.ends_with(&format!("/{name}"))).map(|e| e.data.clone());
        let head: Value = crate::serde_json::from_slice(&get("npc.json").ok_or("no npc.json in the file")?).map_err(|e| format!("npc.json: {e}"))?;
        let s = |k: &str| head.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let id = s("id");
        if !valid_id(&id) {
            return Err(format!("bad id {id:?}: lower case letters, digits and _ only"));
        }
        let topics = head
            .get("topics")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|t| (t.get("ask").and_then(Value::as_str).unwrap_or("").to_string(), t.get("answer").and_then(Value::as_str).unwrap_or("").to_string())).collect())
            .unwrap_or_default();
        let own = head.get("own_dialog").and_then(Value::as_bool).unwrap_or(false);
        let own_dialog = if own { get("dialog.json").map(|d| String::from_utf8_lossy(&d).into_owned()) } else { None };
        if let Some(d) = &own_dialog {
            crate::dialog::parse(d.as_bytes()).map_err(|e| format!("dialog.json: {e}"))?;
        }
        let rules = get("rules.json").map(|r| String::from_utf8_lossy(&r).into_owned());
        if let Some(r) = &rules {
            crate::rules::parse(r.as_bytes()).map_err(|e| format!("rules.json: {e}"))?;
        }
        Ok(Profile {
            id,
            name: s("name"),
            model: if s("model").is_empty() { crate::companion_default_model().into() } else { s("model") },
            greeting: s("greeting"),
            topics,
            own_dialog,
            skin: get("skin.png").unwrap_or_default(),
            model_file: get("model.geo.json"),
            rules,
        })
    }
}

fn serde_json_map() -> crate::serde_json::Map<String, Value> {
    crate::serde_json::Map::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc(d: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in d {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }

    #[test]
    fn ids() {
        assert_eq!(make_id("Old Bob the 2nd!"), "old_bob_the_2nd");
        assert_eq!(make_id("???"), "npc");
        assert!(valid_id("old_bob") && !valid_id("Old Bob") && !valid_id(""));
    }

    #[test]
    fn pack_and_unpack() {
        let mut p = Profile::new("Old Bob");
        p.greeting = "Morning, @player.".into();
        p.topics = vec![("Seen any monsters?".into(), "Only a creeper, by the river.".into())];
        p.skin = vec![137, 80, 78, 71];
        p.rules = Some(r#"{ "rules": [ { "when": "sees_monster", "then": [ { "say": "A @target!" } ] } ] }"#.into());
        let back = Profile::unpack(&p.pack(&crc), &|_| Err("no".into())).unwrap();
        assert_eq!(back, p);
        let d = crate::dialog::parse(back.dialog_json().as_bytes()).unwrap();
        let hello = &d.nodes["hello"];
        assert_eq!(hello.text, ["Morning, @player."]);
        assert!(hello.options.iter().any(|o| o.text == "Seen any monsters?" && o.goto.as_deref() == Some("topic0")));
        assert_eq!(d.nodes["topic0"].text, ["Only a creeper, by the river."]);
    }

    #[test]
    fn a_zipped_folder_and_bad_files() {
        let p = Profile::new("Ann");
        let z = zipfile::write(
            &[Entry { name: "ann/npc.json".into(), data: br#"{"format":1,"id":"ann","name":"Ann","model":"projectfelix:felix","greeting":"Hi","topics":[]}"#.to_vec() }],
            &crc,
        );
        let back = Profile::unpack(&z, &|_| Err("no".into())).unwrap();
        assert_eq!(back.name, p.name);
        let z = zipfile::write(&[Entry { name: "npc.json".into(), data: br#"{"id":"Bad Id"}"#.to_vec() }], &crc);
        assert!(Profile::unpack(&z, &|_| Err("no".into())).is_err());
        let z = zipfile::write(
            &[
                Entry { name: "npc.json".into(), data: br#"{"format":1,"id":"ann","name":"Ann"}"#.to_vec() },
                Entry { name: "rules.json".into(), data: br#"{"rules":[{"when":"flies"}]}"#.to_vec() },
            ],
            &crc,
        );
        assert!(Profile::unpack(&z, &|_| Err("no".into())).unwrap_err().contains("rules.json"));
    }
}
