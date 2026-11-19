//! Quests NPCs give: bring things, kill things, go somewhere, or walk an NPC somewhere. No mod
//! API types here.
//!
//! A quest is `data/<namespace>/felix_quests/<name>.json`, or one of `quests.json` in an NPC made in
//! the game:
//!
//! ```json
//! { "title": "Apples for @npc", "bring": "minecraft:apple", "count": 5,
//!   "offer": "Could you bring me 5 apples, @player?", "progress": "Got the apples yet?",
//!   "done": "Thank you!", "fail": "Never mind, then.", "time": 10,
//!   "reward": [ { "give": "minecraft:emerald", "count": 2 } ], "repeat": false, "min_relation": 0 }
//! ```
//!
//! What it asks: `bring` (an item, `count` of it, taken when the player hands it in), `kill` (an
//! entity type, or `monster` / `animal`, `count` of them), `go_to` (`[x, y, z]`, within `radius`),
//! `walk` (the NPC, following the player, `distance` blocks from where it was asked, or `to`
//! `[x, y, z]` within `radius`), `deliver_to` (an NPC by name, or `#tag` for any NPC with that tag:
//! the player talks to it, handing over `item` if there is one). `time`: minutes to do it in (none:
//! no hurry). `reward`: dialog actions, run when it is done; `gives`: when it is taken (a letter to
//! carry). Its character takes `quest_done` and `quest_failed` as events.
//!
//! Chains and stories: `requires` lists quests the player must have done first (from anyone), so
//! one NPC's quest opens another's; `scene` plays a scene when it is done (the NPC who ends it
//! leads); `delivered` is what the one delivered to says.

use crate::dialog::{self, Action};
use crate::serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum Goal {
    Bring { item: String, count: u32 },
    Kill { target: String, count: u32 },
    GoTo { at: [f64; 3], radius: f64 },
    /// `to`: a place; none: `distance` blocks from where the NPC was.
    Walk { to: Option<[f64; 3]>, distance: f64, radius: f64 },
    /// To an NPC by name, or `#tag`; with an item to hand over, maybe.
    Deliver { to: String, item: Option<String>, count: u32 },
}

impl Goal {
    /// How many of it there are to do (one for a place).
    pub fn needed(&self) -> u32 {
        match self {
            Goal::Bring { count, .. } | Goal::Kill { count, .. } => *count,
            _ => 1,
        }
    }

    /// Whether killing an entity of `kind` (`minecraft:zombie`) and `category` counts.
    pub fn counts_kill(&self, kind: &str, category: &str) -> bool {
        matches!(self, Goal::Kill { target, .. } if target == kind || target == category)
    }

    /// The goal in a few words.
    pub fn describe(&self) -> String {
        let short = |id: &str| id.rsplit(':').next().unwrap_or(id).replace('_', " ");
        match self {
            Goal::Bring { item, count } => format!("bring {count} {}", short(item)),
            Goal::Kill { target, count } => format!("kill {count} {}", short(target)),
            Goal::GoTo { at, .. } => format!("go to {:.0} {:.0} {:.0}", at[0], at[1], at[2]),
            Goal::Walk { to: Some(at), .. } => format!("walk @npc to {:.0} {:.0} {:.0}", at[0], at[1], at[2]),
            Goal::Walk { distance, .. } => format!("walk @npc {distance:.0} blocks away"),
            Goal::Deliver { to, item: Some(i), count } => format!("take {count} {} to {}", short(i), to.trim_start_matches('#')),
            Goal::Deliver { to, item: None, .. } => format!("take word to {}", to.trim_start_matches('#')),
        }
    }

