//! Ball games with an NPC: table tennis, tennis and basketball, played with two or three keys.
//! No mod API types here.
//!
//! Table tennis and tennis are rallies: the ball flies to the player, a prompt says which key
//! returns it (left, right, or smash for a high ball in tennis), and a press in time sends it
//! back. The NPC returns it too, missing more as a rally grows long; rallies speed up. First to
//! five points wins. Basketball is a shooting duel: the player stops a swinging meter with the
//! shoot key (the closer to the top, the surer the shot), the NPC shoots on its own; five shots
//! each, two points a basket, then shot for shot until someone leads.
//!
//! Places are in blocks in the NPC's frame: `x` to the NPC's right, `y` up, `z` forwards (towards
//! the player). Times are game ticks.

use crate::serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sport {
    TableTennis,
    Tennis,
    Basketball,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Player,
    Npc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Smash,
}

impl Sport {
    pub fn name(self) -> &'static str {
        match self {
            Sport::TableTennis => "table_tennis",
            Sport::Tennis => "tennis",
            Sport::Basketball => "basketball",
        }
    }
    pub fn parse(s: &str) -> Option<Sport> {
        match s {
            "table_tennis" | "ping_pong" | "pingpong" => Some(Sport::TableTennis),
            "tennis" => Some(Sport::Tennis),
            "basketball" => Some(Sport::Basketball),
            _ => None,
        }
    }
    /// Where the player stands, ahead of the NPC.
    pub fn distance(self) -> f32 {
        match self {
            Sport::TableTennis => 3.3,
            Sport::Tennis => 9.0,
            Sport::Basketball => 5.0,
        }
    }
    fn flight(self, hits: u32) -> i64 {
        let (start, fastest) = match self {
            Sport::TableTennis => (16, 9),
            Sport::Tennis => (26, 15),
            Sport::Basketball => (20, 20),
        };
        (start - hits as i64).max(fastest)
    }
    /// Ticks either side of the ball's arrival a press counts.
    pub fn window(self) -> i64 {
        match self {
            Sport::TableTennis => 4,
            _ => 5,
        }
    }
    fn hit_height(self) -> f32 {
        if self == Sport::TableTennis { 1.0 } else { 1.15 }
    }
    fn arc(self, high: bool) -> f32 {
        match (self, high) {
            (Sport::TableTennis, _) => 0.35,
            (_, true) => 3.5,
            _ => 1.2,
        }
    }
    /// The basketball hoop's rim.
    pub fn rim() -> [f32; 3] {
        [2.6, 2.6, 2.5]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// `by` is about to serve (the player serves with a key, or after a while on its own).
    Serve { by: Side, at: i64 },
    /// The ball flies from `from`; towards the player `key` is the key that returns it, towards
    /// the NPC the key the player returned it with (which side it comes).
    Flight { from: Side, launched: i64, arrives: i64, key: Key },
    /// A point was won (or, in basketball, a shot ended); play goes on after a pause.
    Point { winner: Option<Side>, at: i64 },
    /// Basketball: the player's meter swings, from `at`.
    Aim { at: i64 },
    /// Basketball: a shot in the air.
    Shot { by: Side, launched: i64, arrives: i64, made: bool },
    Over { winner: Option<Side>, at: i64 },
}

/// What happened, for the server to show and sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Swing(Side),
    Point(Option<Side>),
    Basket(Side),
    Miss(Side),
    Over(Option<Side>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub sport: Sport,
    /// Points: the player's, then the NPC's.
    pub score: [u32; 2],
    pub phase: Phase,
    /// Returns in the rally so far.
    pub hits: u32,
    /// Basketball: shots taken, both sides.
    pub shots: u32,
    pub rng: u64,
    /// How well the NPC plays (0 to 2, see `bond::skill`).
    pub skill: u8,
}

const TARGET: u32 = 5;
const PAUSE: i64 = 30;
const METER: i64 = 40;

fn idx(s: Side) -> usize {
    if s == Side::Player { 0 } else { 1 }
}

