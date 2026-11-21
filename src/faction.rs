//! Villages among others: what a village thinks of each player (their standing there) and of each
//! other village (the tie between them). No mod API types here.
//!
//! Standing runs from -100 to 100. Deeds move it: bringing what the village asked for, killing
//! raiders at its gates, trading with its people, tending its hurt, quests done for its folk and
//! its word carried to another village raise it; hitting and killing its people lower it. By it
//! the player is an enemy there (nobody talks or trades, the guards attack), unwelcome (dearer
//! prices, or less for a coin), a stranger, a friend (cheaper prices, or more for a coin, and a
//! share of the village's food when asked) or an ally (cheaper still, a bigger share). Grudges fade a point a day.
//!
//! A tie between two villages runs the same way: caravans between them, help sent in a hungry
//! time and word carried by players warm it, quarrels cool it. Friends hear what a player does
//! for (or to) the other and think alike; a village at feud thinks less of those who help its
//! rival. Feuds cool off slowly by themselves.

pub const DAY: i64 = 24_000;
pub const MIN: i32 = -100;
pub const MAX: i32 = 100;
/// Small favours (a trade, a wound tended) that count in a day.
pub const FAVOURS_A_DAY: u8 = 3;
/// Ticks in which blows count as one.
pub const HIT_REST: i64 = 60;
/// Days an errand to another village keeps.
pub const ERRAND_DAYS: i64 = 7;

/// What a player is to a village.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rank {
    Enemy,
    Unwelcome,
    Stranger,
    Friend,
    Ally,
}

impl Rank {
    pub fn of(v: i32) -> Rank {
        match v {
            i32::MIN..=-60 => Rank::Enemy,
            -59..=-20 => Rank::Unwelcome,
            -19..=19 => Rank::Stranger,
            20..=59 => Rank::Friend,
            _ => Rank::Ally,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Rank::Enemy => "an enemy",
            Rank::Unwelcome => "unwelcome",
            Rank::Stranger => "a stranger",
            Rank::Friend => "a friend",
            Rank::Ally => "an ally",
        }
    }

    /// What its people ask of the usual price (none: they don't trade).
    pub fn price(self) -> Option<f32> {
        match self {
            Rank::Enemy => None,
            Rank::Unwelcome => Some(1.2),
            Rank::Stranger => Some(1.0),
            Rank::Friend => Some(0.9),
            Rank::Ally => Some(0.8),
        }
    }

    /// How much food the village spares them a day, when asked.
    pub fn share(self) -> u32 {
        match self {
            Rank::Friend => 2,
            Rank::Ally => 4,
            _ => 0,
        }
    }

    /// What a villager says of it to `player`.
    pub fn says(self, village: &str, player: &str) -> String {
        match self {
            Rank::Enemy => format!("You're not welcome in {village}. Go."),
            Rank::Unwelcome => format!("Folk here would rather you moved on, {player}."),
            Rank::Stranger => "Folk here don't know you well yet.".to_string(),
            Rank::Friend => format!("You're a friend of {village}, {player}. Folk here think well of you."),
            Rank::Ally => format!("You're one of us, {player}. {village} stands with you."),
        }
    }
}

/// What `count` things for `price` become at `factor` of the usual price: the price moves when
/// it can; where it is too small to (a thing for one coin), the coin buys more or fewer things.
pub fn deal(price: u32, count: u32, factor: f32) -> (u32, u32) {
    let p = ((price as f32 * factor).round() as u32).max(1);
    if p != price || (factor - 1.0).abs() < 0.01 {
        return (p, count);
    }
    (p, ((count as f32 / factor).round() as u32).max(1))
}

/// What a player did for (or to) a village.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deed {
    /// Brought it this many things it asked for.
    Brought(u32),
    RaiderSlain,
    Traded,
    Tended,
    QuestDone,
    /// Carried word between it and another village.
    Envoy,
    /// Paid raiders to leave it be.
    PaidOff,
    Hit,
    Killed,
}

impl Deed {
    pub fn worth(self) -> i32 {
        match self {
            Deed::Brought(n) => (n as i32 / 8).clamp(1, 6),
            Deed::RaiderSlain => 4,
            Deed::Traded => 1,
            Deed::Tended => 2,
            Deed::QuestDone => 4,
            Deed::Envoy => 6,
            Deed::PaidOff => 5,
            Deed::Hit => -4,
            Deed::Killed => -40,
        }
    }
}

