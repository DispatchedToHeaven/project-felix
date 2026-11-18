//! Dialogs: trees of what an NPC says and what the player can answer, read from data packs.
//!
//! A file is `data/<namespace>/felix_dialogs/<name>.json`, the dialog `<namespace>:<name>`:
//!
//! ```json
//! { "start": "hello",
//!   "nodes": {
//!     "hello": {
//!       "text": ["Hi, @player!", "Nice weather."],
//!       "animation": "animation.felix.wave",
//!       "actions": [ { "sound": "minecraft:entity.villager.yes" } ],
//!       "options": [
//!         { "text": "A gift?", "if": { "not": { "tag": "felix_gift" } },
//!           "actions": [ { "give": "minecraft:apple", "count": 3 }, { "tag": "felix_gift" } ],
//!           "goto": "thanks" },
//!         { "text": "Follow me", "if": { "owner": true }, "actions": [ { "order": "follow" } ] },
//!         { "text": "Bye" } ] } } }
//! ```
//!
//! A node's `actions` run when it is shown; an option's when it is picked, then the dialog goes
//! to its `goto` node, or ends without one. Options whose `if` fails are not shown. Conditions:
//! `tag`, `item` (+ `count`), `owner`, `mode` (`follow`, `wait`, `guard`), `game` (the NPC's table:
//! `none`, `waiting`, `playing`, `over`), `in_game` (the player sits at it), `mood` (the NPC's, as
//! `upset`), `relation_above` / `relation_below` (how much it likes the player), `not`, `all`, `any`.
//! Actions: `command` (run as the server console), `give` / `take` (+ `count`), `tag`, `untag`,
//! `animation` (played on the NPC), `order` (`follow`, `wait`, `guard`), `sound` (at the NPC),
//! `scene` (a scene from `felix_scenes`, played by the NPC), `game` (`chess` or `checkers`, with
//! `vs`: `npc` or `player`), `join_game` (take the free seat), `open_game` (the board, to play or watch),
//! `sport` (`table_tennis`, `tennis` or `basketball`, against the NPC), `edit_animation` (opens the
//! animation editor on the NPC, with an animation name or `animation.<model>.custom`), `feel` (an
//! event for its character: `gift`, `apology`, `praise`, `scold`…), `relation` (+/- how much it likes the player).
//! `@player` is the player's name; `@npc` is the NPC's name in texts and its UUID in commands.
//! No mod API types here.

