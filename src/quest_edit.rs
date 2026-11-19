//! A quest as the quest editor shows it (see `quest_editor`): its settings (title, what the NPC
//! says, the reward at the end, what the player must have done before, a time limit…) and its
//! steps one after another, each a goal with its own words and reward. To and from the quest JSON
//! (see `quest`). No mod API types here.

use crate::dialog_edit::{acts_json, acts_text};
use crate::serde_json::{Map, Value, json};

pub const MAX_STEPS: usize = 8;
/// The goals: name, and what the word field takes.
pub const KINDS: &[(&str, &str)] = &[("bring", "item: minecraft:apple"), ("kill", "zombie, monster, animal"), ("go to", "x y z"), ("walk me", "blocks: 32"), ("deliver to", "NPC name, or #tag")];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepEd {
    pub kind: usize,
    pub what: String,
    pub count: String,
    /// What a delivery hands over (empty: a message).
    pub item: String,
    pub say: String,
    pub progress: String,
    pub done: String,
    pub reward: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuestEd {
    pub title: String,
    pub offer: String,
    pub done: String,
    pub fail: String,
    pub reward: String,
    pub gives: String,
    pub requires: String,
    pub scene: String,
    pub time: String,
    pub min_relation: String,
    pub repeat: bool,
    pub steps: Vec<StepEd>,
    pub rest: Map<String, Value>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn num(v: Option<&Value>) -> String {
    v.and_then(Value::as_f64).map(|x| if x.fract() == 0.0 { format!("{}", x as i64) } else { format!("{x}") }).unwrap_or_default()
}

fn acts(v: Option<&Value>) -> String {
    acts_text(v.and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]))
}

const GOAL_KEYS: &[&str] = &["bring", "kill", "go_to", "walk", "to", "distance", "radius", "deliver_to", "item", "count"];

impl StepEd {
    fn from_json(v: &Value) -> StepEd {
        let count = num(v.get("count"));
        let (kind, what) = if v.get("bring").is_some() {
            (0, s(v, "bring"))
        } else if v.get("kill").is_some() {
            (1, s(v, "kill"))
        } else if let Some(p) = v.get("go_to").and_then(Value::as_array) {
            (2, p.iter().map(|x| num(Some(x))).collect::<Vec<_>>().join(" "))
        } else if v.get("walk").is_some() {
            (3, num(v.get("distance")))
        } else {
            (4, s(v, "deliver_to"))
        };
        StepEd { kind, what, count: if count == "1" { String::new() } else { count }, item: s(v, "item"), say: s(v, "say"), progress: s(v, "progress"), done: s(v, "done"), reward: acts(v.get("reward")) }
    }

    /// Its goal's fields, into `o`.
    fn goal_into(&self, o: &mut Map<String, Value>, what_of: &str) -> Result<(), String> {
        let w = self.what.trim();
        let need = || if w.is_empty() { Err(format!("{what_of}: {} what?", KINDS[self.kind].0)) } else { Ok(()) };
        let count = if self.count.trim().is_empty() { 1 } else { self.count.trim().parse::<u64>().map_err(|_| format!("{what_of}: count is a number"))? };
        match self.kind {
            0 => {
                need()?;
                o.insert("bring".into(), json!(w));
                o.insert("count".into(), json!(count));
            }
            1 => {
                need()?;
                o.insert("kill".into(), json!(w));
                o.insert("count".into(), json!(count));
            }
            2 => {
                let p: Vec<f64> = w.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                if p.len() != 3 {
                    return Err(format!("{what_of}: go to x y z"));
                }
                o.insert("go_to".into(), json!(p));
            }
            3 => {
                o.insert("walk".into(), json!(true));
                if !w.is_empty() {
                    o.insert("distance".into(), json!(w.parse::<f64>().map_err(|_| format!("{what_of}: walk how many blocks?"))?));
                }
            }
            _ => {
                need()?;
                o.insert("deliver_to".into(), json!(w));
                if !self.item.trim().is_empty() {
                    o.insert("item".into(), json!(self.item.trim()));
                    o.insert("count".into(), json!(count));
                }
            }
        }
        Ok(())
    }

    pub fn summary(&self) -> String {
        let n = if self.count.trim().is_empty() || self.count.trim() == "1" { String::new() } else { format!("{} ", self.count.trim()) };
        let what = self.what.trim().trim_start_matches("minecraft:");
        format!("{} {n}{what}", KINDS.get(self.kind).map_or("?", |k| k.0))
    }
}

impl QuestEd {
    pub fn blank() -> QuestEd {
        QuestEd {
            title: "A favour for @npc".into(),
            offer: "Could you help me, @player?".into(),
            done: "Thank you, @player!".into(),
            fail: "Never mind, then.".into(),
            reward: "give minecraft:emerald 2".into(),
            steps: vec![StepEd { kind: 0, what: "minecraft:apple".into(), count: "3".into(), ..StepEd::default() }],
            ..QuestEd::default()
        }
    }

