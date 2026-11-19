//! An NPC made in the game: its name, model, skin, what it says and its rules, and its file,
//! `<id>.felixnpc` (a zip: `npc.json`, `skin.png`, `dialog.json`, `rules.json` when it has rules,
//! `quests.json` when it gives quests, `shop.json` when it trades, `character.json` when its
//! character was made in the game (so it goes along),
//! and `model.geo.json` when it brings its own model). No mod API types here.
//!
//! `npc.json`: `{ "format": 1, "id", "name", "model", "character", "greeting", "topics": [{ "ask", "answer" }] }`.
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
    /// Its character (`projectfelix:kind`…; empty: the default).
    pub character: String,
    /// The quests it gives (`quests.json`, see `quest`).
    pub quests: Option<String>,
    /// Its day (`projectfelix:villager`…; empty: none, it follows its owner).
    pub routine: String,
    /// Its voice (see `voice`; empty: its character's).
    pub voice: String,
    /// Its job (see `job`; empty: none).
    pub job: String,
    /// A dialog made in the game it speaks instead of its greeting and topics (empty: those).
    pub dialog_set: String,
    /// Rules made in the game it follows instead of its own (empty: its own).
    pub rules_set: String,
    /// What it sells and buys (`shop.json`, see `shop`).
    pub shop: Option<String>,
    /// Its character's own file, when that was made in the game (`felixchar:`).
    pub character_file: Option<String>,
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
            character: String::new(),
            quests: None,
            routine: String::new(),
            voice: String::new(),
            job: String::new(),
            dialog_set: String::new(),
            rules_set: String::new(),
            shop: None,
            character_file: None,
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
        if !self.routine.is_empty() {
            options.push(json!({ "text": "Go about your day.", "if": { "all": [ { "owner": true }, { "not": { "mode": "live" } } ] }, "actions": [ { "order": "live" } ] }));
            options.push(json!({ "text": "Live here.", "if": { "owner": true }, "actions": [ { "place": "home" } ] }));
            options.push(json!({ "text": "Work here.", "if": { "owner": true }, "actions": [ { "place": "work" } ] }));
            if !self.job.is_empty() {
                options.push(json!({ "text": "Keep your things here.", "if": { "owner": true }, "actions": [ { "place": "chest" } ] }));
            }
        }
        if self.shop.is_some() {
            options.push(json!({ "text": "Let's trade.", "actions": [ { "shop": true } ] }));
        }
        if self.quests.is_some() {
            options.push(json!({ "text": "Need any help?", "actions": [ { "quests": true } ] }));
        }
        options.push(json!({ "text": "Take this.", "if": { "owner": true }, "actions": [ { "gear": "take" } ] }));
        options.push(json!({ "text": "How strong are you?", "actions": [ { "stats": true } ] }));
        let (more, more_nodes) = social("hello");
        options.extend(more);
        nodes.extend(more_nodes);
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
            "character": self.character,
            "routine": self.routine,
            "voice": self.voice,
            "job": self.job,
            "dialog_set": self.dialog_set,
            "rules_set": self.rules_set,
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
        if let Some(q) = &self.quests {
            entries.push(Entry { name: "quests.json".into(), data: q.clone().into_bytes() });
        }
        if let Some(s) = &self.shop {
            entries.push(Entry { name: "shop.json".into(), data: s.clone().into_bytes() });
        }
        if let Some(c) = &self.character_file {
            entries.push(Entry { name: "character.json".into(), data: c.clone().into_bytes() });
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
        let shop = get("shop.json").map(|r| String::from_utf8_lossy(&r).into_owned());
        if let Some(s) = &shop {
            crate::shop::parse(s.as_bytes()).map_err(|e| format!("shop.json: {e}"))?;
        }
        let character_file = get("character.json").map(|r| String::from_utf8_lossy(&r).into_owned());
        if let Some(c) = &character_file {
            crate::character::parse(c.as_bytes()).map_err(|e| format!("character.json: {e}"))?;
        }
        let quests = get("quests.json").map(|r| String::from_utf8_lossy(&r).into_owned());
        if let Some(q) = &quests {
            let v: Value = crate::serde_json::from_str(q).map_err(|e| format!("quests.json: {e}"))?;
            crate::quest::read_list(&v).map_err(|e| format!("quests.json: {e}"))?;
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
            character: s("character"),
            quests,
            routine: s("routine"),
            voice: s("voice"),
            job: s("job"),
            dialog_set: s("dialog_set"),
            rules_set: s("rules_set"),
            shop,
            character_file,
        })
    }
}

