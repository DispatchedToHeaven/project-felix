//! NPC rules as the rules editor shows them (see `rules_editor`): a list of rules, each an event
//! (with its word, how far it looks and how long it rests), up to three conditions that must all
//! hold, and up to four things to do then and three else. To and from the rules JSON (see
//! `rules`). No mod API types here.

use crate::dialog_edit::{acts_json, acts_text};
use crate::serde_json::{Map, Value, json};

pub const MAX_RULES: usize = 10;
pub const MAX_IFS: usize = 3;
pub const MAX_THEN: usize = 4;
pub const MAX_ELSE: usize = 3;

/// The events: their name, their JSON key, and what their word is (none: they take none).
pub const WHENS: &[(&str, &str, Option<&str>)] = &[
    ("enters biome", "enters", Some("snowy, or minecraft:desert")),
    ("sees player", "sees_player", None),
    ("sees monster", "sees_monster", None),
    ("sees animal", "sees_animal", None),
    ("sees...", "sees", Some("entity type: zombie")),
    ("hears...", "hears", Some("a word said in chat")),
    ("at hour", "at", Some("18:00")),
    ("every...", "every", Some("seconds")),
    ("weather turns", "weather", Some("rain, thunder, clear")),
    ("dawn", "dawn", None),
    ("dusk", "dusk", None),
    ("rain starts", "rain", None),
    ("is hurt", "hurt", None),
    ("hit by player", "hurt_by_player", None),
    ("owner hurt", "owner_hurt", None),
    ("kills", "kills", None),
    ("badly hurt", "low_health", None),
    ("friend hurt", "friend_hurt", None),
    ("friend killed", "friend_killed", None),
    ("feels...", "feels", Some("mood: upset")),
    ("hears rumor", "hears_rumor", None),
    ("quest done", "quest_done", None),
    ("quest failed", "quest_failed", None),
    ("starts...", "starts", Some("work, sleep, play...")),
    ("levels up", "level_up", None),
];

/// The conditions: name, and what the value takes.
pub const IFS: &[(&str, &str)] = &[
    ("biome", "the player's: snowy"),
    ("memory", "key or key=value"),
    ("answer", "dialog:node=b"),
    ("weather", "clear, rain, thunder"),
    ("day", ""),
    ("night", ""),
    ("mood", "happy"),
    ("rel >", "20"),
    ("rel <", "-10"),
    ("player has", "minecraft:apple 3"),
    ("player tag", "felix_x"),
    ("npc tag", "felix_x"),
    ("mode", "follow, wait, guard, live"),
    ("doing", "work, sleep..."),
    ("chance %", "50"),
    ("owner", ""),
    ("owner near", "blocks: 16"),
    ("health below %", "50"),
    ("target", "monster, player, minecraft:zombie"),
    ("custom", "(as the file has it)"),
];
const CUSTOM: usize = 19;

