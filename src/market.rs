//! The village market: what goods are worth, how what the village has moves prices, what a baker
//! makes of what, and what NPCs say when things run short. No mod API types here.
//!
//! NPCs keep emeralds in a purse (see `markets`). A baker buys wheat, potatoes and fish from the
//! farmers' and fishers' chests and bakes or cooks them; a hungry NPC with nothing in its pantry
//! buys food from whoever has some. The goods move between their chests, the emeralds between their
//! purses. What the village's chests hold, against how many live there, makes a good scarce or
//! plentiful, and the shops ask (and pay) more or less for it.

pub const CURRENCY: &str = "minecraft:emerald";
/// A purse holds at most this many.
pub const MOST: u32 = 64;
/// What a working NPC earns a day (its goods sold beyond the village).
pub const WAGE: u32 = 2;
/// What a seller keeps for itself.
pub const KEEP: u64 = 4;
/// What a buyer buys at once.
pub const LOT: u64 = 9;

/// How many of a good go for one emerald.
pub fn per_emerald(item: &str) -> Option<u32> {
    Some(match item {
        "minecraft:wheat" | "minecraft:potato" | "minecraft:carrot" | "minecraft:beetroot" => 12,
        "minecraft:cod" | "minecraft:salmon" => 6,
        "minecraft:bread" | "minecraft:baked_potato" | "minecraft:cooked_cod" | "minecraft:cooked_salmon" | "minecraft:apple" => 4,
        i if i.ends_with("_log") || i.ends_with("_stem") => 8,
        _ => return None,
    })
}

/// What a baker makes: from so many of one thing, one of another.
pub const RECIPES: &[(&str, u32, &str)] = &[
    ("minecraft:wheat", 3, "minecraft:bread"),
    ("minecraft:potato", 1, "minecraft:baked_potato"),
    ("minecraft:cod", 1, "minecraft:cooked_cod"),
    ("minecraft:salmon", 1, "minecraft:cooked_salmon"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Supply {
    Scarce,
    Short,
    Enough,
    Plenty,
}

/// How a village with `have` of a good stands, `people` living there.
pub fn supply(have: u64, people: u64) -> Supply {
    let want = people.max(1) * 3;
    if have == 0 {
        Supply::Scarce
    } else if have * 2 < want {
        Supply::Short
    } else if have <= want * 3 {
        Supply::Enough
    } else {
        Supply::Plenty
    }
}

impl Supply {
    pub fn factor(self) -> f32 {
        match self {
            Supply::Scarce => 1.6,
            Supply::Short => 1.25,
            Supply::Enough => 1.0,
            Supply::Plenty => 0.75,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Supply::Scarce => "scarce",
            Supply::Short => "short",
            Supply::Enough => "enough",
            Supply::Plenty => "plenty",
        }
    }
}

/// Price `base` as supply `s` has it.
pub fn priced(base: u32, s: Supply) -> u32 {
    ((base as f32 * s.factor()).round() as u32).max(1)
}

/// What `n` of `item` cost between NPCs, in emeralds (at least one).
pub fn cost(item: &str, n: u64, s: Supply) -> u32 {
    let per = per_emerald(item).unwrap_or(4) as f32;
    ((n as f32 * s.factor() / per).ceil() as u32).max(1)
}

/// A shop's note on good `item` (none when there is enough).
pub fn note(item: &str, s: Supply) -> Option<String> {
    let name = short(item);
    let mut c = name.chars();
    let name: String = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
    match s {
        Supply::Scarce => Some(format!("({name} is scarce: dearer.)")),
        Supply::Short => Some(format!("({name} is running short.)")),
        Supply::Plenty => Some(format!("(Plenty of {}: cheaper.)", short(item))),
        Supply::Enough => None,
    }
}

pub fn short(item: &str) -> String {
    item.rsplit(':').next().unwrap_or(item).replace('_', " ")
}

/// A purse in words.
pub fn purse_word(coins: u32) -> &'static str {
    match coins {
        0 => "empty",
        1..=4 => "a few emeralds",
        5..=15 => "some emeralds",
        _ => "well off",
    }
}

/// What an NPC says when the village's food runs short, or a baker has nothing to bake.
pub const SHORT_FOOD: &[&str] = &["Food is running short in the village.", "There's hardly any bread left around here.", "If the harvest doesn't come in soon, we'll all go hungry."];
pub const NO_FLOUR: &[&str] = &["No wheat, no bread, I'm afraid.", "The farmers have nothing for me today.", "Empty oven, empty shelves."];
pub const BROKE: &[&str] = &["I can't even pay for wheat.", "My purse is empty."];
pub const BAKER: &[&str] = &["Fresh bread!", "Mind, the oven's hot.", "Smells good, doesn't it?"];

/// What a baker bakes now from what its chest holds (`have` gives the count of an item): the
/// recipe, if one can be made.
pub fn bake(have: &dyn Fn(&str) -> u64) -> Option<(&'static str, u32, &'static str)> {
    RECIPES.iter().find(|(from, n, _)| have(from) >= u64::from(*n)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markets_price_by_supply() {
        assert_eq!(supply(0, 4), Supply::Scarce);
        assert_eq!(supply(5, 4), Supply::Short);
        assert_eq!(supply(12, 4), Supply::Enough);
        assert_eq!(supply(100, 4), Supply::Plenty);
        assert_eq!(priced(4, Supply::Scarce), 6);
        assert_eq!(priced(1, Supply::Plenty), 1);
        assert_eq!(cost("minecraft:wheat", 9, Supply::Enough), 1);
        assert_eq!(cost("minecraft:wheat", 9, Supply::Scarce), 2);
        assert_eq!(cost("minecraft:bread", 2, Supply::Enough), 1);
        assert_eq!(note("minecraft:bread", Supply::Scarce).as_deref(), Some("(Bread is scarce: dearer.)"));
        assert_eq!(note("minecraft:bread", Supply::Enough), None);
        let chest = |i: &str| if i == "minecraft:wheat" { 2 } else if i == "minecraft:cod" { 1 } else { 0 };
        assert_eq!(bake(&chest).map(|r| r.2), Some("minecraft:cooked_cod"));
        assert_eq!(bake(&|_| 0), None);
        assert_eq!(purse_word(0), "empty");
        assert_eq!(per_emerald("minecraft:spruce_log"), Some(8));
    }
}
