//! The village book's pages (see `books`): what a player knows of each NPC near, in words, with
//! the numbers only in the extended view. No mod API types here.

#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub name: String,
    /// Its character's name (`calm`, `cheerful`…).
    pub character: String,
    pub mood: String,
    pub relation: i32,
    /// How it stands with the player in a word (`a friend`; empty: neither way).
    pub standing: String,
    /// Past friends: `a close friend`, `your sweetheart`, `your partner`.
    pub stage: String,
    pub closeness: f32,
    pub job: String,
    pub work: Option<[i32; 3]>,
    pub home: Option<[i32; 3]>,
    /// What its day has it doing (`work`, `sleep`, `event`…; empty: not by a day).
    pub doing: String,
    pub event: String,
    /// Whose it is: `you`, someone else's name, or empty (no one's).
    pub owner: String,
    pub food: u8,
    pub partner: String,
    pub children: Vec<String>,
    pub parents: Vec<String>,
    pub child: bool,
    pub quests: usize,
    /// Its purse (none: not known).
    pub purse: Option<u32>,
}

/// Which way `dx`, `dz` points (north is −z).
pub fn direction(dx: f64, dz: f64) -> &'static str {
    if dx.abs() < 2.0 && dz.abs() < 2.0 {
        return "here";
    }
    const WAYS: [&str; 8] = ["south", "south-west", "west", "north-west", "north", "north-east", "east", "south-east"];
    // Angle from south (+z) turning toward west (−x).
    let a = (-dx).atan2(dz).to_degrees().rem_euclid(360.0);
    WAYS[(((a + 22.5) / 45.0) as usize) % 8]
}

/// A way in letters: `north-east` as `NE`.
pub fn abbr(way: &str) -> String {
    way.split('-').filter_map(|w| w.chars().next()).map(|c| c.to_ascii_uppercase()).collect()
}

/// The list's short mark for how it stands with the player.
pub fn mark(f: &Facts) -> &'static str {
    match (f.stage.as_str(), f.standing.as_str()) {
        ("your partner", _) | ("your sweetheart", _) => "\u{2665}",
        ("a close friend", _) | (_, "a friend") => "+",
        (_, "friendly") => "\u{00B7}",
        (_, "cold") | (_, "hostile") => "-",
        _ => "",
    }
}

fn at(p: [i32; 3]) -> String {
    format!("{} {} {}", p[0], p[1], p[2])
}

fn food_word(food: u8) -> &'static str {
    match food {
        0 => "starving",
        f if f <= crate::need::HUNGRY => "hungry",
        f if f < crate::need::EATS_AT => "could eat",
        _ => "well fed",
    }
}

fn now_word(f: &Facts) -> String {
    match f.doing.as_str() {
        "" if f.owner == "you" => "with you".into(),
        "" if !f.owner.is_empty() => format!("with {}", f.owner),
        "" => "about".into(),
        "work" => "at work".into(),
        "sleep" => "asleep".into(),
        "event" if !f.event.is_empty() => format!("at the {}", f.event),
        "shelter" => "sheltering from the weather".into(),
        other => other.replace('_', " "),
    }
}

/// Its page, line by line.
pub fn lines(f: &Facts, extended: bool) -> Vec<String> {
    let mut out = Vec::new();
    let toward = if f.standing.is_empty() { "neither here nor there".to_string() } else { f.standing.clone() };
    out.push(if extended {
        format!("To you: {toward} (relation {}, closeness {:.0})", f.relation, f.closeness)
    } else {
        format!("To you: {toward}")
    });
    if !f.stage.is_empty() {
        out.push(format!("{} is {}.", f.name, f.stage));
    }
    out.push(if f.character.is_empty() { format!("Mood: {}", f.mood) } else { format!("Nature: {}, feeling {}", f.character, f.mood) });
    out.push(format!("Now: {}", now_word(f)));
    if !f.job.is_empty() {
        out.push(match f.work {
            Some(p) => format!("Work: {} (at {})", f.job, at(p)),
            None => format!("Work: {}", f.job),
        });
    }
    if let Some(h) = f.home {
        out.push(format!("Home: {}", at(h)));
    }
    out.push(if extended { format!("Food: {} ({}/{})", food_word(f.food), f.food, crate::need::FULL) } else { format!("Food: {}", food_word(f.food)) });
    let mut fam = Vec::new();
    if !f.partner.is_empty() {
        fam.push(format!("partner {}", f.partner));
    }
    if !f.children.is_empty() {
        fam.push(format!("{} {}", if f.children.len() == 1 { "child" } else { "children" }, f.children.join(", ")));
    }
    if !f.parents.is_empty() {
        fam.push(format!("{} of {}", if f.child { "a child" } else { "grown child" }, f.parents.join(" and ")));
    }
    if !fam.is_empty() {
        out.push(format!("Family: {}", fam.join("; ")));
    }
    if let Some(c) = f.purse {
        out.push(if extended { format!("Purse: {} ({c})", crate::market::purse_word(c)) } else { format!("Purse: {}", crate::market::purse_word(c)) });
    }
    if f.quests > 0 {
        out.push("Has a favour to ask.".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_hide_numbers() {
        let f = Facts { name: "Mia".into(), mood: "happy".into(), relation: 42, standing: "a friend".into(), stage: "your sweetheart".into(), closeness: 61.0, food: 4, partner: "Tom".into(), ..Facts::default() };
        let plain = lines(&f, false).join("\n");
        assert!(!plain.contains("42") && !plain.contains("61") && !plain.contains("/20"));
        assert!(plain.contains("To you: a friend") && plain.contains("Mia is your sweetheart.") && plain.contains("Food: hungry"));
        let ext = lines(&f, true).join("\n");
        assert!(ext.contains("relation 42") && ext.contains("(4/20)"));
        assert_eq!(mark(&f), "\u{2665}");
        assert_eq!(direction(0.0, -20.0), "north");
        assert_eq!(direction(20.0, 0.0), "east");
        assert_eq!(direction(-15.0, 15.0), "south-west");
        assert_eq!(direction(1.0, 1.0), "here");
        assert_eq!(abbr("north-east"), "NE");
    }
}
