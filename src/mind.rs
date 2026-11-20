//! An NPC's mind, as in a life sim: what it thinks (thoughts, each moving its mood for a while or
//! while something is so), its needs besides food (rest, fun), its traits, and where its mood
//! leads: breaks when it stays low, inspirations when it is high. No mod API types here.
//!
//! Mood is 0 to 100. Its target is `BASE`, plus its traits, plus every thought; the mood moves
//! toward it 12 points a game hour up and 8 down, and stands still while it sleeps. Below 35
//! (minor), 20 (major) or 5 (extreme) it may break, the lower the sooner; a break over leaves it
//! relieved (+40 for three days). At 50 and above it may be inspired, the higher the sooner.
//!
//! Thoughts are memories (they last a while; the same one again stacks, each copy counting less,
//! up to a limit) or now (true while something is: tired, hungry, a cosy home).

use crate::serde_json::{Map, Value, json};

/// Game ticks in an hour, a day.
pub const HOUR: i64 = 1000;
pub const DAY: i64 = 24000;
/// The mood with nothing on its mind.
pub const BASE: f32 = 45.0;
/// Fun by kind: talk, games, gatherings, walks and sights.
pub const TALK: usize = 0;
pub const GAMES: usize = 1;
pub const GATHERING: usize = 2;
pub const SIGHTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trait {
    Kind,
    Rude,
    Cheerful,
    Gloomy,
    Steady,
    Nervous,
    HardWorking,
    Lazy,
    NightOwl,
    Greedy,
    Modest,
    Glutton,
    SoundSleeper,
    Chatty,
    Loner,
}

/// Every trait, its word, and the one it can't go with.
const TRAITS: &[(Trait, &str, Option<Trait>)] = &[
    (Trait::Kind, "kind", Some(Trait::Rude)),
    (Trait::Rude, "rude", Some(Trait::Kind)),
    (Trait::Cheerful, "cheerful", Some(Trait::Gloomy)),
    (Trait::Gloomy, "gloomy", Some(Trait::Cheerful)),
    (Trait::Steady, "steady", Some(Trait::Nervous)),
    (Trait::Nervous, "nervous", Some(Trait::Steady)),
    (Trait::HardWorking, "hard-working", Some(Trait::Lazy)),
    (Trait::Lazy, "lazy", Some(Trait::HardWorking)),
    (Trait::NightOwl, "a night owl", None),
    (Trait::Greedy, "greedy", Some(Trait::Modest)),
    (Trait::Modest, "modest", Some(Trait::Greedy)),
    (Trait::Glutton, "a big eater", None),
    (Trait::SoundSleeper, "a sound sleeper", None),
    (Trait::Chatty, "chatty", Some(Trait::Loner)),
    (Trait::Loner, "a loner", Some(Trait::Chatty)),
];

impl Trait {
    pub fn word(self) -> &'static str {
        TRAITS.iter().find(|t| t.0 == self).map(|t| t.1).unwrap_or("")
    }
    pub fn parse(s: &str) -> Option<Trait> {
        let s = s.trim_start_matches("a_").trim_start_matches("a ");
        TRAITS.iter().find(|t| {
            let w = t.1.trim_start_matches("a ");
            w == s || w.replace([' ', '-'], "_") == s
        })
        .map(|t| t.0)
    }
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

/// An NPC's traits (one to three), by its UUID.
pub fn traits(uuid: u128) -> Vec<Trait> {
    let mut h = mix((uuid as u64) ^ ((uuid >> 64) as u64) ^ 0x7A17_5EED);
    let n = match h % 10 {
        0..=2 => 1,
        3..=7 => 2,
        _ => 3,
    };
    let mut out: Vec<Trait> = Vec::new();
    for _ in 0..12 {
        if out.len() == n {
            break;
        }
        h = mix(h);
        let (t, _, against) = TRAITS[(h % TRAITS.len() as u64) as usize];
        if !out.contains(&t) && against.is_none_or(|a| !out.contains(&a)) {
            out.push(t);
        }
    }
    out
}

