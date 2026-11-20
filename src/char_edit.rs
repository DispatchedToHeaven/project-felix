//! A character as the in-world editor holds it: its moods as boxes where they were put, their
//! ways to other moods, and the rest as text to type in. Back to JSON it is checked by
//! `character::read`. No mod API types here.
//!
//! Lists of lines are typed with ` | ` between them. A way's condition is `<n` or `>n` (the
//! relation with the player of the event below or above n), or nothing. Where the boxes stand is
//! kept in the file's `layout` (the game reads past it).

use crate::serde_json::{Map, Value, json};

/// The canvas the boxes stand on.
pub const CANVAS: [f32; 2] = [220.0, 168.0];
pub const BOX: [f32; 2] = [52.0, 14.0];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WayEd {
    pub on: String,
    pub to: usize,
    pub cond: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MoodEd {
    pub name: String,
    pub at: [f32; 2],
    pub obey: String,
    pub speed: String,
    pub delay: String,
    pub say: String,
    pub refuse: String,
    pub grumble: String,
    pub chatter: String,
    pub animation: String,
    pub particle: String,
    /// Seconds (empty: it doesn't pass), and to which mood.
    pub after: String,
    pub after_to: usize,
    pub ways: Vec<WayEd>,
    /// What the editor doesn't show (its voice…), kept as the file had it.
    pub rest: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharEd {
    pub moods: Vec<MoodEd>,
    pub start: usize,
    pub friend_at: String,
    pub hate_at: String,
    pub owner: String,
    pub forgive: String,
    pub bond: String,
    pub bond_max: String,
    pub shun: String,
    pub relations: Vec<(String, String)>,
    /// What the editor doesn't show (greetings, voice…), kept as the file had it.
    pub rest: Map<String, Value>,
}

fn lines(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" | "),
        _ => String::new(),
    }
}

/// The keys of object `v` but `known`.
fn rest_of(v: &Value, known: &[&str]) -> Map<String, Value> {
    v.as_object().map(|o| o.iter().filter(|(k, _)| !known.contains(&k.as_str())).map(|(k, x)| (k.clone(), x.clone())).collect()).unwrap_or_default()
}

fn split(s: &str) -> Vec<String> {
    s.split('|').map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect()
}

fn num(v: Option<&Value>, d: &str) -> String {
    v.and_then(Value::as_f64).map(|x| if x.fract() == 0.0 { format!("{}", x as i64) } else { format!("{x}") }).unwrap_or_else(|| d.to_string())
}

/// Where box `i` of `n` stands when the file doesn't say: round a circle.
fn round(i: usize, n: usize) -> [f32; 2] {
    let a = i as f32 / n.max(1) as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
    let (cx, cy) = ((CANVAS[0] - BOX[0]) / 2.0, (CANVAS[1] - BOX[1]) / 2.0);
    [(cx + a.cos() * cx * 0.85).round(), (cy + a.sin() * cy * 0.85).round()]
}

impl CharEd {
    /// One calm mood, and the usual numbers.
    pub fn blank() -> CharEd {
        CharEd::from_json(&json!({ "start": "calm", "moods": { "calm": {} } }))
    }