/// What anyone may say to any NPC about how it feels: options for the node `back`, and their nodes.
pub fn social(back: &str) -> (Vec<Value>, Vec<(String, Value)>) {
    let fine = json!({ "any": [ { "mood": "calm" }, { "mood": "happy" } ] });
    let options = vec![
        json!({ "text": "How are you?", "if": fine, "goto": "feel_fine" }),
        json!({ "text": "How are you?", "if": { "not": fine }, "goto": "feel_bad" }),
        json!({ "text": "Have an apple.", "if": { "item": "minecraft:apple" }, "actions": [ { "take": "minecraft:apple" }, { "feel": "gift" } ], "goto": "thanks" }),
        json!({ "text": "I'm sorry.", "if": { "all": [ { "not": fine }, { "fault": true } ] }, "actions": [ { "feel": "apology" } ], "goto": "sorry" }),
        json!({ "text": "Who upset you?", "if": { "all": [ { "not": fine }, { "fault": false } ] }, "goto": "not_you" }),
        json!({ "text": "Here, eat something.", "if": { "hungry": true }, "actions": [ { "feed": true } ] }),
        json!({ "text": "How's your family?", "if": { "family": true }, "actions": [ { "family": true } ] }),
        json!({ "text": "Just between us...", "if": { "stage": "acquaintance" }, "goto": "us" }),
        json!({ "text": "What's on today?", "if": { "event": "any" }, "goto": "event_today" }),
        json!({ "text": "Heard any news?", "actions": [ { "news": true } ] }),
    ];
    let again = json!([ { "text": "Something else...", "goto": back }, { "text": "Bye." } ]);
    let nodes = vec![
        ("feel_fine".to_string(), json!({ "text": "I'm fine, thanks for asking.", "options": again })),
        ("feel_bad".to_string(), json!({ "text": "Not great, @player. Not great at all.", "options": again })),
        ("thanks".to_string(), json!({ "text": "For me? Thank you!", "options": again })),
        ("sorry".to_string(), json!({ "text": "...All right. Let's forget it.", "options": again })),
        ("us".to_string(), json!({ "text": "Yes, @player?", "options": [
            { "text": "I brought you something.", "actions": [ { "gift": true } ] },
            { "text": "What do you like?", "actions": [ { "tastes": true } ] },
            { "text": "Would you go out with me?", "if": { "all": [ { "stage": "close" }, { "romance": "none" } ] }, "actions": [ { "ask_out": true } ] },
            { "text": "Will you marry me?", "if": { "romance": "sweetheart" }, "actions": [ { "propose": true } ] },
            { "text": "Come live with me, here.", "if": { "romance": "partner" }, "actions": [ { "place": "home" } ], "goto": "moved_in" },
            { "text": "I think we should part ways.", "if": { "any": [ { "romance": "sweetheart" }, { "romance": "partner" } ] }, "actions": [ { "part": true } ] },
            { "text": "Something else...", "goto": back } ] })),
        ("moved_in".to_string(), json!({ "text": "Home is wherever you are, @player.", "options": again })),
        ("event_today".to_string(), json!({ "text": "It's the @event, @player! Everyone's here. Come and join us.", "options": again })),
        ("not_you".to_string(), json!({ "text": "It's not you, @player. @upset_by can say sorry, not you.", "options": again })),
    ];
    (options, nodes)
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