/// A player's standing with a village.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Standing {
    pub value: i32,
    /// The player's name (for lists).
    pub name: String,
    /// The day grudges were last let fade.
    pub day: i64,
    /// Small favours counted today.
    pub favour_day: i64,
    pub favours: u8,
    /// When it last hit someone of the village.
    pub hit_at: i64,
    /// The day after the one the village last shared its food with them (0: never).
    pub share_day: i64,
}

impl Standing {
    pub fn rank(&self) -> Rank {
        Rank::of(self.value)
    }

    /// Deed `d` done at `now`: how far the standing moved (small favours count a few times a
    /// day, a flurry of blows once).
    pub fn deed(&mut self, d: Deed, now: i64) -> i32 {
        let day = now.div_euclid(DAY);
        match d {
            Deed::Traded | Deed::Tended => {
                if self.favour_day != day {
                    self.favour_day = day;
                    self.favours = 0;
                }
                if self.favours >= FAVOURS_A_DAY {
                    return 0;
                }
                self.favours += 1;
            }
            Deed::Hit => {
                if self.hit_at != 0 && now >= self.hit_at && now - self.hit_at < HIT_REST {
                    return 0;
                }
                self.hit_at = now.max(1);
            }
            _ => {}
        }
        self.change(d.worth())
    }

    /// Moves it by `by` (kept within bounds): how far it moved.
    pub fn change(&mut self, by: i32) -> i32 {
        let before = self.value;
        self.value = (self.value + by).clamp(MIN, MAX);
        self.value - before
    }

    /// Grudges fade: a point a day below zero.
    pub fn drift(&mut self, day: i64) {
        let days = if self.day == 0 { 0 } else { (day - self.day).clamp(0, 200) as i32 };
        self.day = day;
        if self.value < 0 {
            self.value = (self.value + days).min(0);
        }
    }
}

/// How two villages get on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bond {
    Feud,
    Cool,
    Neighbours,
    Friends,
    Sworn,
}

impl Bond {
    pub fn of(v: i32) -> Bond {
        match v {
            i32::MIN..=-40 => Bond::Feud,
            -39..=-10 => Bond::Cool,
            -9..=29 => Bond::Neighbours,
            30..=59 => Bond::Friends,
            _ => Bond::Sworn,
        }
    }

    /// "We are ...": how a villager puts it.
    pub fn phrase(self, other: &str) -> String {
        match self {
            Bond::Feud => format!("at feud with {other}"),
            Bond::Cool => format!("on cool terms with {other}"),
            Bond::Neighbours => format!("neighbours with {other}, no more than that"),
            Bond::Friends => format!("good friends with {other}"),
            Bond::Sworn => format!("sworn friends with {other}"),
        }
    }
}

/// What warms and cools a tie.
pub const CARAVAN: i32 = 3;
pub const WORD: i32 = 10;
pub const AID: i32 = 8;
pub const QUARREL: i32 = -12;

/// The tie between two villages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tie {
    pub value: i32,
    /// The day its feud was last let cool.
    pub day: i64,
}

impl Tie {
    pub fn bond(&self) -> Bond {
        Bond::of(self.value)
    }

    pub fn change(&mut self, by: i32) -> i32 {
        let before = self.value;
        self.value = (self.value + by).clamp(MIN, MAX);
        self.value - before
    }

    /// Bad blood cools: a point every two days below zero.
    pub fn drift(&mut self, day: i64) {
        if self.day == 0 || day < self.day {
            self.day = day;
            return;
        }
        let steps = ((day - self.day) / 2).min(100);
        self.day += steps * 2;
        if self.value < 0 {
            self.value = (self.value + steps as i32).min(0);
        }
    }
}

/// The name a pair of villages is kept under (either way round).
pub fn key(a: &str, b: &str) -> String {
    if a <= b { format!("{a}|{b}") } else { format!("{b}|{a}") }
}

/// The other village of pair `key`, if `me` is one of them.
pub fn other<'a>(key: &'a str, me: &str) -> Option<&'a str> {
    let (a, b) = key.split_once('|')?;
    if a == me {
        Some(b)
    } else if b == me {
        Some(a)
    } else {
        None
    }
}

