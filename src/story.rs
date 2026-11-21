//! The storyteller: what comes to a village by itself, day by day. No mod API types here.
//!
//! Each village has its own tale. Days go in cycles of ten: five when trouble may come, five
//! quieter ones. On any day something may happen: a wanderer asks to stay (likelier the smaller
//! the village), a trade caravan comes by, a few stray animals wander in; and on the restless days
//! raiders come (more of them the richer and older the village), a blight withers the crops, a
//! sickness goes round, or the stores spoil. Where there are other villages, neighbours may fall
//! out over something, and friends send food to a village that is short of it (see `faction`).
//! The first days are spared, trouble keeps a few days
//! apart, each kind of thing its own longer while, and a village in grief or low spirits is let off
//! more often.

pub const DAY: i64 = 24_000;
/// Days in a cycle, and how many of them trouble may come.
pub const CYCLE: i64 = 10;
pub const ON_DAYS: i64 = 5;
/// Days from when the tale starts before any trouble.
pub const GRACE: i64 = 3;
/// Days between two bad things at least.
pub const BAD_GAP: i64 = 2;
/// A village this big takes in no wanderers.
pub const FULL: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Incident {
    Wanderer,
    Caravan,
    Strays,
    Raid,
    Blight,
    Outbreak,
    Spoilage,
    /// It falls out with another village.
    Quarrel,
    /// A friendly village sends it food.
    Aid,
}

pub const ALL: [Incident; 9] = [
    Incident::Wanderer,
    Incident::Caravan,
    Incident::Strays,
    Incident::Raid,
    Incident::Blight,
    Incident::Outbreak,
    Incident::Spoilage,
    Incident::Quarrel,
    Incident::Aid,
];

impl Incident {
    pub fn id(self) -> &'static str {
        match self {
            Incident::Wanderer => "wanderer",
            Incident::Caravan => "caravan",
            Incident::Strays => "strays",
            Incident::Raid => "raid",
            Incident::Blight => "blight",
            Incident::Outbreak => "outbreak",
            Incident::Spoilage => "spoilage",
            Incident::Quarrel => "quarrel",
            Incident::Aid => "aid",
        }
    }

    pub fn parse(s: &str) -> Option<Incident> {
        ALL.into_iter().find(|i| i.id() == s)
    }

    pub fn bad(self) -> bool {
        matches!(self, Incident::Raid | Incident::Blight | Incident::Outbreak | Incident::Spoilage)
    }

    /// Days at least between two of this kind.
    pub fn gap(self) -> i64 {
        match self {
            Incident::Wanderer => 3,
            Incident::Caravan => 3,
            Incident::Strays => 5,
            Incident::Raid => 6,
            Incident::Blight => 8,
            Incident::Outbreak => 10,
            Incident::Spoilage => 6,
            Incident::Quarrel => 8,
            Incident::Aid => 3,
        }
    }

    /// How likely it is beside the others, for a village like `w`.
    pub fn weight(self, w: &World) -> f32 {
        match self {
            Incident::Wanderer => wanderer_weight(w.people),
            Incident::Caravan => 1.0,
            Incident::Strays => 0.5,
            // Richer villages draw raiders.
            Incident::Raid => 0.8 + (wealth(w) as f32 / 400.0).min(1.2),
            Incident::Blight if w.farmers == 0 => 0.0,
            Incident::Blight => 0.5,
            Incident::Outbreak if w.people < 3 => 0.0,
            Incident::Outbreak => 0.4,
            Incident::Spoilage if w.food < 16 => 0.0,
            Incident::Spoilage => 0.5,
            Incident::Quarrel if w.neighbours == 0 => 0.0,
            Incident::Quarrel => 0.3,
            // Help comes when it is needed, if there is a friend to send it.
            Incident::Aid if w.aid => 4.0,
            Incident::Aid => 0.0,
        }
    }
}

