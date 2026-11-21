//! What an NPC is good at: skills from 0 to 20, which of them it loves doing (a passion: it learns
//! them faster and is glad doing them), and the life it had before (a childhood and a trade, which
//! gave it its first skills). No mod API types here.
//!
//! A skill grows with use: experience is counted, a level needs 1000 more than the one before (level
//! 10: 55 000 in all). Without a passion it learns at a third of the pace, with a burning one half
//! again as fast. Past level 10 a skill fades a little each day it isn't used. A skill makes the work
//! quicker (level 0: 0.6 times, 10: once, 20: 1.4 times) and brings more of it (an extra piece now
//! and then, up to every other time at 20).

use crate::serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skill {
    Farming,
    Woodcutting,
    Fishing,
    Cooking,
    Building,
    Trading,
    Talking,
    Healing,
    Fighting,
}

/// How many skills there are.
pub const N: usize = 9;

pub const SKILLS: [Skill; N] = [Skill::Farming, Skill::Woodcutting, Skill::Fishing, Skill::Cooking, Skill::Building, Skill::Trading, Skill::Talking, Skill::Healing, Skill::Fighting];

impl Skill {
    pub fn index(self) -> usize {
        SKILLS.iter().position(|s| *s == self).unwrap_or(0)
    }
    pub fn name(self) -> &'static str {
        match self {
            Skill::Farming => "farming",
            Skill::Woodcutting => "woodcutting",
            Skill::Fishing => "fishing",
            Skill::Cooking => "cooking",
            Skill::Building => "building",
            Skill::Trading => "trading",
            Skill::Talking => "talking",
            Skill::Healing => "healing",
            Skill::Fighting => "fighting",
        }
    }
    pub fn parse(s: &str) -> Option<Skill> {
        SKILLS.into_iter().find(|k| k.name() == s)
    }
    /// The skill a job uses.
    pub fn of_job(job: &str) -> Option<Skill> {
        match job {
            "farm" => Some(Skill::Farming),
            "wood" => Some(Skill::Woodcutting),
            "fish" => Some(Skill::Fishing),
            "bake" => Some(Skill::Cooking),
            "build" => Some(Skill::Building),
            _ => None,
        }
    }
}

/// Experience all the levels up to `level` need.
pub fn xp_for(level: u8) -> f32 {
    let l = level.min(20) as f32;
    500.0 * l * (l + 1.0)
}

pub fn level_of(xp: f32) -> u8 {
    (0..=20u8).rev().find(|l| xp >= xp_for(*l)).unwrap_or(0)
}

/// How a skill's level reads.
pub fn level_word(level: u8) -> &'static str {
    match level {
        0..=3 => "a beginner",
        4..=7 => "fair",
        8..=11 => "good",
        12..=15 => "very good",
        16..=19 => "a master",
        _ => "legendary",
    }
}

/// How much faster it works at `level`.
pub fn speed(level: u8) -> f32 {
    0.6 + 0.04 * level.min(20) as f32
}

/// The chance a piece of work brings one more.
pub fn bonus(level: u8) -> f32 {
    level.min(20) as f32 / 40.0
}

/// How fast it learns with a passion of 0 (none), 1 or 2 (burning).
pub fn learn_factor(passion: u8) -> f32 {
    match passion {
        0 => 0.35,
        1 => 1.0,
        _ => 1.5,
    }
}

/// What a day without use takes from a skill past level 10.
pub fn fade(level: u8) -> f32 {
    if level > 10 { 40.0 * (level - 10) as f32 } else { 0.0 }
}

/// A life before: what it says of it, and the skills it gave.
pub struct Past {
    pub id: &'static str,
    pub says: &'static str,
    pub skills: &'static [(Skill, u8)],
    /// The job it fits (adulthood), if any.
    pub job: Option<&'static str>,
}

const fn past(id: &'static str, says: &'static str, skills: &'static [(Skill, u8)], job: Option<&'static str>) -> Past {
    Past { id, says, skills, job }
}