/// Its traits as it would be described: "kind, lazy and a night owl".
pub fn words(traits: &[Trait]) -> String {
    let w: Vec<&str> = traits.iter().map(|t| t.word()).collect();
    match w.split_last() {
        None => String::new(),
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// How much faster (or slower) it works.
pub fn work_factor(traits: &[Trait]) -> f32 {
    if traits.contains(&Trait::HardWorking) {
        1.35
    } else if traits.contains(&Trait::Lazy) {
        0.8
    } else {
        1.0
    }
}

/// How much faster it gets hungry.
pub fn hunger_factor(traits: &[Trait]) -> f32 {
    if traits.contains(&Trait::Glutton) { 1.5 } else { 1.0 }
}

/// A thought: what it says of it, how it moves the mood, how long a memory of it lasts (0: a
/// thought of now), and how many copies stack.
pub struct Thought {
    pub id: &'static str,
    pub label: &'static str,
    pub mood: f32,
    pub lasts: i64,
    pub stack: usize,
}

const fn t(id: &'static str, label: &'static str, mood: f32, lasts: i64, stack: usize) -> Thought {
    Thought { id, label, mood, lasts, stack }
}

pub const THOUGHTS: &[Thought] = &[
    // Memories.
    t("gift", "Got a present", 6.0, DAY, 3),
    t("praised", "Kind words", 4.0, DAY, 3),
    t("talked", "A nice talk", 2.0, DAY / 2, 3),
    t("apology", "Got an apology", 3.0, DAY, 1),
    t("scolded", "Got told off", -6.0, DAY, 3),
    t("hurt", "Was hit", -8.0, DAY, 3),
    t("friend_hurt", "A friend was hit", -4.0, DAY, 3),
    t("friend_died", "A friend died", -10.0, 8 * DAY, 3),
    t("family_died", "Lost family", -20.0, 15 * DAY, 2),
    t("saw_death", "Saw someone die", -4.0, 2 * DAY, 4),
    t("won_game", "Won a game", 5.0, DAY, 2),
    t("lost_game", "Lost a game", -2.0, DAY, 2),
    t("festival", "A lovely festival", 8.0, 2 * DAY, 1),
    t("partnered", "In love", 10.0, 8 * DAY, 1),
    t("married", "Got married", 25.0, 10 * DAY, 1),
    t("broke_up", "Heartbroken", -15.0, 8 * DAY, 1),
    t("ate_raw", "Ate raw food", -3.0, DAY / 2, 1),
    t("ate_meal", "A good meal", 3.0, DAY / 2, 1),
    t("ate_treat", "A treat", 6.0, DAY / 2, 1),
    t("slept_rough", "Slept without a bed", -4.0, DAY, 1),
    t("slept_cold", "Slept in the rain", -4.0, DAY, 1),
    t("relieved", "Got it all out", 40.0, 3 * DAY, 1),
    t("insulted", "Was insulted", -5.0, DAY, 5),
    t("hard_work", "Overworked", -3.0, DAY / 2, 1),
    t("liked_work", "Good day's work", 3.0, DAY / 2, 1),
    // Now.
    t("hungry", "Hungry", -6.0, 0, 1),
    t("starving", "Starving", -15.0, 0, 1),
    t("drowsy", "Drowsy", -4.0, 0, 1),
    t("tired", "Tired", -8.0, 0, 1),
    t("exhausted", "Exhausted", -15.0, 0, 1),
    t("fun_high", "Having fun lately", 6.0, 0, 1),
    t("fun_low", "Bored", -5.0, 0, 1),
    t("fun_none", "Bored stiff", -10.0, 0, 1),
    t("home_awful", "Dreary home", -4.0, 0, 1),
    t("home_cosy", "Cosy home", 3.0, 0, 1),
    t("home_fine", "Fine home", 5.0, 0, 1),
    t("home_lovely", "Lovely home", 8.0, 0, 1),
    t("homeless", "No home of its own", -6.0, 0, 1),
    t("wants_more", "Wants a finer home", -5.0, 0, 1),
    t("humble", "Likes a humble home", 3.0, 0, 1),
    t("soaked", "Soaked", -3.0, 0, 1),
    t("pain", "In pain", -6.0, 0, 1),
    t("bad_pain", "Hurting badly", -12.0, 0, 1),
    t("night_day", "Awake in daylight", -4.0, 0, 1),
    t("night_good", "Loves the night", 6.0, 0, 1),
    t("with_love", "Has someone", 4.0, 0, 1),
    t("cheerful", "Cheerful", 6.0, 0, 1),
    t("gloomy", "Gloomy", -6.0, 0, 1),
];

pub fn thought(id: &str) -> Option<&'static Thought> {
    THOUGHTS.iter().find(|t| t.id == id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Break {
    /// Minor: wanders about sadly, does no work.
    Sulk,
    /// Minor: shuts itself in at home and won't talk.
    Hide,
    /// Minor: eats what it can find.
    Binge,
    /// Minor: snaps at whoever is near.
    Snap,
    /// Major: throws its things about.
    Tantrum,
    /// Major: wanders about, lost in thought.
    Daze,
    /// Extreme: walks off for a day.
    Leave,
    /// Extreme: lies down and won't move.
    Freeze,
}

const BREAKS: &[(Break, &str)] = &[
    (Break::Sulk, "sulk"),
    (Break::Hide, "hide"),
    (Break::Binge, "binge"),
    (Break::Snap, "snap"),
    (Break::Tantrum, "tantrum"),
    (Break::Daze, "daze"),
    (Break::Leave, "leave"),
    (Break::Freeze, "freeze"),
];

impl Break {
    pub fn name(self) -> &'static str {
        BREAKS.iter().find(|b| b.0 == self).map(|b| b.1).unwrap_or("")
    }
    pub fn parse(s: &str) -> Option<Break> {
        BREAKS.iter().find(|b| b.1 == s).map(|b| b.0)
    }
    /// How long it lasts (ticks).
    pub fn lasts(self) -> i64 {
        match self {
            Break::Binge => HOUR,
            Break::Sulk | Break::Hide | Break::Snap => 8 * HOUR,
            Break::Tantrum | Break::Daze => 10 * HOUR,
            Break::Leave | Break::Freeze => DAY,
        }
    }
    /// What others see it doing.
    pub fn doing(self) -> &'static str {
        match self {
            Break::Sulk => "sulking",
            Break::Hide => "hiding at home",
            Break::Binge => "eating everything",
            Break::Snap => "snapping at everyone",
            Break::Tantrum => "throwing a tantrum",
            Break::Daze => "wandering in a daze",
            Break::Leave => "gone off on its own",
            Break::Freeze => "lying there, numb",
        }
    }
    /// What it says as it starts.
    pub fn line(self) -> &'static str {
        match self {
            Break::Sulk => "Leave me be. I just need to walk.",
            Break::Hide => "I'm going home. Don't follow me.",
            Break::Binge => "I need to eat something. Anything.",
            Break::Snap => "What are you looking at?!",
            Break::Tantrum => "I've had it! I've had it with all of this!",
            Break::Daze => "...",
            Break::Leave => "I can't stay here. I'm going.",
            Break::Freeze => "I can't... I can't anymore.",
        }
    }
    /// It won't talk or take orders while it lasts.
    pub fn shuts_out(self) -> bool {
        !matches!(self, Break::Binge)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inspiration {
    /// Works twice as fast.
    Work,
    /// Sells for less, buys for more.
    Trade,
    /// Gives a present to the player it likes best.
    Gift,
}

impl Inspiration {
    pub fn name(self) -> &'static str {
        match self {
            Inspiration::Work => "work",
            Inspiration::Trade => "trade",
            Inspiration::Gift => "gift",
        }
    }
    pub fn parse(s: &str) -> Option<Inspiration> {
        [Inspiration::Work, Inspiration::Trade, Inspiration::Gift].into_iter().find(|i| i.name() == s)
    }
    pub fn line(self) -> &'static str {
        match self {
            Inspiration::Work => "I feel like I could work all day!",
            Inspiration::Trade => "I'm in a generous mood today. Come and trade!",
            Inspiration::Gift => "I've got something for someone special...",
        }
    }
    pub fn doing(self) -> &'static str {
        match self {
            Inspiration::Work => "inspired to work",
            Inspiration::Trade => "in a generous mood",
            Inspiration::Gift => "planning a present",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mind {
    pub mood: f32,
    pub rest: f32,
    pub fun: f32,
    /// How tired of each kind of fun it is (0..1).
    pub bored: [f32; 4],
    /// The game time it was brought up to (0: never).
    pub last: i64,
    pub memories: Vec<(String, i64)>,
    pub brk: Option<(Break, i64)>,
    pub insp: Option<(Inspiration, i64)>,
    /// How fine its home is as last looked at (none: it has none), and when.
    pub home: (Option<i32>, i64),
}

impl Default for Mind {
    fn default() -> Mind {
        Mind { mood: 55.0, rest: 90.0, fun: 60.0, bored: [0.0; 4], last: 0, memories: Vec::new(), brk: None, insp: None, home: (None, i64::MIN / 2) }
    }
}

/// The thoughts of now that needs give.
pub fn need_thoughts(food: u8, rest: f32, fun: f32, out: &mut Vec<&'static str>) {
    match food {
        0 => out.push("starving"),
        f if f <= crate::need::HUNGRY => out.push("hungry"),
        _ => {}
    }
    if rest < 8.0 {
        out.push("exhausted");
    } else if rest < 18.0 {
        out.push("tired");
    } else if rest < 30.0 {
        out.push("drowsy");
    }
    if fun < 12.0 {
        out.push("fun_none");
    } else if fun < 30.0 {
        out.push("fun_low");
    } else if fun > 75.0 {
        out.push("fun_high");
    }
}

/// The thought a home of `score` gives (none: no home), by the NPC's traits.
pub fn home_thought(score: Option<i32>, traits: &[Trait]) -> Option<&'static str> {
    let Some(s) = score else { return Some("homeless") };
    if traits.contains(&Trait::Modest) {
        return (s < 25).then_some("humble");
    }
    let t = match s {
        i32::MIN..=-1 => Some("home_awful"),
        0..=9 => None,
        10..=24 => Some("home_cosy"),
        25..=49 => Some("home_fine"),
        _ => Some("home_lovely"),
    };
    if traits.contains(&Trait::Greedy) && s < 25 {
        return Some("wants_more");
    }
    t
}

/// The thoughts of now its traits give.
pub fn trait_thoughts(traits: &[Trait], night: bool, awake: bool, out: &mut Vec<&'static str>) {
    if traits.contains(&Trait::Cheerful) {
        out.push("cheerful");
    }
    if traits.contains(&Trait::Gloomy) {
        out.push("gloomy");
    }
    if traits.contains(&Trait::NightOwl) && awake {
        out.push(if night { "night_good" } else { "night_day" });
    }
}

impl Mind {
    /// Each memory with how much it counts now (copies count less), and each thought of now.
    pub fn thoughts(&self, now: i64, current: &[&'static str]) -> Vec<(&'static Thought, f32)> {
        let mut out: Vec<(&'static Thought, f32)> = Vec::new();
        let mut copies: Vec<i32> = Vec::new();
        for (id, at) in &self.memories {
            let Some(t) = thought(id) else { continue };
            if now - at >= t.lasts || now < *at {
                continue;
            }
            match out.iter().position(|(o, _)| o.id == t.id) {
                Some(i) => {
                    out[i].1 += t.mood * 0.75f32.powi(copies[i]);
                    copies[i] += 1;
                }
                None => {
                    out.push((t, t.mood));
                    copies.push(1);
                }
            }
        }
        for id in current {
            if let Some(t) = thought(id)
                && !out.iter().any(|(o, _)| o.id == t.id)
            {
                out.push((t, t.mood));
            }
        }
        out.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        out
    }

    /// Where its mood is going.
    pub fn target(&self, now: i64, current: &[&'static str]) -> f32 {
        (BASE + self.thoughts(now, current).iter().map(|(_, v)| v).sum::<f32>()).clamp(0.0, 100.0)
    }

    /// Remembers `id` at `now` (the oldest copy goes past its limit).
    pub fn remember(&mut self, id: &str, now: i64) {
        let Some(t) = thought(id).filter(|t| t.lasts > 0) else { return };
        self.memories.retain(|(m, at)| thought(m).is_some_and(|d| now - at < d.lasts && now >= *at));
        let same: Vec<usize> = self.memories.iter().enumerate().filter(|(_, (m, _))| m == id).map(|(i, _)| i).collect();
        if same.len() >= t.stack {
            self.memories.remove(same[0]);
        }
        self.memories.push((id.to_string(), now));
    }

    pub fn has_memory(&self, id: &str, now: i64) -> bool {
        self.memories.iter().any(|(m, at)| m == id && thought(m).is_some_and(|t| now - at < t.lasts && now >= *at))
    }

    /// Fun of `kind` worth `amount`, less the more it had of it lately (talk counts more for the
    /// chatty, less for loners).
    pub fn enjoy(&mut self, kind: usize, amount: f32, traits: &[Trait]) -> f32 {
        let mut a = amount * (1.0 - self.bored[kind]).max(0.0);
        if kind == TALK {
            if traits.contains(&Trait::Chatty) {
                a *= 1.5;
            } else if traits.contains(&Trait::Loner) {
                a *= 0.5;
            }
        }
        self.fun = (self.fun + a).min(100.0);
        self.bored[kind] = (self.bored[kind] + a / 120.0).min(1.0);
        a
    }

    /// Brings needs and mood up to `now`: awake or asleep (in a bed or not), with the thoughts of
    /// now `current`.
    pub fn advance(&mut self, now: i64, awake: bool, in_bed: bool, traits: &[Trait], current: &[&'static str]) {
        if self.last == 0 || now < self.last {
            self.last = now;
            return;
        }
        // Days away count as one.
        let dt = (now - self.last).min(DAY) as f32;
        self.last = now;
        let hours = dt / HOUR as f32;
        if awake {
            self.rest = (self.rest - 3.5 * hours).max(0.0);
            self.fun = (self.fun - 2.5 * hours).max(0.0);
        } else {
            let gain = if in_bed { 11.0 } else { 7.0 } * if traits.contains(&Trait::SoundSleeper) { 1.5 } else { 1.0 };
            self.rest = (self.rest + gain * hours).min(100.0);
        }
        let keep = 0.85f32.powf(dt / DAY as f32);
        for b in &mut self.bored {
            *b *= keep;
        }
        if awake {
            let target = self.target(now, current);
            self.mood = if target > self.mood { (self.mood + 12.0 * hours).min(target) } else { (self.mood - 8.0 * hours).max(target) };
        }
        self.memories.retain(|(m, at)| thought(m).is_some_and(|t| now - at < t.lasts && now >= *at));
        if self.brk.is_some_and(|(_, until)| now >= until) {
            self.brk = None;
            self.remember("relieved", now);
        }
        if self.insp.is_some_and(|(_, until)| now >= until) {
            self.insp = None;
        }
    }

    /// How well it does what it is told.
    pub fn obey_factor(&self) -> f32 {
        if self.brk.is_some_and(|(b, _)| b.shuts_out()) {
            return 0.0;
        }
        if self.mood < 20.0 {
            0.6
        } else if self.mood < 35.0 {
            0.8
        } else {
            1.0
        }
    }

    pub fn breaking(&self) -> Option<Break> {
        self.brk.map(|(b, _)| b)
    }

    pub fn inspired(&self) -> Option<Inspiration> {
        self.insp.map(|(i, _)| i)
    }
}

/// How bad a break its mood may bring (0 minor, 1 major, 2 extreme), by its traits.
pub fn break_level(mood: f32, traits: &[Trait]) -> Option<u8> {
    let shift = if traits.contains(&Trait::Steady) {
        -10.0
    } else if traits.contains(&Trait::Nervous) {
        8.0
    } else {
        0.0
    };
    let minor = 35.0 + shift;
    if mood < minor / 7.0 {
        Some(2)
    } else if mood < minor * 4.0 / 7.0 {
        Some(1)
    } else if mood < minor {
        Some(0)
    } else {
        None
    }
}

/// The chance it breaks within `dt` ticks at `level` (on average after 2 days, half a day, a
/// quarter day).
pub fn break_chance(level: u8, dt: i64) -> f32 {
    let mean = match level {
        0 => 2 * DAY,
        1 => DAY / 2,
        _ => DAY / 4,
    };
    (dt as f32 / mean as f32).min(1.0)
}

/// Which break, at `level`, by its traits and a roll.
pub fn pick_break(level: u8, traits: &[Trait], roll: u64) -> Break {
    let mut pool: Vec<Break> = match level {
        0 => vec![Break::Sulk, Break::Hide, Break::Binge, Break::Snap],
        1 => vec![Break::Tantrum, Break::Daze, Break::Binge],
        _ => vec![Break::Leave, Break::Freeze],
    };
    if traits.contains(&Trait::Kind) {
        pool.retain(|b| *b != Break::Snap);
    }
    if traits.contains(&Trait::Rude) && level == 0 {
        pool.extend([Break::Snap, Break::Snap]);
    }
    if traits.contains(&Trait::Glutton) && level < 2 {
        pool.extend([Break::Binge, Break::Binge]);
    }
    pool[(roll % pool.len() as u64) as usize]
}

/// The chance it is inspired within `dt` ticks: none under 50, on average after 30 days at 50
/// down to 4 days at 100.
pub fn inspire_chance(mood: f32, dt: i64) -> f32 {
    if mood < 50.0 {
        return 0.0;
    }
    let days = 30.0 - (mood - 50.0) / 50.0 * 26.0;
    (dt as f32 / (days * DAY as f32)).min(1.0)
}

pub fn pick_inspiration(roll: u64) -> Inspiration {
    [Inspiration::Work, Inspiration::Trade, Inspiration::Gift][(roll % 3) as usize]
}

/// Its mood in a word.
pub fn mood_word(mood: f32) -> &'static str {
    match mood as i32 {
        i32::MIN..=9 => "miserable",
        10..=24 => "very unhappy",
        25..=39 => "unhappy",
        40..=54 => "all right",
        55..=69 => "content",
        70..=84 => "happy",
        _ => "very happy",
    }
}

/// What the blocks about a home make of it: a score (see `home_thought`).
pub fn block_worth(id: &str) -> f32 {
    let id = id.strip_prefix("minecraft:").unwrap_or(id);
    if id.starts_with("potted_") || id.ends_with("_banner") || id == "bookshelf" || id == "chiseled_bookshelf" || id == "flower_pot" {
        3.0
    } else if id.ends_with("_carpet") || id.contains("lantern") || id.ends_with("torch") || id.ends_with("candle") || id == "lectern" || id == "jukebox" {
        1.5
    } else if matches!(id, "poppy" | "dandelion" | "cornflower" | "allium" | "azure_bluet" | "oxeye_daisy" | "lily_of_the_valley" | "blue_orchid")
        || id.ends_with("_tulip")
        || id == "brewing_stand"
        || id == "decorated_pot"
    {
        1.5
    } else if id.ends_with("glass") || id.ends_with("glass_pane") || id.ends_with("_bed") || id.ends_with("_stairs") || id.ends_with("chest") || id == "barrel" || id == "crafting_table" || id.ends_with("furnace") || id == "smoker" {
        0.5
    } else if matches!(id, "cobweb") {
        -3.0
    } else if matches!(id, "dirt" | "coarse_dirt" | "gravel" | "mud") {
        -0.3
    } else {
        0.0
    }
}

impl Mind {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("mood".into(), json!(self.mood));
        m.insert("rest".into(), json!(self.rest));
        m.insert("fun".into(), json!(self.fun));
        m.insert("bored".into(), json!(self.bored.to_vec()));
        m.insert("last".into(), json!(self.last));
        m.insert("memories".into(), Value::Array(self.memories.iter().map(|(id, at)| json!([id, at])).collect()));
        if let Some((b, until)) = self.brk {
            m.insert("break".into(), json!([b.name(), until]));
        }
        if let Some((i, until)) = self.insp {
            m.insert("inspired".into(), json!([i.name(), until]));
        }
        if let Some(s) = self.home.0 {
            m.insert("home".into(), json!(s));
        }
        m.insert("home_at".into(), json!(self.home.1));
        Value::Object(m)
    }

    pub fn from_json(v: &Value) -> Mind {
        let d = Mind::default();
        let f = |k: &str, or: f32| v.get(k).and_then(Value::as_f64).map(|x| x as f32).unwrap_or(or);
        let pair = |k: &str| -> Option<(String, i64)> {
            let a = v.get(k)?.as_array()?;
            Some((a.first()?.as_str()?.to_string(), a.get(1)?.as_i64()?))
        };
        let mut bored = [0.0; 4];
        if let Some(a) = v.get("bored").and_then(Value::as_array) {
            for (b, x) in bored.iter_mut().zip(a) {
                *b = x.as_f64().unwrap_or(0.0) as f32;
            }
        }
        Mind {
            mood: f("mood", d.mood).clamp(0.0, 100.0),
            rest: f("rest", d.rest).clamp(0.0, 100.0),
            fun: f("fun", d.fun).clamp(0.0, 100.0),
            bored,
            last: v.get("last").and_then(Value::as_i64).unwrap_or(0),
            memories: v
                .get("memories")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| Some((x.get(0)?.as_str()?.to_string(), x.get(1)?.as_i64()?))).take(64).collect())
                .unwrap_or_default(),
            brk: pair("break").and_then(|(n, t)| Some((Break::parse(&n)?, t))),
            insp: pair("inspired").and_then(|(n, t)| Some((Inspiration::parse(&n)?, t))),
            home: (v.get("home").and_then(Value::as_i64).map(|x| x as i32), v.get("home_at").and_then(Value::as_i64).unwrap_or(d.home.1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traits_fit_together() {
        for u in 0..400u128 {
            let t = traits(u.wrapping_mul(0x1234_5678_9abc_def1));
            assert!((1..=3).contains(&t.len()), "{t:?}");
            for (a, _, against) in TRAITS {
                if t.contains(a) {
                    assert!(against.is_none_or(|x| !t.contains(&x)), "{t:?}");
                }
            }
        }
        assert_eq!(Trait::parse("night_owl"), Some(Trait::NightOwl));
        assert_eq!(Trait::parse("hard-working"), Some(Trait::HardWorking));
        assert_eq!(Trait::parse("glutton"), None);
        assert_eq!(Trait::parse("big_eater"), Some(Trait::Glutton));
        assert_eq!(words(&[Trait::Kind, Trait::Lazy, Trait::NightOwl]), "kind, lazy and a night owl");
    }

    #[test]
    fn memories_stack_less_and_fade() {
        let mut m = Mind::default();
        m.remember("gift", 100);
        let one = m.target(100, &[]);
        m.remember("gift", 200);
        let two = m.target(200, &[]);
        assert!((one - BASE - 6.0).abs() < 1e-3 && two > one && two < one + 6.0, "{one} {two}");
        for i in 0..5 {
            m.remember("gift", 300 + i);
        }
        assert_eq!(m.memories.iter().filter(|(id, _)| id == "gift").count(), 3);
        assert!((m.target(400 + DAY, &[]) - BASE).abs() < 1e-3);
        assert!((m.target(100, &["tired"]) - (BASE + 6.0 - 8.0)).abs() < 1e-3 || m.target(100, &["tired"]) < BASE);
    }

    #[test]
    fn needs_run_down_and_mood_follows() {
        let mut m = Mind { last: 1, ..Mind::default() };
        m.advance(1 + 10 * HOUR, true, false, &[], &[]);
        assert!(m.rest < 60.0 && m.fun < 40.0, "{m:?}");
        // Asleep in a bed it rests; the mood stands still.
        let mood = m.mood;
        m.advance(1 + 18 * HOUR, false, true, &[], &[]);
        assert!(m.rest > 90.0 && m.mood == mood, "{m:?}");
        // A day of hunger and boredom brings it low.
        let mut low = vec![];
        need_thoughts(0, 5.0, 5.0, &mut low);
        m.advance(1 + 42 * HOUR, true, false, &[], &low);
        assert!(m.mood < 20.0, "{}", m.mood);
        assert!(break_level(m.mood, &[]).is_some());
    }

    #[test]
    fn breaks_end_in_relief() {
        let mut m = Mind { last: 1, mood: 10.0, ..Mind::default() };
        m.brk = Some((Break::Sulk, 1 + Break::Sulk.lasts()));
        assert_eq!(m.obey_factor(), 0.0);
        m.advance(2 + Break::Sulk.lasts(), true, false, &[], &[]);
        assert!(m.brk.is_none() && m.has_memory("relieved", 2 + Break::Sulk.lasts()));
        assert_eq!(break_level(30.0, &[]), Some(0));
        assert_eq!(break_level(30.0, &[Trait::Steady]), None);
        assert_eq!(break_level(3.0, &[]), Some(2));
        assert_ne!(pick_break(0, &[Trait::Kind], 3), Break::Snap);
        assert_eq!(inspire_chance(40.0, DAY), 0.0);
        assert!(inspire_chance(100.0, DAY) > inspire_chance(60.0, DAY));
    }

    #[test]
    fn fun_wears_thin() {
        let mut m = Mind { fun: 0.0, ..Mind::default() };
        let first = m.enjoy(GAMES, 30.0, &[]);
        let mut last = first;
        for _ in 0..4 {
            last = m.enjoy(GAMES, 30.0, &[]);
        }
        assert!(last < first / 2.0, "{first} {last}");
        assert!(m.enjoy(TALK, 10.0, &[Trait::Chatty]) > 10.0);
    }

    #[test]
    fn homes_and_json() {
        assert_eq!(home_thought(None, &[]), Some("homeless"));
        assert_eq!(home_thought(Some(30), &[]), Some("home_fine"));
        assert_eq!(home_thought(Some(5), &[Trait::Greedy]), Some("wants_more"));
        assert_eq!(home_thought(Some(5), &[Trait::Modest]), Some("humble"));
        assert!(block_worth("minecraft:potted_poppy") > block_worth("minecraft:oak_planks"));
        let mut m = Mind { last: 5, ..Mind::default() };
        m.remember("gift", 5);
        m.brk = Some((Break::Daze, 99));
        m.insp = Some((Inspiration::Trade, 77));
        m.home = (Some(12), 4);
        assert_eq!(Mind::from_json(&m.to_json()), m);
    }
}
