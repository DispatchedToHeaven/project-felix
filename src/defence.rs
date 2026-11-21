//! How a village defends itself. No mod API types here.
//!
//! Its head keeps guards: one for every five people, one more for a while after raiders came. When
//! raiders come, the guards and those who can fight take up arms (the fit ones good enough at
//! fighting, the steady sooner, the nervous hardly ever, half the grown-ups at most); the rest run
//! home. A friendly village sends a few of its own to help. Raiders who lose half their number
//! run; raiders nobody drove off in time plunder the stores and leave. Fighting is a skill:
//! learned by blows given and taken, it makes each blow harder.

/// Days raiders are remembered: a village that saw them lately keeps one more guard.
pub const WARY_DAYS: i64 = 10;
/// The share of its food and logs raiders carry off from a village they were not driven from.
pub const PLUNDER: f32 = 0.3;
/// What a blow given, and a blow taken, teach (experience).
pub const BLOW_GIVEN: f32 = 60.0;
pub const BLOW_TAKEN: f32 = 20.0;

/// How many guards a village of `people` keeps; `wary`: raiders came not long ago.
pub fn guards_wanted(people: usize, wary: bool) -> usize {
    if people < 4 {
        return 0;
    }
    ((people / 5).max(1) + usize::from(wary)).min(people / 3)
}

/// Whether one who is this good at fighting, with this share of its health left (0..1), takes
/// up arms when raiders come; `nerve`: 1 steady, -1 nervous, 0 neither.
pub fn takes_arms(fighting: u8, health: f32, nerve: i8) -> bool {
    health >= 0.5 && i32::from(fighting) + 3 * i32::from(nerve) >= 5
}

/// Who of those willing (id, fighting) stands with the guards: the best, half the `grown` at most.
pub fn militia(mut willing: Vec<(u32, u8)>, grown: usize) -> Vec<u32> {
    willing.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    willing.truncate(grown / 2);
    willing.into_iter().map(|w| w.0).collect()
}

/// Whether raiders have had enough: `left` of the `came` still stand, half or fewer. One alone
/// has nowhere to run.
pub fn routed(came: u32, left: u32) -> bool {
    came >= 2 && left > 0 && left * 2 <= came
}

/// How many a village sends to help another under attack, their tie being `tie`: friends two,
/// sworn friends three, and none from a village too small to spare them.
pub fn helpers(tie: i32, their_people: usize) -> u32 {
    if their_people < 4 {
        return 0;
    }
    match crate::faction::Bond::of(tie) {
        crate::faction::Bond::Friends => 2,
        crate::faction::Bond::Sworn => 3,
        _ => 0,
    }
}

/// What a blow gains from fighting `level`.
pub fn blow(level: u8) -> f32 {
    0.25 * f32::from(level.min(20))
}

/// What raiders take of `have` things.
pub fn plunder(have: u64) -> u64 {
    (have as f32 * PLUNDER) as u64
}

/// The best one to make a guard: of the candidates (uuid, fighting, has no job), the best
/// fighter; of equals, one without work.
pub fn recruit(cands: &[(u128, u8, bool)]) -> Option<u128> {
    cands.iter().max_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)).then(b.0.cmp(&a.0))).map(|c| c.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guards_by_size() {
        assert_eq!(guards_wanted(3, true), 0);
        assert_eq!(guards_wanted(4, false), 1);
        assert_eq!(guards_wanted(5, true), 1, "a third of the village at most");
        assert_eq!(guards_wanted(6, true), 2);
        assert_eq!(guards_wanted(10, false), 2);
        assert_eq!(guards_wanted(16, true), 4);
        assert_eq!(recruit(&[(1, 3, false), (2, 7, false), (3, 7, true)]), Some(3));
        assert_eq!(recruit(&[(9, 2, false), (4, 2, false)]), Some(4));
        assert_eq!(recruit(&[]), None);
    }

    #[test]
    fn who_fights() {
        assert!(takes_arms(5, 1.0, 0));
        assert!(!takes_arms(4, 1.0, 0));
        assert!(takes_arms(2, 1.0, 1), "the steady stand sooner");
        assert!(!takes_arms(7, 1.0, -1), "the nervous hardly ever");
        assert!(takes_arms(8, 1.0, -1));
        assert!(!takes_arms(12, 0.4, 1), "not the badly hurt");
        assert_eq!(militia(vec![(1, 5), (2, 9), (3, 6), (4, 9)], 5), vec![2, 4]);
        assert_eq!(militia(vec![(1, 5)], 1), Vec::<u32>::new());
    }

    #[test]
    fn raiders_and_helpers() {
        assert!(!routed(1, 1));
        assert!(routed(2, 1));
        assert!(!routed(5, 3));
        assert!(routed(5, 2));
        assert!(!routed(4, 0), "none left is a rout of another kind");
        assert_eq!(helpers(40, 6), 2);
        assert_eq!(helpers(80, 6), 3);
        assert_eq!(helpers(80, 3), 0);
        assert_eq!(helpers(10, 9), 0);
        assert_eq!(plunder(100), 30);
        assert_eq!(plunder(2), 0);
        assert!(blow(20) > blow(8) && blow(0) == 0.0);
    }
}
