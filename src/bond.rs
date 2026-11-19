//! How close an NPC and a player are, and where it may lead: from strangers to partners. No mod
//! API types here.
//!
//! Besides how much it likes the player (its relation, see `character`), an NPC keeps how close
//! they are (0 to 100): it grows with time together (following them), talks, gifts it likes,
//! fights won side by side and quests done for it, and fades when they don't see each other for
//! days. Together they make the stage: a stranger, an acquaintance (they met), a friend (it likes
//! them as its character counts friends), a close friend (relation 70 and close 30). Past that the
//! player may ask it out (relation 80, close 50, no one else, its character allows romance): it
//! may say yes (a sweetheart), and after three days more (relation 90, close 80) the player may
//! propose with a diamond: partners. Hurt it until it dislikes the player and it ends it.
//!
//! Each NPC loves two things and can't stand one, by its UUID; it tells a close enough player.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Stranger,
    Acquaintance,
    Friend,
    Close,
    Sweetheart,
    Partner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Romance {
    #[default]
    None,
    Sweetheart,
    Partner,
}

impl Romance {
    pub fn byte(self) -> u8 {
        match self {
            Romance::None => 0,
            Romance::Sweetheart => 1,
            Romance::Partner => 2,
        }
    }
    pub fn from_byte(b: u8) -> Romance {
        match b {
            1 => Romance::Sweetheart,
            2 => Romance::Partner,
            _ => Romance::None,
        }
    }
}

pub const CLOSE_AT: (i32, f32) = (70, 30.0);
pub const ASK_AT: (i32, f32) = (80, 50.0);
pub const PROPOSE_AT: (i32, f32) = (90, 80.0);
/// Days as sweethearts before a proposal.
pub const COURTING: i64 = 3 * 24000;
/// Below this relation a romance ends.
pub const BREAKS_BELOW: i32 = 20;
pub const RING: &str = "minecraft:diamond";

impl Stage {
    pub fn of(relation: i32, close: f32, romance: Romance, met: bool, friend_at: i32) -> Stage {
        match romance {
            Romance::Partner => Stage::Partner,
            Romance::Sweetheart => Stage::Sweetheart,
            Romance::None if relation >= CLOSE_AT.0 && close >= CLOSE_AT.1 => Stage::Close,
            Romance::None if relation >= friend_at => Stage::Friend,
            Romance::None if met => Stage::Acquaintance,
            Romance::None => Stage::Stranger,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Stage::Stranger => "stranger",
            Stage::Acquaintance => "acquaintance",
            Stage::Friend => "friend",
            Stage::Close => "close",
            Stage::Sweetheart => "sweetheart",
            Stage::Partner => "partner",
        }
    }

    pub fn parse(s: &str) -> Option<Stage> {
        [Stage::Stranger, Stage::Acquaintance, Stage::Friend, Stage::Close, Stage::Sweetheart, Stage::Partner].into_iter().find(|x| x.name() == s.trim())
    }

    /// How the player stands with it, in words (empty: nothing to say).
    pub fn word(self) -> &'static str {
        match self {
            Stage::Close => "a close friend",
            Stage::Sweetheart => "your sweetheart",
            Stage::Partner => "your partner",
            _ => "",
        }
    }
}

/// Why a player may not ask it out now (none: they may).
pub fn may_ask(relation: i32, close: f32, romance: Romance, allows: bool, taken: Option<&str>, player_taken: Option<&str>) -> Result<(), String> {
    if !allows {
        return Err("I like you, @player, but not like that.".into());
    }
    if romance != Romance::None {
        return Err("We're already together, silly!".into());
    }
    if let Some(other) = taken {
        return Err(format!("I'm with {other}, @player. You know that."));
    }
    if let Some(other) = player_taken {
        return Err(format!("Aren't you with {other}?"));
    }
    if relation < ASK_AT.0 || close < ASK_AT.1 {
        return Err("I... don't think we know each other well enough yet.".into());
    }
    Ok(())
}

/// The chance (percent) it says yes, by its mood.
pub fn yes_chance(mood: &str) -> u32 {
    match mood {
        "happy" => 95,
        "calm" => 80,
        "upset" | "grieving" => 25,
        "angry" => 5,
        _ => 70,
    }
}

/// Why a player may not propose now.
pub fn may_propose(relation: i32, close: f32, romance: Romance, since: i64, now: i64, has_ring: bool) -> Result<(), String> {
    if romance != Romance::Sweetheart {
        return Err(if romance == Romance::Partner { "We already are, love.".into() } else { "Shouldn't we go out first?".into() });
    }
    if now - since < COURTING || relation < PROPOSE_AT.0 || close < PROPOSE_AT.1 {
        return Err("It's a little soon, don't you think? Give it time.".into());
    }
    if !has_ring {
        return Err("Without a ring? Oh, @player.".into());
    }
    Ok(())
}

