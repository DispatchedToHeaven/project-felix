//! An NPC's character, as two graphs. No mod API types here.
//!
//! - Its moods: nodes (`calm`, `upset`, `angry`…) joined by what happens to it (edges), each mood
//!   with how well the NPC obeys in it, how fast it walks, what it says and plays on coming in.
//! - Its relations: how much it likes each player and each other NPC, from -100 to 100. Players
//!   who hit it, or hit an NPC it is friends with, fall in its eyes; gifts and talk raise them.
//!
//! A character is `data/<namespace>/felix_characters/<name>.json`:
//!
//! ```json
//! { "start": "calm", "friend_at": 30, "hate_at": -50, "owner": 50, "forgive": 2,
//!   "relations": { "hurt_by_player": -10, "friend_hurt": -15, "friend_killed": -40, "gift": 8 },
//!   "moods": {
//!     "calm":  { "on": { "hurt_by_player": "upset", "friend_hurt": "upset", "gift": "happy" } },
//!     "upset": { "obey": 0.5, "speed": 0.75, "delay": 3, "particle": "minecraft:dripping_water",
//!                "say": [ "Why, @player?" ], "refuse": [ "Not now." ], "animation": "animation.felix.sad",
//!                "on": { "after:90": "calm", "gift": "calm",
//!                        "hurt_by_player": [ { "to": "angry", "below": -20 }, { "to": "upset" } ] } } } }
//! ```
//!
//! Events: `hurt`, `hurt_by_player`, `friend_hurt` (a player hit an NPC it is friends with),
//! `friend_killed`, `owner_hurt`, `talk`, `gift`, `apology`, `praise`, `scold`, `refused`, any other
//! name a dialog or a command feels, and `after:<seconds>` in a mood. An edge may ask the relation
//! with the player of the event to be `above` or `below` a value; the first edge that fits is taken.
//! `relations` says how much an event moves the relation with its player (a friend's hurt counts
//! more the closer the friend). Relations below where they began come back `forgive` points a
//! minute; NPCs near each other grow friends `bond` points a minute, up to `bond_max`.

use crate::serde_json::{Map, Value};
use std::collections::HashMap;

pub const LOWEST: i32 = -100;
pub const HIGHEST: i32 = 100;
/// How many relations an NPC keeps.
pub const MAX_BONDS: usize = 48;

#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    pub start: String,
    pub moods: HashMap<String, Mood>,
    /// How an event moves the relation with its player.
    pub relations: HashMap<String, i32>,
    /// Points a minute that relations below their start come back.
    pub forgive: f32,
    /// From here on an NPC (or a player) is a friend.
    pub friend_at: i32,
    /// From here down the NPC won't talk to that player.
    pub hate_at: i32,
    /// Where the relation with its owner begins.
    pub owner: i32,
    /// Points a minute NPCs near each other grow friends, and up to where.
    pub bond: f32,
    pub bond_max: i32,
    /// Said to a player it hates, who wants to talk.
    pub shun: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mood {
    /// The chance it does what it is told (0 to 1), before relations.
    pub obey: f32,
    /// Walking speed, times.
    pub speed: f32,
    /// Seconds it takes before it does what it is told.
    pub delay: f32,
    pub say: Vec<String>,
    pub refuse: Vec<String>,
    /// Said when it does what it is told, but slowly.
    pub grumble: Vec<String>,
    pub animation: Option<String>,
    pub particle: Option<String>,
    pub edges: Vec<Edge>,
    /// After so many seconds in this mood, go to that one.
    pub after: Option<(f32, String)>,
}