/// What a village thinks of a deed that moved a player's standing by `by` in another it has tie
/// `tie` with: friends think alike (half of it); one at feud thinks less of those who help its
/// rival, and does not cheer harm done to it.
pub fn echo(tie: i32, by: i32) -> i32 {
    match Bond::of(tie) {
        Bond::Friends | Bond::Sworn => by / 2,
        Bond::Feud if by > 0 => -by / 2,
        _ => 0,
    }
}

/// Which village a caravan comes from: `ties` are the other villages and the tie with each.
/// Never one at feud; friendlier ones likelier.
pub fn caravan_from(ties: &[(String, i32)], roll: u64) -> Option<&str> {
    let list: Vec<(&str, u64)> = ties.iter().filter(|t| Bond::of(t.1) > Bond::Feud).map(|t| (t.0.as_str(), 1 + (t.1.max(0) as u64) / 20)).collect();
    let total: u64 = list.iter().map(|l| l.1).sum();
    if total == 0 {
        return None;
    }
    let mut at = roll % total;
    for (name, w) in &list {
        if at < *w {
            return Some(*name);
        }
        at -= w;
    }
    None
}

/// How much food a village of `their_people` with `their_food` sends one of `people` short of it
/// (`food`), their tie being `tie`: friends only, and only what they can spare.
pub fn aid(tie: i32, people: usize, food: u64, their_people: usize, their_food: u64) -> Option<u32> {
    if Bond::of(tie) < Bond::Friends || people == 0 || food >= 2 * people as u64 {
        return None;
    }
    let spare = their_food.saturating_sub(4 * their_people as u64);
    let n = (4 * people as u64).min(spare / 2).min(32);
    (n >= 4).then_some(n as u32)
}

/// What neighbours fall out over.
pub const QUARRELS: [&str; 6] = ["the fishing waters", "a strayed flock", "an unpaid debt", "the old boundary stones", "a broken promise", "who cut whose trees"];

pub fn quarrel(roll: u64) -> &'static str {
    QUARRELS[(roll % QUARRELS.len() as u64) as usize]
}

/// Where a village sends a player with its good word: the village it gets on worst with.
pub fn errand_to(ties: &[(String, i32)]) -> Option<&str> {
    ties.iter().min_by_key(|t| t.1).map(|t| t.0.as_str())
}

/// An errand: a village's good word to carry to another.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Errand {
    pub from: String,
    pub to: String,
    pub day: i64,
}

