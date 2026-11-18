//! NPC rules: when something happens, if a condition holds, do this, else that. No mod API types
//! here.
//!
//! A file is `data/<namespace>/felix_rules/<name>.json`, the rules `<namespace>:<name>` (given to an
//! NPC with `/felix rules`), or `rules.json` in an NPC made in the game:
//!
//! ```json
//! { "rules": [
//!   { "when": "sees_monster", "radius": 12, "cooldown": 10,
//!     "if": { "mode": "guard" },
//!     "then": [ { "say": "A @target! Stay back, @player." }, { "attack": true } ],
//!     "else": [ { "say": "Look out, a @target!" } ] },
//!   { "when": { "hears": "hello" }, "then": [ { "tell": "Hello yourself, @player." } ] } ] }
//! ```
//!
//! When: `sees_monster`, `sees_player`, `sees_animal`, `{ "sees": "<entity type>" }` (each one that
//! comes into sight, within `radius`), `hurt`, `hurt_by_player`, `owner_hurt`, `kills`,
//! `{ "hears": "<word>" }` (a player says it in chat, within `radius`), `{ "every": <seconds> }`,
//! `dawn`, `dusk`, `rain`, `low_health` (below a third), `friend_hurt` / `friend_killed` (a player
//! hurt or killed an NPC it is friends with: the player is the event's), `{ "feels": "<mood>" }` (its
//! mood turns to that, see `character`), `hears_rumor` (a friend told it what a player did: the
//! player is the event's), `quest_done` / `quest_failed` (a player did, or failed, a quest it gave), `{ "starts": "<doing>" }`
//! (its day has it start that: `work`, `sleep`…).
//!
//! If (optional; `not`, `all`, `any` combine them): `time` (`day` or `night`), `raining`, `mode`
//! (`follow`, `wait`, `guard`), `health_below` (a fraction), `owner_near` (blocks), `owner` (the player
//! of the event is its owner), `chance` (0 to 1), `target` (an entity type, or `monster`, `animal`,
//! `player`), `tag` (the NPC has it), `player_tag`, `item` (+ `count`, the player has it), `mood`,
//! `relation_above` / `relation_below` (how much it likes the player of the event), `doing` (what its
//! day has it do now).
//!
//! Then and else: `say` (a chat line to the players near the NPC), `tell` (to the player of the
//! event only), `attack` (what it saw or what hurt), `look` (at it), `talk` (start its dialog with
//! the player), and every dialog action (`command`, `animation`, `order`, `sound`, `give`, `tag`...).
//! Texts and commands: `@player`, `@npc`, `@target`. A rule rests `cooldown` seconds (5) after it
//! fires.

use crate::dialog::{self, Action};
use crate::serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum When {
    SeesMonster,
    SeesPlayer,
    SeesAnimal,
    Sees(String),
    Hurt,
    HurtByPlayer,
    OwnerHurt,
    Kills,
    Hears(String),
    Every(f32),
    Dawn,
    Dusk,
    Rain,
    LowHealth,
    FriendHurt,
    FriendKilled,
    Feels(String),
    HearsRumor,
    QuestDone,
    QuestFailed,
    Starts(String),
}

impl When {
    /// Whether it looks for things coming into sight.
    pub fn sight(&self) -> bool {
        matches!(self, When::SeesMonster | When::SeesPlayer | When::SeesAnimal | When::Sees(_))
    }

