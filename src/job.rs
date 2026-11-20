//! Jobs: what an NPC does at its day's `work`, about its `work` place. No mod API types here.
//!
//! - `farm`: it harvests ripe wheat, carrots, potatoes and beetroots, and sows them again.
//! - `wood`: it fells the trees there (a trunk standing on the ground), and plants a sapling.
//! - `fish`: by water, it fishes now and then (the game's fishing loot).
//! - `build`: it builds the plan it was given (see `plan`, `/felix build`), from its chest.
//! - `bake`: it bakes bread from wheat, and cooks potatoes and fish, in its chest (see `market`:
//!   it buys what it needs from the farmers and fishers).
//!
//! What it gets goes in its chest: the chest (or barrel) nearest its `chest` place, else its
//! `work` place. Its shop, if it has one, sells from that chest: the goods come from its work.
//! An NPC's job is its own (`/felix job`, the maker, its file), or its day's: `"job": "farm"`
//! on a `work` entry.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    Farm,
    Wood,
    Fish,
    Build,
    Bake,
}

pub const JOBS: &[Job] = &[Job::Farm, Job::Wood, Job::Fish, Job::Build, Job::Bake];

impl Job {
    pub fn parse(s: &str) -> Option<Job> {
        match s.trim() {
            "farm" | "farmer" => Some(Job::Farm),
            "wood" | "woodcutter" | "lumberjack" => Some(Job::Wood),
            "fish" | "fisher" | "fisherman" => Some(Job::Fish),
            "build" | "builder" => Some(Job::Build),
            "bake" | "baker" | "cook" => Some(Job::Bake),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Job::Farm => "farm",
            Job::Wood => "wood",
            Job::Fish => "fish",
            Job::Build => "build",
            Job::Bake => "bake",
        }
    }

    /// Ticks it rests between two pieces of work.
    pub fn rest(self) -> i64 {
        match self {
            Job::Farm => 30,
            Job::Wood => 160,
            Job::Fish => 500,
            Job::Build => 10,
            Job::Bake => 200,
        }
    }

    /// What it says now and then, at it.
    pub fn lines(self) -> &'static [&'static str] {
        match self {
            Job::Farm => &["A fine crop this year.", "In they go again.", "Ripe and ready!"],
            Job::Wood => &["Timber!", "Another one down.", "A sapling for the next one."],
            Job::Fish => &["Got one!", "They're biting today.", "Quiet water, good fishing."],
            Job::Build => &["Measure twice, cut once.", "Coming along nicely."],
            Job::Bake => crate::market::BAKER,
        }
    }
}

/// Whether `block` is a crop a farmer reaps.
pub fn is_crop(block: &str) -> bool {
    matches!(block, "minecraft:wheat" | "minecraft:carrots" | "minecraft:potatoes" | "minecraft:beetroots")
}

/// Whether `block` is what a tree has about its top: leaves, or a nether tree's cap.
pub fn is_crown(block: &str) -> bool {
    block.ends_with("_leaves") || block.ends_with("wart_block") || block == "minecraft:shroomlight"
}

/// A crop ripe at `age`: the item sown to grow it again.
pub fn ripe(block: &str, age: Option<u32>) -> Option<&'static str> {
    let (full, seed) = match block {
        "minecraft:wheat" => (7, "minecraft:wheat_seeds"),
        "minecraft:carrots" => (7, "minecraft:carrot"),
        "minecraft:potatoes" => (7, "minecraft:potato"),
        "minecraft:beetroots" => (3, "minecraft:beetroot_seeds"),
        _ => return None,
    };
    (age? >= full).then_some(seed)
}

/// Whether `block` is a log a tree's trunk is made of (not a stripped one, not wood blocks).
pub fn is_log(block: &str) -> bool {
    (block.ends_with("_log") || block.ends_with("_stem")) && !block.contains("stripped")
}

/// Whether a trunk standing on `block` is a tree on the ground (not a log on a house).
pub fn is_soil(block: &str) -> bool {
    matches!(
        block,
        "minecraft:dirt" | "minecraft:grass_block" | "minecraft:podzol" | "minecraft:coarse_dirt" | "minecraft:rooted_dirt" | "minecraft:mud" | "minecraft:moss_block" | "minecraft:mycelium" | "minecraft:crimson_nylium" | "minecraft:warped_nylium"
    )
}

/// The sapling that grows log `log`'s tree, to plant where it stood.
pub fn sapling(log: &str) -> Option<String> {
    let wood = log.strip_suffix("_log").or_else(|| log.strip_suffix("_stem"))?;
    Some(match wood {
        "minecraft:mangrove" => "minecraft:mangrove_propagule".into(),
        "minecraft:crimson" | "minecraft:warped" => format!("{wood}_fungus"),
        "minecraft:pale_oak" | "minecraft:oak" | "minecraft:spruce" | "minecraft:birch" | "minecraft:jungle" | "minecraft:acacia" | "minecraft:dark_oak" | "minecraft:cherry" => format!("{wood}_sapling"),
        _ => return None,
    })
}

/// The tallest trunk it fells.
pub const MAX_TRUNK: i32 = 24;

/// How far from its `work` place it works, sideways and up and down.
pub const RADIUS: i32 = 8;
pub const DEPTH: i32 = 3;

/// Of `spots`, the nearest to `from` (sideways first, then up and down).
pub fn nearest(from: [f64; 3], spots: &[[i32; 3]]) -> Option<[i32; 3]> {
    let d = |p: &[i32; 3]| {
        let (dx, dy, dz) = (f64::from(p[0]) + 0.5 - from[0], f64::from(p[1]) - from[1], f64::from(p[2]) + 0.5 - from[2]);
        dx * dx + dz * dz + dy * dy * 0.25
    };
    spots.iter().copied().min_by(|a, b| d(a).total_cmp(&d(b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_know_their_work() {
        assert_eq!(Job::parse("farmer"), Some(Job::Farm));
        assert_eq!(Job::parse("none"), None);
        assert!(JOBS.iter().all(|j| Job::parse(j.name()) == Some(*j)));
        assert_eq!(ripe("minecraft:wheat", Some(7)), Some("minecraft:wheat_seeds"));
        assert_eq!(ripe("minecraft:wheat", Some(6)), None);
        assert_eq!(ripe("minecraft:beetroots", Some(3)), Some("minecraft:beetroot_seeds"));
        assert_eq!(ripe("minecraft:dirt", Some(7)), None);
        assert!(is_log("minecraft:oak_log") && !is_log("minecraft:stripped_oak_log") && !is_log("minecraft:oak_wood"));
        assert_eq!(sapling("minecraft:dark_oak_log").as_deref(), Some("minecraft:dark_oak_sapling"));
        assert_eq!(sapling("minecraft:mangrove_log").as_deref(), Some("minecraft:mangrove_propagule"));
        assert_eq!(sapling("minecraft:crimson_stem").as_deref(), Some("minecraft:crimson_fungus"));
        assert_eq!(sapling("othermod:maple_log"), None);
        assert_eq!(nearest([0.5, 64.0, 0.5], &[[5, 64, 5], [1, 64, 0], [0, 70, 0]]), Some([1, 64, 0]));
    }
}
