//! A day in an NPC's life: from what hour it does what, and where. No mod API types here.
//!
//! A routine is `data/<namespace>/felix_routines/<name>.json`:
//!
//! ```json
//! { "day": [
//!   { "at": "6:00", "do": "work", "say": "Off to work!", "animation": "animation.felix.swing" },
//!   { "at": "12:00", "do": "meet", "say": "Lunch time!" },
//!   { "at": "18:00", "do": "home" },
//!   { "at": "21:00", "do": "sleep", "say": "Good night." } ] }
//! ```
//!
//! `do`: `follow` (goes with its owner), `sleep` (in its `home`), `guard` (fights monsters near its
//! `place`, `work` unless named), `play` (at `meet`, a `game` with another NPC playing there:
//! `chess`, `checkers`, `table_tennis`, `tennis`, `basketball` or `any`), or any other word: the place of that name, where it stays and
//! walks about a little (`work`, `home`, `meet`, `market`…). `place` names another place for it.
//! `say` (a line, or a few to pick from) when it starts; `animation` now and then while it is there.
//! At `work`, an NPC with a job (see `job`) works; `job` (`farm`, `wood`, `fish`) on an entry
//! gives every NPC of the day that job then.
//! Hours are the game's clock (`6:00` is sunrise); the day wraps round midnight.

use crate::serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Doing {
    Follow,
    Sleep,
    Guard,
    Play,
    Be,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Day time, 0 to 24000 (0 is 6:00).
    pub at: i64,
    pub name: String,
    pub doing: Doing,
    pub place: String,
    pub say: Vec<String>,
    pub animation: Option<String>,
    /// What it plays, playing.
    pub game: String,
    /// The job it works at then (see `job`; empty: its own, at `work`).
    pub job: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Routine {
    pub day: Vec<Entry>,
}

/// `6:00`, `18:30` or `21` as day time (0 is 6:00).
pub fn clock(s: &str) -> Option<i64> {
    let s = s.trim();
    let (h, m) = match s.split_once(':') {
        Some((h, m)) => (h.trim().parse::<i64>().ok()?, m.trim().parse::<i64>().ok()?),
        None => (s.parse::<i64>().ok()?, 0),
    };
    if !(0..=24).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    Some(((h - 6).rem_euclid(24) * 1000 + m * 1000 / 60) % 24000)
}

/// Day time back as `h:mm`.
pub fn hour(t: i64) -> String {
    let t = t.rem_euclid(24000);
    format!("{}:{:02}", (t / 1000 + 6) % 24, (t % 1000) * 60 / 1000)
}

pub fn parse(bytes: &[u8]) -> Result<Routine, String> {
    let root: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    read(&root)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

pub fn read(root: &Value) -> Result<Routine, String> {
    let list = root.get("day").unwrap_or(root).as_array().ok_or("no \"day\" list")?;
    let mut day = Vec::new();
    for (i, e) in list.iter().enumerate() {
        let m = e.as_object().ok_or(format!("entry {}: not an object", i + 1))?;
        let at = match m.get("at") {
            Some(Value::String(s)) => clock(s),
            Some(v) => v.as_f64().and_then(|h| clock(&format!("{}", h as i64))),
            None => None,
        }
        .ok_or(format!("entry {}: \"at\" is an hour, as \"6:00\"", i + 1))?;
        let name = m.get("do").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or(format!("entry {}: no \"do\"", i + 1))?.to_string();
        let doing = match name.as_str() {
            "follow" => Doing::Follow,
            "sleep" => Doing::Sleep,
            "guard" => Doing::Guard,
            "play" => Doing::Play,
            _ => Doing::Be,
        };
        let place = m.get("place").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| match doing {
            Doing::Sleep => "home".into(),
            Doing::Guard => "work".into(),
            Doing::Play => "meet".into(),
            _ => name.clone(),
        });
        let game = m.get("game").and_then(Value::as_str).unwrap_or("any").to_string();
        day.push(Entry { at, name, doing, place, say: strings(m.get("say")), animation: m.get("animation").and_then(Value::as_str).map(str::to_string), game, job: m.get("job").and_then(Value::as_str).unwrap_or("").trim().to_string() });
    }
    if day.is_empty() {
        return Err("an empty day".into());
    }
    day.sort_by_key(|e| e.at);
    Ok(Routine { day })
}

impl Routine {
    /// What it does at day time `t`: the last entry begun, round midnight too.
    pub fn now(&self, t: i64) -> (usize, &Entry) {
        let t = t.rem_euclid(24000);
        let i = self.day.iter().rposition(|e| e.at <= t).unwrap_or(self.day.len() - 1);
        (i, &self.day[i])
    }

    /// The day in a line: `6:00 work, 12:00 meet, …`.
    pub fn describe(&self) -> String {
        self.day.iter().map(|e| format!("{} {}", hour(e.at), e.name)).collect::<Vec<_>>().join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks() {
        assert_eq!(clock("6:00"), Some(0));
        assert_eq!(clock("18:00"), Some(12000));
        assert_eq!(clock("12:30"), Some(6500));
        assert_eq!(clock("0:00"), Some(18000));
        assert_eq!(clock("5"), Some(23000));
        assert_eq!(clock("25:00"), None);
        assert_eq!(clock("noon"), None);
        assert_eq!(hour(13000), "19:00");
        assert_eq!(hour(6500), "12:30");
    }

    #[test]
    fn a_day() {
        let r = parse(include_bytes!("../data/projectfelix/felix_routines/villager.json")).unwrap();
        assert_eq!(r.now(500).1.name, "work");
        assert_eq!(r.now(6100).1.name, "meet");
        assert_eq!(r.now(15500).1.doing, Doing::Sleep);
        assert_eq!(r.now(15500).1.place, "home");
        // Before the first hour of the day: still the night's.
        let owl = parse(include_bytes!("../data/projectfelix/felix_routines/night_owl.json")).unwrap();
        let first = owl.day[0].at;
        assert_eq!(owl.now(first - 1).0, owl.day.len() - 1);
        let guard = parse(include_bytes!("../data/projectfelix/felix_routines/guard.json")).unwrap();
        assert!(guard.day.iter().any(|e| e.doing == Doing::Guard && e.place == "work"));
        assert!(r.describe().starts_with("6:00 work"));
        let play = r.now(clock("16:30").unwrap()).1;
        assert_eq!((play.doing.clone(), play.place.as_str(), play.game.as_str()), (Doing::Play, "meet", "any"));
        assert!(parse(br#"{ "day": [ { "at": "7:00" } ] }"#).is_err());
        assert!(parse(br#"{ "day": [] }"#).is_err());
        assert!(parse(br#"{ "day": [ { "at": "late", "do": "x" } ] }"#).is_err());
    }
}