impl Default for Mood {
    fn default() -> Mood {
        Mood {
            obey: 1.0,
            speed: 1.0,
            delay: 0.0,
            say: Vec::new(),
            refuse: Vec::new(),
            grumble: Vec::new(),
            animation: None,
            particle: None,
            edges: Vec::new(),
            after: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub on: String,
    pub to: String,
    pub above: Option<i32>,
    pub below: Option<i32>,
}

impl Edge {
    fn fits(&self, event: &str, relation: i32) -> bool {
        self.on == event && self.above.is_none_or(|a| relation > a) && self.below.is_none_or(|b| relation < b)
    }
}

impl Character {
    pub fn mood(&self, name: &str) -> &Mood {
        static CALM: std::sync::LazyLock<Mood> = std::sync::LazyLock::new(Mood::default);
        self.moods.get(name).or_else(|| self.moods.get(&self.start)).unwrap_or(&CALM)
    }

    /// The mood `event` leads to from `mood` (None: it stays), given the relation with its player.
    pub fn next(&self, mood: &str, event: &str, relation: i32) -> Option<&str> {
        let m = self.moods.get(mood).or_else(|| self.moods.get(&self.start))?;
        m.edges.iter().find(|e| e.fits(event, relation)).map(|e| e.to.as_str()).filter(|to| *to != mood)
    }

    /// The mood `mood` turns into after `seconds` in it, if it is time.
    pub fn timed(&self, mood: &str, seconds: f32) -> Option<&str> {
        let (after, to) = self.moods.get(mood)?.after.as_ref()?;
        (seconds >= *after && to != mood).then_some(to.as_str())
    }

    /// How much `event` moves the relation with its player.
    pub fn delta(&self, event: &str) -> i32 {
        self.relations.get(event).copied().unwrap_or(0)
    }

    /// The chance it does what a player it feels `relation` for tells it, in `mood`.
    pub fn obey_chance(&self, mood: &str, relation: i32) -> f32 {
        let m = self.mood(mood);
        // Liking doesn't make it obey more than its mood lets it; dislike does make it less.
        let like = 1.0 + relation.min(0) as f32 / 133.0;
        if relation <= self.hate_at { 0.0 } else { (m.obey * like).clamp(0.0, 1.0) }
    }

    /// A relation in words.
    pub fn label(&self, value: i32) -> &'static str {
        if value >= self.friend_at {
            "friend"
        } else if value >= 10 {
            "likes"
        } else if value > -10 {
            "neutral"
        } else if value > self.hate_at {
            "dislikes"
        } else {
            "hates"
        }
    }

    /// Every mood a mood leads to (for checks and for showing the graph).
    pub fn leads_to(&self, mood: &str) -> Vec<(String, &str)> {
        let Some(m) = self.moods.get(mood) else { return Vec::new() };
        let mut out: Vec<(String, &str)> = m
            .edges
            .iter()
            .map(|e| {
                let mut on = e.on.clone();
                if let Some(a) = e.above {
                    on += &format!(" above {a}");
                }
                if let Some(b) = e.below {
                    on += &format!(" below {b}");
                }
                (on, e.to.as_str())
            })
            .collect();
        if let Some((secs, to)) = &m.after {
            out.push((format!("after {secs} s"), to.as_str()));
        }
        out
    }
}

/// How much more a friend's hurt counts the closer the friend: 0.5 to 2 times.
pub fn friend_scale(friendship: i32) -> f32 {
    (friendship as f32 / 50.0).clamp(0.5, 2.0)
}

// ---- reading ------------------------------------------------------------------------------------------

pub fn parse(bytes: &[u8]) -> Result<Character, String> {
    let root: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    read(&root)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

pub fn read(root: &Value) -> Result<Character, String> {
    let m = root.as_object().ok_or("not an object")?;
    let raw = m.get("moods").and_then(Value::as_object).ok_or("no \"moods\"")?;
    let mut moods = HashMap::new();
    for (name, v) in raw {
        moods.insert(name.clone(), read_mood(v).map_err(|e| format!("mood {name}: {e}"))?);
    }
    let start = m.get("start").and_then(Value::as_str).unwrap_or("calm").to_string();
    if !moods.contains_key(&start) {
        return Err(format!("no mood {start} to start in"));
    }
    for (name, mood) in &moods {
        for to in mood.edges.iter().map(|e| &e.to).chain(mood.after.as_ref().map(|a| &a.1)) {
            if !moods.contains_key(to) {
                return Err(format!("mood {name} leads to {to}, which is not there"));
            }
        }
    }
    let num = |k: &str, d: f64| m.get(k).and_then(Value::as_f64).unwrap_or(d);
    let relations = match m.get("relations") {
        None => HashMap::new(),
        Some(r) => r
            .as_object()
            .ok_or("relations is not an object")?
            .iter()
            .map(|(k, v)| Ok((k.clone(), v.as_f64().ok_or(format!("relation {k} is not a number"))? as i32)))
            .collect::<Result<_, String>>()?,
    };
    let clamp = |v: f64| (v as i32).clamp(LOWEST, HIGHEST);
    Ok(Character {
        start,
        moods,
        relations,
        forgive: num("forgive", 2.0).max(0.0) as f32,
        friend_at: clamp(num("friend_at", 30.0)),
        hate_at: clamp(num("hate_at", -50.0)),
        owner: clamp(num("owner", 50.0)),
        bond: num("bond", 1.0).max(0.0) as f32,
        bond_max: clamp(num("bond_max", 60.0)),
        shun: strings(m.get("shun")),
    })
}

fn read_mood(v: &Value) -> Result<Mood, String> {
    let m = v.as_object().ok_or("not an object")?;
    let num = |k: &str, d: f64| m.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
    let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
    let mut edges = Vec::new();
    let mut after = None;
    if let Some(on) = m.get("on") {
        let on: &Map<String, Value> = on.as_object().ok_or("\"on\" is not an object")?;
        for (event, to) in on {
            if let Some(secs) = event.strip_prefix("after:") {
                let secs: f32 = secs.trim().parse().map_err(|_| format!("{event}: seconds, as after:90"))?;
                let to = to.as_str().ok_or(format!("{event} leads to a mood name"))?;
                after = Some((secs.max(1.0), to.to_string()));
                continue;
            }
            let list = match to {
                Value::Array(a) => a.clone(),
                other => vec![other.clone()],
            };
            for one in list {
                let edge = match &one {
                    Value::String(to) => Edge { on: event.clone(), to: to.clone(), above: None, below: None },
                    Value::Object(o) => Edge {
                        on: event.clone(),
                        to: o.get("to").and_then(Value::as_str).ok_or(format!("{event}: no \"to\""))?.to_string(),
                        above: o.get("above").and_then(Value::as_f64).map(|x| x as i32),
                        below: o.get("below").and_then(Value::as_f64).map(|x| x as i32),
                    },
                    _ => return Err(format!("{event} leads to a mood name or {{ \"to\": … }}")),
                };
                edges.push(edge);
            }
        }
    }
    Ok(Mood {
        obey: num("obey", 1.0).clamp(0.0, 1.0),
        speed: num("speed", 1.0).clamp(0.1, 3.0),
        delay: num("delay", 0.0).clamp(0.0, 60.0),
        say: strings(m.get("say")),
        refuse: strings(m.get("refuse")),
        grumble: strings(m.get("grumble")),
        animation: s("animation"),
        particle: s("particle"),
        edges,
        after,
    })
}

// ---- what one NPC feels ---------------------------------------------------------------------------------

/// One relation: with a player or an NPC (by UUID), the name it had, and how much.
#[derive(Clone, Debug, PartialEq)]
pub struct Bond {
    pub who: u128,
    pub name: String,
    pub npc: bool,
    pub value: f32,
    /// Where it began (forgiving goes back up to here).
    pub base: i32,
}

/// An NPC's heart: its character, its mood (since when, in game ticks) and its relations.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Heart {
    pub character: Option<String>,
    pub mood: String,
    pub since: i64,
    pub bonds: Vec<Bond>,
}

impl Heart {
    pub fn get(&self, who: u128) -> Option<&Bond> {
        self.bonds.iter().find(|b| b.who == who)
    }