    pub fn from_json(v: &Value) -> CharEd {
        let raw: Vec<(String, Value)> = v.get("moods").and_then(Value::as_object).map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
        let start_name = v.get("start").and_then(Value::as_str).unwrap_or("calm");
        // The start first, then the rest as the file has them.
        let mut names: Vec<String> = raw.iter().map(|r| r.0.clone()).collect();
        if let Some(i) = names.iter().position(|n| n == start_name) {
            let s = names.remove(i);
            names.insert(0, s);
        }
        let index = |n: &str| names.iter().position(|x| x == n).unwrap_or(0);
        let layout = v.get("layout");
        let n = names.len();
        let moods = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let m = raw.iter().find(|r| &r.0 == name).map(|r| r.1.clone()).unwrap_or(Value::Null);
                let mut ways = Vec::new();
                let (mut after, mut after_to) = (String::new(), i);
                if let Some(on) = m.get("on").and_then(Value::as_object) {
                    for (event, to) in on {
                        if let Some(secs) = event.strip_prefix("after:") {
                            after = secs.to_string();
                            after_to = index(to.as_str().unwrap_or(""));
                            continue;
                        }
                        let list = match to {
                            Value::Array(a) => a.clone(),
                            other => vec![other.clone()],
                        };
                        for one in list {
                            let (to, cond) = match &one {
                                Value::String(t) => (t.clone(), String::new()),
                                o => {
                                    let cond = match (o.get("below").and_then(Value::as_f64), o.get("above").and_then(Value::as_f64)) {
                                        (Some(b), _) => format!("<{}", b as i64),
                                        (None, Some(a)) => format!(">{}", a as i64),
                                        _ => String::new(),
                                    };
                                    (o.get("to").and_then(Value::as_str).unwrap_or("").to_string(), cond)
                                }
                            };
                            ways.push(WayEd { on: event.clone(), to: index(&to), cond });
                        }
                    }
                }
                let at = layout
                    .and_then(|l| l.get(name))
                    .and_then(Value::as_array)
                    .and_then(|a| Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32]))
                    .unwrap_or_else(|| round(i, n));
                let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                MoodEd {
                    name: name.clone(),
                    at,
                    obey: num(m.get("obey"), "1"),
                    speed: num(m.get("speed"), "1"),
                    delay: num(m.get("delay"), "0"),
                    say: lines(m.get("say")),
                    refuse: lines(m.get("refuse")),
                    grumble: lines(m.get("grumble")),
                    chatter: lines(m.get("chatter")),
                    animation: s("animation"),
                    particle: s("particle"),
                    after,
                    after_to,
                    ways,
                    rest: rest_of(&m, &["obey", "speed", "delay", "say", "refuse", "grumble", "chatter", "animation", "particle", "on"]),
                }
            })
            .collect();
        let relations = v
            .get("relations")
            .and_then(Value::as_object)
            .map(|r| r.iter().map(|(k, x)| (k.clone(), num(Some(x), "0"))).collect())
            .unwrap_or_default();
        CharEd {
            moods,
            start: 0,
            friend_at: num(v.get("friend_at"), "30"),
            hate_at: num(v.get("hate_at"), "-50"),
            owner: num(v.get("owner"), "50"),
            forgive: num(v.get("forgive"), "2"),
            bond: num(v.get("bond"), "1"),
            bond_max: num(v.get("bond_max"), "60"),
            shun: lines(v.get("shun")),
            relations,
            rest: rest_of(v, &["start", "friend_at", "hate_at", "owner", "forgive", "bond", "bond_max", "shun", "relations", "moods", "layout"]),
        }
    }

    pub fn to_json(&self) -> Result<Value, String> {
        let number = |what: &str, s: &str| -> Result<f64, String> { s.trim().parse::<f64>().map_err(|_| format!("{what}: a number, not \"{s}\"")) };
        let mut moods = Map::new();
        let mut layout = Map::new();
        for m in &self.moods {
            let name = m.name.trim();
            if name.is_empty() {
                return Err("a mood without a name".into());
            }
            if moods.contains_key(name) {
                return Err(format!("two moods named {name}"));
            }
            let mut o = Map::new();
            let mood_name = |i: usize| self.moods.get(i).map(|x| x.name.trim().to_string()).unwrap_or_default();
            o.insert("obey".into(), json!(number(&format!("{name}: obeys"), &m.obey)?));
            o.insert("speed".into(), json!(number(&format!("{name}: speed"), &m.speed)?));
            o.insert("delay".into(), json!(number(&format!("{name}: delay"), &m.delay)?));
            for (k, v) in [("say", &m.say), ("refuse", &m.refuse), ("grumble", &m.grumble), ("chatter", &m.chatter)] {
                let l = split(v);
                if !l.is_empty() {
                    o.insert(k.into(), json!(l));
                }
            }
            for (k, v) in [("animation", &m.animation), ("particle", &m.particle)] {
                if !v.trim().is_empty() {
                    o.insert(k.into(), json!(v.trim()));
                }
            }
            let mut on = Map::new();
            for w in &m.ways {
                let event = w.on.trim();
                if event.is_empty() {
                    continue;
                }
                let c = w.cond.trim().replace(' ', "");
                let edge = if c.is_empty() {
                    json!(mood_name(w.to))
                } else {
                    let (key, v) = match (c.strip_prefix('<'), c.strip_prefix('>')) {
                        (Some(v), _) => ("below", v),
                        (_, Some(v)) => ("above", v),
                        _ => return Err(format!("{name}, {event}: the condition is <n or >n, not {c}")),
                    };
                    let v: i64 = v.parse().map_err(|_| format!("{name}, {event}: the condition is <n or >n, not {c}"))?;
                    json!({ "to": mood_name(w.to), key: v })
                };
                // Several ways on one event: a list, the first that fits taken.
                match on.get_mut(event) {
                    Some(Value::Array(a)) => a.push(edge),
                    Some(one) => *one = Value::Array(vec![one.clone(), edge]),
                    None => {
                        on.insert(event.to_string(), edge);
                    }
                }
            }
            if !m.after.trim().is_empty() {
                let secs = number(&format!("{name}: after"), &m.after)?;
                on.insert(format!("after:{secs}"), json!(mood_name(m.after_to)));
            }
            // Lists hold objects or names; a list of one name reads back the same as the name.
            if !on.is_empty() {
                o.insert("on".into(), Value::Object(on));
            }
            for (k, x) in &m.rest {
                o.entry(k.clone()).or_insert_with(|| x.clone());
            }
            moods.insert(name.to_string(), Value::Object(o));
            layout.insert(name.to_string(), json!([m.at[0].round(), m.at[1].round()]));
        }
        let mut relations = Map::new();
        for (event, delta) in &self.relations {
            if !event.trim().is_empty() {
                relations.insert(event.trim().to_string(), json!(number(&format!("relation {event}"), delta)? as i64));
            }
        }
        let mut v = json!({
            "start": self.moods.get(self.start).map(|m| m.name.trim().to_string()).unwrap_or_default(),
            "friend_at": number("friend at", &self.friend_at)? as i64,
            "hate_at": number("hate at", &self.hate_at)? as i64,
            "owner": number("owner", &self.owner)? as i64,
            "forgive": number("forgive", &self.forgive)?,
            "bond": number("bond", &self.bond)?,
            "bond_max": number("bond max", &self.bond_max)? as i64,
            "shun": split(&self.shun),
            "relations": Value::Object(relations),
            "moods": Value::Object(moods),
            "layout": Value::Object(layout),
        });
        if let Value::Object(top) = &mut v {
            for (k, x) in &self.rest {
                top.entry(k.clone()).or_insert_with(|| x.clone());
            }
        }
        crate::character::read(&v)?;
        Ok(v)
    }

    /// A new mood, near the middle; its index.
    pub fn add_mood(&mut self) -> usize {
        let mut k = self.moods.len() + 1;
        while self.moods.iter().any(|m| m.name == format!("mood{k}")) {
            k += 1;
        }
        let i = self.moods.len();
        let at = [(CANVAS[0] - BOX[0]) / 2.0 + (i % 3) as f32 * 8.0, (CANVAS[1] - BOX[1]) / 2.0 + (i % 4) as f32 * 8.0];
        self.moods.push(MoodEd { name: format!("mood{k}"), at, obey: "1".into(), speed: "1".into(), delay: "0".into(), after_to: 0, ..MoodEd::default() });
        i
    }

    /// Takes mood `i` away, and every way to it; the start stays a mood.
    pub fn remove_mood(&mut self, i: usize) {
        if self.moods.len() <= 1 || i >= self.moods.len() {
            return;
        }
        self.moods.remove(i);
        let fix = |j: usize| if j > i { j - 1 } else { j };
        for m in &mut self.moods {
            m.ways.retain(|w| w.to != i);
            for w in &mut m.ways {
                w.to = fix(w.to);
            }
            if m.after_to == i {
                m.after.clear();
                m.after_to = 0;
            } else {
                m.after_to = fix(m.after_to);
            }
        }
        if self.start == i {
            self.start = 0;
        } else {
            self.start = fix(self.start);
        }
    }

    /// The mood whose box is under canvas point `p` (the last drawn, on top).
    pub fn mood_at(&self, p: [f32; 2]) -> Option<usize> {
        self.moods.iter().rposition(|m| p[0] >= m.at[0] && p[0] < m.at[0] + BOX[0] && p[1] >= m.at[1] && p[1] < m.at[1] + BOX[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_samples_go_round() {
        for text in [
            include_str!("../data/projectfelix/felix_characters/kind.json"),
            include_str!("../data/projectfelix/felix_characters/touchy.json"),
            include_str!("../data/projectfelix/felix_characters/stoic.json"),
        ] {
            let v: Value = crate::serde_json::from_str(text).unwrap();
            let ed = CharEd::from_json(&v);
            assert_eq!(ed.moods[ed.start].name, "calm");
            let back = ed.to_json().unwrap();
            assert_eq!(crate::character::read(&back).unwrap(), crate::character::read(&v).unwrap());
            // And again, with the layout it now has.
            assert_eq!(CharEd::from_json(&back), ed);
        }
    }

    #[test]
    fn moods_come_and_go() {
        let v: Value = crate::serde_json::from_str(include_str!("../data/projectfelix/felix_characters/kind.json")).unwrap();
        let mut ed = CharEd::from_json(&v);
        let n = ed.moods.len();
        let i = ed.add_mood();
        assert_eq!(ed.moods.len(), n + 1);
        ed.moods[0].ways.push(WayEd { on: "dance".into(), to: i, cond: ">10".into() });
        let back = ed.to_json().unwrap();
        assert_eq!(back["moods"]["calm"]["on"]["dance"], json!({ "to": ed.moods[i].name, "above": 10 }));
        let upset = ed.moods.iter().position(|m| m.name == "upset").unwrap();
        ed.remove_mood(upset);
        assert!(ed.moods.iter().all(|m| m.name != "upset"));
        ed.to_json().unwrap();
        assert_eq!(ed.moods[ed.start].name, "calm");
        // The last one stays.
        let mut one = CharEd::blank();
        one.remove_mood(0);
        assert_eq!(one.moods.len(), 1);
        assert!(one.mood_at([one.moods[0].at[0] + 1.0, one.moods[0].at[1] + 1.0]) == Some(0));
    }

    #[test]
    fn mistakes_are_named() {
        let mut ed = CharEd::blank();
        ed.moods[0].obey = "lots".into();
        assert!(ed.to_json().unwrap_err().contains("obeys"));
        let mut ed = CharEd::blank();
        ed.moods[0].ways.push(WayEd { on: "gift".into(), to: 0, cond: "=5".into() });
        assert!(ed.to_json().unwrap_err().contains("<n or >n"));
        let mut ed = CharEd::blank();
        ed.add_mood();
        ed.moods[1].name = "calm".into();
        assert!(ed.to_json().unwrap_err().contains("two moods"));
    }
}