pub const CHILDHOODS: &[Past] = &[
    past("farm_child", "I grew up on a farm, up before the sun", &[(Skill::Farming, 3), (Skill::Cooking, 1)], None),
    past("fisher_kid", "I grew up by the water, mending my father's nets", &[(Skill::Fishing, 4)], None),
    past("market_runner", "I ran errands at the market as a child", &[(Skill::Trading, 3), (Skill::Talking, 2)], None),
    past("forest_child", "I grew up in a hut deep in the woods", &[(Skill::Woodcutting, 3), (Skill::Building, 1)], None),
    past("kitchen_help", "I was a kitchen helper at an inn", &[(Skill::Cooking, 3), (Skill::Talking, 1)], None),
    past("orphan", "I grew up an orphan, on the streets", &[(Skill::Talking, 2), (Skill::Trading, 2)], None),
    past("bookworm", "I spent my childhood with my nose in books", &[(Skill::Talking, 3)], None),
    past("builders_kid", "My mother built houses; I carried her tools", &[(Skill::Building, 3), (Skill::Woodcutting, 1)], None),
    // New ones go last: an NPC's past is picked by its place in the list.
    past("herb_child", "I gathered herbs with my grandmother, who healed the village", &[(Skill::Healing, 4), (Skill::Farming, 1)], None),
    past("scrapper", "I was a scrappy child, in a fight every other day", &[(Skill::Fighting, 3)], None),
];

pub const ADULTHOODS: &[Past] = &[
    past("farmer", "then I worked the fields for years", &[(Skill::Farming, 6)], Some("farm")),
    past("woodsman", "then I felled trees for a living", &[(Skill::Woodcutting, 6), (Skill::Building, 1)], Some("wood")),
    past("sailor", "then I sailed with a fishing crew", &[(Skill::Fishing, 6), (Skill::Talking, 1)], Some("fish")),
    past("baker", "then I kept a bakery", &[(Skill::Cooking, 6), (Skill::Trading, 1)], Some("bake")),
    past("carpenter", "then I was a carpenter", &[(Skill::Building, 6), (Skill::Woodcutting, 1)], Some("build")),
    past("peddler", "then I peddled wares from town to town", &[(Skill::Trading, 6), (Skill::Talking, 2)], None),
    past("guard", "then I stood guard at a city gate", &[(Skill::Fighting, 6), (Skill::Talking, 1)], None),
    past("storyteller", "then I told stories for a living", &[(Skill::Talking, 6)], None),
    past("drifter", "then I drifted, never staying long", &[(Skill::Fishing, 2), (Skill::Woodcutting, 2), (Skill::Farming, 1)], None),
    past("healer", "then I tended the sick and the hurt", &[(Skill::Healing, 6), (Skill::Talking, 1)], None),
    past("soldier", "then I soldiered in some lord's wars", &[(Skill::Fighting, 7)], None),
    past("hunter", "then I hunted in the hills", &[(Skill::Fighting, 4), (Skill::Cooking, 1)], None),
];

fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

/// An NPC's life before, by its UUID: its childhood and its trade, mostly the one of its `job`.
pub fn past_of(uuid: u128, job: Option<&str>) -> (&'static Past, Option<&'static Past>) {
    past_in(uuid, job, CHILDHOODS.len(), ADULTHOODS.len())
}

/// How many pasts there were before healing came: NPCs made then keep the past they had.
pub const LEGACY: (usize, usize) = (8, 9);

/// An NPC's past from the first `nc` childhoods and `na` trades.
pub fn past_in(uuid: u128, job: Option<&str>, nc: usize, na: usize) -> (&'static Past, Option<&'static Past>) {
    let (cs, ads) = (&CHILDHOODS[..nc.min(CHILDHOODS.len())], &ADULTHOODS[..na.min(ADULTHOODS.len())]);
    let h = mix((uuid as u64) ^ ((uuid >> 64) as u64) ^ 0xBAC5_7041);
    let child = &cs[(h % cs.len() as u64) as usize];
    // Children have no trade yet.
    if job == Some("child") {
        return (child, None);
    }
    let fits: Vec<&Past> = ads.iter().filter(|p| job.is_some() && p.job == job).collect();
    let h2 = mix(h);
    let adult = if !fits.is_empty() && h2 % 10 < 7 { fits[(h2 >> 8) as usize % fits.len()] } else { &ads[(h2 >> 16) as usize % ads.len()] };
    (child, Some(adult))
}