/// The actions: name, and what the value takes (empty: none).
pub const DOS: &[(&str, &str)] = &[
    ("dialog", "felixdialog:name (walks up, talks)"),
    ("say", "over its head: Hi, @player!"),
    ("tell", "to the player: ..."),
    ("talk", ""),
    ("come", ""),
    ("look", ""),
    ("attack", ""),
    ("remember", "key=value"),
    ("forget", "key"),
    ("relation", "+5 or -5"),
    ("feel", "praise, scold, gift..."),
    ("animation", "animation.felix.wave"),
    ("sound", "minecraft:entity.villager.yes"),
    ("give", "minecraft:apple 3"),
    ("take", "minecraft:apple 3"),
    ("tag", "felix_x"),
    ("order", "follow, wait, guard, live"),
    ("scene", "projectfelix:greeting"),
    ("command", "say @player hi"),
    ("custom", "(as the file has it)"),
];
const DO_CUSTOM: usize = 19;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IfEd {
    pub kind: usize,
    pub value: String,
    pub not: bool,
    pub raw: Option<Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DoEd {
    pub kind: usize,
    pub value: String,
    pub raw: Option<Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuleEd {
    pub when: usize,
    pub word: String,
    pub radius: String,
    pub cooldown: String,
    pub ifs: Vec<IfEd>,
    pub then: Vec<DoEd>,
    pub otherwise: Vec<DoEd>,
    /// An event the editor has no row for, kept as it was.
    pub raw_when: Option<Value>,
}

fn num(v: &Value) -> String {
    v.as_f64().map(|x| if x.fract() == 0.0 { format!("{}", x as i64) } else { format!("{x}") }).unwrap_or_default()
}

/// Day time back as an hour (0 is 6:00).
fn hour(t: &str) -> String {
    t.to_string()
}

impl IfEd {
    pub fn from_json(v: &Value) -> IfEd {
        if let Some(inner) = v.get("not") {
            let c = IfEd::from_json(inner);
            return if c.kind != CUSTOM && !c.not { IfEd { not: true, ..c } } else { IfEd { kind: CUSTOM, raw: Some(v.clone()), ..IfEd::default() } };
        }
        let one = |kind: usize, value: String| IfEd { kind, value, not: false, raw: None };
        let o = v.as_object();
        let has = |k: &str| o.is_some_and(|o| o.contains_key(k) && o.len() <= 2);
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        if has("biome") {
            one(0, s("biome"))
        } else if has("flag") {
            let is = s("is");
            one(1, if is.is_empty() { s("flag") } else { format!("{}={is}", s("flag")) })
        } else if has("answer") {
            one(2, format!("{}={}", s("answer"), s("is")))
        } else if has("weather") {
            one(3, s("weather"))
        } else if has("time") && s("time") == "day" {
            one(4, String::new())
        } else if has("time") && s("time") == "night" {
            one(5, String::new())
        } else if has("mood") {
            one(6, s("mood"))
        } else if has("relation_above") {
            one(7, num(&v["relation_above"]))
        } else if has("relation_below") {
            one(8, num(&v["relation_below"]))
        } else if has("item") {
            let n = v.get("count").map(num).unwrap_or_default();
            one(9, if n.is_empty() || n == "1" { s("item") } else { format!("{} {n}", s("item")) })
        } else if has("player_tag") {
            one(10, s("player_tag"))
        } else if has("tag") {
            one(11, s("tag"))
        } else if has("mode") {
            one(12, s("mode"))
        } else if has("doing") {
            one(13, s("doing"))
        } else if has("chance") {
            one(14, format!("{}", (v["chance"].as_f64().unwrap_or(0.5) * 100.0).round() as i64))
        } else if let Some(b) = v.get("owner").and_then(Value::as_bool).filter(|_| has("owner")) {
            IfEd { kind: 15, not: !b, ..IfEd::default() }
        } else if has("owner_near") {
            one(16, num(&v["owner_near"]))
        } else if has("health_below") {
            one(17, format!("{}", (v["health_below"].as_f64().unwrap_or(0.5) * 100.0).round() as i64))
        } else if has("target") {
            one(18, s("target"))
        } else if let Some(r) = v.get("raining").and_then(Value::as_bool).filter(|_| has("raining")) {
            IfEd { kind: 3, value: "rain".into(), not: !r, raw: None }
        } else {
            IfEd { kind: CUSTOM, raw: Some(v.clone()), ..IfEd::default() }
        }
    }

    pub fn to_json(&self) -> Result<Value, String> {
        let v = self.value.trim();
        let name = IFS.get(self.kind).map_or("?", |k| k.0);
        let need = || -> Result<(), String> { if v.is_empty() { Err(format!("{name}: what?")) } else { Ok(()) } };
        let number = || -> Result<f64, String> { v.parse::<f64>().map_err(|_| format!("{name}: a number, not \"{v}\"")) };
        let pair = || -> Result<(String, String), String> {
            let (a, b) = v.rsplit_once('=').ok_or(format!("{name}: write it as name=value"))?;
            Ok((a.trim().to_string(), b.trim().to_string()))
        };
        let c = match self.kind {
            0 => {
                need()?;
                json!({ "biome": v })
            }
            1 => {
                need()?;
                match v.split_once('=') {
                    Some((k, x)) => json!({ "flag": k.trim(), "is": x.trim() }),
                    None => json!({ "flag": v }),
                }
            }
            2 => {
                let (k, a) = pair()?;
                json!({ "answer": k, "is": a })
            }
            3 => {
                if !["clear", "rain", "thunder"].contains(&v) {
                    return Err(format!("weather: clear, rain or thunder, not \"{v}\""));
                }
                json!({ "weather": v })
            }
            4 => json!({ "time": "day" }),
            5 => json!({ "time": "night" }),
            6 => {
                need()?;
                json!({ "mood": v })
            }
            7 => json!({ "relation_above": number()? as i64 }),
            8 => json!({ "relation_below": number()? as i64 }),
            9 => {
                need()?;
                let mut parts = v.split_whitespace();
                let item = parts.next().unwrap_or("");
                json!({ "item": item, "count": parts.next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(1) })
            }
            10 => {
                need()?;
                json!({ "player_tag": v })
            }
            11 => {
                need()?;
                json!({ "tag": v })
            }
            12 => {
                need()?;
                json!({ "mode": v })
            }
            13 => {
                need()?;
                json!({ "doing": v })
            }
            14 => json!({ "chance": (number()? / 100.0).clamp(0.0, 1.0) }),
            15 => json!({ "owner": true }),
            16 => json!({ "owner_near": number()? }),
            17 => json!({ "health_below": (number()? / 100.0).clamp(0.0, 1.0) }),
            18 => {
                need()?;
                json!({ "target": v })
            }
            _ => return self.raw.clone().ok_or("a condition the editor can't show".into()),
        };
        Ok(if self.not { json!({ "not": c }) } else { c })
    }
}

impl DoEd {
    pub fn from_json(v: &Value) -> DoEd {
        let Some(o) = v.as_object() else { return DoEd { kind: DO_CUSTOM, raw: Some(v.clone()), ..DoEd::default() } };
        let key = o.keys().find(|k| !matches!(k.as_str(), "count" | "is" | "vs")).cloned().unwrap_or_default();
        let Some(kind) = DOS.iter().position(|d| d.0 == key) else { return DoEd { kind: DO_CUSTOM, raw: Some(v.clone()), ..DoEd::default() } };
        let value = match key.as_str() {
            "say" | "tell" | "dialog" | "command" | "animation" | "sound" | "order" | "scene" | "feel" | "forget" | "tag" => o[&key].as_str().unwrap_or("").to_string(),
            "talk" | "come" | "look" | "attack" => String::new(),
            "relation" => {
                let n = o[&key].as_f64().unwrap_or(0.0) as i64;
                format!("{n:+}")
            }
            _ => {
                // give, take, remember: as the "does" line has them, without the key.
                let t = acts_text(std::slice::from_ref(v));
                t.strip_prefix(&key).unwrap_or(&t).trim().to_string()
            }
        };
        DoEd { kind, value, raw: None }
    }

    pub fn to_json(&self) -> Result<Value, String> {
        let v = self.value.trim();
        let name = DOS.get(self.kind).map_or("?", |d| d.0);
        if self.kind == DO_CUSTOM {
            return self.raw.clone().ok_or("an action the editor can't show".into());
        }
        if DOS[self.kind].1.is_empty() {
            return Ok(json!({ name: true }));
        }
        if v.is_empty() {
            return Err(format!("{name}: what?"));
        }
        Ok(match name {
            "say" | "tell" | "dialog" => json!({ name: v }),
            "relation" => json!({ "relation": v.trim_start_matches('+').parse::<f64>().map_err(|_| format!("relation: a number, as +5 or -5, not \"{v}\""))? as i64 }),
            _ => acts_json(&format!("{name} {v}"))?.pop().ok_or(format!("{name}: what?"))?,
        })
    }
}

impl RuleEd {
    pub fn new() -> RuleEd {
        RuleEd {
            when: 0,
            word: "snowy".into(),
            radius: "16".into(),
            cooldown: "60".into(),
            then: vec![DoEd { kind: 0, value: String::new(), raw: None }],
            ..RuleEd::default()
        }
    }

    pub fn from_json(v: &Value) -> RuleEd {
        let w = v.get("when").cloned().unwrap_or(Value::Null);
        let (when, word, raw_when) = match &w {
            Value::String(s) => match WHENS.iter().position(|x| x.1 == s && x.2.is_none()) {
                Some(i) => (i, String::new(), None),
                None => (0, String::new(), Some(w.clone())),
            },
            Value::Object(o) if o.len() == 1 => {
                let (k, x) = o.iter().next().map(|(k, x)| (k.clone(), x.clone())).unwrap_or_default();
                match WHENS.iter().position(|e| e.1 == k && e.2.is_some()) {
                    Some(i) => (i, x.as_str().map(str::to_string).unwrap_or_else(|| num(&x)), None),
                    None => (0, String::new(), Some(w.clone())),
                }
            }
            _ => (0, String::new(), Some(w.clone())),
        };
        let ifs = match v.get("if") {
            None => Vec::new(),
            Some(c) => match c.get("all").and_then(Value::as_array) {
                Some(list) if c.as_object().is_some_and(|o| o.len() == 1) && list.len() <= MAX_IFS => list.iter().map(IfEd::from_json).collect(),
                _ => vec![IfEd::from_json(c)],
            },
        };
        let dos = |k: &str| -> Vec<DoEd> {
            match v.get(k) {
                Some(Value::Array(a)) => a.iter().map(DoEd::from_json).collect(),
                Some(one) => vec![DoEd::from_json(one)],
                None => Vec::new(),
            }
        };
        RuleEd {
            when,
            word,
            radius: v.get("radius").map(num).unwrap_or_default(),
            cooldown: v.get("cooldown").map(num).unwrap_or_default(),
            ifs,
            then: dos("then"),
            otherwise: dos("else"),
            raw_when,
        }
    }

    pub fn to_json(&self, n: usize) -> Result<Value, String> {
        let what = format!("Rule {}", n + 1);
        let when = match &self.raw_when {
            Some(w) => w.clone(),
            None => {
                let (name, key, word) = WHENS[self.when.min(WHENS.len() - 1)];
                match word {
                    None => json!(key),
                    Some(_) if self.word.trim().is_empty() => return Err(format!("{what}: \"{name}\" needs its word")),
                    Some(_) if key == "every" => json!({ "every": self.word.trim().parse::<f64>().map_err(|_| format!("{what}: every how many seconds?"))? }),
                    Some(_) => json!({ key: hour(self.word.trim()) }),
                }
            }
        };
        let mut m = Map::new();
        m.insert("when".into(), when);
        for (k, s) in [("radius", &self.radius), ("cooldown", &self.cooldown)] {
            if !s.trim().is_empty() {
                m.insert(k.into(), json!(s.trim().parse::<f64>().map_err(|_| format!("{what}: {k} is a number"))?));
            }
        }
        let ifs: Vec<Value> = self.ifs.iter().map(IfEd::to_json).collect::<Result<_, _>>().map_err(|e| format!("{what}: {e}"))?;
        match ifs.len() {
            0 => {}
            1 => {
                m.insert("if".into(), ifs[0].clone());
            }
            _ => {
                m.insert("if".into(), json!({ "all": ifs }));
            }
        }
        let then: Vec<Value> = self.then.iter().map(DoEd::to_json).collect::<Result<_, _>>().map_err(|e| format!("{what}: {e}"))?;
        m.insert("then".into(), json!(then));
        if !self.otherwise.is_empty() {
            let e: Vec<Value> = self.otherwise.iter().map(DoEd::to_json).collect::<Result<_, _>>().map_err(|e| format!("{what}, else: {e}"))?;
            m.insert("else".into(), json!(e));
        }
        Ok(Value::Object(m))
    }

    /// In a few words, for the list.
    pub fn summary(&self) -> String {
        let when = if self.raw_when.is_some() {
            "(custom event)".to_string()
        } else {
            let (name, _, word) = WHENS[self.when.min(WHENS.len() - 1)];
            match word {
                Some(_) => format!("{} {}", name.trim_end_matches("..."), self.word.trim()),
                None => name.to_string(),
            }
        };
        let first = self.then.first().map(|d| {
            let name = DOS.get(d.kind).map_or("?", |x| x.0);
            if d.value.trim().is_empty() { name.to_string() } else { format!("{name} {}", d.value.trim()) }
        });
        format!("{when} > {}", first.unwrap_or_else(|| "nothing".into()))
    }
}

/// A rules file as the editor's list.
pub fn from_json(v: &Value) -> Vec<RuleEd> {
    v.get("rules").unwrap_or(v).as_array().map(|a| a.iter().map(RuleEd::from_json).collect()).unwrap_or_default()
}

/// The editor's list as a rules file, checked as the game reads it.
pub fn to_json(rules: &[RuleEd]) -> Result<Value, String> {
    let list: Vec<Value> = rules.iter().enumerate().map(|(i, r)| r.to_json(i)).collect::<Result<_, _>>()?;
    let v = json!({ "rules": list });
    crate::rules::read(&v)?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_go_round() {
        let v: Value = crate::serde_json::from_str(r#"{ "rules": [
            { "when": { "enters": "snowy" }, "radius": 24, "cooldown": 300,
              "if": { "all": [ { "not": { "flag": "cold_talk" } }, { "time": "day" } ] },
              "then": [ { "dialog": "felixdialog:cold_road" }, { "remember": "cold_talk" } ],
              "else": [ { "say": "Brr." } ] },
            { "when": "sees_monster", "then": [ { "attack": true }, { "give": "minecraft:apple", "count": 2 }, { "relation": -3 } ] },
            { "when": { "at": "18:00" }, "if": { "answer": "felixdialog:cold_road:ask=a" }, "then": [ { "say": "Evening!" } ] },
            { "when": { "every": 30 }, "if": { "any": [ { "tag": "a" } ] }, "then": { "game": "chess", "vs": "player" } } ] }"#).unwrap();
        let list = from_json(&v);
        assert_eq!(list.len(), 4);
        assert_eq!((list[0].when, list[0].word.as_str()), (0, "snowy"));
        assert!(list[0].ifs[0].not && list[0].ifs[0].kind == 1 && list[0].ifs[1].kind == 4);
        assert_eq!(list[0].then[0].value, "felixdialog:cold_road");
        assert_eq!(list[1].then[1].value, "minecraft:apple 2");
        assert_eq!(list[1].then[2].value, "-3");
        assert_eq!(list[3].ifs[0].kind, CUSTOM);
        assert_eq!(list[3].then[0].kind, DO_CUSTOM);
        assert_eq!(list[0].summary(), "enters biome snowy > dialog felixdialog:cold_road");
        let back = to_json(&list).unwrap();
        assert_eq!(from_json(&back), list);
        let r = crate::rules::read(&back).unwrap();
        assert_eq!(r[0].when, crate::rules::When::EntersBiome("snowy".into()));
        assert_eq!(r[2].when, crate::rules::When::At(crate::routine::clock("18:00").unwrap()));
    }

    #[test]
    fn mistakes_are_named() {
        let mut r = RuleEd::new();
        assert!(to_json(&[r.clone()]).unwrap_err().contains("dialog"));
        r.then[0].value = "felixdialog:x".into();
        assert!(to_json(&[r.clone()]).is_ok());
        r.word.clear();
        assert!(to_json(&[r.clone()]).unwrap_err().contains("word"));
        r.word = "snowy".into();
        r.ifs.push(IfEd { kind: 3, value: "snow".into(), ..IfEd::default() });
        assert!(to_json(&[r]).unwrap_err().contains("weather"));
    }
}