    pub fn value(&self, who: u128) -> Option<i32> {
        self.get(who).map(|b| b.value.round() as i32)
    }

    /// Moves the relation with `who` by `delta` (from `base` if it is new); the new value.
    pub fn change(&mut self, who: u128, name: &str, npc: bool, base: i32, delta: f32) -> i32 {
        let i = match self.bonds.iter().position(|b| b.who == who) {
            Some(i) => i,
            None => {
                self.bonds.push(Bond { who, name: name.to_string(), npc, value: base as f32, base });
                self.bonds.len() - 1
            }
        };
        let b = &mut self.bonds[i];
        if !name.is_empty() {
            b.name = name.to_string();
        }
        b.value = (b.value + delta).clamp(LOWEST as f32, HIGHEST as f32);
        let v = b.value.round() as i32;
        self.trim();
        v
    }

    /// Sets the relation with `who`.
    pub fn set(&mut self, who: u128, name: &str, npc: bool, value: i32) {
        let base = self.get(who).map_or(0, |b| b.base);
        let now = self.get(who).map_or(base as f32, |b| b.value);
        self.change(who, name, npc, base, value as f32 - now);
    }

    /// `minutes` of forgiving: relations below where they began come back `rate` points a minute.
    pub fn forgive(&mut self, minutes: f32, rate: f32) {
        for b in &mut self.bonds {
            if b.value < b.base as f32 {
                b.value = (b.value + rate * minutes).min(b.base as f32);
            }
        }
    }