use crate::serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub start: String,
    pub nodes: HashMap<String, Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub text: Vec<String>,
    pub animation: Option<String>,
    pub actions: Vec<Action>,
    pub options: Vec<Choice>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub text: String,
    pub condition: Option<Cond>,
    pub actions: Vec<Action>,
    pub goto: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cond {
    Tag(String),
    Item(String, u32),
    Owner(bool),
    Mode(String),
    Game(String),
    InGame(bool),
    Mood(String),
    RelationAbove(i32),
    RelationBelow(i32),
    Not(Box<Cond>),
    All(Vec<Cond>),
    Any(Vec<Cond>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Command(String),
    Give(String, u32),
    Take(String, u32),
    Tag(String),
    Untag(String),
    Animation(String),
    Order(String),
    Sound(String),
    Scene(String),
    Game(String, bool),
    Sport(String),
    EditAnimation(Option<String>),
    JoinGame,
    OpenGame,
    Feel(String),
    Relation(i32),
}

/// What conditions ask about the player and the NPC.
pub trait Facts {
    fn has_tag(&self, tag: &str) -> bool;
    fn item_count(&self, item: &str) -> u32;
    fn is_owner(&self) -> bool;
    fn mode(&self) -> &str;
    /// The NPC's table: `none`, `waiting`, `playing` or `over`.
    fn game(&self) -> &str {
        "none"
    }
    fn in_game(&self) -> bool {
        false
    }
    /// The NPC's mood.
    fn mood(&self) -> &str {
        "calm"
    }
    /// How much the NPC likes the player.
    fn relation(&self) -> i32 {
        0
    }
}

impl Cond {
    pub fn holds(&self, f: &dyn Facts) -> bool {
        match self {
            Cond::Tag(t) => f.has_tag(t),
            Cond::Item(item, n) => f.item_count(item) >= *n,
            Cond::Owner(want) => f.is_owner() == *want,
            Cond::Mode(m) => f.mode() == m,
            Cond::Game(g) => f.game() == g,
            Cond::InGame(want) => f.in_game() == *want,
            Cond::Mood(m) => f.mood() == m,
            Cond::RelationAbove(v) => f.relation() > *v,
            Cond::RelationBelow(v) => f.relation() < *v,
            Cond::Not(c) => !c.holds(f),
            Cond::All(list) => list.iter().all(|c| c.holds(f)),
            Cond::Any(list) => list.iter().any(|c| c.holds(f)),
        }
    }
}

impl Choice {
    pub fn shown(&self, f: &dyn Facts) -> bool {
        self.condition.as_ref().is_none_or(|c| c.holds(f))
    }
}

/// `@player` and `@npc` filled in.
pub fn fill(text: &str, player: &str, npc: &str) -> String {
    text.replace("@player", player).replace("@npc", npc)
}

/// The command lines an action stands for (the ones the server runs as its console).
pub fn commands(action: &Action, player: &str, npc: &str) -> Vec<String> {
    match action {
        Action::Command(c) => vec![fill(c.trim_start_matches('/'), player, npc)],
        Action::Give(item, n) => vec![format!("give {player} {item} {n}")],
        Action::Take(item, n) => vec![format!("clear {player} {item} {n}")],
        Action::Tag(t) => vec![format!("tag {player} add {t}")],
        Action::Untag(t) => vec![format!("tag {player} remove {t}")],
        Action::Sound(s) => vec![format!("execute as {npc} at @s run playsound {s} neutral @a ~ ~ ~")],
        Action::Animation(_) | Action::Order(_) | Action::Scene(_) | Action::Game(..) | Action::Sport(_) | Action::EditAnimation(_) | Action::JoinGame | Action::OpenGame | Action::Feel(_) | Action::Relation(_) => Vec::new(),
    }
}

pub fn parse(bytes: &[u8]) -> Result<Dialog, String> {
    let root: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    let root = root.as_object().ok_or("not an object")?;
    let start = root.get("start").and_then(Value::as_str).ok_or("no \"start\"")?.to_string();
    let raw = root.get("nodes").and_then(Value::as_object).ok_or("no \"nodes\"")?;
    let mut nodes = HashMap::new();
    for (name, n) in raw {
        let n = n.as_object().ok_or_else(|| format!("node {name} is not an object"))?;
        nodes.insert(name.clone(), read_node(n).map_err(|e| format!("node {name}: {e}"))?);
    }
    if !nodes.contains_key(&start) {
        return Err(format!("start node {start} is missing"));
    }
    for (name, n) in &nodes {
        for o in &n.options {
            if let Some(g) = &o.goto
                && !nodes.contains_key(g)
            {
                return Err(format!("node {name}: option {:?} goes to missing node {g}", o.text));
            }
        }
    }
    Ok(Dialog { start, nodes })
}

fn texts(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

fn read_node(n: &Map<String, Value>) -> Result<Node, String> {
    let mut options = Vec::new();
    if let Some(list) = n.get("options").and_then(Value::as_array) {
        for (i, o) in list.iter().enumerate() {
            let o = o.as_object().ok_or_else(|| format!("option {i} is not an object"))?;
            options.push(Choice {
                text: o.get("text").and_then(Value::as_str).ok_or_else(|| format!("option {i} has no text"))?.to_string(),
                condition: o.get("if").map(read_cond).transpose().map_err(|e| format!("option {i}: {e}"))?,
                actions: read_actions(o.get("actions")).map_err(|e| format!("option {i}: {e}"))?,
                goto: o.get("goto").and_then(Value::as_str).map(str::to_string),
            });
        }
    }
    Ok(Node {
        text: texts(n.get("text")),
        animation: n.get("animation").and_then(Value::as_str).map(str::to_string),
        actions: read_actions(n.get("actions"))?,
        options,
    })
}

fn read_cond(v: &Value) -> Result<Cond, String> {
    let m = v.as_object().ok_or("a condition is not an object")?;
    let count = m.get("count").and_then(Value::as_u64).unwrap_or(1) as u32;
    let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
    let list = |k: &str| -> Result<Option<Vec<Cond>>, String> {
        match m.get(k) {
            Some(Value::Array(a)) => Ok(Some(a.iter().map(read_cond).collect::<Result<_, _>>()?)),
            Some(_) => Err(format!("{k} is not a list")),
            None => Ok(None),
        }
    };
    if let Some(t) = s("tag") {
        Ok(Cond::Tag(t))
    } else if let Some(i) = s("item") {
        Ok(Cond::Item(i, count))
    } else if let Some(b) = m.get("owner").and_then(Value::as_bool) {
        Ok(Cond::Owner(b))
    } else if let Some(mode) = s("mode") {
        Ok(Cond::Mode(mode))
    } else if let Some(g) = s("game") {
        Ok(Cond::Game(g))
    } else if let Some(b) = m.get("in_game").and_then(Value::as_bool) {
        Ok(Cond::InGame(b))
    } else if let Some(mood) = s("mood") {
        Ok(Cond::Mood(mood))
    } else if let Some(v) = m.get("relation_above").and_then(Value::as_f64) {
        Ok(Cond::RelationAbove(v as i32))
    } else if let Some(v) = m.get("relation_below").and_then(Value::as_f64) {
        Ok(Cond::RelationBelow(v as i32))
    } else if let Some(n) = m.get("not") {
        Ok(Cond::Not(Box::new(read_cond(n)?)))
    } else if let Some(all) = list("all")? {
        Ok(Cond::All(all))
    } else if let Some(any) = list("any")? {
        Ok(Cond::Any(any))
    } else {
        Err(format!("unknown condition {v}"))
    }
}

pub fn read_actions(v: Option<&Value>) -> Result<Vec<Action>, String> {
    let Some(v) = v else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or("actions is not a list")?;
    list.iter()
        .map(|a| {
            let m = a.as_object().ok_or("an action is not an object")?;
            let count = m.get("count").and_then(Value::as_u64).unwrap_or(1) as u32;
            let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
            Ok(if let Some(c) = s("command") {
                Action::Command(c)
            } else if let Some(i) = s("give") {
                Action::Give(i, count)
            } else if let Some(i) = s("take") {
                Action::Take(i, count)
            } else if let Some(t) = s("tag") {
                Action::Tag(t)
            } else if let Some(t) = s("untag") {
                Action::Untag(t)
            } else if let Some(a) = s("animation") {
                Action::Animation(a)
            } else if let Some(o) = s("order") {
                Action::Order(o)
            } else if let Some(snd) = s("sound") {
                Action::Sound(snd)
            } else if let Some(sc) = s("scene") {
                Action::Scene(sc)
            } else if let Some(g) = s("game") {
                Action::Game(g, m.get("vs").and_then(Value::as_str) != Some("player"))
            } else if let Some(sp) = s("sport") {
                Action::Sport(sp)
            } else if let Some(v) = m.get("edit_animation") {
                Action::EditAnimation(v.as_str().map(str::to_string))
            } else if m.contains_key("join_game") {
                Action::JoinGame
            } else if m.contains_key("open_game") {
                Action::OpenGame
            } else if let Some(f) = s("feel") {
                Action::Feel(f)
            } else if let Some(v) = m.get("relation").and_then(Value::as_f64) {
                Action::Relation(v as i32)
            } else {
                return Err(format!("unknown action {a}"));
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct F {
        tags: Vec<&'static str>,
        apples: u32,
        owner: bool,
    }
    impl Facts for F {
        fn has_tag(&self, t: &str) -> bool {
            self.tags.contains(&t)
        }
        fn item_count(&self, item: &str) -> u32 {
            if item == "minecraft:apple" { self.apples } else { 0 }
        }
        fn is_owner(&self) -> bool {
            self.owner
        }
        fn mode(&self) -> &str {
            "follow"
        }
    }

    const SAMPLE: &str = r#"{ "start": "hello", "nodes": {
        "hello": { "text": "Hi, @player!", "animation": "animation.felix.wave",
          "options": [
            { "text": "A gift?", "if": { "not": { "tag": "gift" } }, "actions": [ { "give": "minecraft:apple", "count": 3 }, { "tag": "gift" } ], "goto": "thanks" },
            { "text": "Trade", "if": { "all": [ { "item": "minecraft:apple", "count": 2 }, { "owner": true } ] }, "actions": [ { "take": "minecraft:apple", "count": 2 } ] },
            { "text": "Bye" } ] },
        "thanks": { "text": ["Here.", "Enjoy."], "options": [ { "text": "Thanks", "if": { "any": [ { "mode": "wait" }, { "tag": "gift" } ] } } ] } } }"#;

    #[test]
    fn reads_and_filters() {
        let d = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(d.start, "hello");
        let hello = &d.nodes["hello"];
        assert_eq!(hello.animation.as_deref(), Some("animation.felix.wave"));
        let new = F { tags: vec![], apples: 0, owner: false };
        let shown: Vec<&str> = hello.options.iter().filter(|o| o.shown(&new)).map(|o| o.text.as_str()).collect();
        assert_eq!(shown, ["A gift?", "Bye"]);
        let rich = F { tags: vec!["gift"], apples: 5, owner: true };
        let shown: Vec<&str> = hello.options.iter().filter(|o| o.shown(&rich)).map(|o| o.text.as_str()).collect();
        assert_eq!(shown, ["Trade", "Bye"]);
        assert_eq!(d.nodes["thanks"].text, ["Here.", "Enjoy."]);
        assert!(d.nodes["thanks"].options[0].shown(&rich));
    }

    #[test]
    fn actions_become_commands() {
        let d = parse(SAMPLE.as_bytes()).unwrap();
        let gift = &d.nodes["hello"].options[0];
        let lines: Vec<String> = gift.actions.iter().flat_map(|a| commands(a, "Alex", "1-2-3-4-5")).collect();
        assert_eq!(lines, ["give Alex minecraft:apple 3", "tag Alex add gift"]);
        assert_eq!(fill("Hi, @player!", "Alex", "x"), "Hi, Alex!");
        assert_eq!(commands(&Action::Command("/say @player meets @npc".into()), "Alex", "u"), ["say Alex meets u"]);
    }

    #[test]
    fn refuses_broken_files() {
        assert!(parse(br#"{ "start": "a", "nodes": {} }"#).unwrap_err().contains("start node"));
        assert!(parse(br#"{ "start": "a", "nodes": { "a": { "options": [ { "text": "x", "goto": "b" } ] } } }"#).unwrap_err().contains("missing node b"));
        assert!(parse(br#"{ "start": "a", "nodes": { "a": { "options": [ { "text": "x", "if": { "weird": 1 } } ] } } }"#).is_err());
    }
}