impl Match {
    pub fn new(sport: Sport, now: i64, seed: u64) -> Match {
        let phase = if sport == Sport::Basketball { Phase::Aim { at: now + PAUSE } } else { Phase::Serve { by: Side::Npc, at: now } };
        Match { sport, score: [0, 0], phase, hits: 0, shots: 0, rng: seed | 1, skill: 1 }
    }

    fn random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 40) as f32 / (1u64 << 24) as f32
    }

    fn incoming_key(&mut self) -> Key {
        let r = self.random();
        if self.sport == Sport::Tennis && r < 0.22 {
            Key::Smash
        } else if r < 0.61 {
            Key::Left
        } else {
            Key::Right
        }
    }

    fn launch(&mut self, from: Side, now: i64, key: Key) -> Event {
        let arrives = now + self.sport.flight(self.hits);
        self.phase = Phase::Flight { from, launched: now, arrives, key };
        Event::Swing(from)
    }

    fn point(&mut self, winner: Side, now: i64) -> Event {
        self.score[idx(winner)] += 1;
        self.phase = Phase::Point { winner: Some(winner), at: now };
        self.hits = 0;
        Event::Point(Some(winner))
    }

    /// The meter's reading at `now`, 0 to 1 and back, while the player aims.
    pub fn meter(&self, now: i64) -> f32 {
        let Phase::Aim { at } = self.phase else { return 0.0 };
        let t = (now - at).rem_euclid(METER) as f32 / METER as f32;
        1.0 - (2.0 * t - 1.0).abs()
    }

    /// A key the player pressed at `now`.
    pub fn press(&mut self, key: Key, now: i64) -> Vec<Event> {
        match self.phase {
            Phase::Serve { by: Side::Player, at } if now >= at => {
                let key = if key == Key::Smash { Key::Left } else { key };
                vec![self.launch(Side::Player, now, key)]
            }
            Phase::Flight { from: Side::Npc, arrives, key: wanted, .. } => {
                let w = self.sport.window();
                if now < arrives - w {
                    return Vec::new(); // too early: nothing yet
                }
                if now <= arrives + w && key == wanted {
                    self.hits += 1;
                    let back = if key == Key::Smash { Key::Left } else { key };
                    vec![self.launch(Side::Player, now, back)]
                } else {
                    vec![self.point(Side::Npc, now)]
                }
            }
            Phase::Aim { at } if now >= at && key == Key::Smash => {
                let m = self.meter(now);
                let chance = if m >= 0.85 { 0.92 } else { m * 0.6 };
                let made = self.random() < chance;
                self.phase = Phase::Shot { by: Side::Player, launched: now, arrives: now + self.sport.flight(0), made };
                vec![Event::Swing(Side::Player)]
            }
            _ => Vec::new(),
        }
    }

    /// The player's side played by another NPC (before `tick`): it serves, returns as the NPC
    /// does, and shoots without the meter.
    pub fn auto(&mut self, now: i64) -> Vec<Event> {
        match self.phase {
            Phase::Serve { by: Side::Player, at } if now >= at + PAUSE => {
                let key = match self.incoming_key() {
                    Key::Smash => Key::Left,
                    k => k,
                };
                vec![self.launch(Side::Player, now, key)]
            }
            Phase::Flight { from: Side::Npc, arrives, key, .. } if now >= arrives => {
                let returns = (0.9 - 0.035 * self.hits.saturating_sub(3) as f32).max(0.45);
                if self.random() < returns {
                    self.hits += 1;
                    let back = if key == Key::Smash { Key::Left } else { key };
                    vec![self.launch(Side::Player, now, back)]
                } else {
                    vec![self.point(Side::Npc, now)]
                }
            }
            Phase::Aim { at } if now >= at + 20 => {
                let made = self.random() < 0.55;
                self.phase = Phase::Shot { by: Side::Player, launched: now, arrives: now + self.sport.flight(0), made };
                vec![Event::Swing(Side::Player)]
            }
            _ => Vec::new(),
        }
    }

    /// The game going on by itself: serves, the NPC's returns and shots, missed balls, pauses.
    pub fn tick(&mut self, now: i64) -> Vec<Event> {
        match self.phase {
            Phase::Serve { by: Side::Npc, at } if now >= at + PAUSE => {
                let key = self.incoming_key();
                vec![self.launch(Side::Npc, now, key)]
            }
            Phase::Serve { by: Side::Player, at } if now >= at + 100 => {
                vec![self.launch(Side::Player, now, Key::Left)]
            }
            Phase::Flight { from: Side::Npc, arrives, .. } if now > arrives + self.sport.window() => {
                vec![self.point(Side::Npc, now)]
            }
            Phase::Flight { from: Side::Player, arrives, .. } if now >= arrives => {
                let base = [0.86, 0.93, 0.97][usize::from(self.skill.min(2))];
                let returns = (base - 0.035 * self.hits.saturating_sub(3) as f32).max(0.45);
                if self.random() < returns {
                    self.hits += 1;
                    let key = self.incoming_key();
                    vec![self.launch(Side::Npc, now, key)]
                } else {
                    vec![self.point(Side::Player, now)]
                }
            }
            Phase::Shot { by, arrives, made, .. } if now >= arrives => {
                self.shots += 1;
                if made {
                    self.score[idx(by)] += 2;
                }
                self.phase = Phase::Point { winner: made.then_some(by), at: now };
                vec![if made { Event::Basket(by) } else { Event::Miss(by) }]
            }
            Phase::Aim { at } if now >= at + METER * 4 => {
                // Too slow to shoot: the shot is lost.
                self.shots += 1;
                self.phase = Phase::Point { winner: None, at: now };
                vec![Event::Miss(Side::Player)]
            }
            Phase::Point { winner, at } if now >= at + PAUSE => {
                if let Some(w) = self.winner() {
                    self.phase = Phase::Over { winner: w, at: now };
                    return vec![Event::Over(w)];
                }
                if self.sport == Sport::Basketball {
                    // The NPC shoots after the player's shot, the player after the NPC's.
                    if self.shots % 2 == 1 {
                        let made = self.random() < [0.42, 0.55, 0.66][usize::from(self.skill.min(2))];
                        self.phase = Phase::Shot { by: Side::Npc, launched: now + 15, arrives: now + 15 + self.sport.flight(0), made };
                        return vec![Event::Swing(Side::Npc)];
                    }
                    self.phase = Phase::Aim { at: now };
                    return Vec::new();
                }
                // Whoever won the point serves.
                let by = winner.unwrap_or(Side::Npc);
                self.phase = Phase::Serve { by, at: now };
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// The winner once the game is decided (`Some(None)` would be a draw; none happen).
    fn winner(&self) -> Option<Option<Side>> {
        let [p, n] = self.score;
        if self.sport == Sport::Basketball {
            if self.shots >= 10 && self.shots % 2 == 0 && p != n {
                return Some(Some(if p > n { Side::Player } else { Side::Npc }));
            }
            return None;
        }
        if p >= TARGET {
            Some(Some(Side::Player))
        } else if n >= TARGET {
            Some(Some(Side::Npc))
        } else {
            None
        }
    }

    pub fn over(&self) -> bool {
        matches!(self.phase, Phase::Over { .. })
    }

    /// The prompt for the player at `now`: the key to press and how far the ball has come (0 to 1).
    pub fn prompt(&self, now: i64) -> Option<(Key, f32)> {
        match self.phase {
            Phase::Flight { from: Side::Npc, launched, arrives, key } => {
                Some((key, ((now - launched) as f32 / (arrives - launched).max(1) as f32).clamp(0.0, 1.5)))
            }
            _ => None,
        }
    }

    /// Where the ball is at `now` (fractional ticks), in the NPC's frame; none when it isn't shown.
    pub fn ball(&self, now: f64) -> Option<[f32; 3]> {
        let s = self.sport;
        let d = s.distance();
        let h = s.hit_height();
        let npc_hand = [0.35, h, 0.3];
        let player_hand = |k: Key| {
            let lat = match k {
                Key::Left => 0.7,
                Key::Right => -0.7,
                Key::Smash => 0.0,
            };
            [lat, if k == Key::Smash { h + 0.9 } else { h }, d - 0.4]
        };
        let arc = |a: [f32; 3], b: [f32; 3], t: f32, height: f32| {
            let mut p = [0.0; 3];
            for i in 0..3 {
                p[i] = a[i] + (b[i] - a[i]) * t;
            }
            p[1] += height * 4.0 * t * (1.0 - t);
            p
        };
        // The player's ball low and to the right of their view, not in front of their eyes.
        let shooter = |by: Side| if by == Side::Npc { [0.3, 1.6, 0.4] } else { [-0.35, 1.15, d - 0.75] };
        match self.phase {
            Phase::Serve { by: Side::Npc, .. } => Some(npc_hand),
            Phase::Serve { by: Side::Player, .. } => Some([-0.35, h - 0.1, d - 0.7]),
            Phase::Flight { from, launched, arrives, key } => {
                let t = ((now - launched as f64) / (arrives - launched).max(1) as f64) as f32;
                let (a, b) = if from == Side::Npc { (npc_hand, player_hand(key)) } else { (player_hand(key), npc_hand) };
                let high = from == Side::Npc && key == Key::Smash;
                // Past the player (a miss) it keeps going a little, then is gone.
                (t <= 1.6).then(|| arc(a, b, t, s.arc(high)))
            }
            Phase::Aim { .. } => Some(shooter(Side::Player)),
            Phase::Shot { by, launched, arrives, made } => {
                let t = ((now - launched as f64) / (arrives - launched).max(1) as f64) as f32;
                if t < 0.0 {
                    return Some(shooter(by));
                }
                let rim = Sport::rim();
                let target = if made { rim } else { [rim[0] - 0.35, rim[1] + 0.15, rim[2] + 0.3] };
                if t <= 1.0 {
                    Some(arc(shooter(by), target, t, 1.8))
                } else {
                    // Through the net, or off the rim, and down.
                    let f = (t - 1.0).min(1.0);
                    let drift = if made { 0.0 } else { -1.2 * f };
                    Some([target[0] + drift, (target[1] - 2.5 * f).max(0.15), target[2] + drift * 0.5])
                }
            }
            Phase::Point { .. } | Phase::Over { .. } => None,
        }
    }

    // ---- as JSON, for the attachment clients read ----

    pub fn to_json(&self) -> Value {
        let side = |s: Side| if s == Side::Player { "player" } else { "npc" };
        let key = |k: Key| match k {
            Key::Left => "left",
            Key::Right => "right",
            Key::Smash => "smash",
        };
        let phase = match self.phase {
            Phase::Serve { by, at } => json!({"p": "serve", "by": side(by), "at": at}),
            Phase::Flight { from, launched, arrives, key: k } => json!({"p": "flight", "from": side(from), "launched": launched, "arrives": arrives, "key": key(k)}),
            Phase::Point { winner, at } => json!({"p": "point", "winner": winner.map(side), "at": at}),
            Phase::Aim { at } => json!({"p": "aim", "at": at}),
            Phase::Shot { by, launched, arrives, made } => json!({"p": "shot", "by": side(by), "launched": launched, "arrives": arrives, "made": made}),
            Phase::Over { winner, at } => json!({"p": "over", "winner": winner.map(side), "at": at}),
        };
        json!({"sport": self.sport.name(), "score": self.score, "phase": phase, "hits": self.hits, "shots": self.shots, "rng": self.rng.to_string(), "skill": self.skill})
    }

    pub fn from_json(v: &Value) -> Option<Match> {
        let side = |v: &Value| match v.as_str()? {
            "player" => Some(Side::Player),
            "npc" => Some(Side::Npc),
            _ => None,
        };
        let key = |v: &Value| match v.as_str()? {
            "left" => Some(Key::Left),
            "right" => Some(Key::Right),
            "smash" => Some(Key::Smash),
            _ => None,
        };
        let p = v.get("phase")?;
        let i = |k: &str| p.get(k).and_then(Value::as_i64);
        let phase = match p.get("p")?.as_str()? {
            "serve" => Phase::Serve { by: side(p.get("by")?)?, at: i("at")? },
            "flight" => Phase::Flight { from: side(p.get("from")?)?, launched: i("launched")?, arrives: i("arrives")?, key: key(p.get("key")?)? },
            "point" => Phase::Point { winner: p.get("winner").and_then(side), at: i("at")? },
            "aim" => Phase::Aim { at: i("at")? },
            "shot" => Phase::Shot { by: side(p.get("by")?)?, launched: i("launched")?, arrives: i("arrives")?, made: p.get("made")?.as_bool()? },
            "over" => Phase::Over { winner: p.get("winner").and_then(side), at: i("at")? },
            _ => return None,
        };
        let score = v.get("score")?.as_array()?;
        Some(Match {
            sport: Sport::parse(v.get("sport")?.as_str()?)?,
            score: [score.first()?.as_u64()? as u32, score.get(1)?.as_u64()? as u32],
            phase,
            hits: v.get("hits")?.as_u64()? as u32,
            shots: v.get("shots")?.as_u64()? as u32,
            rng: v.get("rng")?.as_str()?.parse().ok()?,
            skill: v.get("skill").and_then(Value::as_u64).map_or(1, |s| s.min(2) as u8),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `m` from `now` to `until`, answering every ball with the right key at its arrival.
    fn play_well(m: &mut Match, mut now: i64, until: i64) -> i64 {
        while now < until && !m.over() {
            if let Phase::Flight { from: Side::Npc, arrives, key, .. } = m.phase
                && now == arrives
            {
                m.press(key, now);
            }
            if let Phase::Serve { by: Side::Player, .. } = m.phase {
                m.press(Key::Right, now);
            }
            if let Phase::Aim { .. } = m.phase
                && m.meter(now) > 0.95
            {
                m.press(Key::Smash, now);
            }
            m.tick(now);
            now += 1;
        }
        now
    }

    #[test]
    fn a_good_player_wins_table_tennis() {
        let mut m = Match::new(Sport::TableTennis, 0, 42);
        play_well(&mut m, 0, 200_000);
        assert!(m.over());
        assert_eq!(m.score[0], TARGET);
        assert!(matches!(m.phase, Phase::Over { winner: Some(Side::Player), .. }));
    }

    #[test]
    fn idle_player_loses_tennis_and_rallies_speed_up() {
        let mut m = Match::new(Sport::Tennis, 0, 7);
        let mut now = 0;
        while !m.over() && now < 100_000 {
            m.tick(now);
            now += 1;
        }
        assert_eq!(m.score, [0, TARGET]);
        assert!(Sport::Tennis.flight(20) < Sport::Tennis.flight(0));
    }

    #[test]
    fn wrong_key_or_early_press() {
        let mut m = Match::new(Sport::TableTennis, 0, 3);
        let mut now = 0;
        while !matches!(m.phase, Phase::Flight { .. }) {
            m.tick(now);
            now += 1;
        }
        let Phase::Flight { arrives, key, .. } = m.phase else { unreachable!() };
        assert!(m.press(key, arrives - 10).is_empty()); // too early: ignored
        let wrong = if key == Key::Left { Key::Right } else { Key::Left };
        assert_eq!(m.press(wrong, arrives), vec![Event::Point(Some(Side::Npc))]);
        assert_eq!(m.score, [0, 1]);
    }

    #[test]
    fn basketball_ends_after_five_shots_each() {
        let mut m = Match::new(Sport::Basketball, 0, 11);
        play_well(&mut m, 0, 200_000);
        assert!(m.over());
        assert!(m.shots >= 10 && m.shots % 2 == 0);
        assert_ne!(m.score[0], m.score[1]);
    }

    #[test]
    fn json_round_trip_and_ball_path() {
        let mut m = Match::new(Sport::Tennis, 100, 5);
        m.tick(100 + PAUSE);
        let back = Match::from_json(&m.to_json()).unwrap();
        assert_eq!(back, m);
        let Phase::Flight { launched, arrives, .. } = m.phase else { panic!("{:?}", m.phase) };
        let start = m.ball(launched as f64).unwrap();
        let mid = m.ball((launched + arrives) as f64 / 2.0).unwrap();
        let end = m.ball(arrives as f64).unwrap();
        assert!(start[2] < mid[2] && mid[2] < end[2]);
        assert!(mid[1] > start[1]);
    }
}