    /// Whether a place is reached: `at` is the player (go to) or the NPC (walk); `start`, where
    /// the NPC was when it asked.
    pub fn reached(&self, at: [f64; 3], start: [f64; 3]) -> bool {
        let d = |a: [f64; 3], b: [f64; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
        match self {
            Goal::GoTo { at: to, radius } | Goal::Walk { to: Some(to), radius, .. } => d(at, *to) <= *radius,
            Goal::Walk { to: None, distance, .. } => d([at[0], 0.0, at[2]], [start[0], 0.0, start[2]]) >= *distance,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quest {
    pub title: String,
    pub goal: Goal,
    pub offer: String,
    pub progress: String,
    pub done: String,
    pub fail: String,
    /// Minutes.
    pub time: Option<f32>,
    pub reward: Vec<Action>,
    pub repeat: bool,
    pub min_relation: i32,
    /// Quests the player must have done first.
    pub requires: Vec<String>,
    /// A scene played when it is done.
    pub scene: Option<String>,
    /// Dialog actions run when it is taken.
    pub gives: Vec<Action>,
    /// What the one delivered to says (`@giver`: who sent it).
    pub delivered: String,
}

pub fn parse(bytes: &[u8]) -> Result<Quest, String> {
    let root: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    read(&root)
}

fn point(v: Option<&Value>) -> Option<[f64; 3]> {
    let a = v?.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])
}

pub fn read(v: &Value) -> Result<Quest, String> {
    let m = v.as_object().ok_or("not an object")?;
    let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
    let count = m.get("count").and_then(Value::as_u64).unwrap_or(1).clamp(1, 999) as u32;
    let radius = m.get("radius").and_then(Value::as_f64).unwrap_or(4.0).max(1.0);
    let id = |x: String| if x.contains(':') || x == "monster" || x == "animal" { x } else { format!("minecraft:{x}") };
    let goal = if let Some(item) = s("bring").filter(|x| !x.trim().is_empty()) {
        Goal::Bring { item: id(item.trim().to_string()), count }
    } else if let Some(target) = s("kill").filter(|x| !x.trim().is_empty()) {
        Goal::Kill { target: id(target.trim().to_string()), count }
    } else if m.contains_key("go_to") {
        Goal::GoTo { at: point(m.get("go_to")).ok_or("go_to is [x, y, z]")?, radius }
    } else if m.contains_key("walk") {
        let to = point(m.get("to"));
        let distance = m.get("distance").and_then(Value::as_f64).unwrap_or(32.0).max(4.0);
        Goal::Walk { to, distance, radius }
    } else if let Some(to) = s("deliver_to").map(|t| t.trim().to_string()).filter(|t| !t.is_empty() && t != "#") {
        Goal::Deliver { to, item: s("item").filter(|i| !i.trim().is_empty()).map(|i| id(i.trim().to_string())), count }
    } else {
        return Err("no bring, kill, go_to, walk or deliver_to".into());
    };
    let text = |k: &str, d: &str| s(k).filter(|x| !x.trim().is_empty()).unwrap_or_else(|| d.to_string());
    let title = text("title", &goal.describe());
    Ok(Quest {
        offer: text("offer", &format!("Could you {}, @player?", goal.describe())),
        progress: text("progress", "How is it going, @player?"),
        done: text("done", "Thank you, @player!"),
        fail: text("fail", "Never mind, then."),
        title,
        goal,
        time: m.get("time").and_then(Value::as_f64).map(|t| (t as f32).max(0.5)),
        reward: dialog::read_actions(m.get("reward"))?,
        repeat: m.get("repeat").and_then(Value::as_bool).unwrap_or(false),
        min_relation: m.get("min_relation").and_then(Value::as_f64).unwrap_or(-100.0) as i32,
        requires: match m.get("requires") {
            Some(Value::String(r)) => vec![r.clone()],
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        },
        scene: s("scene").filter(|x| !x.trim().is_empty()),
        gives: dialog::read_actions(m.get("gives"))?,
        delivered: text("delivered", "From @giver? Thank you, @player."),
    })
}

/// A list of quests: `{ "quests": [...] }`, or the list alone.
pub fn read_list(root: &Value) -> Result<Vec<Quest>, String> {
    let list = root.get("quests").unwrap_or(root).as_array().ok_or("no \"quests\" list")?;
    list.iter().enumerate().map(|(i, q)| read(q).map_err(|e| format!("quest {}: {e}", i + 1))).collect()
}

// ---- a player's quests ------------------------------------------------------------------------------

/// A quest a player took from an NPC.
#[derive(Clone, Debug, PartialEq)]
pub struct Active {
    pub quest: String,
    pub npc: u128,
    pub npc_name: String,
    /// How many done (kills), or 1 when a place is reached.
    pub progress: u32,
    /// Game ticks it must be done by.
    pub deadline: Option<i64>,
    /// Where the NPC stood when it asked.
    pub start: [f64; 3],
}

/// A player's quests: those under way and those done (`quest@npc`, not to be given again).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Log {
    pub active: Vec<Active>,
    pub done: Vec<String>,
}

/// How many quests a player may have at once.
pub const MAX_ACTIVE: usize = 5;

pub fn done_key(quest: &str, npc: u128) -> String {
    format!("{quest}@{npc:032x}")
}

impl Log {
    pub fn with(&self, npc: u128) -> Option<&Active> {
        self.active.iter().find(|a| a.npc == npc)
    }