const LOVES: &[&str] = &[
    "minecraft:sunflower",
    "minecraft:poppy",
    "minecraft:cake",
    "minecraft:cookie",
    "minecraft:golden_apple",
    "minecraft:emerald",
    "minecraft:book",
    "minecraft:honey_bottle",
    "minecraft:pumpkin_pie",
    "minecraft:glow_berries",
    "minecraft:amethyst_shard",
    "minecraft:lily_of_the_valley",
    "minecraft:blue_orchid",
    "minecraft:sweet_berries",
    "minecraft:cornflower",
    "minecraft:music_disc_cat",
];
const HATES: &[&str] = &["minecraft:rotten_flesh", "minecraft:spider_eye", "minecraft:poisonous_potato", "minecraft:bone", "minecraft:pufferfish", "minecraft:slime_ball"];

fn mix(mut x: u64) -> u64 {
    x ^= x >> 31;
    x = x.wrapping_mul(0x7fb5_d329_728e_a185);
    x ^= x >> 27;
    x = x.wrapping_mul(0x81da_def4_bc2d_d44d);
    x ^ (x >> 33)
}

/// What an NPC loves (two things) and can't stand (one), by its UUID.
pub fn tastes(uuid: u128) -> ([&'static str; 2], &'static str) {
    let h = mix((uuid as u64) ^ ((uuid >> 64) as u64));
    let a = (h % LOVES.len() as u64) as usize;
    let b = ((h >> 16) % (LOVES.len() as u64 - 1)) as usize;
    let b = if b >= a { b + 1 } else { b };
    ([LOVES[a], LOVES[b]], HATES[((h >> 32) % HATES.len() as u64) as usize])
}

/// How it takes a gift of `item`: relation, closeness, and its word for it.
pub fn gift(item: &str, uuid: u128) -> (i32, f32, &'static str) {
    let (loves, hates) = tastes(uuid);
    let flower = item.ends_with("_tulip") || ["minecraft:dandelion", "minecraft:poppy", "minecraft:allium", "minecraft:azure_bluet", "minecraft:oxeye_daisy", "minecraft:rose_bush", "minecraft:peony", "minecraft:lilac"].contains(&item);
    if loves.contains(&item) {
        (8, 6.0, "love")
    } else if item == hates {
        (-6, -3.0, "hate")
    } else if flower || crate::need::nutrition(item).is_some() || item == RING {
        (3, 2.0, "like")
    } else {
        (1, 0.5, "meh")
    }
}

/// An item id in words: `minecraft:lily_of_the_valley` → `lily of the valley`.
pub fn short(item: &str) -> String {
    item.rsplit(':').next().unwrap_or(item).replace('_', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bonds_grow_into_more() {
        assert_eq!(Stage::of(10, 0.0, Romance::None, false, 30), Stage::Stranger);
        assert_eq!(Stage::of(10, 0.0, Romance::None, true, 30), Stage::Acquaintance);
        assert_eq!(Stage::of(40, 10.0, Romance::None, true, 30), Stage::Friend);
        assert_eq!(Stage::of(75, 35.0, Romance::None, true, 30), Stage::Close);
        assert_eq!(Stage::of(-5, 0.0, Romance::Partner, true, 30), Stage::Partner);
        assert!(Stage::Close < Stage::Sweetheart);
        assert_eq!(Stage::parse("close"), Some(Stage::Close));
        assert!(may_ask(85, 60.0, Romance::None, true, None, None).is_ok());
        assert!(may_ask(85, 40.0, Romance::None, true, None, None).unwrap_err().contains("well enough"));
        assert!(may_ask(85, 60.0, Romance::None, true, Some("Will"), None).unwrap_err().contains("Will"));
        assert!(may_ask(85, 60.0, Romance::None, false, None, None).is_err());
        assert!(may_propose(95, 90.0, Romance::Sweetheart, 0, COURTING, true).is_ok());
        assert!(may_propose(95, 90.0, Romance::Sweetheart, 0, COURTING - 1, true).is_err());
        assert!(may_propose(95, 90.0, Romance::Sweetheart, 0, COURTING, false).unwrap_err().contains("ring"));
        let (loves, hates) = tastes(12345);
        assert_ne!(loves[0], loves[1]);
        assert_eq!(gift(loves[1], 12345).2, "love");
        assert_eq!(gift(hates, 12345).2, "hate");
        assert_eq!(gift("minecraft:bread", 12345).2, "like");
        assert_eq!(short("minecraft:lily_of_the_valley"), "lily of the valley");
        assert!(yes_chance("happy") > yes_chance("angry"));
    }
}
