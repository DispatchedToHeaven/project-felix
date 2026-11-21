//! What NPCs say to each other when they meet, besides news (see `heart`): chit-chat, a
//! heart-to-heart, kind words, a slight or an insult, and what each does to how they think of each
//! other. Some pairs just get on (or don't): their fit. An insult may come to blows. No mod API
//! types here.

use crate::mind::Trait;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Chat,
    HeartToHeart,
    KindWords,
    Slight,
    Insult,
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

/// How well two NPCs get on, from -1 to 1, the same both ways.
pub fn fit(a: u128, b: u128) -> f32 {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let h = mix((lo as u64) ^ ((hi as u64).rotate_left(29)) ^ ((lo >> 64) as u64) ^ ((hi >> 64) as u64).rotate_left(7));
    (h % 2001) as f32 / 1000.0 - 1.0
}

/// How likely each act is, from a speaker with `traits`, in `mood`, with a hearer it fits `fit`.
pub fn weights(traits: &[Trait], mood: f32, fit: f32) -> Vec<(Act, f32)> {
    let rude = if traits.contains(&Trait::Rude) { 2.5 } else { 1.0 };
    let kind = traits.contains(&Trait::Kind);
    let sour = if mood < 30.0 { 2.0 } else { 1.0 };
    let mut w = vec![
        (Act::Chat, 1.0),
        (Act::HeartToHeart, 0.08 * (1.0 + fit)),
        (Act::Slight, if kind { 0.0 } else { 0.03 * rude * sour * (1.0 - fit * 0.5) }),
        (Act::Insult, if kind { 0.0 } else { 0.01 * rude * sour * (1.0 - fit * 0.5) }),
    ];
    if kind {
        w.push((Act::KindWords, 0.12));
    }
    w
}

/// The act a roll (0..1) picks.
pub fn pick(w: &[(Act, f32)], roll: f32) -> Act {
    let total: f32 = w.iter().map(|x| x.1).sum();
    let mut r = roll.clamp(0.0, 0.999_999) * total;
    for (a, x) in w {
        if r < *x {
            return *a;
        }
        r -= x;
    }
    Act::Chat
}

/// How the hearer's opinion of the speaker moves, and the speaker's of the hearer.
pub fn opinion(a: Act) -> (f32, f32) {
    match a {
        Act::Chat => (0.6, 0.6),
        Act::HeartToHeart => (6.0, 6.0),
        Act::KindWords => (5.0, 1.0),
        Act::Slight => (-4.0, 0.0),
        Act::Insult => (-10.0, -1.0),
    }
}

/// What the hearer keeps of it (a thought, see `mind`).
pub fn thought(a: Act) -> Option<&'static str> {
    match a {
        Act::HeartToHeart => Some("heart_to_heart"),
        Act::KindWords => Some("praised"),
        Act::Insult => Some("insulted"),
        _ => None,
    }
}

/// The chance it comes to blows (the hearer hits back), more with a short temper.
pub fn fight_chance(a: Act, hearer: &[Trait]) -> f32 {
    let base = match a {
        Act::Insult => 0.08,
        Act::Slight => 0.01,
        _ => 0.0,
    };
    if hearer.contains(&Trait::Rude) || hearer.contains(&Trait::Nervous) { base * 2.0 } else { base }
}

/// After a fight they think better of each other (it cleared the air) or worse, by a roll.
pub fn after_fight(roll: f32) -> f32 {
    if roll < 0.5 { 8.0 } else { -12.0 }
}

/// What the speaker says (`@n`: the hearer).
pub fn line(a: Act, roll: f32) -> &'static str {
    let pick = |l: &'static [&'static str]| l[((roll * l.len() as f32) as usize).min(l.len() - 1)];
    match a {
        Act::Chat => pick(&["Morning, @n.", "Fine weather, isn't it, @n?", "Busy day, @n?", "Did you see the crops this year, @n?", "How's the family, @n?"]),
        Act::HeartToHeart => pick(&["@n, can I tell you something I've never told anyone?", "You know, @n, I think about home a lot.", "I'm glad we talk, @n. Really."]),
        Act::KindWords => pick(&["You're doing great, @n.", "I'm glad you're here, @n.", "That was kind of you, @n."]),
        Act::Slight => pick(&["Oh. It's you, @n.", "Is that how you always do it, @n?", "Hmph."]),
        Act::Insult => pick(&["You're useless, @n.", "Nobody asked you, @n.", "What a waste of space you are, @n."]),
    }
}

/// What the hearer says back.
pub fn reply(a: Act, roll: f32) -> &'static str {
    let pick = |l: &'static [&'static str]| l[((roll * l.len() as f32) as usize).min(l.len() - 1)];
    match a {
        Act::Chat => pick(&["Morning!", "Can't complain.", "Aye, it is."]),
        Act::HeartToHeart => pick(&["Of course. I'm listening.", "Me too. Me too.", "Thank you for telling me."]),
        Act::KindWords => pick(&["Oh! Thank you.", "That means a lot."]),
        Act::Slight => pick(&["...", "Excuse me?"]),
        Act::Insult => pick(&["How dare you!", "Say that again. I dare you.", "That's it."]),
    }
}

/// Partners who think this little of each other part.
pub const PART_AT: i32 = -10;
/// One this disliked is a rival.
pub const RIVAL_AT: i32 = -20;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_is_mutual() {
        for i in 0..50u128 {
            let (a, b) = (i * 7919 + 1, i * 104_729 + 3);
            assert_eq!(fit(a, b), fit(b, a));
            assert!((-1.0..=1.0).contains(&fit(a, b)));
        }
    }

    #[test]
    fn kind_ones_never_insult() {
        let w = weights(&[Trait::Kind], 10.0, -1.0);
        for i in 0..100 {
            let a = pick(&w, i as f32 / 100.0);
            assert!(!matches!(a, Act::Insult | Act::Slight), "{a:?}");
        }
        let rude = weights(&[Trait::Rude], 10.0, -1.0);
        let calm = weights(&[], 80.0, 1.0);
        let bad = |w: &[(Act, f32)]| w.iter().filter(|x| matches!(x.0, Act::Insult | Act::Slight)).map(|x| x.1).sum::<f32>();
        assert!(bad(&rude) > bad(&calm) * 5.0);
        assert_eq!(pick(&calm, 0.0), Act::Chat);
        assert!(fight_chance(Act::Insult, &[Trait::Rude]) > fight_chance(Act::Insult, &[]));
        assert_eq!(fight_chance(Act::Chat, &[Trait::Rude]), 0.0);
        assert!(opinion(Act::Insult).0 < 0.0 && opinion(Act::HeartToHeart).0 > opinion(Act::Chat).0);
    }
}