    /// Whether `quest` from `npc` may be taken now.
    pub fn may_take(&self, quest: &str, q: &Quest, npc: u128) -> bool {
        self.active.len() < MAX_ACTIVE
            && self.with(npc).is_none()
            && (q.repeat || !self.done.contains(&done_key(quest, npc)))
            && q.requires.iter().all(|r| self.has_done(r))
    }

    /// Whether the player did `quest`, for anyone.
    pub fn has_done(&self, quest: &str) -> bool {
        let prefix = format!("{quest}@");
        self.done.iter().any(|d| d.starts_with(&prefix))
    }

    /// Ends the quest from `npc` (done: remembered, unless it repeats).
    pub fn finish(&mut self, npc: u128, done: bool, repeat: bool) -> Option<Active> {
        let i = self.active.iter().position(|a| a.npc == npc)?;
        let a = self.active.remove(i);
        // Done is remembered (for chains), repeating ones too: they don't ask it.
        if done && !self.done.contains(&done_key(&a.quest, npc)) {
            let _ = repeat;
            self.done.push(done_key(&a.quest, npc));
            if self.done.len() > 256 {
                self.done.remove(0);
            }
        }
        Some(a)
    }
}

// ---- the maker's rows ----------------------------------------------------------------------------------

/// The kinds the maker offers, and what their word is.
pub const KINDS: &[(&str, &str)] = &[("bring", "item"), ("kill", "entity type"), ("go to", "x y z"), ("walk me", "blocks"), ("deliver to", "NPC name")];

/// A quest as the maker shows it: a kind and its word, how many, what the NPC says, the reward.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub kind: usize,
    pub what: String,
    pub count: String,
    pub offer: String,
    /// An item and how many: `minecraft:emerald 2`.
    pub reward: String,
    /// A quest the player must have done first (its id).
    pub after: String,
}

impl Row {
    pub fn to_json(&self) -> Result<Value, String> {
        let mut m = crate::serde_json::Map::new();
        let what = self.what.trim();
        let count: u32 = self.count.trim().parse().unwrap_or(1).max(1);
        match self.kind {
            0 | 1 => {
                if what.is_empty() {
                    return Err(format!("\"{}\" needs an {}", KINDS[self.kind].0, KINDS[self.kind].1));
                }
                m.insert(if self.kind == 0 { "bring" } else { "kill" }.into(), json!(what));
                m.insert("count".into(), json!(count));
            }
            2 => {
                let p: Vec<f64> = what.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                if p.len() != 3 {
                    return Err("\"go to\" needs x y z".into());
                }
                m.insert("go_to".into(), json!(p));
            }
            3 => {
                m.insert("walk".into(), json!(true));
                m.insert("distance".into(), json!(what.parse::<f64>().unwrap_or(32.0)));
            }
            _ => {
                if what.is_empty() {
                    return Err("\"deliver to\" needs an NPC's name".into());
                }
                m.insert("deliver_to".into(), json!(what));
            }
        }
        if !self.after.trim().is_empty() {
            m.insert("requires".into(), json!(self.after.trim()));
        }
        if !self.offer.trim().is_empty() {
            m.insert("offer".into(), json!(self.offer.trim()));
        }
        let mut words = self.reward.split_whitespace();
        if let Some(item) = words.next() {
            let n = words.next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(1);
            m.insert("reward".into(), json!([ { "give": item, "count": n } ]));
        }
        Ok(Value::Object(m))
    }

