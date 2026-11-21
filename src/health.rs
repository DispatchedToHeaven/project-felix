//! An NPC's health beyond its hit points: its wounds (bleeding until tended, healing slowly,
//! quicker in bed and tended well), infections a neglected wound may take, sicknesses caught
//! sleeping rough or soaked, and the scars left. No mod API types here.
//!
//! A wound of 3 points or more may bleed (6 or more always does): a point an hour until tended or
//! it stops by itself. Wounds heal half a point an hour, twice that in bed, more when tended well.
//! A wound left untended for four hours may turn infected; an infection, like a sickness, grows each
//! day while the body fights it off; whichever gets there first wins, and losing is death. Tending
//! (by a healer, or a player) slows it, and rest in bed helps the body: a cough passes by itself,
//! the flu with rest, a fever or an infection only with someone's care, and the worn out are in
//! danger from them all.

use crate::serde_json::{Value, json};

pub const HOUR: i64 = 1000;
pub const DAY: i64 = 24_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Wound {
    pub hp: f32,
    /// What made it: a blow, a bite or a cut (`attack`), or a fall, a burn (`bruise`).
    pub kind: String,
    pub bleeding: bool,
    /// How well it was tended (0: not yet).
    pub tended: f32,
    pub at: i64,
    /// Infected: how far the infection got (0..1), and how far the body (0..1).
    pub infection: Option<(f32, f32)>,
    /// Whether it was looked at for infection yet.
    pub checked: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sickness {
    pub name: String,
    pub severity: f32,
    pub immunity: f32,
    /// When and how well it was last tended.
    pub tended_at: i64,
    pub quality: f32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Health {
    pub wounds: Vec<Wound>,
    pub sick: Option<Sickness>,
    pub scars: u8,
    pub last: i64,
}

/// What a stretch of time did.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Change {
    /// Points to give back (healing) and to take (bleeding).
    pub heal: f32,
    pub bleed: f32,
    /// What it died of, if it did.
    pub died_of: Option<String>,
    pub infected: bool,
    pub scarred: bool,
    pub recovered: Option<String>,
}

/// A new wound of `hp` points; `attack`: something hit it.
pub fn wound(hp: f32, attack: bool, roll: f32, now: i64) -> Wound {
    let bleeding = hp >= 6.0 || (hp >= 3.0 && attack && roll < 0.6);
    Wound { hp, kind: if attack { "wound" } else { "bruise" }.into(), bleeding, tended: 0.0, at: now, infection: None, checked: false }
}

/// How well something tends: a medicine's strength (none: 0.3), and the healer's skill.
pub fn potency(item: Option<&str>) -> f32 {
    match item.map(|i| i.trim_start_matches("minecraft:")) {
        Some("golden_apple") | Some("enchanted_golden_apple") => 1.6,
        Some("honey_bottle") => 1.0,
        Some("glow_berries") | Some("kelp") | Some("dried_kelp") => 0.6,
        Some("sweet_berries") | Some("apple") => 0.5,
        _ => 0.3,
    }
}

/// What a healer can tend with, best first.
pub const MEDICINE: &[&str] = &["minecraft:golden_apple", "minecraft:honey_bottle", "minecraft:glow_berries", "minecraft:sweet_berries", "minecraft:dried_kelp"];

pub fn quality(potency: f32, skill: u8) -> f32 {
    (potency * (0.3 + 0.045 * skill.min(20) as f32)).clamp(0.1, 1.5)
}

/// How fast a sickness grows a day when nobody tends it: a cough is mild, a fever hard.
pub fn strength(name: &str) -> f32 {
    if name.contains("cough") {
        0.4
    } else if name.contains("fever") {
        0.6
    } else {
        0.5
    }
}

/// How fast an infection grows a day in a wound tended this well (0: not at all).
pub fn festers(tended: f32) -> f32 {
    0.7 * (1.0 - 0.55 * tended.clamp(0.0, 1.0))
}

/// The chance it falls sick over `dt` ticks: one in seventy a day, twice that sleeping rough, twice
/// again soaked.
pub fn catch_chance(dt: i64, rough: bool, soaked: bool) -> f32 {
    let per_day = (1.0 / 70.0) * if rough { 2.0 } else { 1.0 } * if soaked { 2.0 } else { 1.0 };
    per_day * dt as f32 / DAY as f32
}

impl Health {
    pub fn add_wound(&mut self, w: Wound) {
        self.wounds.push(w);
        // A body has only so many: the smallest close up.
        if self.wounds.len() > 8 {
            self.wounds.sort_by(|a, b| b.hp.total_cmp(&a.hp));
            self.wounds.truncate(8);
        }
    }

    pub fn fall_sick(&mut self, name: &str) {
        if self.sick.is_none() {
            self.sick = Some(Sickness { name: name.into(), severity: 0.05, immunity: 0.0, tended_at: i64::MIN / 2, quality: 0.0 });
        }
    }

    /// Whether a healer should see to it: an untended wound, or a sickness or infection not tended
    /// in half a day.
    pub fn needs_tending(&self, now: i64) -> bool {
        self.wounds.iter().any(|w| w.tended == 0.0) || self.sick.as_ref().is_some_and(|s| now - s.tended_at >= DAY / 2 || now < s.tended_at)
    }

    /// Tended with `quality`: its wounds stop bleeding, its sickness slows.
    pub fn tend(&mut self, quality: f32, now: i64) {
        for w in &mut self.wounds {
            if w.tended == 0.0 || w.infection.is_some() {
                w.tended = w.tended.max(quality);
                w.bleeding = false;
            }
        }
        if let Some(s) = &mut self.sick {
            s.tended_at = now;
            s.quality = quality;
        }
    }

    /// How much it hurts (0..1).
    pub fn pain(&self) -> f32 {
        let wounds: f32 = self.wounds.iter().map(|w| w.hp * 0.025 + w.infection.map_or(0.0, |i| i.0 * 0.3)).sum();
        let sick = self.sick.as_ref().map_or(0.0, |s| s.severity * 0.4);
        (wounds + sick + self.scars as f32 * 0.02).min(1.0)
    }

    /// Too ill or hurt to be up: it keeps to its bed.
    pub fn bedridden(&self) -> bool {
        self.sick.as_ref().is_some_and(|s| s.severity > 0.35) || self.wounds.iter().any(|w| w.bleeding || w.infection.is_some_and(|i| i.0 > 0.35)) || self.pain() > 0.45
    }

    /// Brings it to `now`: `in_bed` while it sleeps or keeps to its bed, `rested` how rested it is (0..100), and
    /// `roll` for chances.
    pub fn advance(&mut self, now: i64, in_bed: bool, rested: f32, roll: &mut dyn FnMut() -> f32) -> Change {
        let mut c = Change::default();
        if self.last == 0 || now < self.last {
            self.last = now;
            return c;
        }
        let dt = (now - self.last).min(DAY);
        self.last = now;
        let hours = dt as f32 / HOUR as f32;
        let days = dt as f32 / DAY as f32;
        let body = if in_bed { 1.3 } else { 1.0 } * if rested < 20.0 { 0.8 } else { 1.0 };
        // Bleeding: a point an hour each, two at most; small ones stop by themselves in three hours.
        let mut bleed = 0.0;
        for w in &mut self.wounds {
            if w.bleeding {
                bleed += hours;
                if w.hp < 6.0 && now - w.at >= 3 * HOUR {
                    w.bleeding = false;
                }
            }
        }
        c.bleed = bleed.min(2.0 * hours);
        // Infection: an untended wound looked at after four hours.
        for w in &mut self.wounds {
            if !w.checked && now - w.at >= 4 * HOUR {
                w.checked = true;
                let chance = if w.kind == "wound" { 0.25 } else { 0.1 } * (1.0 - (w.tended / 1.2).min(0.9));
                if roll() < chance {
                    w.infection = Some((0.05, 0.0));
                    c.infected = true;
                }
            }
            if let Some((sev, imm)) = &mut w.infection {
                *sev += days * festers(w.tended);
                *imm += days * 0.6 * body;
                if *sev >= 1.0 {
                    c.died_of = Some("an infection".into());
                }
            }
        }
        let cleared = self.wounds.iter().any(|w| w.infection.is_some_and(|(s, i)| i >= s && i >= 0.5));
        for w in &mut self.wounds {
            if w.infection.is_some_and(|(s, i)| i >= s && i >= 0.5) {
                w.infection = None;
            }
        }
        if cleared {
            c.recovered = Some("infection".into());
        }
        // Healing: half a point an hour, twice in bed, more tended well; an infected wound doesn't.
        let mut heal = 0.5 * hours * if in_bed { 2.0 } else { 1.0 };
        for w in self.wounds.iter_mut().filter(|w| w.infection.is_none() && !w.bleeding) {
            if heal <= 0.0 {
                break;
            }
            let rate = 1.0 + w.tended;
            let take = (heal * rate).min(w.hp);
            w.hp -= take;
            c.heal += take;
            heal -= take / rate;
        }
        let before = self.wounds.len();
        self.wounds.retain(|w| w.hp > 0.01);
        // A wound that closed may leave a scar.
        if before > self.wounds.len() && roll() < 0.04 {
            self.scars = self.scars.saturating_add(1).min(5);
            c.scarred = true;
        }
        // Sickness: it grows, the body fights back; tending in the last half day slows it.
        if let Some(s) = &mut self.sick {
            let tended = now - s.tended_at < DAY / 2 && now >= s.tended_at;
            s.severity += days * strength(&s.name) * if tended { 0.75 - 0.35 * s.quality.min(1.0) } else { 1.0 };
            s.immunity += days * 0.5 * body;
            if s.severity >= 1.0 {
                c.died_of = Some(s.name.clone());
            } else if s.immunity >= s.severity && s.immunity >= 0.4 {
                c.recovered = Some(s.name.clone());
                self.sick = None;
            }
        }
        c
    }

    /// What it says of itself.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        let n = self.wounds.len();
        if n > 0 {
            let bleeding = self.wounds.iter().filter(|w| w.bleeding).count();
            let infected = self.wounds.iter().filter(|w| w.infection.is_some()).count();
            let tended = self.wounds.iter().filter(|w| w.tended > 0.0).count();
            parts.push(format!("{n} wound{}", if n == 1 { "" } else { "s" }));
            if bleeding > 0 {
                parts.push("I'm bleeding".into());
            }
            if infected > 0 {
                parts.push("one's gone bad, it's hot and swollen".into());
            }
            if tended == n {
                parts.push("all bandaged".into());
            }
        }
        if let Some(s) = &self.sick {
            parts.push(format!("I've got {}{}", s.name, if s.severity > 0.6 { ", and it's bad" } else { "" }));
        }
        if self.scars > 0 {
            parts.push(format!("{} old scar{}", self.scars, if self.scars == 1 { "" } else { "s" }));
        }
        if parts.is_empty() { "I'm in good health, thank you.".into() } else { format!("{}.", parts.join("; ")) }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "wounds": self.wounds.iter().map(|w| json!({ "hp": w.hp, "kind": w.kind, "bleeding": w.bleeding, "tended": w.tended, "at": w.at,
                "infection": w.infection.map(|(s, i)| json!([s, i])), "checked": w.checked })).collect::<Vec<_>>(),
            "sick": self.sick.as_ref().map(|s| json!({ "name": s.name, "severity": s.severity, "immunity": s.immunity, "tended_at": s.tended_at, "quality": s.quality })),
            "scars": self.scars,
            "last": self.last,
        })
    }

    pub fn from_json(v: &Value) -> Health {
        let f = |x: &Value, k: &str| x.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let wounds = v
            .get("wounds")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .take(8)
                    .map(|w| Wound {
                        hp: f(w, "hp").clamp(0.0, 100.0),
                        kind: w.get("kind").and_then(Value::as_str).unwrap_or("wound").into(),
                        bleeding: w.get("bleeding").and_then(Value::as_bool).unwrap_or(false),
                        tended: f(w, "tended").clamp(0.0, 1.5),
                        at: w.get("at").and_then(Value::as_i64).unwrap_or(0),
                        infection: w.get("infection").and_then(Value::as_array).and_then(|i| Some((i.first()?.as_f64()? as f32, i.get(1)?.as_f64()? as f32))),
                        checked: w.get("checked").and_then(Value::as_bool).unwrap_or(true),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let sick = v.get("sick").filter(|s| !s.is_null()).map(|s| Sickness {
            name: s.get("name").and_then(Value::as_str).unwrap_or("a fever").into(),
            severity: f(s, "severity"),
            immunity: f(s, "immunity"),
            tended_at: s.get("tended_at").and_then(Value::as_i64).unwrap_or(i64::MIN / 2),
            quality: f(s, "quality"),
        });
        Health { wounds, sick, scars: v.get("scars").and_then(Value::as_u64).unwrap_or(0).min(5) as u8, last: v.get("last").and_then(Value::as_i64).unwrap_or(0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rolls(v: f32) -> impl FnMut() -> f32 {
        move || v
    }

    #[test]
    fn wounds_bleed_heal_and_close() {
        let mut h = Health { last: 1, ..Health::default() };
        h.add_wound(wound(7.0, true, 0.9, 1));
        assert!(h.wounds[0].bleeding && h.bedridden() && h.needs_tending(1));
        let c = h.advance(1 + HOUR, false, 80.0, &mut rolls(0.99));
        assert!(c.bleed > 0.9 && c.heal == 0.0, "{c:?}");
        h.tend(0.8, 1 + HOUR);
        assert!(!h.wounds[0].bleeding && !h.needs_tending(1 + HOUR));
        let mut healed = 0.0;
        for i in 2..12 {
            healed += h.advance(1 + i * HOUR, true, 80.0, &mut rolls(0.99)).heal;
        }
        assert!((healed - 7.0).abs() < 0.01 && h.wounds.is_empty(), "{healed} {h:?}");
    }

    #[test]
    fn neglect_infects_and_kills() {
        let mut h = Health { last: 1, ..Health::default() };
        h.add_wound(wound(4.0, true, 0.9, 1));
        let c = h.advance(1 + 5 * HOUR, false, 80.0, &mut rolls(0.0));
        assert!(c.infected && h.wounds[0].infection.is_some());
        let mut died = None;
        for d in 1..4 {
            died = died.or(h.advance(1 + 5 * HOUR + d * DAY, false, 10.0, &mut rolls(0.99)).died_of);
        }
        assert_eq!(died.as_deref(), Some("an infection"));
        // Tended and in bed, the body wins.
        let mut t = Health { last: 1, ..Health::default() };
        t.add_wound(wound(4.0, true, 0.9, 1));
        t.advance(1 + 5 * HOUR, false, 80.0, &mut rolls(0.0));
        t.tend(1.0, 1 + 5 * HOUR);
        let mut rec = None;
        for d in 1..4 {
            let c = t.advance(1 + 5 * HOUR + d * DAY, true, 80.0, &mut rolls(0.99));
            assert!(c.died_of.is_none());
            rec = rec.or(c.recovered);
        }
        assert_eq!(rec.as_deref(), Some("infection"));
    }

    /// Lives through `days` of sickness `name` (or of an infected wound, `name` empty) step by
    /// step as the game does: to bed when it is bad, tended twice a day with `care` if any.
    fn lives(name: &str, care: Option<f32>, rested: f32) -> (bool, i64) {
        let mut h = Health { last: 1, ..Health::default() };
        if name.is_empty() {
            h.add_wound(wound(4.0, true, 0.9, 1));
            h.wounds[0].infection = Some((0.05, 0.0));
            h.wounds[0].checked = true;
        } else {
            h.fall_sick(name);
        }
        for step in 1..=(6 * DAY / 100) {
            let now = 1 + step * 100;
            if let Some(q) = care
                && h.needs_tending(now)
            {
                h.tend(q, now);
            }
            let c = h.advance(now, h.bedridden(), rested, &mut rolls(0.99));
            if c.died_of.is_some() {
                return (false, now);
            }
            if c.recovered.is_some() {
                return (true, now);
            }
        }
        panic!("{name}: neither well nor dead: {h:?}");
    }

    #[test]
    fn rest_and_care_decide() {
        // A cough passes by itself, even worn out; the flu with rest.
        assert!(lives("a cough", None, 80.0).0);
        assert!(lives("a bad cough", None, 10.0).0);
        assert!(lives("the flu", None, 80.0).0);
        assert!(!lives("the flu", None, 10.0).0, "worn out, nobody cares for it");
        // A fever needs someone: even poor care (no medicine, no skill) saves the rested.
        assert!(!lives("a fever", None, 80.0).0);
        assert!(lives("a fever", Some(0.1), 80.0).0);
        assert!(lives("a fever", Some(0.8), 10.0).0, "good care saves the worn out too");
        // So does an infected wound.
        assert!(!lives("", None, 80.0).0);
        assert!(lives("", Some(0.1), 80.0).0);
        assert!(lives("", Some(1.0), 10.0).0);
        // Nobody is ill for long: a day or two either way.
        assert!(lives("the flu", None, 80.0).1 < 2 * DAY);
        assert!(lives("a fever", None, 80.0).1 > DAY, "there is a day to help");
    }

    #[test]
    fn sickness_and_json() {
        let mut h = Health { last: 1, ..Health::default() };
        h.fall_sick("the flu");
        let mut out = None;
        for d in 1..5 {
            h.tend(0.8, 1 + d * DAY - 10);
            let c = h.advance(1 + d * DAY, true, 80.0, &mut rolls(0.99));
            assert!(c.died_of.is_none(), "{h:?}");
            out = out.or(c.recovered);
        }
        assert_eq!(out.as_deref(), Some("the flu"));
        assert!(catch_chance(DAY, true, true) > catch_chance(DAY, false, false) * 3.0);
        assert!(quality(potency(Some("minecraft:honey_bottle")), 10) > quality(potency(None), 10));
        let mut j = Health { last: 9, scars: 2, ..Health::default() };
        j.add_wound(wound(5.0, true, 0.1, 3));
        j.fall_sick("a fever");
        assert_eq!(Health::from_json(&j.to_json()), j);
        assert!(j.describe().contains("fever"));
    }
}