    /// Whether an entity of `kind` (`minecraft:zombie`) and `category` (`monster`, `animal`,
    /// `player`, …) is what it looks for.
    pub fn looks_for(&self, kind: &str, category: &str) -> bool {
        match self {
            When::SeesMonster => category == "monster",
            When::SeesPlayer => category == "player",
            When::SeesAnimal => category == "animal",
            When::Sees(k) => k == kind,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cond {
    Day(bool),
    Raining(bool),
    Mode(String),
    HealthBelow(f32),
    OwnerNear(f64),
    Owner(bool),
    Chance(f32),
    Target(String),
    Tag(String),
    PlayerTag(String),
    Item(String, u32),
    Mood(String),
    RelationAbove(i32),
    RelationBelow(i32),
    Doing(String),
    Not(Box<Cond>),
    All(Vec<Cond>),
    Any(Vec<Cond>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Do {
    Say(String),
    Tell(String),
    Attack,
    Look,
    Talk,
    Act(Action),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub when: When,
    pub radius: f64,
    pub cond: Option<Cond>,
    pub then: Vec<Do>,
    pub otherwise: Vec<Do>,
    /// Seconds.
    pub cooldown: f32,
}

/// What conditions ask about the NPC, the world and the event.
pub trait Facts {
    fn is_day(&self) -> bool;
    fn raining(&self) -> bool;
    fn mode(&self) -> &str;
    /// Health as a fraction of the most.
    fn health(&self) -> f32;
    fn owner_distance(&self) -> Option<f64>;
    /// Whether the player of the event (if any) is the NPC's owner.
    fn player_is_owner(&self) -> bool;
    /// 0 to 1.
    fn random(&self) -> f32;
    /// The other side of the event: its entity type and category.
    fn target(&self) -> Option<(&str, &str)>;
    fn npc_tag(&self, tag: &str) -> bool;
    fn player_tag(&self, tag: &str) -> bool;
    fn player_items(&self, item: &str) -> u32;
    fn mood(&self) -> &str {
        "calm"
    }
    /// How much the NPC likes the player of the event (none: there is none).
    fn relation(&self) -> Option<i32> {
        None
    }
    fn doing(&self) -> &str {
        ""
    }
}

impl Cond {
    pub fn holds(&self, f: &dyn Facts) -> bool {
        match self {
            Cond::Day(day) => f.is_day() == *day,
            Cond::Raining(r) => f.raining() == *r,
            Cond::Mode(m) => f.mode() == m,
            Cond::HealthBelow(h) => f.health() < *h,
            Cond::OwnerNear(r) => f.owner_distance().is_some_and(|d| d <= *r),
            Cond::Owner(want) => f.player_is_owner() == *want,
            Cond::Chance(p) => f.random() < *p,
            Cond::Target(t) => f.target().is_some_and(|(kind, category)| kind == t || category == t),
            Cond::Tag(t) => f.npc_tag(t),
            Cond::PlayerTag(t) => f.player_tag(t),
            Cond::Item(item, n) => f.player_items(item) >= *n,
            Cond::Mood(m) => f.mood() == m,
            Cond::RelationAbove(v) => f.relation().is_some_and(|r| r > *v),
            Cond::RelationBelow(v) => f.relation().is_some_and(|r| r < *v),
            Cond::Doing(d) => f.doing() == d,
            Cond::Not(c) => !c.holds(f),
            Cond::All(list) => list.iter().all(|c| c.holds(f)),
            Cond::Any(list) => list.iter().any(|c| c.holds(f)),
        }
    }
}

impl Rule {
    /// What to do: `then` if the condition holds (or there is none), `else` otherwise.
    pub fn decide(&self, f: &dyn Facts) -> &[Do] {
        if self.cond.as_ref().is_none_or(|c| c.holds(f)) { &self.then } else { &self.otherwise }
    }
}

/// `@player`, `@npc` and `@target` filled in.
pub fn fill(text: &str, player: &str, npc: &str, target: &str) -> String {
    dialog::fill(text, player, npc).replace("@target", target)
}

/// A rules file: `{ "rules": [...] }`, or the list alone.
pub fn parse(bytes: &[u8]) -> Result<Vec<Rule>, String> {
    let root: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    read(&root)
}

pub fn read(root: &Value) -> Result<Vec<Rule>, String> {
    let list = root.get("rules").unwrap_or(root).as_array().ok_or("no \"rules\" list")?;
    list.iter().enumerate().map(|(i, r)| read_rule(r).map_err(|e| format!("rule {}: {e}", i + 1))).collect()
}

fn read_rule(v: &Value) -> Result<Rule, String> {
    let m = v.as_object().ok_or("not an object")?;
    let when = read_when(m.get("when").ok_or("no \"when\"")?)?;
    let cond = m.get("if").map(read_cond).transpose()?;
    Ok(Rule {
        when,
        radius: m.get("radius").and_then(Value::as_f64).unwrap_or(12.0).clamp(1.0, 48.0),
        cond,
        then: read_dos(m.get("then"))?,
        otherwise: read_dos(m.get("else"))?,
        cooldown: m.get("cooldown").and_then(Value::as_f64).unwrap_or(5.0).max(0.0) as f32,
    })
}

fn read_when(v: &Value) -> Result<When, String> {
    if let Some(s) = v.as_str() {
        return Ok(match s {
            "sees_monster" => When::SeesMonster,
            "sees_player" => When::SeesPlayer,
            "sees_animal" => When::SeesAnimal,
            "hurt" => When::Hurt,
            "hurt_by_player" => When::HurtByPlayer,
            "owner_hurt" => When::OwnerHurt,
            "kills" => When::Kills,
            "dawn" => When::Dawn,
            "dusk" => When::Dusk,
            "rain" => When::Rain,
            "low_health" => When::LowHealth,
            "friend_hurt" => When::FriendHurt,
            "friend_killed" => When::FriendKilled,
            "hears_rumor" => When::HearsRumor,
            "quest_done" => When::QuestDone,
            "quest_failed" => When::QuestFailed,
            _ => return Err(format!("unknown \"when\": {s}")),
        });
    }
    let m = v.as_object().ok_or("\"when\" is neither a name nor an object")?;
    if let Some(k) = m.get("sees").and_then(Value::as_str) {
        if k.trim().is_empty() {
            return Err("\"sees\" needs an entity type".into());
        }
        let k = k.trim();
        let k = if k.contains(':') { k.to_string() } else { format!("minecraft:{k}") };
        Ok(When::Sees(k))
    } else if let Some(w) = m.get("hears").and_then(Value::as_str) {
        if w.trim().is_empty() {
            return Err("\"hears\" needs a word".into());
        }
        Ok(When::Hears(w.trim().to_lowercase()))
    } else if let Some(d) = m.get("starts").and_then(Value::as_str).map(str::trim).filter(|d| !d.is_empty()) {
        Ok(When::Starts(d.to_string()))
    } else if let Some(f) = m.get("feels").and_then(Value::as_str) {
        if f.trim().is_empty() {
            return Err("\"feels\" needs a mood".into());
        }
        Ok(When::Feels(f.trim().to_string()))
    } else if let Some(s) = m.get("every").and_then(Value::as_f64) {
        Ok(When::Every((s as f32).max(1.0)))
    } else {
        Err(format!("unknown \"when\": {v}"))
    }
}

fn read_cond(v: &Value) -> Result<Cond, String> {
    let m = v.as_object().ok_or("a condition is not an object")?;
    let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
    let list = |k: &str| -> Result<Vec<Cond>, String> { m.get(k).and_then(Value::as_array).ok_or(format!("{k} needs a list"))?.iter().map(read_cond).collect() };
    Ok(if let Some(t) = s("time") {
        match t.as_str() {
            "day" => Cond::Day(true),
            "night" => Cond::Day(false),
            _ => return Err(format!("time is day or night, not {t}")),
        }
    } else if let Some(r) = m.get("raining") {
        Cond::Raining(r.as_bool().ok_or("raining is true or false")?)
    } else if let Some(mode) = s("mode") {
        Cond::Mode(mode)
    } else if let Some(h) = m.get("health_below").and_then(Value::as_f64) {
        Cond::HealthBelow(h as f32)
    } else if let Some(r) = m.get("owner_near").and_then(Value::as_f64) {
        Cond::OwnerNear(r)
    } else if let Some(o) = m.get("owner") {
        Cond::Owner(o.as_bool().ok_or("owner is true or false")?)
    } else if let Some(p) = m.get("chance").and_then(Value::as_f64) {
        Cond::Chance(p as f32)
    } else if let Some(t) = s("target") {
        Cond::Target(t)
    } else if let Some(t) = s("tag") {
        Cond::Tag(t)
    } else if let Some(t) = s("player_tag") {
        Cond::PlayerTag(t)
    } else if let Some(i) = s("item") {
        Cond::Item(i, m.get("count").and_then(Value::as_u64).unwrap_or(1) as u32)
    } else if let Some(mood) = s("mood") {
        Cond::Mood(mood)
    } else if let Some(v) = m.get("relation_above").and_then(Value::as_f64) {
        Cond::RelationAbove(v as i32)
    } else if let Some(v) = m.get("relation_below").and_then(Value::as_f64) {
        Cond::RelationBelow(v as i32)
    } else if let Some(d) = s("doing") {
        Cond::Doing(d)
    } else if let Some(c) = m.get("not") {
        Cond::Not(Box::new(read_cond(c)?))
    } else if m.contains_key("all") {
        Cond::All(list("all")?)
    } else if m.contains_key("any") {
        Cond::Any(list("any")?)
    } else {
        return Err(format!("unknown condition {v}"));
    })
}

fn read_dos(v: Option<&Value>) -> Result<Vec<Do>, String> {
    let Some(v) = v else { return Ok(Vec::new()) };
    // One action alone, or a list.
    let list = match v.as_array() {
        Some(l) => l.clone(),
        None => vec![v.clone()],
    };
    list.iter()
        .map(|a| {
            let m = a.as_object().ok_or("an action is not an object")?;
            let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
            Ok(if let Some(t) = s("say") {
                Do::Say(t)
            } else if let Some(t) = s("tell") {
                Do::Tell(t)
            } else if m.contains_key("attack") {
                Do::Attack
            } else if m.contains_key("look") {
                Do::Look
            } else if m.contains_key("talk") {
                Do::Talk
            } else {
                let mut one = dialog::read_actions(Some(&Value::Array(vec![a.clone()])))?;
                Do::Act(one.pop().ok_or("an empty action")?)
            })
        })
        .collect()
}

// ---- the maker's rows: one event, one condition, one action each way ---------------------------

/// The events the maker offers: label, and whether it takes a word (an entity type, a word heard,
/// seconds).
pub const WHENS: &[(&str, Option<&str>)] = &[
    ("sees a monster", None),
    ("sees a player", None),
    ("sees an animal", None),
    ("sees a...", Some("entity type")),
    ("is hurt", None),
    ("is hit by a player", None),
    ("its owner is hurt", None),
    ("kills something", None),
    ("hears...", Some("word")),
    ("every...", Some("seconds")),
    ("at dawn", None),
    ("at dusk", None),
    ("when rain starts", None),
    ("is badly hurt", None),
    ("a friend is hurt", None),
    ("a friend is killed", None),
    ("feels...", Some("mood")),
    ("hears a rumor", None),
    ("a quest is done", None),
    ("a quest failed", None),
    ("starts...", Some("work, sleep...")),
];

fn when_json(i: usize, arg: &str) -> Value {
    match i {
        0 => json!("sees_monster"),
        1 => json!("sees_player"),
        2 => json!("sees_animal"),
        3 => json!({ "sees": arg.trim() }),
        4 => json!("hurt"),
        5 => json!("hurt_by_player"),
        6 => json!("owner_hurt"),
        7 => json!("kills"),
        8 => json!({ "hears": arg.trim() }),
        9 => json!({ "every": arg.trim().parse::<f64>().unwrap_or(30.0) }),
        10 => json!("dawn"),
        11 => json!("dusk"),
        12 => json!("rain"),
        13 => json!("low_health"),
        14 => json!("friend_hurt"),
        15 => json!("friend_killed"),
        16 => json!({ "feels": arg.trim() }),
        17 => json!("hears_rumor"),
        18 => json!("quest_done"),
        19 => json!("quest_failed"),
        _ => json!({ "starts": arg.trim() }),
    }
}

/// The conditions the maker offers.
pub const CONDS: &[&str] = &[
    "always",
    "if day",
    "if night",
    "if raining",
    "if not raining",
    "if following",
    "if waiting",
    "if guarding",
    "if hurt (half)",
    "if owner near",
    "if player is owner",
    "if player not owner",
    "half the time",
    "if calm",
    "if upset",
    "if angry",
    "if likes player",
    "if dislikes player",
    "if at work",
    "if asleep",
];

fn cond_json(i: usize) -> Option<Value> {
    Some(match i {
        1 => json!({ "time": "day" }),
        2 => json!({ "time": "night" }),
        3 => json!({ "raining": true }),
        4 => json!({ "raining": false }),
        5 => json!({ "mode": "follow" }),
        6 => json!({ "mode": "wait" }),
        7 => json!({ "mode": "guard" }),
        8 => json!({ "health_below": 0.5 }),
        9 => json!({ "owner_near": 16.0 }),
        10 => json!({ "owner": true }),
        11 => json!({ "owner": false }),
        12 => json!({ "chance": 0.5 }),
        13 => json!({ "mood": "calm" }),
        14 => json!({ "mood": "upset" }),
        15 => json!({ "mood": "angry" }),
        16 => json!({ "relation_above": 10 }),
        17 => json!({ "relation_below": -10 }),
        18 => json!({ "doing": "work" }),
        19 => json!({ "doing": "sleep" }),
        _ => return None,
    })
}

/// The actions the maker offers: label, the JSON key, and whether it takes a text.
pub const DOS: &[(&str, &str, bool)] = &[
    ("nothing", "", false),
    ("says", "say", true),
    ("tells player", "tell", true),
    ("attacks it", "attack", false),
    ("looks at it", "look", false),
    ("starts talk", "talk", false),
    ("animation", "animation", true),
    ("order", "order", true),
    ("sound", "sound", true),
    ("command", "command", true),
    ("feels...", "feel", true),
];

fn do_json(i: usize, arg: &str) -> Option<Value> {
    let (_, key, text) = DOS.get(i).copied().filter(|d| !d.1.is_empty())?;
    let mut m = crate::serde_json::Map::new();
    m.insert(key.into(), if text { json!(arg.trim()) } else { json!(true) });
    Some(Value::Object(m))
}

/// A rule as the maker shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub when: usize,
    pub when_arg: String,
    pub cond: usize,
    pub then: usize,
    pub then_arg: String,
    pub otherwise: usize,
    pub otherwise_arg: String,
}

impl Row {
    pub fn to_json(&self) -> Value {
        let mut m = crate::serde_json::Map::new();
        m.insert("when".into(), when_json(self.when, &self.when_arg));
        if let Some(c) = cond_json(self.cond) {
            m.insert("if".into(), c);
        }
        m.insert("then".into(), Value::Array(do_json(self.then, &self.then_arg).into_iter().collect()));
        if let Some(e) = do_json(self.otherwise, &self.otherwise_arg).filter(|_| self.cond != 0) {
            m.insert("else".into(), Value::Array(vec![e]));
        }
        Value::Object(m)
    }

    /// A rule back as a row, when it is one a row can show (else None: it is kept as it is).
    pub fn from_json(v: &Value) -> Option<Row> {
        let m = v.as_object()?;
        if m.keys().any(|k| !["when", "if", "then", "else"].contains(&k.as_str())) {
            return None;
        }
        let w = m.get("when")?;
        let every = w.get("every").and_then(Value::as_f64).map(|s| (9, if s.fract() == 0.0 { format!("{}", s as i64) } else { s.to_string() }));
        let (when, when_arg) = every.or_else(|| (0..WHENS.len()).find_map(|i| {
            let arg = match WHENS[i].1 {
                None => String::new(),
                Some(_) => w.as_object()?.values().next().map(|a| a.as_str().map(str::to_string).unwrap_or_else(|| a.to_string()))?,
            };
            (when_json(i, &arg) == *w).then_some((i, arg))
        }))?;
        let cond = match m.get("if") {
            None => 0,
            Some(c) => (1..CONDS.len()).find(|i| cond_json(*i).as_ref() == Some(c))?,
        };
        let one = |k: &str| -> Option<(usize, String)> {
            let list = match m.get(k) {
                None => return Some((0, String::new())),
                Some(v) => v.as_array()?.clone(),
            };
            match list.as_slice() {
                [] => Some((0, String::new())),
                [a] => {
                    let (key, val) = a.as_object().filter(|o| o.len() == 1)?.iter().next()?;
                    let i = DOS.iter().position(|d| d.1 == key)?;
                    let arg = if DOS[i].2 { val.as_str()?.to_string() } else { String::new() };
                    Some((i, arg))
                }
                _ => None,
            }
        };
        let (then, then_arg) = one("then")?;
        let (otherwise, otherwise_arg) = one("else")?;
        Some(Row { when, when_arg, cond, then, then_arg, otherwise, otherwise_arg })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct F {
        day: bool,
        mode: &'static str,
        owner: bool,
        target: Option<(&'static str, &'static str)>,
    }
    impl Facts for F {
        fn is_day(&self) -> bool {
            self.day
        }
        fn raining(&self) -> bool {
            false
        }
        fn mode(&self) -> &str {
            self.mode
        }
        fn health(&self) -> f32 {
            1.0
        }
        fn owner_distance(&self) -> Option<f64> {
            Some(4.0)
        }
        fn player_is_owner(&self) -> bool {
            self.owner
        }
        fn random(&self) -> f32 {
            0.3
        }
        fn target(&self) -> Option<(&str, &str)> {
            self.target
        }
        fn npc_tag(&self, _: &str) -> bool {
            false
        }
        fn player_tag(&self, t: &str) -> bool {
            t == "friend"
        }
        fn player_items(&self, _: &str) -> u32 {
            0
        }
    }

    const SAMPLE: &str = r#"{ "rules": [
      { "when": "sees_monster", "radius": 12, "cooldown": 10, "if": { "mode": "guard" },
        "then": [ { "say": "A @target! Stay back, @player." }, { "attack": true } ],
        "else": { "say": "Look out, a @target!" } },
      { "when": { "hears": " Hello " }, "if": { "any": [ { "owner": true }, { "player_tag": "friend" } ] },
        "then": [ { "tell": "Hi, @player." }, { "animation": "animation.felix.wave" } ] },
      { "when": { "every": 0.2 }, "if": { "all": [ { "time": "night" }, { "chance": 0.5 }, { "target": "monster" } ] }, "then": [ { "command": "say @npc" } ] } ] }"#;

    #[test]
    fn reads_and_decides() {
        let r = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].when, When::SeesMonster);
        assert!(r[0].when.sight() && r[0].when.looks_for("minecraft:zombie", "monster") && !r[0].when.looks_for("minecraft:cow", "animal"));
        assert_eq!(r[0].cooldown, 10.0);
        let guard = F { day: true, mode: "guard", owner: false, target: Some(("minecraft:zombie", "monster")) };
        let follow = F { mode: "follow", ..guard };
        assert_eq!(r[0].decide(&guard), [Do::Say("A @target! Stay back, @player.".into()), Do::Attack]);
        assert_eq!(r[0].decide(&follow), [Do::Say("Look out, a @target!".into())]);
        assert_eq!(r[1].when, When::Hears("hello".into()));
        assert_eq!(r[1].decide(&follow).len(), 2, "a friend by tag");
        assert_eq!(r[1].cooldown, 5.0);
        assert!(matches!(r[1].then[1], Do::Act(Action::Animation(_))));
        assert_eq!(r[2].when, When::Every(1.0), "at least a second");
        let night = F { day: false, ..follow };
        assert_eq!(r[2].decide(&night).len(), 1);
        assert!(r[2].decide(&guard).is_empty());
        assert_eq!(fill("A @target near @npc for @player", "Alex", "Bob", "Zombie"), "A Zombie near Bob for Alex");
    }

    #[test]
    fn refuses_broken_rules() {
        assert!(parse(br#"{ "rules": [ { "when": "flies" } ] }"#).unwrap_err().contains("rule 1"));
        assert!(parse(br#"[ { "when": "hurt", "if": { "weird": 1 } } ]"#).is_err());
        assert!(parse(br#"[ { "when": "hurt", "then": [ { "dance": 1 } ] } ]"#).is_err());
        assert!(parse(br#"[ { "when": { "hears": " " } } ]"#).is_err());
        assert_eq!(parse(br#"[ { "when": { "sees": "cow" } } ]"#).unwrap()[0].when, When::Sees("minecraft:cow".into()));
    }

    #[test]
    fn rows_round_trip() {
        let rows = [
            Row { when: 0, cond: 7, then: 1, then_arg: "A @target!".into(), otherwise: 3, ..Row::default() },
            Row { when: 8, when_arg: "hello".into(), cond: 0, then: 2, then_arg: "Hi".into(), ..Row::default() },
            Row { when: 9, when_arg: "30".into(), cond: 12, then: 6, then_arg: "animation.felix.wave".into(), ..Row::default() },
            Row { when: 3, when_arg: "minecraft:cow".into(), cond: 0, then: 0, ..Row::default() },
            Row { when: 14, cond: 17, then: 1, then_arg: "Stop it, @player!".into(), otherwise: 10, otherwise_arg: "scold".into(), ..Row::default() },
            Row { when: 16, when_arg: "upset".into(), cond: 0, then: 6, then_arg: "animation.felix.sad".into(), ..Row::default() },
            Row { when: 13, cond: 15, then: 1, then_arg: "Ow".into(), ..Row::default() },
            Row { when: 17, cond: 17, then: 1, then_arg: "I heard about you, @player.".into(), ..Row::default() },
        ];
        for row in rows {
            let v = row.to_json();
            read(&json!([v.clone()])).unwrap();
            assert_eq!(Row::from_json(&v), Some(row), "{v}");
        }
        // Rules a row can't show stay files.
        let r: Value = crate::serde_json::from_str(SAMPLE).unwrap();
        assert!(Row::from_json(&r["rules"][0]).is_none());
        // "every 30" in a row reads as the number it is.
        let v = json!({ "when": { "every": 30 }, "then": [] });
        assert_eq!(Row::from_json(&v).map(|r| r.when_arg), Some("30".into()));
    }
}
