//! What a companion says along the way: about where they are (a new kind of land, a cave,
//! another world), the time and the weather, and when its player is hurt; and the memories it
//! brings up. Closer companions say it warmer. No mod API types here.

use crate::bond::Stage;

/// The kind of land a biome is, for remarks and memories (none: nothing to say).
pub fn land(biome: &str) -> Option<&'static str> {
    let b = biome.rsplit(':').next().unwrap_or(biome);
    let has = |w: &str| b.contains(w);
    Some(if has("snow") || has("frozen") || has("ice") || has("grove") {
        "snow"
    } else if has("desert") {
        "desert"
    } else if has("badlands") {
        "badlands"
    } else if has("jungle") || has("bamboo") {
        "jungle"
    } else if has("swamp") || has("mangrove") {
        "swamp"
    } else if has("ocean") || has("beach") || has("river") {
        "water"
    } else if has("mushroom") {
        "mushrooms"
    } else if has("cherry") {
        "cherry"
    } else if has("peaks") || has("slopes") || has("windswept") || has("stony") {
        "mountains"
    } else if has("dark_forest") || has("pale") {
        "dark woods"
    } else if has("forest") || has("taiga") || has("birch") {
        "forest"
    } else if has("savanna") {
        "savanna"
    } else if has("plains") || has("meadow") || has("sunflower") {
        "plains"
    } else if has("cave") || has("deep_dark") || has("dripstone") {
        "caves"
    } else if has("nether") || has("crimson") || has("warped") || has("basalt") || has("soul") {
        "nether"
    } else if has("end") {
        "end"
    } else {
        return None;
    })
}

fn pick<'a>(lines: &[&'a str], seed: u64) -> &'a str {
    lines[(seed % lines.len() as u64) as usize]
}

/// What it says coming into land `land`.
pub fn on_land(land: &str, stage: Stage, seed: u64) -> String {
    let lines: &[&str] = match land {
        "snow" => &["Brr! Snow, @player. I can't feel my toes.", "Look at all this snow!", "Cold, cold, cold."],
        "desert" => &["So hot... I'd kill for some water.", "Sand everywhere. In my boots, too.", "Watch out for husks, @player."],
        "badlands" => &["Red rock as far as I can see.", "They say there's gold in these hills."],
        "jungle" => &["Listen to those birds!", "Mind the vines, @player.", "It's so green here."],
        "swamp" => &["Ugh, my feet are wet.", "Witches live in swamps, you know.", "It smells funny here."],
        "water" => &["The sea! Isn't it lovely?", "Smell that salt air.", "I could watch the waves all day."],
        "mushrooms" => &["Giant mushrooms! What is this place?", "Are those cows... red?"],
        "cherry" => &["Cherry blossoms... how beautiful.", "It's like walking in a painting."],
        "mountains" => &["What a view from up here!", "Careful on these slopes, @player."],
        "dark woods" => &["It's dark under these trees. Stay close.", "I don't like these woods."],
        "forest" => &["Nice and shady here.", "I love the smell of the woods."],
        "savanna" => &["So warm and wide open here.", "Look, acacia trees!"],
        "plains" => &["Open fields. Good for a walk.", "What a nice day for a walk."],
        "caves" => &["It's dark down here. Light the way?", "Did you hear that?", "Caves give me the creeps."],
        "nether" => &["It's so hot! Where have you brought me?", "This place is terrible. Let's be quick."],
        "end" => &["Where... where are we?", "So quiet. I don't like it."],
        _ => &["Where are we now?"],
    };
    let line = pick(lines, seed).to_string();
    match stage {
        Stage::Sweetheart | Stage::Partner if seed % 3 == 0 => format!("{line} At least I'm here with you."),
        _ => line,
    }
}

/// What it says when the day turns, or the weather.
pub fn on_turn(what: &str, stage: Stage, seed: u64) -> String {
    let lines: &[&str] = match what {
        "dusk" => &["It's getting dark. Monsters will be out soon.", "Night's coming, @player. Let's be careful."],
        "dawn" => &["Morning! We made it through the night.", "Look, the sun's coming up."],
        "rain" => &["Rain... let's find some shelter.", "I'm getting soaked!"],
        "thunder" => &["Thunder! Stay away from tall trees.", "That storm is right on top of us!"],
        "clear" => &["The rain's stopped.", "Sun's out again."],
        _ => &["Hm."],
    };
    let line = pick(lines, seed).to_string();
    match (what, stage) {
        ("dusk", Stage::Sweetheart | Stage::Partner) if seed % 2 == 0 => "The stars will be out soon. Will you watch them with me?".into(),
        _ => line,
    }
}

/// What it says when its player is badly hurt.
pub fn on_hurt(stage: Stage, seed: u64) -> String {
    let lines: &[&str] = match stage {
        Stage::Partner | Stage::Sweetheart => &["@player! Hold on, I've got you!", "Don't you dare fall, @player. Here!"],
        Stage::Close => &["You're hurt! Here, take this.", "Hang in there, @player!"],
        _ => &["Watch out, @player, you're hurt!", "Careful! You don't look good."],
    };
    pick(lines, seed).to_string()
}

/// Whether it helps (heals) its hurt player at this stage.
pub fn heals(stage: Stage) -> bool {
    stage >= Stage::Close
}

/// A memory to bring up: they met `days` ago, fought `fights` monsters side by side, went through
/// `places`, and it loved `gift` from them.
pub fn memory(days: i64, fights: u32, places: &[String], gift: &str, seed: u64) -> String {
    let mut options: Vec<String> = Vec::new();
    if days >= 1 {
        options.push(format!("Can you believe we met {days} day{} ago?", if days == 1 { "" } else { "s" }));
    }
    if fights >= 3 {
        options.push(format!("We've fought {fights} monsters side by side, you know. Not bad, @player."));
    }
    for p in places {
        options.push(match p.as_str() {
            "snow" => "Remember the snow? I was so cold, and you just laughed.".into(),
            "desert" => "Remember the desert? I still have sand in my boots.".into(),
            "jungle" => "Remember the jungle? All those birds!".into(),
            "nether" => "I still dream of that awful burning place you took me to.".into(),
            "caves" => "Remember those caves? I was so scared.".into(),
            "water" => "Remember the sea? I'd like to go back some day.".into(),
            "cherry" => "Remember the cherry blossoms? That was a good day.".into(),
            other => format!("Remember the {other}? That was quite a trip."),
        });
    }
    if !gift.is_empty() {
        options.push(format!("I still have the {} you gave me.", crate::bond::short(gift)));
    }
    if options.is_empty() {
        return "We haven't been through much together yet. Let's change that!".into();
    }
    options[(seed % options.len() as u64) as usize].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companions_remark() {
        assert_eq!(land("minecraft:snowy_taiga"), Some("snow"));
        assert_eq!(land("minecraft:dark_forest"), Some("dark woods"));
        assert_eq!(land("minecraft:the_void"), None);
        assert!(on_land("snow", Stage::Friend, 1).contains("now") || on_land("snow", Stage::Friend, 1).contains("Cold") || on_land("snow", Stage::Friend, 1).contains("Brr"));
        assert!(on_land("snow", Stage::Partner, 3).ends_with("here with you."));
        assert!(heals(Stage::Close) && !heals(Stage::Friend));
        assert!(memory(0, 0, &[], "", 0).contains("haven't"));
        assert!(memory(2, 0, &[], "", 0).contains("2 days"));
        assert!(memory(0, 0, &["snow".into()], "", 0).contains("snow"));
        assert!(memory(0, 0, &[], "minecraft:amethyst_shard", 0).contains("amethyst shard"));
    }
}