impl Errand {
    pub fn stale(&self, day: i64) -> bool {
        day - self.day > ERRAND_DAYS || day < self.day
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_and_prices() {
        assert_eq!(Rank::of(-100), Rank::Enemy);
        assert_eq!(Rank::of(-60), Rank::Enemy);
        assert_eq!(Rank::of(-59), Rank::Unwelcome);
        assert_eq!(Rank::of(0), Rank::Stranger);
        assert_eq!(Rank::of(20), Rank::Friend);
        assert_eq!(Rank::of(60), Rank::Ally);
        assert!(Rank::Enemy.price().is_none());
        assert!(Rank::Ally.price() < Rank::Friend.price() && Rank::Friend.price() < Rank::Stranger.price());
        assert!(Rank::Unwelcome.price() > Rank::Stranger.price());
        assert!(Rank::Ally.share() > Rank::Friend.share() && Rank::Stranger.share() == 0);
        assert!(Rank::Friend.says("Oakvale", "Sam").contains("Oakvale"));
        // Dear things get cheaper, cheap ones come by the handful.
        assert_eq!(deal(10, 1, 0.8), (8, 1));
        assert_eq!(deal(10, 1, 1.2), (12, 1));
        assert_eq!(deal(1, 8, 0.8), (1, 10));
        assert_eq!(deal(1, 8, 0.9), (1, 9));
        assert_eq!(deal(1, 8, 1.2), (1, 7));
        assert_eq!(deal(1, 1, 0.8), (1, 1));
        assert_eq!(deal(5, 3, 1.0), (5, 3));
    }

    #[test]
    fn deeds_move_standing() {
        let mut s = Standing::default();
        assert_eq!(s.deed(Deed::Brought(20), 100), 2);
        assert_eq!(s.deed(Deed::Brought(1), 100), 1);
        assert_eq!(s.deed(Deed::Brought(999), 100), 6);
        assert_eq!(s.deed(Deed::RaiderSlain, 100), 4);
        assert_eq!(s.value, 13);
        // Small favours count three times a day.
        let got: i32 = (0..10).map(|_| s.deed(Deed::Traded, 200)).sum();
        assert_eq!(got, 3);
        assert_eq!(s.deed(Deed::Tended, 300), 0);
        assert_eq!(s.deed(Deed::Tended, DAY + 300), 2);
        // A flurry of blows is one blow; later ones count again.
        let mut h = Standing::default();
        assert_eq!(h.deed(Deed::Hit, 1000), -4);
        assert_eq!(h.deed(Deed::Hit, 1010), 0);
        assert_eq!(h.deed(Deed::Hit, 1100), -4);
        // A killing makes an enemy of a stranger in two.
        assert_eq!(h.deed(Deed::Killed, 2000), -40);
        assert_eq!(h.rank(), Rank::Unwelcome);
        h.deed(Deed::Killed, 3000);
        assert_eq!(h.rank(), Rank::Enemy);
        h.deed(Deed::Killed, 4000);
        assert_eq!(h.deed(Deed::Killed, 5000), 0, "no lower than the lowest");
        assert_eq!(h.value, MIN);
    }

    #[test]
    fn grudges_fade() {
        let mut s = Standing { value: -30, ..Standing::default() };
        s.drift(10);
        assert_eq!(s.value, -30, "the first day only starts the count");
        s.drift(15);
        assert_eq!(s.value, -25);
        s.drift(100);
        assert_eq!(s.value, 0);
        let mut f = Standing { value: 40, day: 3, ..Standing::default() };
        f.drift(50);
        assert_eq!(f.value, 40, "a good name keeps");
    }

    #[test]
    fn ties_and_echoes() {
        assert_eq!(key("Oak", "Ash"), "Ash|Oak");
        assert_eq!(key("Ash", "Oak"), "Ash|Oak");
        assert_eq!(other("Ash|Oak", "Oak"), Some("Ash"));
        assert_eq!(other("Ash|Oak", "Elm"), None);
        assert_eq!(Bond::of(-40), Bond::Feud);
        assert_eq!(Bond::of(0), Bond::Neighbours);
        assert_eq!(Bond::of(30), Bond::Friends);
        assert_eq!(Bond::of(80), Bond::Sworn);
        // Friends think alike, a rival the opposite of help only.
        assert_eq!(echo(40, 6), 3);
        assert_eq!(echo(40, -40), -20);
        assert_eq!(echo(-50, 6), -3);
        assert_eq!(echo(-50, -40), 0);
        assert_eq!(echo(0, 6), 0);
        let mut t = Tie { value: -41, day: 0 };
        t.drift(10);
        t.drift(14);
        assert_eq!(t.value, -39);
        assert_eq!(t.bond(), Bond::Cool);
        t.drift(15);
        t.drift(16);
        assert_eq!(t.value, -38, "odd days are not lost");
        assert_eq!(t.change(500), 138);
        t.drift(400);
        assert_eq!(t.value, MAX);
    }

    #[test]
    fn caravans_aid_and_errands() {
        let ties = vec![("Ash".to_string(), -60), ("Elm".to_string(), 0), ("Oak".to_string(), 80)];
        let from: Vec<&str> = (0..600u64).filter_map(|r| caravan_from(&ties, r)).collect();
        assert_eq!(from.len(), 600);
        assert!(!from.contains(&"Ash"), "none from a village at feud");
        let oak = from.iter().filter(|n| **n == "Oak").count();
        assert!(oak > 400, "{oak}");
        assert_eq!(caravan_from(&ties[..1], 5), None);
        assert_eq!(caravan_from(&[], 5), None);
        assert_eq!(errand_to(&ties), Some("Ash"));
        assert_eq!(errand_to(&[]), None);
        // Friends with food to spare send some to the hungry.
        assert_eq!(aid(40, 5, 2, 6, 100), Some(20));
        assert_eq!(aid(40, 5, 2, 6, 30), None, "they have little themselves");
        assert_eq!(aid(40, 5, 20, 6, 100), None, "not hungry");
        assert_eq!(aid(10, 5, 2, 6, 100), None, "not friends");
        assert_eq!(aid(90, 20, 0, 6, 1000), Some(32));
        assert!(Errand { from: "Ash".into(), to: "Oak".into(), day: 3 }.stale(11));
        assert!(!Errand { from: "Ash".into(), to: "Oak".into(), day: 3 }.stale(10));
        assert!(QUARRELS.contains(&quarrel(77)));
    }
}