    /// Keeps the relations that mean the most, and no more than [`MAX_BONDS`].
    fn trim(&mut self) {
        if self.bonds.len() <= MAX_BONDS {
            return;
        }
        // The least telling first: those nearest where they began.
        self.bonds.sort_by(|a, b| (b.value - b.base as f32).abs().total_cmp(&(a.value - a.base as f32).abs()));
        self.bonds.truncate(MAX_BONDS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIND: &str = include_str!("../data/projectfelix/felix_characters/kind.json");
    const TOUCHY: &str = include_str!("../data/projectfelix/felix_characters/touchy.json");
    const STOIC: &str = include_str!("../data/projectfelix/felix_characters/stoic.json");

    #[test]
    fn the_samples_read() {
        for (name, text) in [("kind", KIND), ("touchy", TOUCHY), ("stoic", STOIC)] {
            let c = parse(text.as_bytes()).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(c.start, "calm", "{name}");
            assert!(c.delta("hurt_by_player") < 0 && c.delta("friend_hurt") < 0 && c.delta("gift") > 0, "{name}");
            // Every mood comes back to calm sooner or later.
            for (m, mood) in &c.moods {
                assert!(m == "calm" || mood.after.is_some(), "{name}: {m} never ends");
            }
        }
    }

    #[test]
    fn moods_move_along_the_graph() {
        let c = parse(KIND.as_bytes()).unwrap();
        assert_eq!(c.next("calm", "friend_hurt", 0), Some("upset"));
        assert_eq!(c.next("calm", "gift", 0), Some("happy"));
        assert_eq!(c.next("calm", "nothing at all", 0), None);
        // Hit again when it already dislikes you: angry; else still upset (no move).
        assert_eq!(c.next("upset", "hurt_by_player", -40), Some("angry"));
        assert_eq!(c.next("upset", "hurt_by_player", 10), None);
        assert_eq!(c.next("upset", "apology", 0), Some("calm"));
        assert_eq!(c.timed("upset", 10.0), None);
        assert_eq!(c.timed("upset", 600.0), Some("calm"));
        assert!(c.mood("upset").obey < c.mood("calm").obey);
        assert!(c.mood("upset").speed < 1.0);
        // An unknown mood reads as the start.
        assert_eq!(c.next("lost", "gift", 0), Some("happy"));
        assert!(c.leads_to("upset").contains(&("after 180 s".to_string(), "calm")));
        assert!(c.leads_to("upset").contains(&("hurt_by_player below -20".to_string(), "angry")));
    }

    #[test]
    fn obeys_less_when_upset_or_disliked() {
        let c = parse(KIND.as_bytes()).unwrap();
        assert_eq!(c.obey_chance("calm", 50), 1.0);
        assert_eq!(c.obey_chance("calm", 0), 1.0);
        assert!(c.obey_chance("calm", -40) < 0.75);
        assert!(c.obey_chance("upset", 0) < c.obey_chance("calm", 0));
        assert!(c.obey_chance("angry", -20) < c.obey_chance("upset", -20));
        assert_eq!(c.obey_chance("calm", c.hate_at), 0.0);
        assert_eq!(c.label(60), "friend");
        assert_eq!(c.label(0), "neutral");
        assert_eq!(c.label(-80), "hates");
        assert_eq!(friend_scale(100), 2.0);
        assert_eq!(friend_scale(5), 0.5);
    }

    #[test]
    fn hearts_remember_and_forgive() {
        let mut h = Heart::default();
        assert_eq!(h.change(7, "Alex", false, 50, -15.0), 35);
        assert_eq!(h.change(7, "", false, 50, -200.0), LOWEST);
        assert_eq!(h.get(7).map(|b| b.name.as_str()), Some("Alex"));
        h.forgive(10.0, 2.0);
        assert_eq!(h.value(7), Some(-80));
        h.forgive(1000.0, 2.0);
        assert_eq!(h.value(7), Some(50), "back to where it began, not above");
        h.set(9, "Bob", true, 70);
        assert_eq!(h.value(9), Some(70));
        h.forgive(5.0, 2.0);
        assert_eq!(h.value(9), Some(70), "liking does not fade");
        for who in 100..200 {
            h.change(who, "x", false, 0, (who - 100) as f32 / 10.0);
        }
        assert_eq!(h.bonds.len(), MAX_BONDS);
        assert!(h.get(9).is_some() && h.get(7).is_none(), "the strong ones stay, the forgiven go");
    }

    #[test]
    fn refuses_broken_graphs() {
        assert!(parse(br#"{ "moods": { "calm": { "on": { "gift": "glad" } } } }"#).unwrap_err().contains("glad"));
        assert!(parse(br#"{ "start": "x", "moods": { "calm": {} } }"#).is_err());
        assert!(parse(br#"{ "moods": { "calm": { "on": { "after:soon": "calm" } } } }"#).is_err());
        let c = parse(br#"{ "moods": { "calm": {} } }"#).unwrap();
        assert_eq!((c.friend_at, c.hate_at, c.owner), (30, -50, 50));
    }
}
