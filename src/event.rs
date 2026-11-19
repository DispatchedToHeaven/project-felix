//! Village events: days in the calendar when the NPCs gather in one place, play a scene, sell
//! for less and give quests of the day. No mod API types here.
//!
//! An event is `data/<namespace>/felix_events/<name>.json`:
//!
//! ```json
//! { "name": "harvest fair", "every": 7, "day": 6, "from": "10:00", "until": "16:00",
//!   "place": "meet", "scene": "projectfelix:fair", "scene_after": 20,
//!   "say": [ "The fair is on!", "To the fair!" ], "animation": "animation.felix.cheer",
//!   "prices": 0.75, "quests": [ "projectfelix:fair_pies" ], "who": "felix_villager" }
//! ```
//!
//! It is on day `day` of every `every` days (counted from the world's first, from 0), from
//! `from` until `until` (the game's clock; past midnight is fine). Every NPC living by a day
//! (with tag `who`, if it names one) leaves what it does for it, goes to the event's spot
//! (`/felix event here`), else its own place `place` (else its `meet`, else its home), says one
//! of `say`, and plays `animation` now and then there. `scene` is played there `scene_after`
//! seconds in, led by the NPC nearest
//! the spot. Its NPCs sell for `prices` times as much, and offer `quests` besides their own.

use crate::serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub name: String,
    pub every: i64,
    pub day: i64,
    /// Day time (0 is 6:00), from and until.
    pub from: i64,
    pub until: i64,
    pub place: String,
    pub scene: Option<String>,
    pub scene_after: f32,
    pub say: Vec<String>,
    pub animation: Option<String>,
    pub prices: f32,
    pub quests: Vec<String>,
    pub who: Option<String>,
}

const DAY: i64 = 24000;

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

pub fn parse(bytes: &[u8]) -> Result<Event, String> {
    let v: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    read(&v)
}

pub fn read(v: &Value) -> Result<Event, String> {
    let m = v.as_object().ok_or("not an object")?;
    let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string);
    let hour = |k: &str, d: &str| -> Result<i64, String> {
        let text = s(k).unwrap_or_else(|| d.to_string());
        crate::routine::clock(&text).ok_or(format!("\"{k}\" is an hour, as \"10:00\", not {text}"))
    };
    let every = m.get("every").and_then(Value::as_i64).unwrap_or(1).clamp(1, 365);
    let day = m.get("day").and_then(Value::as_i64).unwrap_or(0);
    if !(0..every).contains(&day) {
        return Err(format!("\"day\" is from 0 to {} (every - 1)", every - 1));
    }
    let (from, until) = (hour("from", "10:00")?, hour("until", "16:00")?);
    if from == until {
        return Err("\"from\" and \"until\" are the same hour".into());
    }
    Ok(Event {
        name: s("name").unwrap_or_else(|| "festival".into()),
        every,
        day,
        from,
        until,
        place: s("place").unwrap_or_else(|| "meet".into()),
        scene: s("scene"),
        scene_after: m.get("scene_after").and_then(Value::as_f64).unwrap_or(20.0).clamp(0.0, 600.0) as f32,
        say: strings(m.get("say")),
        animation: s("animation"),
        prices: m.get("prices").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.1, 10.0) as f32,
        quests: strings(m.get("quests")),
        who: s("who").map(|w| w.trim_start_matches('#').to_string()),
    })
}

impl Event {
    fn on_day(&self, day: i64) -> bool {
        day.rem_euclid(self.every) == self.day
    }

    /// How long it lasts, in ticks.
    pub fn length(&self) -> i64 {
        (self.until - self.from).rem_euclid(DAY)
    }

    /// Whether it is on at day time `t` (counted from the world's first day).
    pub fn on_at(&self, t: i64) -> bool {
        let (d, tod) = (t.div_euclid(DAY), t.rem_euclid(DAY));
        if self.from < self.until {
            self.on_day(d) && (self.from..self.until).contains(&tod)
        } else {
            // Past midnight: the hours after it belong to the day before.
            (self.on_day(d) && tod >= self.from) || (self.on_day(d - 1) && tod < self.until)
        }
    }

    /// The day time it next starts at, from `t` on.
    pub fn next_start(&self, t: i64) -> i64 {
        let d = t.div_euclid(DAY);
        (0..=self.every + 1).map(|k| (d + k) * DAY + self.from).find(|s| *s >= t && self.on_day(s.div_euclid(DAY))).unwrap_or(t)
    }

    /// Price `base` at it.
    pub fn price(&self, base: u32) -> u32 {
        ((base as f32 * self.prices).round() as u32).max(1)
    }
}

/// `t` (day time) as `day 3, 10:00`.
pub fn when(t: i64) -> String {
    let tod = t.rem_euclid(DAY);
    let mins = (tod * 60 / 1000 + 6 * 60) % (24 * 60);
    format!("day {}, {}:{:02}", t.div_euclid(DAY), mins / 60, mins % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_calendar_holds() {
        let fair = parse(include_bytes!("../data/projectfelix/felix_events/harvest_fair.json")).unwrap();
        assert_eq!((fair.every, fair.day), (7, 6));
        let start = 6 * DAY + crate::routine::clock("10:00").unwrap();
        assert!(fair.on_at(start) && fair.on_at(start + 1000) && !fair.on_at(start - 1));
        assert!(!fair.on_at(start + fair.length()) && !fair.on_at(start + DAY));
        assert!(fair.on_at(start + 7 * DAY));
        assert_eq!(fair.next_start(0), start);
        assert_eq!(fair.next_start(start + 1), start + 7 * DAY);
        assert_eq!(fair.price(4), 3);
        assert_eq!(when(start), "day 6, 10:00");
        let night = parse(br#"{ "from": "22:00", "until": "2:00", "every": 2, "day": 1 }"#).unwrap();
        let t = DAY + crate::routine::clock("23:00").unwrap();
        assert!(night.on_at(t) && night.on_at(t + 2000) && !night.on_at(t + 4000));
        assert_eq!(night.length(), 4000);
        assert!(parse(br#"{ "every": 3, "day": 3 }"#).is_err());
        assert!(parse(br#"{ "from": "9:00", "until": "9:00" }"#).is_err());
    }
}