/// How the village stands today.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct World {
    pub day: i64,
    pub people: usize,
    pub food: u64,
    pub logs: u64,
    pub mood: f32,
    pub farmers: usize,
    /// Days since someone of the village died (large: none lately).
    pub since_death: i64,
    /// The other villages it knows of.
    pub neighbours: usize,
    /// Whether it is short of food and a friendly village could send some.
    pub aid: bool,
}

/// What it is worth to raiders.
pub fn wealth(w: &World) -> u64 {
    w.food + w.logs / 2 + 20 * w.people as u64
}

/// A small village takes in wanderers gladly, a big one hardly, a full one not at all.
pub fn wanderer_weight(people: usize) -> f32 {
    match people {
        0..=2 => 3.0,
        3..=5 => 1.5,
        6..=9 => 0.7,
        10..FULL => 0.2,
        _ => 0.0,
    }
}

/// A village's tale so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Teller {
    /// The day it started.
    pub since: i64,
    /// Where in its cycle it starts (each village its own).
    pub phase: i64,
    /// The last day of each kind.
    pub last: Vec<(Incident, i64)>,
}

impl Teller {
    pub fn new(day: i64, seed: u64) -> Teller {
        Teller { since: day, phase: (seed % CYCLE as u64) as i64, last: Vec::new() }
    }

    pub fn last_of(&self, i: Incident) -> Option<i64> {
        self.last.iter().find(|l| l.0 == i).map(|l| l.1)
    }

    pub fn last_bad(&self) -> Option<i64> {
        self.last.iter().filter(|l| l.0.bad()).map(|l| l.1).max()
    }

    pub fn happened(&mut self, i: Incident, day: i64) {
        self.last.retain(|l| l.0 != i);
        self.last.push((i, day));
    }

    /// Whether trouble may come on `day`.
    pub fn restless(&self, day: i64) -> bool {
        (day + self.phase).rem_euclid(CYCLE) < ON_DAYS
    }

    /// Whether `i` may come on `day` at all.
    pub fn may(&self, i: Incident, day: i64) -> bool {
        if self.last_of(i).is_some_and(|d| day - d < i.gap()) {
            return false;
        }
        if i.bad() {
            return self.restless(day) && day - self.since >= GRACE && self.last_bad().is_none_or(|d| day - d >= BAD_GAP);
        }
        true
    }
}

/// The chance something happens on a day: more on restless days.
pub fn chance(restless: bool) -> f32 {
    if restless { 0.55 } else { 0.3 }
}

/// What happens to the village `w` today, if anything; `roll` is a fresh random number.
pub fn pick(t: &Teller, w: &World, roll: u64) -> Option<Incident> {
    let r = (roll % 10_000) as f32 / 10_000.0;
    if r >= chance(t.restless(w.day)) {
        return None;
    }
    // A village in grief or low spirits is let off more often.
    let spare = if w.since_death < 3 || w.mood < 30.0 { 0.3 } else { 1.0 };
    let list: Vec<(Incident, f32)> = ALL
        .into_iter()
        .filter(|i| t.may(*i, w.day))
        .map(|i| (i, i.weight(w) * if i.bad() { spare } else { 1.0 }))
        .filter(|(_, x)| *x > 0.0)
        .collect();
    let total: f32 = list.iter().map(|(_, x)| x).sum();
    if total <= 0.0 {
        return None;
    }
    let mut at = ((roll >> 20) % 10_000) as f32 / 10_000.0 * total;
    for (i, x) in &list {
        if at < *x {
            return Some(*i);
        }
        at -= x;
    }
    list.last().map(|(i, _)| *i)
}

/// How many raiders come, and how many of them shoot: more the richer and older the village.
pub fn raiders(w: &World, days_in: i64) -> (u32, u32) {
    let n = (1 + w.people as u64 / 4 + wealth(w) / 150 + days_in.max(0) as u64 / 10).clamp(1, 8) as u32;
    (n, n / 3)
}