/// The past named `child` and `adult` (empty: none yet).
pub fn past_named(child: &str, adult: &str) -> Option<(&'static Past, Option<&'static Past>)> {
    let c = CHILDHOODS.iter().find(|p| p.id == child)?;
    Some((c, ADULTHOODS.iter().find(|p| p.id == adult)))
}

/// Its skills to begin with (levels) and passions (0, 1, 2), by its UUID and past.
pub fn start(uuid: u128, job: Option<&str>) -> ([u8; N], [u8; N]) {
    let (child, adult) = past_of(uuid, job);
    let mut h = mix((uuid as u64).rotate_left(17) ^ ((uuid >> 64) as u64) ^ 0x5C111);
    let mut levels = [0u8; N];
    let mut passions = [0u8; N];
    for i in 0..N {
        h = mix(h);
        levels[i] = (h % 4) as u8;
        passions[i] = match (h >> 8) % 100 {
            0..=69 => 0,
            70..=91 => 1,
            _ => 2,
        };
    }
    for p in std::iter::once(child).chain(adult) {
        for (s, n) in p.skills {
            let i = s.index();
            levels[i] = (levels[i] + n).min(20);
            // What it did for years it mostly likes.
            if *n >= 3 && passions[i] == 0 && (h >> (i * 3)) % 2 == 0 {
                passions[i] = 1;
            }
        }
    }
    (levels, passions)
}

/// An NPC's skills: experience of each, and passions.
#[derive(Clone, Debug, PartialEq)]
pub struct Skills {
    pub xp: [f32; N],
    pub passion: [u8; N],
    /// When each was last used (game time).
    pub used: [i64; N],
    /// The day fading was last counted.
    pub faded: i64,
    /// Its past: childhood and trade ids (none: not kept yet, an NPC made before it was).
    pub past: Option<(String, String)>,
}

impl Skills {
    pub fn new(uuid: u128, job: Option<&str>) -> Skills {
        let (levels, passion) = start(uuid, job);
        let (c, a) = past_of(uuid, job);
        Skills { xp: levels.map(xp_for), passion, used: [0; N], faded: 0, past: Some((c.id.to_string(), a.map_or(String::new(), |a| a.id.to_string()))) }
    }

    pub fn level(&self, s: Skill) -> u8 {
        level_of(self.xp[s.index()])
    }

    /// It practised `s`, worth `amount` experience before its passion; the level it reached, if
    /// it went up.
    pub fn practice(&mut self, s: Skill, amount: f32, now: i64) -> Option<u8> {
        let i = s.index();
        let before = level_of(self.xp[i]);
        self.xp[i] = (self.xp[i] + amount * learn_factor(self.passion[i])).min(xp_for(20));
        self.used[i] = now;
        let after = level_of(self.xp[i]);
        (after > before).then_some(after)
    }

    /// A day's fading of the skills past 10 it didn't use that day.
    pub fn fade_day(&mut self, now: i64, day: i64) {
        if now - self.faded < day && now >= self.faded {
            return;
        }
        self.faded = now;
        for i in 0..N {
            if now - self.used[i] >= day || now < self.used[i] {
                let l = level_of(self.xp[i]);
                self.xp[i] = (self.xp[i] - fade(l)).max(xp_for(l.saturating_sub(1)));
            }
        }
    }

    /// Its best skills, best first: (skill, level, passion).
    pub fn best(&self, n: usize) -> Vec<(Skill, u8, u8)> {
        let mut v: Vec<(Skill, u8, u8)> = SKILLS.iter().map(|s| (*s, self.level(*s), self.passion[s.index()])).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
        v.truncate(n);
        v
    }

