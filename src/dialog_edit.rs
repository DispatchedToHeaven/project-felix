//! A dialog as the dialog editor shows it (see `dialog_editor`): its nodes as boxes on a canvas,
//! each either a talk (what the NPC says, a line at a time or all together, then the player's
//! answers, or on to the next node) or a check (where the talk goes by what holds: an answer given
//! before, the player's memory, the relation, the biome, the time…). An answer's hidden effects
//! (relation, a feeling, actions) are fields of their own. To and from the dialog JSON. No mod
//! API types here.

use crate::serde_json::{Map, Value, json};

pub const CANVAS: [f32; 2] = [170.0, 186.0];
pub const BOX: [f32; 2] = [54.0, 12.0];
/// No node: the talk ends.
pub const END: usize = usize::MAX;
pub const MAX_LINES: usize = 7;
pub const MAX_ANSWERS: usize = 4;
pub const MAX_CHECKS: usize = 6;

/// The conditions the editor offers: their name and what the value field takes.
pub const CONDS: &[(&str, &str)] = &[
    ("always", ""),
    ("answer", "node=b"),
    ("memory", "key or key=value"),
    ("rel >", "20"),
    ("rel <", "-10"),
    ("mood", "happy"),
    ("item", "minecraft:apple 3"),
    ("tag", "felix_x"),
    ("met", ""),
    ("biome", "snowy, or minecraft:desert"),
    ("time", "day, dusk, night, dawn"),
    ("weather", "clear, rain, thunder"),
    ("chance", "percent: 50"),
    ("event", "any, or its id"),
    ("owner", ""),
    ("custom", "(as the file has it)"),
];
const CUSTOM: usize = 15;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CondEd {
    pub kind: usize,
    pub value: String,
    pub not: bool,
    /// A condition the editor has no row for, kept as it was.
    pub raw: Option<Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnswerEd {
    pub text: String,
    pub cond: CondEd,
    /// How much it moves the relation (hidden from the player).
    pub relation: String,
    /// An event for the NPC's character (`praise`, `scold`…).
    pub feel: String,
    /// More it does: `give minecraft:apple 3; remember met_ann`.
    pub act: String,
    pub goto: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckEd {
    pub cond: CondEd,
    pub goto: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeEd {
    pub name: String,
    pub at: [f32; 2],
    pub check: bool,
    pub lines: Vec<String>,
    /// Its lines a page each (else all at once).
    pub paged: bool,
    pub more: String,
    pub animation: String,
    /// What it does when it comes up.
    pub act: String,
    pub next: usize,
    pub answers: Vec<AnswerEd>,
    pub checks: Vec<CheckEd>,
    pub rest: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DialogEd {
    pub nodes: Vec<NodeEd>,
    pub start: usize,
    pub rest: Map<String, Value>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn num(v: &Value) -> String {
    v.as_f64().map(|x| if x.fract() == 0.0 { format!("{}", x as i64) } else { format!("{x}") }).unwrap_or_default()
}

// ---- conditions -------------------------------------------------------------------------------------------

impl CondEd {
    pub fn from_json(v: Option<&Value>) -> CondEd {
        let Some(v) = v else { return CondEd::default() };
        if let Some(inner) = v.get("not") {
            let mut c = CondEd::from_json(Some(inner));
            if c.kind != CUSTOM && !c.not {
                c.not = true;
                return c;
            }
            return CondEd { kind: CUSTOM, raw: Some(v.clone()), ..CondEd::default() };
        }
        let o = v.as_object();
        let has = |k: &str| o.is_some_and(|o| o.contains_key(k)) && o.is_some_and(|o| o.len() <= 2);
        let one = |kind: usize, value: String| CondEd { kind, value, not: false, raw: None };
        if has("answer") {
            one(1, format!("{}={}", s(v, "answer"), s(v, "is")))
        } else if has("flag") {
            let is = s(v, "is");
            one(2, if is.is_empty() { s(v, "flag") } else { format!("{}={is}", s(v, "flag")) })
        } else if has("relation_above") {
            one(3, num(&v["relation_above"]))
        } else if has("relation_below") {
            one(4, num(&v["relation_below"]))
        } else if has("mood") {
            one(5, s(v, "mood"))
        } else if has("item") {
            let n = v.get("count").map(num).unwrap_or_default();
            one(6, if n.is_empty() || n == "1" { s(v, "item") } else { format!("{} {n}", s(v, "item")) })
        } else if has("tag") {
            one(7, s(v, "tag"))
        } else if let Some(b) = v.get("met").and_then(Value::as_bool).filter(|_| has("met")) {
            CondEd { kind: 8, not: !b, ..CondEd::default() }
        } else if has("biome") {
            one(9, s(v, "biome"))
        } else if has("time") {
            one(10, s(v, "time"))
        } else if has("weather") {
            one(11, s(v, "weather"))
        } else if has("chance") {
            one(12, num(&v["chance"]))
        } else if has("event") {
            one(13, s(v, "event"))
        } else if let Some(b) = v.get("owner").and_then(Value::as_bool).filter(|_| has("owner")) {
            CondEd { kind: 14, not: !b, ..CondEd::default() }
        } else {
            CondEd { kind: CUSTOM, raw: Some(v.clone()), ..CondEd::default() }
        }
    }

    /// As JSON (none: always).
    pub fn to_json(&self) -> Result<Option<Value>, String> {
        let v = self.value.trim();
        let pair = |what: &str| -> Result<(String, String), String> {
            let (a, b) = v.split_once('=').ok_or(format!("{what}: write it as name=value"))?;
            Ok((a.trim().to_string(), b.trim().to_string()))
        };
        let number = |what: &str| -> Result<f64, String> { v.parse::<f64>().map_err(|_| format!("{what}: a number, not \"{v}\"")) };
        let need = |what: &str| -> Result<(), String> { if v.is_empty() { Err(format!("{what}: what?")) } else { Ok(()) } };
        let c = match self.kind {
            0 => return Ok(None),
            1 => {
                let (n, a) = pair("answered")?;
                json!({ "answer": n, "is": a })
            }
            2 => {
                need("remembers")?;
                match v.split_once('=') {
                    Some((k, x)) => json!({ "flag": k.trim(), "is": x.trim() }),
                    None => json!({ "flag": v }),
                }
            }
            3 => json!({ "relation_above": number("relation >")? as i64 }),
            4 => json!({ "relation_below": number("relation <")? as i64 }),
            5 => {
                need("mood")?;
                json!({ "mood": v })
            }
            6 => {
                need("has item")?;
                let mut parts = v.split_whitespace();
                let item = parts.next().unwrap_or("");
                let n = parts.next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(1);
                json!({ "item": item, "count": n })
            }
            7 => {
                need("tag")?;
                json!({ "tag": v })
            }
            8 => json!({ "met": true }),
            9 => {
                need("biome")?;
                json!({ "biome": v })
            }
            10 => {
                if !["day", "dusk", "night", "dawn"].contains(&v) {
                    return Err(format!("time: day, dusk, night or dawn, not \"{v}\""));
                }
                json!({ "time": v })
            }
            11 => {
                if !["clear", "rain", "thunder"].contains(&v) {
                    return Err(format!("weather: clear, rain or thunder, not \"{v}\""));
                }
                json!({ "weather": v })
            }
            12 => json!({ "chance": number("chance")?.clamp(0.0, 100.0) as i64 }),
            13 => json!({ "event": if v.is_empty() { "any" } else { v } }),
            14 => json!({ "owner": true }),
            _ => return Ok(self.raw.clone()),
        };
        Ok(Some(if self.not { json!({ "not": c }) } else { c }))
    }

    /// In words, for the canvas and the lists.
    pub fn label(&self) -> String {
        let name = CONDS.get(self.kind).map_or("?", |c| c.0);
        let body = if self.value.trim().is_empty() { name.to_string() } else { format!("{name} {}", self.value.trim()) };
        if self.not { format!("not {body}") } else { body }
    }
}

// ---- actions as text ----------------------------------------------------------------------------------------

/// Actions as a line: `give minecraft:apple 3; remember met=yes; command say hi`.
pub fn acts_text(list: &[Value]) -> String {
    list.iter()
        .filter_map(|a| {
            let o = a.as_object()?;
            let (k, v) = o.iter().find(|(k, _)| !matches!(k.as_str(), "count" | "is" | "vs"))?;
            let value = match v {
                Value::String(x) => x.clone(),
                Value::Bool(true) => String::new(),
                Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(","),
                other => num(other),
            };
            let mut t = if value.is_empty() { k.clone() } else { format!("{k} {value}") };
            if let Some(n) = o.get("count").map(num).filter(|n| n != "1") {
                t.push_str(&format!(" {n}"));
            }
            if let Some(is) = o.get("is").and_then(Value::as_str).filter(|x| *x != "yes") {
                t.push_str(&format!("={is}"));
            }
            if let Some(vs) = o.get("vs").and_then(Value::as_str) {
                t.push_str(&format!(" vs={vs}"));
            }
            Some(t)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// A line of actions back as JSON, checked as the game reads them.
pub fn acts_json(text: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for part in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (key, rest) = part.split_once(' ').map_or((part, ""), |(k, r)| (k, r.trim()));
        let v = match key {
            "command" => json!({ "command": rest.trim_start_matches('/') }),
            "remember" => match rest.split_once('=') {
                Some((k, x)) => json!({ "remember": k.trim(), "is": x.trim() }),
                None => json!({ "remember": rest }),
            },
            "quests" if rest.is_empty() => json!({ "quests": true }),
            "quests" => json!({ "quests": rest.split(',').map(str::trim).collect::<Vec<_>>() }),
            _ if rest.is_empty() => json!({ key: true }),
            _ => {
                let mut words = rest.split_whitespace();
                let value = words.next().unwrap_or("");
                let mut o = Map::new();
                match value.parse::<f64>() {
                    Ok(n) if matches!(key, "relation" | "trade") => o.insert(key.to_string(), json!(n as i64)),
                    _ => o.insert(key.to_string(), json!(value)),
                };
                for w in words {
                    if let Some((a, b)) = w.split_once('=') {
                        o.insert(a.to_string(), json!(b));
                    } else if let Ok(n) = w.parse::<u64>() {
                        o.insert("count".into(), json!(n));
                    } else {
                        return Err(format!("\"{part}\": what is {w}?"));
                    }
                }
                Value::Object(o)
            }
        };
        out.push(v);
    }
    crate::dialog::read_actions(Some(&Value::Array(out.clone()))).map_err(|e| format!("{e} (in \"{text}\")"))?;
    Ok(out)
}

// ---- the dialog ---------------------------------------------------------------------------------------------

fn lines_of(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// Where box `i` of `n` stands when the file doesn't say: in rows.
fn grid(i: usize) -> [f32; 2] {
    let cols = ((CANVAS[0] - 4.0) / (BOX[0] + 2.0)).floor().max(1.0) as usize;
    [4.0 + (i % cols) as f32 * (BOX[0] + 2.0), 4.0 + (i / cols) as f32 * (BOX[1] + 14.0)]
}

impl DialogEd {
    pub fn blank() -> DialogEd {
        DialogEd::from_json(&json!({ "start": "hello", "nodes": {
            "hello": { "text": [ "Hello, @player." ], "options": [ { "text": "Hi!", "goto": "bye" }, { "text": "Not now." } ] },
            "bye": { "text": [ "See you, then." ] } } }))
    }

    pub fn from_json(v: &Value) -> DialogEd {
        let raw: Vec<(String, Value)> = v.get("nodes").and_then(Value::as_object).map(|m| m.iter().map(|(k, x)| (k.clone(), x.clone())).collect()).unwrap_or_default();
        let start = s(v, "start");
        let mut names: Vec<String> = raw.iter().map(|r| r.0.clone()).collect();
        if let Some(i) = names.iter().position(|n| *n == start) {
            let first = names.remove(i);
            names.insert(0, first);
        }
        let index = |n: &str| names.iter().position(|x| x == n).unwrap_or(END);
        let goto = |v: Option<&Value>| v.and_then(Value::as_str).map_or(END, index);
        let layout = v.get("layout");
        let nodes = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let n = raw.iter().find(|r| &r.0 == name).map(|r| r.1.clone()).unwrap_or(Value::Null);
                let answers = n
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|o| {
                                let acts: Vec<Value> = o.get("actions").and_then(Value::as_array).cloned().unwrap_or_default();
                                let relation: i64 = acts.iter().filter_map(|x| x.get("relation").and_then(Value::as_f64)).map(|x| x as i64).sum();
                                let feel = acts.iter().find_map(|x| x.get("feel").and_then(Value::as_str)).unwrap_or("").to_string();
                                let rest: Vec<Value> = acts.into_iter().filter(|x| x.get("relation").is_none() && x.get("feel").is_none()).collect();
                                AnswerEd {
                                    text: s(o, "text"),
                                    cond: CondEd::from_json(o.get("if")),
                                    relation: if relation == 0 { String::new() } else { format!("{relation:+}") },
                                    feel,
                                    act: acts_text(&rest),
                                    goto: goto(o.get("goto")),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let checks: Vec<CheckEd> = n
                    .get("branch")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(|b| CheckEd { cond: CondEd::from_json(b.get("if")), goto: goto(b.get("goto")) }).collect())
                    .unwrap_or_default();
                let known = ["text", "paged", "more", "animation", "actions", "next", "options", "branch"];
                let rest = n.as_object().map(|o| o.iter().filter(|(k, _)| !known.contains(&k.as_str())).map(|(k, x)| (k.clone(), x.clone())).collect()).unwrap_or_default();
                let at = layout
                    .and_then(|l| l.get(name))
                    .and_then(Value::as_array)
                    .and_then(|a| Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32]))
                    .unwrap_or_else(|| grid(i));
                NodeEd {
                    name: name.clone(),
                    at,
                    check: !checks.is_empty(),
                    lines: lines_of(n.get("text")),
                    paged: n.get("paged").and_then(Value::as_bool).unwrap_or(false),
                    more: s(&n, "more"),
                    animation: s(&n, "animation"),
                    act: acts_text(n.get("actions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])),
                    next: goto(n.get("next")),
                    answers,
                    checks,
                    rest,
                }
            })
            .collect();
        let rest = v.as_object().map(|o| o.iter().filter(|(k, _)| !matches!(k.as_str(), "start" | "nodes" | "layout")).map(|(k, x)| (k.clone(), x.clone())).collect()).unwrap_or_default();
        DialogEd { nodes, start: 0, rest }
    }

    pub fn to_json(&self) -> Result<Value, String> {
        let mut nodes = Map::new();
        let mut layout = Map::new();
        let name_of = |i: usize| self.nodes.get(i).map(|n| n.name.trim().to_string());
        for n in &self.nodes {
            let name = n.name.trim();
            if name.is_empty() {
                return Err("a node without a name".into());
            }
            if nodes.contains_key(name) {
                return Err(format!("two nodes named {name}"));
            }
            let mut o = n.rest.clone();
            if n.check {
                let mut branch = Vec::new();
                for (i, c) in n.checks.iter().enumerate() {
                    let to = name_of(c.goto).ok_or(format!("{name}: check {} goes nowhere", i + 1))?;
                    let mut b = Map::new();
                    if let Some(cond) = c.cond.to_json().map_err(|e| format!("{name}, check {}: {e}", i + 1))? {
                        b.insert("if".into(), cond);
                    }
                    b.insert("goto".into(), json!(to));
                    branch.push(Value::Object(b));
                }
                if branch.is_empty() {
                    return Err(format!("{name}: a check with nothing to check"));
                }
                o.insert("branch".into(), json!(branch));
            } else {
                let lines: Vec<&str> = n.lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
                o.insert("text".into(), json!(lines));
                if n.paged && lines.len() > 1 {
                    o.insert("paged".into(), json!(true));
                }
                if !n.more.trim().is_empty() {
                    o.insert("more".into(), json!(n.more.trim()));
                }
                if !n.animation.trim().is_empty() {
                    o.insert("animation".into(), json!(n.animation.trim()));
                }
                let acts = acts_json(&n.act).map_err(|e| format!("{name}: {e}"))?;
                if !acts.is_empty() {
                    o.insert("actions".into(), json!(acts));
                }
                let mut options = Vec::new();
                for (i, a) in n.answers.iter().enumerate() {
                    let what = format!("{name}, answer {}", crate::dialog::letter(i));
                    if a.text.trim().is_empty() {
                        return Err(format!("{what}: no words"));
                    }
                    let mut x = Map::new();
                    x.insert("text".into(), json!(a.text.trim()));
                    if let Some(c) = a.cond.to_json().map_err(|e| format!("{what}: {e}"))? {
                        x.insert("if".into(), c);
                    }
                    let mut acts = Vec::new();
                    if !a.relation.trim().is_empty() {
                        let r: f64 = a.relation.trim().trim_start_matches('+').parse().map_err(|_| format!("{what}: relation is a number, as +5 or -5"))?;
                        acts.push(json!({ "relation": r as i64 }));
                    }
                    if !a.feel.trim().is_empty() {
                        acts.push(json!({ "feel": a.feel.trim() }));
                    }
                    acts.extend(acts_json(&a.act).map_err(|e| format!("{what}: {e}"))?);
                    if !acts.is_empty() {
                        x.insert("actions".into(), json!(acts));
                    }
                    if let Some(g) = name_of(a.goto) {
                        x.insert("goto".into(), json!(g));
                    }
                    options.push(Value::Object(x));
                }
                if !options.is_empty() {
                    o.insert("options".into(), json!(options));
                }
            }
            if let Some(g) = name_of(n.next) {
                o.insert("next".into(), json!(g));
            }
            nodes.insert(name.to_string(), Value::Object(o));
            layout.insert(name.to_string(), json!([n.at[0].round(), n.at[1].round()]));
        }
        let mut v = self.rest.clone();
        v.insert("start".into(), json!(name_of(self.start).ok_or("no start node")?));
        v.insert("nodes".into(), Value::Object(nodes));
        v.insert("layout".into(), Value::Object(layout));
        let v = Value::Object(v);
        crate::dialog::parse(&crate::serde_json::to_vec(&v).unwrap_or_default())?;
        Ok(v)
    }

    /// A new node near the middle (a talk, or a check); its index.
    pub fn add_node(&mut self, check: bool) -> usize {
        let base = if check { "check" } else { "talk" };
        let mut k = self.nodes.len() + 1;
        while self.nodes.iter().any(|n| n.name == format!("{base}{k}")) {
            k += 1;
        }
        let i = self.nodes.len();
        let at = [((CANVAS[0] - BOX[0]) / 2.0 + (i % 3) as f32 * 6.0).round(), ((CANVAS[1] - BOX[1]) / 2.0 + (i % 4) as f32 * 6.0).round()];
        let mut n = NodeEd { name: format!("{base}{k}"), at, check, next: END, ..NodeEd::default() };
        if check {
            n.checks.push(CheckEd { cond: CondEd { kind: 1, value: "hello=a".into(), ..CondEd::default() }, goto: END });
        } else {
            n.lines.push(String::new());
        }
        self.nodes.push(n);
        i
    }

    /// Takes node `i` away; what went to it ends instead; the start stays a node.
    pub fn remove_node(&mut self, i: usize) {
        if self.nodes.len() <= 1 || i >= self.nodes.len() {
            return;
        }
        self.nodes.remove(i);
        let fix = |g: &mut usize| {
            if *g == i {
                *g = END;
            } else if *g != END && *g > i {
                *g -= 1;
            }
        };
        for n in &mut self.nodes {
            fix(&mut n.next);
            for a in &mut n.answers {
                fix(&mut a.goto);
            }
            for c in &mut n.checks {
                fix(&mut c.goto);
            }
        }
        if self.start == i {
            self.start = 0;
        } else if self.start > i {
            self.start -= 1;
        }
    }

    /// The node whose box is at `p` on the canvas (the one on top).
    pub fn node_at(&self, p: [f32; 2]) -> Option<usize> {
        self.nodes.iter().rposition(|n| p[0] >= n.at[0] && p[0] < n.at[0] + BOX[0] && p[1] >= n.at[1] && p[1] < n.at[1] + BOX[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialog_goes_round() {
        let v: Value = crate::serde_json::from_str(r#"{ "start": "a", "nodes": {
            "a": { "text": ["One.", "Two."], "paged": true, "next": "q", "animation": "animation.felix.wave" },
            "q": { "text": "Yes?", "options": [
                { "text": "Yes", "if": { "not": { "answer": "q", "is": "b" } }, "actions": [ { "relation": 5 }, { "feel": "praise" }, { "give": "minecraft:apple", "count": 3 }, { "remember": "said", "is": "yes" } ], "goto": "c" },
                { "text": "No", "if": { "all": [ { "tag": "x" }, { "tag": "y" } ] }, "actions": [ { "relation": -5 } ] } ] },
            "c": { "branch": [ { "if": { "biome": "snowy" }, "goto": "a" }, { "goto": "q" } ], "next": "q" } } }"#).unwrap();
        let ed = DialogEd::from_json(&v);
        assert_eq!(ed.nodes[0].name, "a");
        let by = |n: &str| ed.nodes.iter().find(|x| x.name == n).unwrap();
        let q = by("q");
        assert_eq!((q.answers[0].relation.as_str(), q.answers[0].feel.as_str()), ("+5", "praise"));
        assert_eq!(q.answers[0].act, "give minecraft:apple 3; remember said");
        assert!(q.answers[0].cond.not && q.answers[0].cond.kind == 1 && q.answers[0].cond.value == "q=b");
        assert_eq!(q.answers[1].cond.kind, CUSTOM);
        assert!(by("c").check && by("c").checks[1].cond.kind == 0);
        let back = ed.to_json().unwrap();
        assert_eq!(DialogEd::from_json(&back), ed);
        let d = crate::dialog::parse(&crate::serde_json::to_vec(&back).unwrap()).unwrap();
        assert_eq!(d.nodes["q"].options[0].actions.len(), 4);
        assert!(d.nodes["a"].paged);
    }

    #[test]
    fn mistakes_are_named() {
        let mut ed = DialogEd::blank();
        ed.nodes[0].answers[0].relation = "lots".into();
        assert!(ed.to_json().unwrap_err().contains("relation"));
        let mut ed = DialogEd::blank();
        ed.nodes[0].answers[0].act = "fly away".into();
        assert!(ed.to_json().is_err());
        let mut ed = DialogEd::blank();
        let c = ed.add_node(true);
        assert!(ed.to_json().unwrap_err().contains("goes nowhere"));
        ed.nodes[c].checks[0].goto = 0;
        ed.nodes[c].checks[0].cond.kind = 10;
        ed.nodes[c].checks[0].cond.value = "noon".into();
        assert!(ed.to_json().unwrap_err().contains("time"));
        ed.nodes[c].checks[0].cond.value = "night".into();
        assert!(ed.to_json().is_ok());
        ed.remove_node(0);
        assert_eq!(ed.nodes[ed.nodes.len() - 1].checks[0].goto, END);
    }

    #[test]
    fn acts_read_back() {
        let t = "give minecraft:apple 3; remember met=no; command say hi there; quests; order follow";
        let j = acts_json(t).unwrap();
        assert_eq!(j[1], json!({ "remember": "met", "is": "no" }));
        assert_eq!(j[2], json!({ "command": "say hi there" }));
        assert_eq!(acts_text(&j), t);
    }
}