/// How long a village has to drive raiders off before they plunder it and leave (game time).
pub const RAID_LASTS: i64 = DAY / 8;
/// How long a caravan stays.
pub const CARAVAN_STAYS: i64 = DAY;

/// The caravans there are (their shops), and which one comes.
pub const CARAVANS: [(&str, &str); 4] = [
    ("projectfelix:caravan_goods", "a trade caravan"),
    ("projectfelix:caravan_herbs", "a herb seller's cart"),
    ("projectfelix:caravan_smith", "a travelling smith"),
    ("projectfelix:caravan_arms", "an arms dealer's cart"),
];

pub fn caravan(roll: u64) -> (&'static str, &'static str) {
    CARAVANS[(roll % CARAVANS.len() as u64) as usize]
}

/// What the village swaps with a caravan, if anything: (what it gives, how many, what it gets,
/// how many). Short of food and with wood to spare, wood for bread; short of wood and with wheat
/// to spare, wheat for logs.
pub fn swap(people: usize, food: u64, logs: u64, wheat: u64) -> Option<(&'static str, u32, &'static str, u32)> {
    let need = 4 * people as u64;
    if food < need && logs >= 24 {
        return Some(("logs", 16, "minecraft:bread", 12));
    }
    if logs < 16 && wheat >= 32 && food >= need {
        return Some(("minecraft:wheat", 32, "minecraft:oak_log", 16));
    }
    None
}

/// The animals that may wander in, and how many.
pub fn strays(roll: u64) -> (&'static str, u32) {
    let kinds = ["minecraft:sheep", "minecraft:cow", "minecraft:chicken", "minecraft:pig"];
    (kinds[(roll % 4) as usize], 2 + ((roll >> 8) % 2) as u32)
}

/// How many fall sick in an outbreak.
pub fn sick_count(people: usize, roll: u64) -> usize {
    (1 + people / 4 + (roll % 2) as usize).min(people)
}

/// Of each food, the share that spoils.
pub const SPOIL: f32 = 0.35;

