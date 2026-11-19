//! NPCs' needs: food. No mod API types here.
//!
//! An NPC is fed from 0 to 20 and loses a point every two minutes. Under 15 it eats from its
//! pantry (its job's chest, else a chest by its home); under 7 it is hungry (it says so, its
//! character feels `hungry`, it obeys worse); at 0 it starves (it obeys worse still). A player can
//! feed it from their own food (the `feed` dialog action).

pub const FULL: u8 = 20;
pub const EATS_AT: u8 = 14;
pub const HUNGRY: u8 = 6;
/// Ticks a point of food lasts.
pub const PER_POINT: i64 = 2400;

/// What NPCs eat, best first, and how much each feeds.
pub const FOODS: &[(&str, u8)] = &[
    ("minecraft:bread", 5),
    ("minecraft:baked_potato", 5),
    ("minecraft:cooked_beef", 8),
    ("minecraft:cooked_porkchop", 8),
    ("minecraft:cooked_mutton", 6),
    ("minecraft:cooked_chicken", 6),
    ("minecraft:cooked_salmon", 6),
    ("minecraft:cooked_cod", 5),
    ("minecraft:pumpkin_pie", 8),
    ("minecraft:mushroom_stew", 6),
    ("minecraft:beetroot_soup", 6),
    ("minecraft:rabbit_stew", 10),
    ("minecraft:apple", 4),
    ("minecraft:carrot", 3),
    ("minecraft:cookie", 2),
    ("minecraft:melon_slice", 2),
    ("minecraft:sweet_berries", 2),
    ("minecraft:glow_berries", 2),
    ("minecraft:beetroot", 1),
    ("minecraft:potato", 1),
    ("minecraft:dried_kelp", 1),
    ("minecraft:cod", 2),
    ("minecraft:salmon", 2),
    ("minecraft:honey_bottle", 6),
    ("minecraft:golden_apple", 4),
];

pub fn nutrition(item: &str) -> Option<u8> {
    FOODS.iter().find(|f| f.0 == item).map(|f| f.1)
}

/// Food `food` last counted at `last`, now: what is left, and when it was counted to.
pub fn after(food: u8, last: i64, now: i64) -> (u8, i64) {
    if now < last {
        return (food, now);
    }
    let lost = (now - last) / PER_POINT;
    (food.saturating_sub(lost.min(255) as u8), last + lost * PER_POINT)
}

/// How it feels about its food, in a word (empty: fine).
pub fn word(food: u8) -> &'static str {
    match food {
        0 => "starving",
        f if f <= HUNGRY => "hungry",
        _ => "",
    }
}

/// How much its food lets it obey (times its mood's).
pub fn obey_factor(food: u8) -> f32 {
    match food {
        0 => 0.4,
        f if f <= HUNGRY => 0.75,
        _ => 1.0,
    }
}

/// What a hungry NPC says.
pub const HUNGRY_LINES: &[&str] = &["I'm getting hungry.", "My stomach is growling.", "Has anyone got something to eat?", "I could really use a meal."];
pub const STARVING_LINES: &[&str] = &["I'm starving...", "Please, anything to eat...", "I can't work like this. I need food."];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn food_runs_down() {
        assert_eq!(after(20, 0, PER_POINT * 3 + 10), (17, PER_POINT * 3));
        assert_eq!(after(2, 0, PER_POINT * 10), (0, PER_POINT * 10));
        assert_eq!(after(10, 500, 100), (10, 100));
        assert_eq!(word(5), "hungry");
        assert_eq!(word(0), "starving");
        assert_eq!(word(12), "");
        assert!(obey_factor(0) < obey_factor(5) && obey_factor(5) < obey_factor(20));
        assert_eq!(nutrition("minecraft:bread"), Some(5));
        assert_eq!(nutrition("minecraft:dirt"), None);
    }
}
