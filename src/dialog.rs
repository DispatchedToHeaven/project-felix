//! Dialogs: trees of what an NPC says and what the player can answer, read from data packs.
//!
//! A file is `data/<namespace>/felix_dialogs/<name>.json`, the dialog `<namespace>:<name>`:
//!
//! ```json
//! { "start": "hello",
//!   "nodes": {
//!     "hello": {
//!       "text": ["Hi, @player!", "Nice weather."],
//!       "animation": "animation.felix.wave",
//!       "actions": [ { "sound": "minecraft:entity.villager.yes" } ],
//!       "options": [
//!         { "text": "A gift?", "if": { "not": { "tag": "felix_gift" } },
//!           "actions": [ { "give": "minecraft:apple", "count": 3 }, { "tag": "felix_gift" } ],
//!           "goto": "thanks" },
//!         { "text": "Follow me", "if": { "owner": true }, "actions": [ { "order": "follow" } ] },
//!         { "text": "Bye" } ] } } }
//! ```
//!
//! A node's `actions` run when it is shown; an option's when it is picked, then the dialog goes
//! to its `goto` node, or ends without one. Options whose `if` fails are not shown. Conditions:
//! `tag`, `item` (+ `count`), `owner`, `mode` (`follow`, `wait`, `guard`), `game` (the NPC's table:
//! `none`, `waiting`, `playing`, `over`), `in_game` (the player sits at it), `mood` (the NPC's, as
//! `upset`), `relation_above` / `relation_below` (how much it likes the player), `doing` (what its
//! day has it do: `work`, `sleep`…), `routine` (it has one), `met` (it has talked with the player before), `remembers` (news of
//! the player and it: `gift`, `helped`, `hurt`, `let_down`…), `fault` (the player is the one who
//! upset it, or no one in particular did), `event` (it is at an event: its id, or `any`), `not`,
//! `all`, `any`. Text may name `@upset_by` and `@event`.
//! Actions: `command` (run as the server console), `give` / `take` (+ `count`), `tag`, `untag`,
//! `animation` (played on the NPC), `order` (`follow`, `wait`, `guard`), `sound` (at the NPC),
//! `scene` (a scene from `felix_scenes`, played by the NPC), `game` (`chess` or `checkers`, with
//! `vs`: `npc` or `player`), `join_game` (take the free seat), `open_game` (the board, to play or watch),
//! `sport` (`table_tennis`, `tennis` or `basketball`, against the NPC), `edit_animation` (opens the
//! animation editor on the NPC, with an animation name or `animation.<model>.custom`), `feel` (an
//! event for its character: `gift`, `apology`, `praise`, `scold`…), `relation` (+/- how much it likes the player), `news` (it tells, aloud, the newest
//! thing it saw or heard a player do), `quests` (its quest board: a list of quest ids, or `true`
//! for its own), `quest` (the player takes that quest), `hand_in`, `give_up` (the quest for this NPC),
//! `place` (its place of that name, `home`, `work`…, is where the player stands), `shop` (its stall: a
//! shop id, or `true` for its own), `gear` (`take`: what the player holds; `return`: all of it
//! back), `stats` (it says its level, health, armour and hits), `mind` (it says how it feels), `skills` (its past and what it is good at).
//! `remember` (+ `is`, default `yes`: the player's memory, for later talks and other NPCs),
//! `forget`, `dialog` (goes on in another dialog, from its start), `feed` (the player gives it the
//! best food they have, see `need`), `family` (it tells the player about its family, see
//! `family`). Conditions `hungry` and `family` (it has one: true or false).
//! Bonds (see `bond`): condition `stage` (the player is at least that: `acquaintance`, `friend`,
//! `close`, `sweetheart`, `partner`) and `romance` (`none`, `sweetheart`, `partner`); actions
//! `gift` (the player gives it what they hold), `tastes` (it says what it loves), `ask_out`,
//! `propose` (with a diamond), `part` (the player ends the romance), `reminisce` (it brings up a
//! memory of them: when they met, fights, lands, a gift). Quests: conditions `quest_done` and
//! `quest_active` (a quest id).
//! `@player` is the player's name; `@npc` is the NPC's name in texts and its UUID in commands.
//!
//! Long talks: a node with `"paged": true` says its `text` a line at a time, the player going on
//! with `more` (default `...`); one without options goes on to `next` (or ends). A node with
//! `branch` says nothing: the first branch whose `if` holds (none: always) is where the talk
//! goes, else `next`. Every answer picked in a node of two or more is remembered, so a later
//! `answer` condition (`{ "answer": "question", "is": "b" }`: answers count a, b, c… as written) can
//! ask what the player said. More conditions: `flag` (+ `is`: what the player's memory holds),
//! `biome` (the player's: an id, or a word in it, as `snowy`), `time` (`day`, `dusk`,
//! `night`, `dawn`), `weather` (`clear`, `rain`, `thunder`), `chance` (percent).
//! No mod API types here.