/// The news, for players nearby.
pub fn news(i: Incident, village: &str) -> String {
    match i {
        Incident::Wanderer => format!("A wanderer has come to {village}, asking to stay."),
        Incident::Caravan => format!("A caravan has come to {village}."),
        Incident::Strays => format!("A few stray animals have wandered into {village}."),
        Incident::Raid => format!("Raiders are coming for {village}!"),
        Incident::Blight => format!("A blight has struck the fields of {village}."),
        Incident::Outbreak => format!("A sickness is going round {village}."),
        Incident::Spoilage => format!("Rot has got into the stores of {village}."),
        Incident::Quarrel => format!("{village} has fallen out with its neighbours."),
        Incident::Aid => format!("Friends have sent food to {village}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(day: i64) -> World {
        World { day, people: 6, food: 60, logs: 40, mood: 55.0, farmers: 2, since_death: 99, neighbours: 0, aid: false }
    }

    #[test]
    fn spared_at_first_and_quiet_days() {
        let t = Teller::new(0, 0);
        // Day 1 is restless (phase 0) but in the grace days.
        assert!(!t.may(Incident::Raid, 1));
        assert!(t.may(Incident::Raid, 3));
        // Days 5..9 are quiet: no trouble, good things still.
        assert!(!t.may(Incident::Raid, 6));
        assert!(t.may(Incident::Caravan, 6));
        for roll in 0..2_000u64 {
            let r = roll.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            if let Some(i) = pick(&t, &world(7), r) {
                assert!(!i.bad(), "{i:?} on a quiet day");
            }
        }
    }

    #[test]
    fn kinds_keep_apart() {
        let mut t = Teller::new(0, 0);
        t.happened(Incident::Raid, 3);
        assert!(!t.may(Incident::Raid, 4));
        // Another bad thing waits two days too.
        assert!(!t.may(Incident::Blight, 4));
        assert!(t.may(Incident::Blight, 10));
        assert!(!t.may(Incident::Raid, 8));
        assert!(t.may(Incident::Raid, 10));
        t.happened(Incident::Caravan, 10);
        assert_eq!(t.last_of(Incident::Caravan), Some(10));
        t.happened(Incident::Caravan, 14);
        assert_eq!(t.last.len(), 2);
    }

    #[test]
    fn small_villages_draw_wanderers() {
        assert!(wanderer_weight(2) > wanderer_weight(8));
        assert_eq!(wanderer_weight(FULL), 0.0);
        let t = Teller::new(0, 0);
        let count = |people: usize| {
            let w = World { people, ..world(7) };
            (0..4_000u64).filter(|r| pick(&t, &w, r.wrapping_mul(0x2545_F491_4F6C_DD1D)) == Some(Incident::Wanderer)).count()
        };
        assert!(count(2) > 2 * count(10), "{} {}", count(2), count(10));
    }

    #[test]
    fn grief_spares_the_village() {
        let t = Teller::new(0, 0);
        let bad = |w: &World| (0..4_000u64).filter(|r| pick(&t, w, r.wrapping_mul(0x2545_F491_4F6C_DD1D)).is_some_and(Incident::bad)).count();
        let fine = world(4);
        let grieving = World { since_death: 1, ..world(4) };
        assert!(bad(&grieving) * 2 < bad(&fine), "{} {}", bad(&grieving), bad(&fine));
    }

    #[test]
    fn some_days_bring_nothing() {
        let t = Teller::new(0, 0);
        let none = (0..4_000u64).filter(|r| pick(&t, &world(7), r.wrapping_mul(0x9E37_79B9_7F4A_7C15)).is_none()).count();
        assert!((2_400..3_200).contains(&none), "{none}");
    }

    #[test]
    fn raids_grow() {
        let small = World { people: 2, food: 10, logs: 0, ..world(5) };
        let big = World { people: 12, food: 400, logs: 300, ..world(5) };
        assert_eq!(raiders(&small, 3), (1, 0));
        let (n, ranged) = raiders(&big, 40);
        assert!(n >= 6 && ranged >= 2, "{n} {ranged}");
        assert!(raiders(&big, 400).0 <= 8);
    }

    #[test]
    fn swaps_what_it_lacks() {
        assert_eq!(swap(6, 10, 40, 0), Some(("logs", 16, "minecraft:bread", 12)));
        assert_eq!(swap(6, 60, 4, 64), Some(("minecraft:wheat", 32, "minecraft:oak_log", 16)));
        assert_eq!(swap(6, 60, 40, 64), None);
        assert_eq!(swap(6, 10, 4, 0), None);
    }

    #[test]
    fn neighbours_matter() {
        let t = Teller::new(0, 0);
        let count = |w: &World, i: Incident| (0..4_000u64).filter(|r| pick(&t, w, r.wrapping_mul(0x2545_F491_4F6C_DD1D)) == Some(i)).count();
        // Alone, no quarrels and no help.
        assert_eq!(count(&world(7), Incident::Quarrel), 0);
        assert_eq!(count(&world(7), Incident::Aid), 0);
        let near = World { neighbours: 2, ..world(7) };
        assert!(count(&near, Incident::Quarrel) > 0);
        assert_eq!(count(&near, Incident::Aid), 0);
        // Short of food with a friend who can help, help is what mostly comes.
        let hungry = World { neighbours: 2, aid: true, food: 2, ..world(7) };
        assert!(count(&hungry, Incident::Aid) > count(&hungry, Incident::Caravan) * 2);
        assert!(!Incident::Quarrel.bad() && !Incident::Aid.bad());
    }

    #[test]
    fn names_round_trip() {
        for i in ALL {
            assert_eq!(Incident::parse(i.id()), Some(i));
        }
        assert!(news(Incident::Raid, "Oakvale").contains("Oakvale"));
        assert!(sick_count(1, 7) <= 1);
        assert_eq!(strays(5).0, "minecraft:cow");
    }
}
