//! NPC families: couples, and the children they have. No mod API types here.
//!
//! Two grown NPCs who have long been friends (each likes the other `COUPLE_AT` or more), near
//! each other, may become a couple: they live in one home. A couple has a child now and then
//! (`CHILD_EVERY` game days apart, `MAX_CHILDREN` at most): a small NPC by their home that grows up
//! in `GROWS_IN` days. It has one parent's look and character and the other's voice, lives by a
//! child's day (play, home, sleep) and, grown, by a villager's, with a parent's job.

/// How much each must like the other.
pub const COUPLE_AT: i32 = 50;
/// The chance a fitting pair becomes a couple at a look (every half minute).
pub const COUPLE_CHANCE: u32 = 8;
pub const MAX_CHILDREN: usize = 2;
/// Ticks between children, and to grow up.
pub const CHILD_EVERY: i64 = 2 * 24000;
pub const GROWS_IN: i64 = 3 * 24000;

const NAMES: &[&str] = &["Tim", "Sue", "Max", "Lily", "Ben", "Rosa", "Finn", "Nell", "Otto", "Ivy", "Leo", "Mira", "Sam", "Tess", "Jon", "Pip", "Kit", "Bea", "Ned", "Ada"];

/// A child's name, by a seed, not one of `taken`.
pub fn child_name(seed: u64, taken: &[String]) -> String {
    let start = (seed % NAMES.len() as u64) as usize;
    (0..NAMES.len())
        .map(|k| NAMES[(start + k) % NAMES.len()].to_string())
        .find(|n| !taken.contains(n))
        .unwrap_or_else(|| format!("{}{}", NAMES[start], seed % 100))
}

/// Which parent a child takes its look and character from (the other gives its voice).
pub fn takes_after(seed: u64) -> usize {
    ((seed >> 7) % 2) as usize
}

/// A family in words: `I live with Ann. We have two children: Tim and Sue.`
pub fn describe(partner: Option<&str>, children: &[String], parents: &[String]) -> String {
    let mut parts = Vec::new();
    if !parents.is_empty() {
        parts.push(format!("My parents are {}.", parents.join(" and ")));
    }
    if let Some(p) = partner {
        parts.push(format!("I live with {p}."));
    }
    match children.len() {
        0 => {}
        1 => parts.push(format!("We have a child, {}.", children[0])),
        n => parts.push(format!("We have {} children: {}.", if n == 2 { "two".to_string() } else { n.to_string() }, children.join(" and "))),
    }
    if parts.is_empty() { "It's just me.".into() } else { parts.join(" ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_have_names() {
        let a = child_name(3, &[]);
        assert_eq!(child_name(3, std::slice::from_ref(&a)), NAMES[4]);
        assert!(takes_after(0) < 2 && takes_after(1 << 7) == 1);
        assert_eq!(describe(Some("Ann"), &["Tim".into(), "Sue".into()], &[]), "I live with Ann. We have two children: Tim and Sue.");
        assert_eq!(describe(None, &[], &["Ann".into(), "Bob".into()]), "My parents are Ann and Bob.");
        assert_eq!(describe(None, &[], &[]), "It's just me.");
    }
}