    /// A quest back as a row, when a row can show it (else None: it is kept as it is).
    pub fn from_json(v: &Value) -> Option<Row> {
        let m = v.as_object()?;
        if m.keys().any(|k| !["bring", "kill", "count", "go_to", "walk", "distance", "offer", "reward", "deliver_to", "requires"].contains(&k.as_str())) {
            return None;
        }
        let after = match m.get("requires") {
            None => String::new(),
            Some(Value::String(r)) => r.clone(),
            Some(_) => return None,
        };
        let count = m.get("count").and_then(Value::as_u64).map(|c| c.to_string()).unwrap_or_default();
        let (kind, what) = if let Some(i) = m.get("bring").and_then(Value::as_str) {
            (0, i.to_string())
        } else if let Some(k) = m.get("kill").and_then(Value::as_str) {
            (1, k.to_string())
        } else if let Some(p) = point(m.get("go_to")) {
            (2, format!("{} {} {}", p[0], p[1], p[2]))
        } else if m.contains_key("walk") {
            (3, m.get("distance").and_then(Value::as_f64).map(|d| d.to_string()).unwrap_or_default())
        } else if let Some(t) = m.get("deliver_to").and_then(Value::as_str) {
            (4, t.to_string())
        } else {
            return None;
        };
        let reward = match m.get("reward").and_then(Value::as_array).map(Vec::as_slice) {
            None | Some([]) => String::new(),
            Some([one]) => {
                let item = one.get("give")?.as_str()?;
                let n = one.get("count").and_then(Value::as_u64).unwrap_or(1);
                if one.as_object()?.len() > 2 {
                    return None;
                }
                if n == 1 { item.to_string() } else { format!("{item} {n}") }
            }
            _ => return None,
        };
        let offer = m.get("offer").and_then(Value::as_str).unwrap_or_default().to_string();
        Some(Row { kind, what, count, offer, reward, after })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_read() {
        for text in [
            include_str!("../data/projectfelix/felix_quests/apples.json"),
            include_str!("../data/projectfelix/felix_quests/zombies.json"),
            include_str!("../data/projectfelix/felix_quests/walk.json"),
        ] {
            let q = parse(text.as_bytes()).unwrap();
            assert!(!q.reward.is_empty(), "{}", q.title);
        }
    }

    #[test]
    fn goals() {
        let q = parse(br#"{ "kill": "zombie", "count": 3, "time": 10 }"#).unwrap();
        assert_eq!(q.goal, Goal::Kill { target: "minecraft:zombie".into(), count: 3 });
        assert!(q.goal.counts_kill("minecraft:zombie", "monster") && !q.goal.counts_kill("minecraft:cow", "animal"));
        assert!(parse(br#"{ "kill": "monster" }"#).unwrap().goal.counts_kill("minecraft:skeleton", "monster"));
        assert_eq!(q.offer, "Could you kill 3 zombie, @player?");
        assert_eq!(q.time, Some(10.0));
        let walk = parse(br#"{ "walk": true, "distance": 20 }"#).unwrap();
        assert!(!walk.goal.reached([10.0, 64.0, 0.0], [0.0, 64.0, 0.0]));
        assert!(walk.goal.reached([0.0, 80.0, 25.0], [0.0, 64.0, 0.0]));
        let go = parse(br#"{ "go_to": [100, 64, 100], "radius": 5 }"#).unwrap();
        assert!(go.goal.reached([97.0, 64.0, 100.0], [0.0; 3]) && !go.goal.reached([90.0, 64.0, 100.0], [0.0; 3]));
        assert!(parse(br#"{ "go_to": [1, 2] }"#).is_err());
        assert!(parse(br#"{ "title": "nothing to do" }"#).is_err());
        assert!(parse(br#"{ "bring": "apple", "reward": [ { "dance": 1 } ] }"#).is_err());
    }

    #[test]
    fn a_log_keeps_track() {
        let q = parse(br#"{ "bring": "apple" }"#).unwrap();
        let mut log = Log::default();
        assert!(log.may_take("a:apples", &q, 9));
        log.active.push(Active { quest: "a:apples".into(), npc: 9, npc_name: "Ann".into(), progress: 0, deadline: None, start: [0.0; 3] });
        assert!(!log.may_take("a:other", &q, 9), "one at a time from one NPC");
        assert!(log.may_take("a:apples", &q, 10));
        assert_eq!(log.finish(9, true, false).map(|a| a.npc_name), Some("Ann".into()));
        assert!(!log.may_take("a:apples", &q, 9), "done, and it doesn't repeat");
        assert!(log.finish(9, true, false).is_none());
    }

    #[test]
    fn chains() {
        let first = parse(br#"{ "bring": "apple" }"#).unwrap();
        let second = parse(br##"{ "deliver_to": "#felix_guest", "item": "paper", "requires": "a:first", "gives": [ { "give": "minecraft:paper" } ], "scene": "a:party" }"##).unwrap();
        assert_eq!(second.goal, Goal::Deliver { to: "#felix_guest".into(), item: Some("minecraft:paper".into()), count: 1 });
        assert_eq!(second.goal.describe(), "take 1 paper to felix_guest");
        assert_eq!(second.scene.as_deref(), Some("a:party"));
        assert_eq!(second.gives.len(), 1);
        let mut log = Log::default();
        assert!(!log.may_take("a:second", &second, 2), "not before the first");
        log.active.push(Active { quest: "a:first".into(), npc: 1, npc_name: "Ann".into(), progress: 0, deadline: None, start: [0.0; 3] });
        log.finish(1, true, true);
        assert!(log.has_done("a:first"));
        assert!(log.may_take("a:first", &first, 1) || first.repeat == false);
        assert!(log.may_take("a:second", &second, 2), "the chain goes on, at another NPC");
        for s in [include_str!("../data/projectfelix/felix_quests/festival_1.json"), include_str!("../data/projectfelix/felix_quests/festival_2.json"), include_str!("../data/projectfelix/felix_quests/festival_3.json")] {
            parse(s.as_bytes()).unwrap();
        }
    }

    #[test]
    fn rows_round_trip() {
        let rows = [
            Row { kind: 0, what: "minecraft:apple".into(), count: "5".into(), offer: "Apples, @player?".into(), reward: "minecraft:emerald 2".into(), after: String::new() },
            Row { kind: 1, what: "monster".into(), count: "3".into(), offer: String::new(), reward: "minecraft:bread".into(), after: String::new() },
            Row { kind: 2, what: "10 64 -20".into(), count: String::new(), offer: String::new(), reward: String::new(), after: String::new() },
            Row { kind: 3, what: "40".into(), count: String::new(), offer: "A walk?".into(), reward: String::new(), after: String::new() },
            Row { kind: 4, what: "Bob".into(), count: String::new(), offer: "Tell Bob, @player?".into(), reward: String::new(), after: "felixnpc:ann/0".into() },
        ];
        for row in rows {
            let v = row.to_json().unwrap();
            read(&v).unwrap();
            assert_eq!(Row::from_json(&v), Some(row), "{v}");
        }
        assert!(Row { kind: 0, ..Row::default() }.to_json().is_err());
        assert!(Row { kind: 2, what: "1 2".into(), ..Row::default() }.to_json().is_err());
        assert!(Row::from_json(&json!({ "bring": "x", "time": 5 })).is_none(), "time: kept as a file");
    }
}