use crate::serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub start: String,
    pub nodes: HashMap<String, Node>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Node {
    pub text: Vec<String>,
    pub animation: Option<String>,
    pub actions: Vec<Action>,
    pub options: Vec<Choice>,
    /// Where it goes on without an answer (no options), or when no branch holds.
    pub next: Option<String>,
    /// Its text a line at a time.
    pub paged: bool,
    /// What the button to go on says.
    pub more: Option<String>,
    /// A node that says nothing: the first that holds is where the talk goes.
    pub branch: Vec<(Option<Cond>, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub text: String,
    pub condition: Option<Cond>,
    pub actions: Vec<Action>,
    pub goto: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cond {
    Tag(String),
    Item(String, u32),
    Owner(bool),
    Mode(String),
    Game(String),
    InGame(bool),
    Mood(String),
    RelationAbove(i32),
    RelationBelow(i32),
    Doing(String),
    Routine(bool),
    Met(bool),
    /// Whether the player is the one who upset the NPC (or no one in particular did).
    Fault(bool),
    /// The NPC is at an event: this one (its id or name), or `any`.
    Event(String),
    Remembers(String),
    /// What the player's memory holds under a key (any value, or this one).
    Flag(String, Option<String>),
    /// What the player answered at a node of this dialog (a, b, c…).
    Answer(String, String),
    Biome(String),
    Time(String),
    Weather(String),
    /// Percent.
    Chance(u32),
    Hungry(bool),
    Family(bool),
    /// The player is at least at this stage with it.
    Stage(String),
    Romance(String),
    /// The player did that quest (for anyone), or has it under way.
    QuestDone(String),
    QuestActive(String),
    Not(Box<Cond>),
    All(Vec<Cond>),
    Any(Vec<Cond>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Command(String),
    Give(String, u32),
    Take(String, u32),
    Tag(String),
    Untag(String),
    Animation(String),
    Order(String),
    Sound(String),
    Scene(String),
    Game(String, bool),
    Sport(String),
    EditAnimation(Option<String>),
    JoinGame,
    OpenGame,
    Feel(String),
    Relation(i32),
    News,
    Quests(Vec<String>),
    Quest(String),
    HandIn,
    GiveUp,
    Place(String),
    Shop(String),
    Trade(i32),
    Gear(String),
    Stats,
    /// It says how it feels: its mood, what is on its mind, its traits.
    Mind,
    /// It tells its past and what it is good at.
    Skills,
    Remember(String, String),
    Forget(String),
    Dialog(String),
    Feed,
    Family,
    Gift,
    Tastes,
    AskOut,
    Propose,
    Part,
    Reminisce,
}

/// What conditions ask about the player and the NPC.
pub trait Facts {
    fn has_tag(&self, tag: &str) -> bool;
    fn item_count(&self, item: &str) -> u32;
    fn is_owner(&self) -> bool;
    fn mode(&self) -> &str;
    /// The NPC's table: `none`, `waiting`, `playing` or `over`.
    fn game(&self) -> &str {
        "none"
    }
    fn in_game(&self) -> bool {
        false
    }
    /// The NPC's mood.
    fn mood(&self) -> &str {
        "calm"
    }
    /// How much the NPC likes the player.
    fn relation(&self) -> i32 {
        0
    }
    /// What its day has it do.
    fn doing(&self) -> &str {
        ""
    }
    fn has_routine(&self) -> bool {
        false
    }
    /// Whether it talked with the player before.
    fn met(&self) -> bool {
        false
    }
    /// Whether it has news of `kind` about the player.
    fn remembers(&self, _kind: &str) -> bool {
        false
    }
    /// Whether the player brought on its mood (or no one in particular did).
    fn fault(&self) -> bool {
        true
    }
    /// The event the NPC is at (its id; empty: none).
    fn event(&self) -> &str {
        ""
    }
    /// What the player's memory holds under `key`.
    fn flag(&self, _key: &str) -> Option<&str> {
        None
    }
    /// What the player answered at node `node` of the dialog going on.
    fn answer(&self, _node: &str) -> Option<&str> {
        None
    }
    /// The biome the player is in.
    fn biome(&self) -> &str {
        ""
    }
    /// `day`, `dusk`, `night` or `dawn`.
    fn time_of_day(&self) -> &str {
        "day"
    }
    /// `clear`, `rain` or `thunder`.
    fn weather(&self) -> &str {
        "clear"
    }
    /// A roll from 0 to 99.
    fn roll(&self) -> u32 {
        0
    }
    /// Whether the NPC is hungry.
    fn hungry(&self) -> bool {
        false
    }
    /// Whether the NPC has a family (a partner, children, parents).
    fn has_family(&self) -> bool {
        false
    }
    /// Where the player stands with it (see `bond`): its rank, from 0 (a stranger).
    fn stage(&self) -> u8 {
        0
    }
    /// `none`, `sweetheart` or `partner`.
    fn romance(&self) -> &str {
        "none"
    }
    fn quest_done(&self, _id: &str) -> bool {
        false
    }
    fn quest_active(&self, _id: &str) -> bool {
        false
    }
}

/// A stage's rank, from 0 (a stranger) to 5 (partners).
pub fn stage_rank(s: &str) -> Option<u8> {
    ["stranger", "acquaintance", "friend", "close", "sweetheart", "partner"].iter().position(|x| *x == s.trim()).map(|i| i as u8)
}

/// Whether biome `id` is `want`: the same id, or (a word) a part of its name.
pub fn biome_is(id: &str, want: &str) -> bool {
    let want = want.trim();
    if want.is_empty() || id.is_empty() {
        return false;
    }
    if want.contains(':') { id == want } else { id.rsplit(':').next().unwrap_or(id).contains(want) }
}

impl Cond {
    pub fn holds(&self, f: &dyn Facts) -> bool {
        match self {
            Cond::Tag(t) => f.has_tag(t),
            Cond::Item(item, n) => f.item_count(item) >= *n,
            Cond::Owner(want) => f.is_owner() == *want,
            Cond::Mode(m) => f.mode() == m,
            Cond::Game(g) => f.game() == g,
            Cond::InGame(want) => f.in_game() == *want,
            Cond::Mood(m) => f.mood() == m,
            Cond::RelationAbove(v) => f.relation() > *v,
            Cond::RelationBelow(v) => f.relation() < *v,
            Cond::Doing(d) => f.doing() == d,
            Cond::Routine(want) => f.has_routine() == *want,
            Cond::Met(want) => f.met() == *want,
            Cond::Fault(want) => f.fault() == *want,
            Cond::Event(e) => {
                let at = f.event();
                !at.is_empty() && (e == "any" || at == e || at.rsplit(':').next() == Some(e.as_str()))
            }
            Cond::Remembers(k) => f.remembers(k),
            Cond::Flag(k, v) => f.flag(k).is_some_and(|x| v.as_deref().is_none_or(|v| x == v)),
            Cond::Answer(n, a) => f.answer(n) == Some(a.as_str()),
            Cond::Biome(b) => biome_is(f.biome(), b),
            Cond::Time(t) => f.time_of_day() == t,
            Cond::Weather(w) => match w.as_str() {
                "rain" => f.weather() != "clear",
                other => f.weather() == other,
            },
            Cond::Chance(p) => f.roll() < *p,
            Cond::Hungry(want) => f.hungry() == *want,
            Cond::Family(want) => f.has_family() == *want,
            Cond::Stage(s) => stage_rank(s).is_some_and(|r| f.stage() >= r),
            Cond::Romance(r) => f.romance() == r,
            Cond::QuestDone(q) => f.quest_done(q),
            Cond::QuestActive(q) => f.quest_active(q),
            Cond::Not(c) => !c.holds(f),
            Cond::All(list) => list.iter().all(|c| c.holds(f)),
            Cond::Any(list) => list.iter().any(|c| c.holds(f)),
        }
    }
}

impl Choice {
    pub fn shown(&self, f: &dyn Facts) -> bool {
        self.condition.as_ref().is_none_or(|c| c.holds(f))
    }
}

/// The letter of answer `i` (a, b, c…).
pub fn letter(i: usize) -> String {
    char::from(b'a' + (i.min(25)) as u8).to_string()
}

/// Where branch node `n` sends the talk.
pub fn route(n: &Node, f: &dyn Facts) -> Option<String> {
    n.branch.iter().find(|(c, _)| c.as_ref().is_none_or(|c| c.holds(f))).map(|b| b.1.clone()).or_else(|| n.next.clone())
}

/// A condition in words, for those who make dialogs.
pub fn describe_cond(c: &Cond) -> String {
    match c {
        Cond::Tag(t) => format!("tag {t}"),
        Cond::Item(i, n) => format!("has {n} {}", i.trim_start_matches("minecraft:")),
        Cond::Owner(b) => if *b { "owner".into() } else { "not owner".into() },
        Cond::Mode(m) => format!("mode {m}"),
        Cond::Game(g) => format!("game {g}"),
        Cond::InGame(b) => format!("in game {b}"),
        Cond::Mood(m) => format!("mood {m}"),
        Cond::RelationAbove(v) => format!("rel > {v}"),
        Cond::RelationBelow(v) => format!("rel < {v}"),
        Cond::Doing(d) => format!("doing {d}"),
        Cond::Routine(b) => format!("routine {b}"),
        Cond::Met(b) => if *b { "met".into() } else { "not met".into() },
        Cond::Fault(b) => format!("fault {b}"),
        Cond::Event(e) => format!("event {e}"),
        Cond::Remembers(k) => format!("remembers {k}"),
        Cond::Flag(k, v) => match v {
            Some(v) => format!("{k}={v}"),
            None => format!("{k} set"),
        },
        Cond::Answer(n, a) => format!("answered {a} at {n}"),
        Cond::Biome(b) => format!("biome {b}"),
        Cond::Time(t) => t.clone(),
        Cond::Weather(w) => w.clone(),
        Cond::Chance(p) => format!("{p}%"),
        Cond::Hungry(b) => if *b { "hungry".into() } else { "not hungry".into() },
        Cond::Family(b) => if *b { "has family".into() } else { "no family".into() },
        Cond::Stage(s) => format!("{s}+"),
        Cond::Romance(r) => format!("romance {r}"),
        Cond::QuestDone(q) => format!("did {q}"),
        Cond::QuestActive(q) => format!("doing {q}"),
        Cond::Not(c) => format!("not ({})", describe_cond(c)),
        Cond::All(l) => l.iter().map(describe_cond).collect::<Vec<_>>().join(" and "),
        Cond::Any(l) => l.iter().map(describe_cond).collect::<Vec<_>>().join(" or "),
    }
}

/// What an answer does that the player doesn't see, in short: for the extended view.
pub fn describe(c: &Choice) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(cond) = &c.condition {
        parts.push(format!("if {}", describe_cond(cond)));
    }
    for a in &c.actions {
        parts.push(match a {
            Action::Relation(d) => format!("rel {d:+}"),
            Action::Feel(e) => format!("feel {e}"),
            Action::Remember(k, v) => format!("{k}={v}"),
            Action::Forget(k) => format!("forget {k}"),
            Action::Give(i, n) => format!("give {n} {}", i.trim_start_matches("minecraft:")),
            Action::Take(i, n) => format!("take {n} {}", i.trim_start_matches("minecraft:")),
            Action::Dialog(d) => format!("to {d}"),
            Action::Quest(q) => format!("quest {q}"),
            Action::Tag(t) => format!("tag {t}"),
            Action::Command(c) => format!("/{}", c.trim_start_matches('/')),
            other => format!("{other:?}").to_lowercase(),
        });
    }
    if let Some(g) = &c.goto {
        parts.push(format!("> {g}"));
    }
    parts.join(", ")
}

/// `@player` and `@npc` filled in.
pub fn fill(text: &str, player: &str, npc: &str) -> String {
    text.replace("@player", player).replace("@npc", npc)
}

/// The command lines an action stands for (the ones the server runs as its console).
pub fn commands(action: &Action, player: &str, npc: &str) -> Vec<String> {
    match action {
        Action::Command(c) => vec![fill(c.trim_start_matches('/'), player, npc)],
        Action::Give(item, n) => vec![format!("give {player} {item} {n}")],
        Action::Take(item, n) => vec![format!("clear {player} {item} {n}")],
        Action::Tag(t) => vec![format!("tag {player} add {t}")],
        Action::Untag(t) => vec![format!("tag {player} remove {t}")],
        Action::Sound(s) => vec![format!("execute as {npc} at @s run playsound {s} neutral @a ~ ~ ~")],
        Action::Animation(_) | Action::Order(_) | Action::Scene(_) | Action::Game(..) | Action::Sport(_) | Action::EditAnimation(_) | Action::JoinGame | Action::OpenGame | Action::Feel(_) | Action::Relation(_) | Action::News | Action::Quests(_) | Action::Quest(_) | Action::HandIn | Action::GiveUp | Action::Place(_) | Action::Shop(_) | Action::Trade(_) | Action::Gear(_) | Action::Stats | Action::Mind | Action::Skills | Action::Remember(..) | Action::Forget(_) | Action::Dialog(_) | Action::Feed | Action::Family | Action::Gift | Action::Tastes | Action::AskOut | Action::Propose | Action::Part | Action::Reminisce => Vec::new(),
    }
}

pub fn parse(bytes: &[u8]) -> Result<Dialog, String> {
    let root: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    let root = root.as_object().ok_or("not an object")?;
    let start = root.get("start").and_then(Value::as_str).ok_or("no \"start\"")?.to_string();
    let raw = root.get("nodes").and_then(Value::as_object).ok_or("no \"nodes\"")?;
    let mut nodes = HashMap::new();
    for (name, n) in raw {
        let n = n.as_object().ok_or_else(|| format!("node {name} is not an object"))?;
        nodes.insert(name.clone(), read_node(n).map_err(|e| format!("node {name}: {e}"))?);
    }
    if !nodes.contains_key(&start) {
        return Err(format!("start node {start} is missing"));
    }
    for (name, n) in &nodes {
        for o in &n.options {
            if let Some(g) = &o.goto
                && !nodes.contains_key(g)
            {
                return Err(format!("node {name}: option {:?} goes to missing node {g}", o.text));
            }
        }
        for g in n.next.iter().chain(n.branch.iter().map(|b| &b.1)) {
            if !nodes.contains_key(g) {
                return Err(format!("node {name} goes on to missing node {g}"));
            }
        }
    }
    Ok(Dialog { start, nodes })
}

fn texts(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

fn read_node(n: &Map<String, Value>) -> Result<Node, String> {
    let mut options = Vec::new();
    if let Some(list) = n.get("options").and_then(Value::as_array) {
        for (i, o) in list.iter().enumerate() {
            let o = o.as_object().ok_or_else(|| format!("option {i} is not an object"))?;
            options.push(Choice {
                text: o.get("text").and_then(Value::as_str).ok_or_else(|| format!("option {i} has no text"))?.to_string(),
                condition: o.get("if").map(read_cond).transpose().map_err(|e| format!("option {i}: {e}"))?,
                actions: read_actions(o.get("actions")).map_err(|e| format!("option {i}: {e}"))?,
                goto: o.get("goto").and_then(Value::as_str).map(str::to_string),
            });
        }
    }
    let mut branch = Vec::new();
    if let Some(list) = n.get("branch").and_then(Value::as_array) {
        for (i, b) in list.iter().enumerate() {
            let b = b.as_object().ok_or_else(|| format!("branch {i} is not an object"))?;
            let goto = b.get("goto").and_then(Value::as_str).ok_or_else(|| format!("branch {i} has no goto"))?.to_string();
            branch.push((b.get("if").map(read_cond).transpose().map_err(|e| format!("branch {i}: {e}"))?, goto));
        }
    }
    Ok(Node {
        text: texts(n.get("text")),
        animation: n.get("animation").and_then(Value::as_str).map(str::to_string),
        actions: read_actions(n.get("actions"))?,
        options,
        next: n.get("next").and_then(Value::as_str).map(str::to_string),
        paged: n.get("paged").and_then(Value::as_bool).unwrap_or(false),
        more: n.get("more").and_then(Value::as_str).map(str::to_string),
        branch,
    })
}

pub fn read_cond(v: &Value) -> Result<Cond, String> {
    let m = v.as_object().ok_or("a condition is not an object")?;
    let count = m.get("count").and_then(Value::as_u64).unwrap_or(1) as u32;
    let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
    let list = |k: &str| -> Result<Option<Vec<Cond>>, String> {
        match m.get(k) {
            Some(Value::Array(a)) => Ok(Some(a.iter().map(read_cond).collect::<Result<_, _>>()?)),
            Some(_) => Err(format!("{k} is not a list")),
            None => Ok(None),
        }
    };
    if let Some(t) = s("tag") {
        Ok(Cond::Tag(t))
    } else if let Some(i) = s("item") {
        Ok(Cond::Item(i, count))
    } else if let Some(b) = m.get("owner").and_then(Value::as_bool) {
        Ok(Cond::Owner(b))
    } else if let Some(mode) = s("mode") {
        Ok(Cond::Mode(mode))
    } else if let Some(g) = s("game") {
        Ok(Cond::Game(g))
    } else if let Some(b) = m.get("in_game").and_then(Value::as_bool) {
        Ok(Cond::InGame(b))
    } else if let Some(mood) = s("mood") {
        Ok(Cond::Mood(mood))
    } else if let Some(v) = m.get("relation_above").and_then(Value::as_f64) {
        Ok(Cond::RelationAbove(v as i32))
    } else if let Some(v) = m.get("relation_below").and_then(Value::as_f64) {
        Ok(Cond::RelationBelow(v as i32))
    } else if let Some(d) = s("doing") {
        Ok(Cond::Doing(d))
    } else if let Some(b) = m.get("routine").and_then(Value::as_bool) {
        Ok(Cond::Routine(b))
    } else if let Some(b) = m.get("met").and_then(Value::as_bool) {
        Ok(Cond::Met(b))
    } else if let Some(e) = s("event") {
        Ok(Cond::Event(e))
    } else if let Some(b) = m.get("fault").and_then(Value::as_bool) {
        Ok(Cond::Fault(b))
    } else if let Some(k) = s("remembers") {
        Ok(Cond::Remembers(k))
    } else if let Some(k) = s("flag") {
        Ok(Cond::Flag(k, s("is")))
    } else if let Some(n) = s("answer") {
        Ok(Cond::Answer(n, s("is").ok_or("answer needs \"is\": a, b, c…")?))
    } else if let Some(b) = s("biome") {
        Ok(Cond::Biome(b))
    } else if let Some(t) = s("time") {
        Ok(Cond::Time(t))
    } else if let Some(w) = s("weather") {
        Ok(Cond::Weather(w))
    } else if let Some(p) = m.get("chance").and_then(Value::as_f64) {
        Ok(Cond::Chance(p.clamp(0.0, 100.0) as u32))
    } else if let Some(b) = m.get("hungry").and_then(Value::as_bool) {
        Ok(Cond::Hungry(b))
    } else if let Some(b) = m.get("family").and_then(Value::as_bool) {
        Ok(Cond::Family(b))
    } else if let Some(st) = s("stage") {
        stage_rank(&st).ok_or(format!("stage is stranger, acquaintance, friend, close, sweetheart or partner, not {st}"))?;
        Ok(Cond::Stage(st))
    } else if let Some(r) = s("romance") {
        Ok(Cond::Romance(r))
    } else if let Some(q) = s("quest_done") {
        Ok(Cond::QuestDone(q))
    } else if let Some(q) = s("quest_active") {
        Ok(Cond::QuestActive(q))
    } else if let Some(n) = m.get("not") {
        Ok(Cond::Not(Box::new(read_cond(n)?)))
    } else if let Some(all) = list("all")? {
        Ok(Cond::All(all))
    } else if let Some(any) = list("any")? {
        Ok(Cond::Any(any))
    } else {
        Err(format!("unknown condition {v}"))
    }
}

pub fn read_actions(v: Option<&Value>) -> Result<Vec<Action>, String> {
    let Some(v) = v else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or("actions is not a list")?;
    list.iter()
        .map(|a| {
            let m = a.as_object().ok_or("an action is not an object")?;
            let count = m.get("count").and_then(Value::as_u64).unwrap_or(1) as u32;
            let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
            Ok(if let Some(c) = s("command") {
                Action::Command(c)
            } else if let Some(i) = s("give") {
                Action::Give(i, count)
            } else if let Some(i) = s("take") {
                Action::Take(i, count)
            } else if let Some(t) = s("tag") {
                Action::Tag(t)
            } else if let Some(t) = s("untag") {
                Action::Untag(t)
            } else if let Some(a) = s("animation") {
                Action::Animation(a)
            } else if let Some(o) = s("order") {
                Action::Order(o)
            } else if let Some(snd) = s("sound") {
                Action::Sound(snd)
            } else if let Some(sc) = s("scene") {
                Action::Scene(sc)
            } else if let Some(g) = s("game") {
                Action::Game(g, m.get("vs").and_then(Value::as_str) != Some("player"))
            } else if let Some(sp) = s("sport") {
                Action::Sport(sp)
            } else if let Some(v) = m.get("edit_animation") {
                Action::EditAnimation(v.as_str().map(str::to_string))
            } else if m.contains_key("join_game") {
                Action::JoinGame
            } else if m.contains_key("open_game") {
                Action::OpenGame
            } else if let Some(q) = m.get("quests") {
                Action::Quests(q.as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default())
            } else if let Some(q) = s("quest") {
                Action::Quest(q)
            } else if let Some(g) = s("gear") {
                Action::Gear(g)
            } else if m.contains_key("stats") {
                Action::Stats
            } else if m.contains_key("mind") {
                Action::Mind
            } else if m.contains_key("skills") {
                Action::Skills
            } else if let Some(sh) = m.get("shop") {
                Action::Shop(sh.as_str().unwrap_or("").to_string())
            } else if let Some(t) = m.get("trade").and_then(Value::as_i64) {
                Action::Trade(t as i32)
            } else if let Some(p) = s("place") {
                Action::Place(p)
            } else if m.contains_key("hand_in") {
                Action::HandIn
            } else if m.contains_key("give_up") {
                Action::GiveUp
            } else if m.contains_key("news") {
                Action::News
            } else if let Some(f) = s("feel") {
                Action::Feel(f)
            } else if let Some(v) = m.get("relation").and_then(Value::as_f64) {
                Action::Relation(v as i32)
            } else if let Some(k) = s("remember") {
                Action::Remember(k, s("is").unwrap_or_else(|| "yes".into()))
            } else if let Some(k) = s("forget") {
                Action::Forget(k)
            } else if let Some(d) = s("dialog") {
                Action::Dialog(d)
            } else if m.contains_key("feed") {
                Action::Feed
            } else if m.contains_key("family") {
                Action::Family
            } else if m.contains_key("gift") {
                Action::Gift
            } else if m.contains_key("tastes") {
                Action::Tastes
            } else if m.contains_key("ask_out") {
                Action::AskOut
            } else if m.contains_key("propose") {
                Action::Propose
            } else if m.contains_key("part") {
                Action::Part
            } else if m.contains_key("reminisce") {
                Action::Reminisce
            } else {
                return Err(format!("unknown action {a}"));
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct F {
        tags: Vec<&'static str>,
        apples: u32,
        owner: bool,
    }
    impl Facts for F {
        fn has_tag(&self, t: &str) -> bool {
            self.tags.contains(&t)
        }
        fn item_count(&self, item: &str) -> u32 {
            if item == "minecraft:apple" { self.apples } else { 0 }
        }
        fn is_owner(&self) -> bool {
            self.owner
        }
        fn mode(&self) -> &str {
            "follow"
        }
    }

    const SAMPLE: &str = r#"{ "start": "hello", "nodes": {
        "hello": { "text": "Hi, @player!", "animation": "animation.felix.wave",
          "options": [
            { "text": "A gift?", "if": { "not": { "tag": "gift" } }, "actions": [ { "give": "minecraft:apple", "count": 3 }, { "tag": "gift" } ], "goto": "thanks" },
            { "text": "Trade", "if": { "all": [ { "item": "minecraft:apple", "count": 2 }, { "owner": true } ] }, "actions": [ { "take": "minecraft:apple", "count": 2 } ] },
            { "text": "Bye" } ] },
        "thanks": { "text": ["Here.", "Enjoy."], "options": [ { "text": "Thanks", "if": { "any": [ { "mode": "wait" }, { "tag": "gift" } ] } } ] } } }"#;

    #[test]
    fn reads_and_filters() {
        let d = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(d.start, "hello");
        let hello = &d.nodes["hello"];
        assert_eq!(hello.animation.as_deref(), Some("animation.felix.wave"));
        let new = F { tags: vec![], apples: 0, owner: false };
        let shown: Vec<&str> = hello.options.iter().filter(|o| o.shown(&new)).map(|o| o.text.as_str()).collect();
        assert_eq!(shown, ["A gift?", "Bye"]);
        let rich = F { tags: vec!["gift"], apples: 5, owner: true };
        let shown: Vec<&str> = hello.options.iter().filter(|o| o.shown(&rich)).map(|o| o.text.as_str()).collect();
        assert_eq!(shown, ["Trade", "Bye"]);
        assert_eq!(d.nodes["thanks"].text, ["Here.", "Enjoy."]);
        assert!(d.nodes["thanks"].options[0].shown(&rich));
    }

    #[test]
    fn actions_become_commands() {
        let d = parse(SAMPLE.as_bytes()).unwrap();
        let gift = &d.nodes["hello"].options[0];
        let lines: Vec<String> = gift.actions.iter().flat_map(|a| commands(a, "Alex", "1-2-3-4-5")).collect();
        assert_eq!(lines, ["give Alex minecraft:apple 3", "tag Alex add gift"]);
        assert_eq!(fill("Hi, @player!", "Alex", "x"), "Hi, Alex!");
        assert_eq!(commands(&Action::Command("/say @player meets @npc".into()), "Alex", "u"), ["say Alex meets u"]);
    }

    #[test]
    fn long_talks_branch() {
        let d = parse(br#"{ "start": "a", "nodes": {
            "a": { "text": ["One.", "Two."], "paged": true, "next": "q" },
            "q": { "text": "Yes or no?", "options": [ { "text": "Yes", "actions": [ { "relation": -5 }, { "remember": "said" } ], "goto": "check" }, { "text": "No", "goto": "check" } ] },
            "check": { "branch": [ { "if": { "answer": "q", "is": "b" }, "goto": "no" }, { "if": { "biome": "snowy" }, "goto": "cold" } ], "next": "yes" },
            "yes": { "text": "Good." }, "no": { "text": "Shame." }, "cold": { "text": "Brr." } } }"#).unwrap();
        assert!(d.nodes["a"].paged && d.nodes["a"].next.as_deref() == Some("q"));
        struct G(Option<&'static str>, &'static str);
        impl Facts for G {
            fn has_tag(&self, _: &str) -> bool { false }
            fn item_count(&self, _: &str) -> u32 { 0 }
            fn is_owner(&self) -> bool { false }
            fn mode(&self) -> &str { "wait" }
            fn answer(&self, n: &str) -> Option<&str> { if n == "q" { self.0 } else { None } }
            fn biome(&self) -> &str { self.1 }
        }
        let check = &d.nodes["check"];
        assert_eq!(route(check, &G(Some("b"), "")).as_deref(), Some("no"));
        assert_eq!(route(check, &G(Some("a"), "minecraft:snowy_taiga")).as_deref(), Some("cold"));
        assert_eq!(route(check, &G(Some("a"), "minecraft:plains")).as_deref(), Some("yes"));
        assert_eq!(describe(&d.nodes["q"].options[0]), "rel -5, said=yes, > check");
        assert_eq!(letter(1), "b");
        assert!(biome_is("minecraft:snowy_plains", "minecraft:snowy_plains") && !biome_is("minecraft:plains", "snowy"));
        assert!(parse(br#"{ "start": "a", "nodes": { "a": { "next": "zz" } } }"#).is_err());
    }

    #[test]
    fn refuses_broken_files() {
        assert!(parse(br#"{ "start": "a", "nodes": {} }"#).unwrap_err().contains("start node"));
        assert!(parse(br#"{ "start": "a", "nodes": { "a": { "options": [ { "text": "x", "goto": "b" } ] } } }"#).unwrap_err().contains("missing node b"));
        assert!(parse(br#"{ "start": "a", "nodes": { "a": { "options": [ { "text": "x", "if": { "weird": 1 } } ] } } }"#).is_err());
    }
}