    pub fn from_json(v: &Value) -> QuestEd {
        let steps = match v.get("steps").and_then(Value::as_array) {
            Some(list) if !list.is_empty() => list.iter().map(StepEd::from_json).collect(),
            _ => {
                let mut one = StepEd::from_json(v);
                one.say.clear();
                one.done.clear();
                one.reward.clear();
                vec![one]
            }
        };
        let known = ["title", "offer", "done", "fail", "reward", "gives", "requires", "scene", "time", "min_relation", "repeat", "steps", "progress"];
        let rest = v.as_object().map(|o| o.iter().filter(|(k, _)| !known.contains(&k.as_str()) && !GOAL_KEYS.contains(&k.as_str())).map(|(k, x)| (k.clone(), x.clone())).collect()).unwrap_or_default();
        let requires = match v.get("requires") {
            Some(Value::String(r)) => r.clone(),
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
            _ => String::new(),
        };
        let mut q = QuestEd {
            title: s(v, "title"),
            offer: s(v, "offer"),
            done: s(v, "done"),
            fail: s(v, "fail"),
            reward: acts(v.get("reward")),
            gives: acts(v.get("gives")),
            requires,
            scene: s(v, "scene"),
            time: num(v.get("time")),
            min_relation: num(v.get("min_relation")),
            repeat: v.get("repeat").and_then(Value::as_bool).unwrap_or(false),
            steps,
            rest,
        };
        // One step: its progress line is the quest's.
        if q.steps.len() == 1 {
            q.steps[0].progress = s(v, "progress");
        }
        q
    }

    pub fn to_json(&self) -> Result<Value, String> {
        let mut o = self.rest.clone();
        let text = |o: &mut Map<String, Value>, k: &str, v: &str| {
            if !v.trim().is_empty() {
                o.insert(k.into(), json!(v.trim()));
            }
        };
        text(&mut o, "title", &self.title);
        text(&mut o, "offer", &self.offer);
        text(&mut o, "done", &self.done);
        text(&mut o, "fail", &self.fail);
        text(&mut o, "scene", &self.scene);
        let reward = acts_json(&self.reward).map_err(|e| format!("reward: {e}"))?;
        if !reward.is_empty() {
            o.insert("reward".into(), json!(reward));
        }
        let gives = acts_json(&self.gives).map_err(|e| format!("gives: {e}"))?;
        if !gives.is_empty() {
            o.insert("gives".into(), json!(gives));
        }
        let req: Vec<&str> = self.requires.split_whitespace().collect();
        if !req.is_empty() {
            o.insert("requires".into(), json!(req));
        }
        for (k, v) in [("time", &self.time), ("min_relation", &self.min_relation)] {
            if !v.trim().is_empty() {
                o.insert(k.into(), json!(v.trim().parse::<f64>().map_err(|_| format!("{k} is a number"))?));
            }
        }
        if self.repeat {
            o.insert("repeat".into(), json!(true));
        }
        match self.steps.as_slice() {
            [] => return Err("a quest with no steps".into()),
            [one] => {
                one.goal_into(&mut o, "the goal")?;
                text(&mut o, "progress", &one.progress);
            }
            many => {
                let mut list = Vec::new();
                for (i, st) in many.iter().enumerate() {
                    let what = format!("step {}", i + 1);
                    let mut x = Map::new();
                    st.goal_into(&mut x, &what)?;
                    text(&mut x, "say", &st.say);
                    text(&mut x, "progress", &st.progress);
                    text(&mut x, "done", &st.done);
                    let r = acts_json(&st.reward).map_err(|e| format!("{what}, reward: {e}"))?;
                    if !r.is_empty() {
                        x.insert("reward".into(), json!(r));
                    }
                    list.push(Value::Object(x));
                }
                o.insert("steps".into(), json!(list));
            }
        }
        let v = Value::Object(o);
        crate::quest::read(&v)?;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quests_go_round() {
        let v: Value = crate::serde_json::from_str(r##"{ "title": "The lost ring", "offer": "Help me?", "done": "Thanks!", "reward": [ { "give": "minecraft:emerald", "count": 5 } ],
            "requires": ["projectfelix:apples"], "repeat": true, "time": 20,
            "steps": [
              { "bring": "minecraft:apple", "count": 3, "say": "First, apples.", "done": "Good!" },
              { "kill": "minecraft:zombie", "count": 2, "reward": [ { "give": "minecraft:bread", "count": 1 } ] },
              { "deliver_to": "#felix_guest", "item": "minecraft:paper", "say": "Take this to my friend." } ] }"##).unwrap();
        let ed = QuestEd::from_json(&v);
        assert_eq!(ed.steps.len(), 3);
        assert_eq!(ed.steps[1].summary(), "kill 2 zombie");
        assert_eq!(ed.steps[1].reward, "give minecraft:bread");
        let back = ed.to_json().unwrap();
        assert_eq!(QuestEd::from_json(&back), ed);
        let q = crate::quest::read(&back).unwrap();
        assert_eq!(q.step_count(), 3);
        let s1 = q.view(1);
        assert_eq!(s1.goal.describe(), "kill 2 zombie");
        assert_eq!(s1.done, "Take this to my friend.");
        let s2 = q.view(2);
        assert_eq!(s2.done, "Thanks!");
        assert_eq!(s2.reward.len(), 1);
        assert_eq!(q.view(0).done, "Good!");
        // One step: the compact form.
        let one = QuestEd::blank().to_json().unwrap();
        assert!(one.get("bring").is_some() && one.get("steps").is_none());
        let mut bad = QuestEd::blank();
        bad.steps[0].kind = 2;
        bad.steps[0].what = "1 2".into();
        assert!(bad.to_json().unwrap_err().contains("x y z"));
    }
}