    pub fn to_json(&self) -> Value {
        json!({ "xp": self.xp.to_vec(), "passion": self.passion.to_vec(), "used": self.used.to_vec(), "faded": self.faded, "past": self.past.as_ref().map(|(c, a)| json!([c, a])) })
    }

    pub fn from_json(v: &Value) -> Option<Skills> {
        let arr = |k: &str| v.get(k).and_then(Value::as_array).filter(|a| a.len() >= 7);
        let xp = arr("xp")?;
        let mut s = Skills { xp: [0.0; N], passion: [0; N], used: [0; N], faded: v.get("faded").and_then(Value::as_i64).unwrap_or(0), past: None };
        s.past = v.get("past").and_then(Value::as_array).and_then(|p| Some((p.first()?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string())));
        // Skills added since (healing) start at nothing.
        for i in 0..N.min(xp.len()) {
            s.xp[i] = (xp[i].as_f64().unwrap_or(0.0) as f32).clamp(0.0, xp_for(20));
            s.passion[i] = arr("passion").and_then(|a| a.get(i)?.as_u64()).unwrap_or(0).min(2) as u8;
            s.used[i] = arr("used").and_then(|a| a.get(i)?.as_i64()).unwrap_or(0);
        }
        // Fighting, to one made before it was a skill: what a life gives anyone, more to one who stood guard.
        if xp.len() <= Skill::Fighting.index() {
            let guard = s.past.as_ref().is_some_and(|p| p.1 == "guard");
            s.xp[Skill::Fighting.index()] = xp_for(if guard { 6 } else { 2 });
        }
        Some(s)
    }
}

/// How it speaks of a skill: "good at farming, and loves it".
pub fn describe(s: Skill, level: u8, passion: u8) -> String {
    let love = match passion {
        0 => "",
        1 => ", and I like it",
        _ => ", and I love it",
    };
    let word = level_word(level);
    if word.starts_with("a ") { format!("{word} at {}{love}", s.name()) } else { format!("{word} at {}{love}", s.name()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_learning() {
        assert_eq!(xp_for(10), 55_000.0);
        assert_eq!(level_of(54_999.0), 9);
        assert_eq!(level_of(55_000.0), 10);
        assert_eq!(level_of(1e9), 20);
        let mut s = Skills { xp: [0.0; N], passion: [0, 2, 0, 0, 0, 0, 0, 0, 0], used: [0; N], faded: 0, past: None };
        assert_eq!(s.practice(Skill::Woodcutting, 700.0, 5), Some(1));
        assert_eq!(s.practice(Skill::Farming, 700.0, 5), None);
        assert!(s.xp[1] > s.xp[0] * 4.0);
        s.xp[0] = xp_for(14) + 10.0;
        s.fade_day(30_000, 24_000);
        assert_eq!(s.level(Skill::Farming), 13);
        assert!(speed(20) > speed(10) && speed(10) > speed(0));
    }

    #[test]
    fn pasts_fit_jobs() {
        let mut fits = 0;
        for u in 0..300u128 {
            let uuid = u.wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835);
            let (_, adult) = past_of(uuid, Some("farm"));
            if adult.is_some_and(|a| a.job == Some("farm")) {
                fits += 1;
            }
            let (levels, passions) = start(uuid, Some("farm"));
            assert!(levels.iter().all(|l| *l <= 20) && passions.iter().all(|p| *p <= 2));
            assert!(past_of(uuid, Some("child")).1.is_none());
        }
        assert!(fits > 150, "{fits}");
        let s = Skills::new(7, Some("fish"));
        assert_eq!(Skills::from_json(&s.to_json()), Some(s.clone()));
        // One saved before fighting was a skill can fight a little; one who stood guard, well.
        let mut old = s.to_json();
        for k in ["xp", "passion", "used"] {
            old[k].as_array_mut().unwrap().truncate(8);
        }
        assert_eq!(Skills::from_json(&old).unwrap().level(Skill::Fighting), 2);
        old["past"] = json!(["orphan", "guard"]);
        assert_eq!(Skills::from_json(&old).unwrap().level(Skill::Fighting), 6);
    }
}
