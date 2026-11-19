//! A scene as the scene editor holds it: the cast where they stand (top down, in blocks from the
//! lead: right and forward), and the timeline as rows to type in. What the editor doesn't show
//! (props, the end) is kept as the file has it. Back to JSON it is checked by `scene::parse`.
//! No mod API types here.

use crate::serde_json::{Map, Value, json};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CastEd {
    pub role: String,
    /// Right and forward, blocks.
    pub at: [f32; 2],
    /// Degrees from the way the lead faces.
    pub turn: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CueEd {
    pub at: String,
    /// 0: the lead; then the cast.
    pub who: usize,
    pub say: String,
    pub animation: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneEd {
    pub length: String,
    pub lead_animation: String,
    pub cast: Vec<CastEd>,
    pub cues: Vec<CueEd>,
    /// Props, the end and anything else, as they were.
    pub rest: Map<String, Value>,
}

fn num(v: Option<&Value>) -> String {
    v.and_then(Value::as_f64).map(|x| if x.fract() == 0.0 { format!("{}", x as i64) } else { format!("{x}") }).unwrap_or_default()
}

impl SceneEd {
    pub fn blank() -> SceneEd {
        SceneEd {
            length: "8".into(),
            cues: vec![CueEd { at: "0.5".into(), who: 0, say: "Hello, @player!".into(), animation: "animation.felix.wave".into() }],
            ..SceneEd::default()
        }
    }

    pub fn from_json(v: &Value) -> SceneEd {
        let mut rest = v.as_object().cloned().unwrap_or_default();
        for k in ["length", "actor", "cast", "timeline"] {
            rest.remove(k);
        }
        let cast: Vec<CastEd> = v
            .get("cast")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|r| {
                        let o = r.get("offset").and_then(Value::as_array);
                        let n = |i: usize| o.and_then(|o| o.get(i)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                        CastEd { role: r.get("role").and_then(Value::as_str).unwrap_or("").to_string(), at: [n(0), n(2)], turn: r.get("turn").and_then(Value::as_f64).unwrap_or(180.0) as f32 }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let who = |name: &str| if name == "lead" { 0 } else { cast.iter().position(|c| c.role == name).map_or(0, |i| i + 1) };
        let cues = v
            .get("timeline")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|c| {
                        let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                        CueEd { at: num(c.get("at")), who: who(c.get("who").and_then(Value::as_str).unwrap_or("lead")), say: s("say"), animation: s("animation") }
                    })
                    .collect()
            })
            .unwrap_or_default();
        SceneEd {
            length: num(v.get("length")),
            lead_animation: v.get("actor").and_then(|a| a.get("animation")).and_then(Value::as_str).unwrap_or("").to_string(),
            cast,
            cues,
            rest,
        }
    }

    /// The names of who can act: the lead, then the cast.
    pub fn who_names(&self) -> Vec<String> {
        std::iter::once("lead".to_string()).chain(self.cast.iter().map(|c| c.role.clone())).collect()
    }

    pub fn to_json(&self) -> Result<Value, String> {
        let length: f64 = self.length.trim().parse().map_err(|_| format!("length: seconds, not \"{}\"", self.length))?;
        let mut m = self.rest.clone();
        m.insert("length".into(), json!(length));
        if !self.lead_animation.trim().is_empty() {
            m.insert("actor".into(), json!({ "animation": self.lead_animation.trim() }));
        }
        if !self.cast.is_empty() {
            let cast: Vec<Value> = self
                .cast
                .iter()
                .map(|c| json!({ "role": c.role.trim(), "offset": [ (c.at[0] * 10.0).round() / 10.0, 0, (c.at[1] * 10.0).round() / 10.0 ], "turn": c.turn }))
                .collect();
            m.insert("cast".into(), Value::Array(cast));
        }
        let names = self.who_names();
        let mut cues = Vec::new();
        for (i, c) in self.cues.iter().enumerate() {
            let at: f64 = c.at.trim().parse().map_err(|_| format!("line {}: the time is seconds, not \"{}\"", i + 1, c.at))?;
            let mut o = Map::new();
            o.insert("at".into(), json!(at));
            o.insert("who".into(), json!(names.get(c.who).cloned().unwrap_or_else(|| "lead".into())));
            if !c.say.trim().is_empty() {
                o.insert("say".into(), json!(c.say.trim()));
            }
            if !c.animation.trim().is_empty() {
                o.insert("animation".into(), json!(c.animation.trim()));
            }
            cues.push((at, Value::Object(o)));
        }
        cues.sort_by(|a, b| a.0.total_cmp(&b.0));
        if !cues.is_empty() {
            m.insert("timeline".into(), Value::Array(cues.into_iter().map(|c| c.1).collect()));
        }
        let v = Value::Object(m);
        crate::scene::parse(v.to_string().as_bytes())?;
        Ok(v)
    }

    /// A new role, beside the lead; its index in the cast.
    pub fn add_role(&mut self) -> usize {
        let mut k = self.cast.len() + 1;
        while self.cast.iter().any(|c| c.role == format!("actor{k}")) {
            k += 1;
        }
        let side = if self.cast.len() % 2 == 0 { 1.5 } else { -1.5 };
        self.cast.push(CastEd { role: format!("actor{k}"), at: [side, 1.5 + self.cast.len() as f32 * 0.5], turn: 180.0 });
        self.cast.len() - 1
    }

    /// Takes role `i` away, and its lines with it.
    pub fn remove_role(&mut self, i: usize) {
        if i >= self.cast.len() {
            return;
        }
        self.cast.remove(i);
        let gone = i + 1;
        self.cues.retain(|c| c.who != gone);
        for c in &mut self.cues {
            if c.who > gone {
                c.who -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_greeting_goes_round() {
        let v: Value = crate::serde_json::from_slice(include_bytes!("../data/projectfelix/felix_scenes/greeting.json")).unwrap();
        let ed = SceneEd::from_json(&v);
        assert_eq!(ed.cast.len(), 1);
        assert_eq!(ed.cues[1].who, 1);
        let back = ed.to_json().unwrap();
        assert_eq!(crate::scene::parse(back.to_string().as_bytes()).unwrap(), crate::scene::parse(v.to_string().as_bytes()).unwrap());
        // Props and the end are kept.
        let chess: Value = crate::serde_json::from_slice(include_bytes!("../data/projectfelix/felix_scenes/chess.json")).unwrap();
        let back = SceneEd::from_json(&chess).to_json().unwrap();
        assert_eq!(back["props"], chess["props"]);
        assert_eq!(back["end"], chess["end"]);
    }

    #[test]
    fn roles_come_and_go() {
        let mut ed = SceneEd::blank();
        let a = ed.add_role();
        let b = ed.add_role();
        ed.cues.push(CueEd { at: "2".into(), who: a + 1, say: "One".into(), ..CueEd::default() });
        ed.cues.push(CueEd { at: "3".into(), who: b + 1, say: "Two".into(), ..CueEd::default() });
        ed.to_json().unwrap();
        ed.remove_role(a);
        assert_eq!(ed.cues.len(), 2, "its line went with it");
        assert_eq!(ed.cues[1].who, 1);
        assert_eq!(ed.who_names(), vec!["lead".to_string(), "actor2".to_string()]);
        ed.cues[0].at = "soon".into();
        assert!(ed.to_json().unwrap_err().contains("line 1"));
    }
}
