//! Characters at work on the server (see `character`): an NPC's mood moves along its graph as
//! things happen to it and to its friends, its relations with players and other NPCs rise and fall,
//! and it does what it is told worse when it is upset or dislikes who tells it.
//!
//! - A player who hits an NPC falls in its eyes; one who hits (or kills) an NPC it is friends with
//!   falls in that one's eyes too, the more the closer the friends, if it saw it or stood near.
//! - Upset, it walks slower, takes its time over orders or refuses them; it won't talk at all to a
//!   player it hates. Gifts, apologies and praise in its dialog mend things, and so does time.
//! - NPCs that spend time near each other grow friends; NPCs of one owner start as friends.
//! - Friends standing together now and then tell each other what they saw players do (news), and
//!   think of those players a little as the teller does; with no news they just chat.
//!
//! An NPC's character is `projectfelix:kind` unless `/felix character` (or its file) says otherwise.
//! Characters made in the character editor (`/felix character edit`) are kept in
//! `horizon-mod-data/projectfelix/characters/<name>.json` on the server, as `felixchar:<name>`.
use crate::character::{self, Bond, Character, Heart, News};
use crate::companion::{self, Mode};
use mod_api::{
    MobRef, AddServerReloadListenersEvent, AttachmentCodec, AttachmentError, AttachmentType, Command, CommandArgumentValue,
    CommandSource, Compound, EntityRef, FileToIdConverter, Flow, Holder, Identifier, LivingRef, PlayerRef,
    RegisterCommandsEvent, ReloadApply, ReloadResourceError, RunModCommandEvent, ServerHost, ServerReloadContext,
    ServerReloadListener, Tag, Text, ConfigPath, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId,
};
use crate::serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

pub const DEFAULT: &str = "projectfelix:kind";
/// Ticks between looks.
const SCAN: i64 = 10;
/// How far an NPC notices a friend being hurt (when it sees it), and how near it always does.
const NOTICE: f64 = 24.0;
const CLOSE: f64 = 8.0;
/// How far its words carry.
const VOICE: f64 = 24.0;
/// How near NPCs must be to grow friends.
const TOGETHER: f64 = 8.0;
const MINUTE: i64 = 1200;
/// A kind word raises a relation once a minute at most (talk: once in two); gifts always.
const KIND_REST: i64 = 1200;
/// Ticks before another gift moves a relation (a quarter of a day: food handed over is no way to buy a friend).
const GIFT_REST: i64 = 6000;
/// Ticks between a mood's particles.
const PUFF: i64 = 40;
/// Ticks between gossip rounds, the chance a pair talks in one, and how long a pair then rests.
const GOSSIP: i64 = 400;
const GOSSIP_CHANCE: f32 = 0.4;
const PAIR_REST: i64 = 1200;

// ---- saved -----------------------------------------------------------------------------------------------

fn uuid_tag(u: u128) -> Tag {
    Tag::IntArray(vec![(u >> 96) as i32, (u >> 64) as i32, (u >> 32) as i32, u as i32])
}

fn tag_uuid(t: Option<&Tag>) -> Option<u128> {
    match t {
        Some(Tag::IntArray(v)) if v.len() == 4 => Some(v.iter().fold(0u128, |acc, x| (acc << 32) | (*x as u32 as u128))),
        _ => None,
    }
}

fn encode(h: &Option<Heart>) -> Result<Compound, AttachmentError> {
    let mut c = Compound::new();
    let Some(h) = h else { return Ok(c) };
    if let Some(ch) = &h.character {
        c.insert("character".into(), Tag::String(ch.as_str().into()));
    }
    c.insert("mood".into(), Tag::String(h.mood.as_str().into()));
    c.insert("since".into(), Tag::Long(h.since));
    if let Some(by) = h.by {
        c.insert("by".into(), uuid_tag(by));
    }
    let bonds = h
        .bonds
        .iter()
        .map(|b| {
            let mut x = Compound::new();
            x.insert("who".into(), uuid_tag(b.who));
            x.insert("name".into(), Tag::String(b.name.as_str().into()));
            x.insert("npc".into(), Tag::Byte(b.npc as i8));
            x.insert("value".into(), Tag::Double(f64::from(b.value).to_bits()));
            x.insert("base".into(), Tag::Int(b.base));
            x.insert("met".into(), Tag::Int(b.met as i32));
            x.insert("seen".into(), Tag::Long(b.seen));
            Tag::Compound(x)
        })
        .collect();
    c.insert("bonds".into(), Tag::List(bonds));
    let news = h
        .news
        .iter()
        .map(|n| {
            let mut x = Compound::new();
            x.insert("about".into(), uuid_tag(n.about));
            x.insert("name".into(), Tag::String(n.name.as_str().into()));
            x.insert("kind".into(), Tag::String(n.kind.as_str().into()));
            x.insert("victim".into(), uuid_tag(n.victim));
            x.insert("victim_name".into(), Tag::String(n.victim_name.as_str().into()));
            x.insert("delta".into(), Tag::Double(f64::from(n.delta).to_bits()));
            x.insert("hops".into(), Tag::Byte(n.hops as i8));
            x.insert("from".into(), Tag::String(n.from.as_str().into()));
            x.insert("at".into(), Tag::Long(n.at));
            Tag::Compound(x)
        })
        .collect();
    c.insert("news".into(), Tag::List(news));
    Ok(c)
}

fn decode(c: &Compound) -> Result<Option<Heart>, AttachmentError> {
    let text = |c: &Compound, k: &str| match c.get(&k.into()) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    };
    let Some(mood) = text(c, "mood") else { return Ok(None) };
    let since = match c.get(&"since".into()) {
        Some(Tag::Long(t)) => *t,
        _ => 0,
    };
    let mut bonds = Vec::new();
    if let Some(Tag::List(list)) = c.get(&"bonds".into()) {
        for t in list {
            let Tag::Compound(x) = t else { return Err(AttachmentError::Decode) };
            let Some(who) = tag_uuid(x.get(&"who".into())) else { continue };
            let value = match x.get(&"value".into()) {
                Some(Tag::Double(v)) => f64::from_bits(*v) as f32,
                _ => 0.0,
            };
            let base = match x.get(&"base".into()) {
                Some(Tag::Int(v)) => *v,
                _ => 0,
            };
            let npc = matches!(x.get(&"npc".into()), Some(Tag::Byte(1)));
            let met = match x.get(&"met".into()) {
                Some(Tag::Int(v)) => (*v).max(0) as u32,
                _ => 0,
            };
            let seen = match x.get(&"seen".into()) {
                Some(Tag::Long(v)) => *v,
                _ => 0,
            };
            bonds.push(Bond { who, name: text(x, "name").unwrap_or_default(), npc, value, base, met, seen });
        }
    }
    let mut news = Vec::new();
    if let Some(Tag::List(list)) = c.get(&"news".into()) {
        for t in list {
            let Tag::Compound(x) = t else { continue };
            let (Some(about), Some(victim)) = (tag_uuid(x.get(&"about".into())), tag_uuid(x.get(&"victim".into()))) else { continue };
            let delta = match x.get(&"delta".into()) {
                Some(Tag::Double(v)) => f64::from_bits(*v) as f32,
                _ => 0.0,
            };
            let hops = match x.get(&"hops".into()) {
                Some(Tag::Byte(v)) => *v as u8,
                _ => 0,
            };
            let at = match x.get(&"at".into()) {
                Some(Tag::Long(v)) => *v,
                _ => 0,
            };
            let t = |k: &str| text(x, k).unwrap_or_default();
            news.push(News { about, name: t("name"), kind: t("kind"), victim, victim_name: t("victim_name"), delta, hops, from: t("from"), at });
        }
    }
    Ok(Some(Heart { character: text(c, "character"), mood, since, bonds, news, by: tag_uuid(c.get(&"by".into())) }))
}

pub static HEART: AttachmentType<Option<Heart>> = AttachmentType {
    namespace: "projectfelix",
    name: "heart",
    default: || None,
    codec: Some(AttachmentCodec { encode, decode }),
    copy_on_death: false,
    sync: false,
};

// ---- characters from data packs -------------------------------------------------------------------------

static LOADED: LazyLock<Mutex<HashMap<String, Arc<Character>>>> = LazyLock::new(Default::default);
/// The files as they were, for the editor: data packs' and those made in the game.
static RAW: LazyLock<Mutex<HashMap<String, Value>>> = LazyLock::new(Default::default);
static MADE: LazyLock<Mutex<MadeHere>> = LazyLock::new(Default::default);

/// Characters made in the game, and the players the editor was opened for.
#[derive(Default)]
struct MadeHere {
    read: bool,
    characters: HashMap<String, (Arc<Character>, Value)>,
    editors: HashSet<u128>,
}

const MADE_FOLDER: &str = "characters";
const MADE_PREFIX: &str = "felixchar:";
static BUILT_IN: LazyLock<Arc<Character>> = LazyLock::new(|| {
    Arc::new(character::parse(include_bytes!("../data/projectfelix/felix_characters/kind.json")).expect("kind.json reads"))
});

fn character_named(id: &str) -> Arc<Character> {
    if let Some(c) = MADE.lock().ok().and_then(|m| m.characters.get(id).map(|x| x.0.clone())) {
        return c;
    }
    let loaded = LOADED.lock().ok().and_then(|l| l.get(id).or_else(|| l.get(DEFAULT)).cloned());
    loaded.unwrap_or_else(|| BUILT_IN.clone())
}

pub fn character_exists(id: &str) -> bool {
    id == DEFAULT || LOADED.lock().is_ok_and(|l| l.contains_key(id)) || MADE.lock().is_ok_and(|m| m.characters.contains_key(id))
}

struct Listener;

impl ServerReloadListener for Listener {
    fn prepare(&self, cx: &ServerReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        let mut raw = HashMap::new();
        for id in cx.list() {
            let Some(bytes) = cx.read(id) else { continue };
            match character::parse(bytes) {
                Ok(c) => {
                    next.insert(id.to_string(), Arc::new(c));
                    if let Ok(v) = crate::serde_json::from_slice::<Value>(bytes) {
                        raw.insert(id.to_string(), v);
                    }
                }
                Err(e) => eprintln!("[projectfelix] character {id} not loaded: {e}"),
            }
        }
        Ok(<dyn ReloadApply>::new(move || {
            if let Ok(mut live) = LOADED.lock() {
                *live = next;
            }
            if let Ok(mut r) = RAW.lock() {
                *r = raw;
            }
        }))
    }
}

/// Reads the characters made in the game, the first time the level ticks.
pub fn level_tick(host: &mut dyn ServerHost) {
    if !crate::saves::new_world(host, "characters") {
        return;
    }
    if let Ok(mut m) = MADE.lock() {
        *m = Default::default();
    }
    let files = host.world_data_files();
    let mut names = Vec::new();
    if let Ok(dir) = ConfigPath::new(MADE_FOLDER) {
        let mut after: Option<String> = None;
        while let Ok(page) = files.list(&dir, after.as_deref(), 256) {
            if page.is_empty() {
                break;
            }
            names.extend(page.iter().map(|e| e.name.clone()));
            after = page.last().map(|e| e.name.clone());
        }
    }
    let mut made = HashMap::new();
    for name in names.into_iter().filter(|n| n.ends_with(".json")) {
        let Ok(p) = ConfigPath::new(&format!("{MADE_FOLDER}/{name}")) else { continue };
        let read = files.read(&p).map_err(|e| format!("{e:?}")).and_then(|b| crate::serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string()));
        match read.and_then(|v| Ok((Arc::new(character::read(&v)?), v))) {
            Ok(c) => {
                made.insert(format!("{MADE_PREFIX}{}", name.trim_end_matches(".json")), c);
            }
            Err(e) => eprintln!("[projectfelix] character {name} not loaded: {e}"),
        }
    }
    if let Ok(mut m) = MADE.lock() {
        m.read = true;
        m.characters.extend(made);
    }
}

/// Every character there is: the data packs' and those made in the game.
fn all_characters() -> Vec<String> {
    let mut ids: Vec<String> = LOADED.lock().map(|l| l.keys().cloned().collect()).unwrap_or_default();
    if !ids.iter().any(|i| i == DEFAULT) {
        ids.push(DEFAULT.into());
    }
    ids.extend(MADE.lock().map(|m| m.characters.keys().cloned().collect::<Vec<_>>()).unwrap_or_default());
    ids.sort();
    ids
}

/// Tells `player`'s client the characters made in the game (for the NPC maker).
pub fn send_list(host: &mut dyn ServerHost, player: PlayerRef) {
    let mut made: Vec<String> = MADE.lock().map(|m| m.characters.keys().cloned().collect()).unwrap_or_default();
    made.sort();
    let mut dialogs: Vec<String> = crate::dialogs::made_all().into_iter().map(|(n, _)| format!("{}{n}", crate::dialogs::PREFIX)).collect();
    dialogs.sort();
    let mut rules: Vec<String> = crate::rulesets::made_all().into_iter().map(|(n, _)| format!("{}{n}", crate::rulesets::PREFIX)).collect();
    rules.sort();
    let body = crate::serde_json::to_vec(&json!({ "characters": made, "dialogs": dialogs, "rules": rules })).unwrap_or_default();
    let _ = player.send_payload(host, &PayloadId(crate::char_editor::LIST.into()), PayloadBytes(body));
}

/// Opens the character editor for `player` on character `id` (one not there yet: a copy of the
/// default, to be saved under that name).
fn open_editor(host: &mut dyn ServerHost, player: PlayerRef, id: &str) {
    let made = MADE.lock().ok().and_then(|m| m.characters.get(id).map(|x| x.1.clone()));
    let file = made.or_else(|| RAW.lock().ok().and_then(|r| r.get(id).cloned()));
    let (file, exists) = match file {
        Some(f) => (f, true),
        None => {
            let base = RAW.lock().ok().and_then(|r| r.get(DEFAULT).cloned());
            (base.unwrap_or_else(|| crate::serde_json::from_slice(include_bytes!("../data/projectfelix/felix_characters/kind.json")).unwrap_or(Value::Null)), false)
        }
    };
    let name = id.rsplit(':').next().unwrap_or(id).to_string();
    if let Ok(mut m) = MADE.lock() {
        m.editors.insert(player.uuid());
    }
    let body = json!({ "id": if exists { id } else { "" }, "name": name, "file": file });
    let _ = player.send_payload(host, &PayloadId(crate::char_editor::OPEN.into()), PayloadBytes(crate::serde_json::to_vec(&body).unwrap_or_default()));
}

/// The editor's Save: checked, written to `characters/<name>.json`, and taken at once.
pub struct SaveCharacter;

impl IPayloadHandler for SaveCharacter {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        let tell = |host: &mut dyn ServerHost, line: String| {
            let _ = player.tell(host, Text::literal(line));
        };
        if !MADE.lock().is_ok_and(|m| m.editors.contains(&player.uuid())) {
            tell(host, "Open the editor with /felix character edit first".into());
            return Ok(());
        }
        if let Err(why) = crate::saves::may_save(&*host, player, &payload.0) {
            tell(host, why.into());
            return Ok(());
        }
        let Ok(v) = crate::serde_json::from_slice::<Value>(&payload.0) else { return Ok(()) };
        let name = crate::profile::make_id(v.get("name").and_then(Value::as_str).unwrap_or("mine"));
        let Some(file) = v.get("file").cloned() else { return Ok(()) };
        let c = match character::read(&file) {
            Ok(c) => c,
            Err(e) => {
                tell(host, format!("Not kept: {e}"));
                return Ok(());
            }
        };
        let bytes = crate::serde_json::to_vec_pretty(&file).unwrap_or_default();
        let written = ConfigPath::new(&format!("{MADE_FOLDER}/{name}.json")).map_err(|e| format!("{e:?}")).and_then(|p| host.world_data_files().write(&p, &bytes).map_err(|e| format!("{e:?}")));
        if let Err(e) = written {
            tell(host, format!("Not written: {e}"));
            return Ok(());
        }
        let id = format!("{MADE_PREFIX}{name}");
        if let Ok(mut m) = MADE.lock() {
            m.characters.insert(id.clone(), (Arc::new(c), file));
        }
        tell(host, format!("Kept as {id}: /felix character <npc> {id}, or pick it in the NPC maker"));
        send_list(host, player);
        Ok(())
    }
}

pub fn add_listener(e: &mut AddServerReloadListenersEvent<'_>) {
    let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:characters"), FileToIdConverter::json("felix_characters")) else {
        return;
    };
    if let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener)) {
        eprintln!("[projectfelix] characters listener not added: {err:?}");
    }
}

// ---- live hearts ------------------------------------------------------------------------------------------

/// A loaded NPC's heart, and what it takes to read it.
struct Live {
    heart: Heart,
    uuid: u128,
    name: String,
    owner: Option<u128>,
    dirty: bool,
}

impl Live {
    fn character(&self) -> Arc<Character> {
        character_named(self.heart.character.as_deref().unwrap_or(DEFAULT))
    }
}

enum Heard {
    Hurt { victim: u32, by: u32 },
    Died { victim: u32, uuid: u128, name: String, at: [f64; 3], killer: u32, dim: &'static str },
}

/// An order it will carry out, when it has taken its time.
struct Later {
    npc: u32,
    mode: Mode,
    at: i64,
}

#[derive(Default)]
struct State {
    live: HashMap<u32, Live>,
    /// What it heard of, and how many looks it waited for its level.
    heard: Vec<(Heard, u8)>,
    later: Vec<Later>,
    /// When a kind event last raised a relation: (NPC, player, event).
    kind: HashMap<(u32, u128, String), i64>,
    /// When two NPCs last talked.
    pairs: HashMap<(u32, u32), i64>,
    seed: u64,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// The server stopped: nothing of its world is kept.
pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

fn random() -> f32 {
    let Ok(mut s) = STATE.lock() else { return 0.5 };
    let mut x = s.seed | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    s.seed = x;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

fn pick(lines: &[String]) -> Option<&String> {
    if lines.is_empty() { None } else { lines.get(((random() * lines.len() as f32) as usize).min(lines.len() - 1)) }
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn now_of(host: &dyn mod_api::Host, e: EntityRef) -> i64 {
    e.level(host).and_then(|l| l.game_time(host)).unwrap_or(0)
}

/// Reads NPC `e`'s heart into the live ones, if it isn't there yet.
fn load(host: &mut dyn ServerHost, e: EntityRef) -> bool {
    if STATE.lock().is_ok_and(|s| s.live.contains_key(&e.id())) {
        return true;
    }
    if !companion::is_npc(&*host, e) {
        return false;
    }
    let Some(uuid) = e.uuid(host) else { return false };
    let saved = HEART.get(host, Holder::Entity(e.id())).ok().flatten();
    let now = now_of(&*host, e);
    let heart = saved.unwrap_or_else(|| Heart { character: None, mood: character_named(DEFAULT).start.clone(), since: now, bonds: Vec::new(), news: Vec::new(), by: None });
    let live = Live { heart, uuid, name: e.name(&*host).unwrap_or_else(|| "NPC".into()), owner: LivingRef::new(e).owner(&*host), dirty: false };
    if let Ok(mut s) = STATE.lock() {
        s.live.entry(e.id()).or_insert(live);
    }
    true
}

/// The NPC's friend and hate lines (its character's), as last known.
pub fn thresholds(id: u32) -> (i32, i32) {
    let c = STATE.lock().ok().and_then(|s| s.live.get(&id).map(|l| l.character()));
    c.map_or((30, -50), |c| (c.friend_at, c.hate_at))
}

/// NPC `npc`'s character, unless it has the default.
pub fn character_of(host: &mut dyn ServerHost, npc: EntityRef) -> Option<String> {
    if !load(host, npc) {
        return None;
    }
    STATE.lock().ok()?.live.get(&npc.id())?.heart.character.clone()
}

/// Keeps a character a village brought as `felixchar:<name>`, unless this world has one.
pub fn keep_character(host: &mut dyn ServerHost, name: &str, file: Value) -> Result<String, String> {
    let id = format!("{MADE_PREFIX}{name}");
    if MADE.lock().is_ok_and(|m| m.characters.contains_key(&id)) {
        return Ok(id);
    }
    let c = character::read(&file)?;
    let bytes = crate::serde_json::to_vec_pretty(&file).unwrap_or_default();
    let p = ConfigPath::new(&format!("{MADE_FOLDER}/{name}.json")).map_err(|e| format!("{e:?}"))?;
    host.world_data_files().write(&p, &bytes).map_err(|e| format!("{e:?}"))?;
    if let Ok(mut m) = MADE.lock() {
        m.characters.insert(id.clone(), (Arc::new(c), file));
    }
    Ok(id)
}

/// A character made in the game, as its file (to go into an NPC's file).
pub fn made_json(id: &str) -> Option<Value> {
    MADE.lock().ok()?.characters.get(id).map(|x| x.1.clone())
}

/// Player `uuid` left the game: the editor is not theirs to save from until they open it again.
pub fn left(uuid: u128) {
    if let Ok(mut m) = MADE.lock() {
        m.editors.remove(&uuid);
    }
}

/// A character an NPC's file brought (one made in the game, `felixchar:`): taken in, unless this
/// world has its own of that name. A data pack's character is never replaced by a file's.
pub fn adopt(id: &str, file: &str) {
    if !id.starts_with(MADE_PREFIX) {
        return;
    }
    let Ok(v) = crate::serde_json::from_str::<Value>(file) else { return };
    let Ok(c) = character::read(&v) else { return };
    if let Ok(mut m) = MADE.lock() {
        m.characters.entry(id.to_string()).or_insert((Arc::new(c), v));
    }
}

/// The mood an NPC is in (for rules and dialogs), as last known.
pub fn mood_of(id: u32) -> String {
    STATE.lock().ok().and_then(|s| s.live.get(&id).map(|l| l.heart.mood.clone())).unwrap_or_else(|| "calm".into())
}

/// Whether NPC `id` talked with `who` before this talk, and the kinds of news it has of them.
/// Whether player `who` is the one who upset NPC `id` (or no one in particular did).
pub fn fault_of(id: u32, who: u128) -> bool {
    STATE.lock().ok().and_then(|s| s.live.get(&id).map(|l| l.heart.by.is_none_or(|b| b == who))).unwrap_or(true)
}

/// Whether NPC `npc`'s character says no to romance.
pub fn no_romance(host: &mut dyn ServerHost, npc: EntityRef) -> bool {
    if !load(host, npc) {
        return false;
    }
    STATE.lock().ok().and_then(|s| s.live.get(&npc.id()).map(|l| !l.character().romance)).unwrap_or(false)
}

/// What NPC `id`'s voice comes from: its character's voice, its mood, and that mood's own.
pub fn voice_facts(id: u32) -> (Option<String>, String, Option<(f32, f32)>) {
    let Ok(s) = STATE.lock() else { return (None, String::new(), None) };
    let Some(l) = s.live.get(&id) else { return (None, String::new(), None) };
    let c = l.character();
    (c.voice.clone(), l.heart.mood.clone(), c.mood(&l.heart.mood).voice)
}

/// The name of the one who upset NPC `id`, for `@upset_by`.
pub fn upset_by_name(id: u32) -> String {
    STATE
        .lock()
        .ok()
        .and_then(|s| {
            let l = s.live.get(&id)?;
            l.heart.get(l.heart.by?).map(|b| b.name.clone())
        })
        .unwrap_or_else(|| "Someone".into())
}

pub fn memory_of(id: u32, who: u128) -> (bool, Vec<String>) {
    let Ok(s) = STATE.lock() else { return (false, Vec::new()) };
    let Some(l) = s.live.get(&id) else { return (false, Vec::new()) };
    let met = l.heart.get(who).is_some_and(|b| b.met > 1);
    let kinds = l.heart.news.iter().filter(|n| n.about == who).map(|n| n.kind.clone()).collect();
    (met, kinds)
}

/// How much NPC `id` likes `who`, if its heart is known (none: an NPC just loaded, or gone).
pub fn relation_known(id: u32, who: u128) -> Option<i32> {
    let s = STATE.lock().ok()?;
    let l = s.live.get(&id)?;
    Some(l.heart.value(who).unwrap_or_else(|| base_for(l, &l.character(), who, None)))
}

/// How much NPC `id` likes the player or NPC `who`, as last known.
pub fn relation_of(id: u32, who: u128) -> i32 {
    let Ok(s) = STATE.lock() else { return 0 };
    let Some(l) = s.live.get(&id) else { return 0 };
    l.heart.value(who).unwrap_or_else(|| base_for(l, &l.character(), who, None))
}

/// Where the relation with `who` begins: its owner high; NPCs of its owner as friends.
fn base_for(l: &Live, c: &Character, who: u128, their_owner: Option<u128>) -> i32 {
    if Some(who) == l.owner {
        c.owner
    } else if their_owner.is_some() && their_owner == l.owner {
        c.friend_at
    } else {
        0
    }
}

fn player_from(host: &dyn mod_api::Host, e: EntityRef) -> Option<PlayerRef> {
    e.level(host)?.online_players(host)?.into_iter().find(|p| p.entity().id() == e.id())
}

/// Says `line` aloud: to every player near the NPC.
fn say(host: &mut dyn ServerHost, me: EntityRef, name: &str, line: &str) {
    crate::speech::show(host, me, line);
    let copy = crate::speech::chat_copy();
    if copy == crate::speech::ChatCopy::None {
        return;
    }
    let Some(at) = me.position(&*host) else { return };
    let players = me.level(&*host).and_then(|l| l.online_players(&*host)).unwrap_or_default();
    for p in players {
        if !p.entity().position(&*host).is_some_and(|x| dist(x, at) <= VOICE) {
            continue;
        }
        // A line that names the player is said to them: it goes to their chat too.
        let named = p.name(&*host).is_some_and(|n| !n.is_empty() && line.contains(&n));
        if copy == crate::speech::ChatCopy::All || named {
            let _ = p.tell(host, Text::literal(format!("<{name}> {line}")));
        }
    }
}

/// NPC `npc` says `line` to `player`: over its head, and in their chat.
pub fn say_to(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef, line: &str) {
    crate::speech::show(host, npc, line);
    if crate::speech::chat_copy() == crate::speech::ChatCopy::None {
        return;
    }
    let name = STATE.lock().ok().and_then(|s| s.live.get(&npc.id()).map(|l| l.name.clone())).or_else(|| npc.name(&*host)).unwrap_or_else(|| "NPC".into());
    let _ = player.tell(host, Text::literal(format!("<{name}> {line}")));
}

fn fill(line: &str, player: &str, npc: &str, about: &str) -> String {
    crate::rules::fill(line, player, npc, about)
}

// ---- what happens to it ------------------------------------------------------------------------------------

/// Event `event` for NPC `id`, about `player` (if any): moves the relation with them (times
/// `scale`) and the mood along the graph. `about` is who it concerns (`@target`), as a friend hurt.
fn feel_at(host: &mut dyn ServerHost, id: u32, event: &str, player: Option<PlayerRef>, scale: f32, about: Option<(u128, &str)>) {
    let Some(me) = host.entity(id) else { return };
    if !load(host, me) {
        return;
    }
    let now = now_of(&*host, me);
    let who = player.map(|p| (p.uuid(), p.name(&*host).unwrap_or_default()));
    let (c, labels, next, name, counts) = {
        let Ok(mut s) = STATE.lock() else { return };
        let gate = match &who {
            Some((u, _)) => {
                let delta = s.live.get(&id).map_or(0, |l| l.character().delta(event));
                let key = (id, *u, event.to_string());
                let rest = match event {
                    "talk" => KIND_REST * 2,
                    "gift" => GIFT_REST,
                    _ => KIND_REST,
                };
                // Kind words and gifts count once in a while; hurts always.
                let open = delta <= 0 || s.kind.get(&key).is_none_or(|t| now - t >= rest || now < *t);
                if open && delta > 0 {
                    s.kind.insert(key, now);
                }
                open
            }
            None => true,
        };
        let Some(l) = s.live.get_mut(&id) else { return };
        let c = l.character();
        let mut labels = None;
        let relation = match &who {
            Some((u, n)) => {
                let base = base_for(l, &c, *u, None);
                let before = l.heart.value(*u).unwrap_or(base);
                let delta = c.delta(event) as f32 * scale;
                let after = if gate && delta != 0.0 { l.heart.change(*u, n, false, base, delta) } else { before };
                // What it saw is news, for its friends.
                if let Some(kind) = News::kind_of(event).filter(|_| gate && delta.abs() >= 3.0) {
                    let (victim, victim_name) = about.map(|(u, n)| (u, n.to_string())).unwrap_or((l.uuid, l.name.clone()));
                    let from = l.name.clone();
                    l.heart.learn(News { about: *u, name: n.clone(), kind: kind.into(), victim, victim_name, delta, hops: 0, from, at: now });
                }
                if c.label(before) != c.label(after) {
                    labels = Some((c.label(after), after < before));
                }
                l.dirty = true;
                after
            }
            None => 0,
        };
        // Only the one who upset it can say sorry for it.
        let theirs = match (&who, l.heart.by) {
            (Some((u, _)), Some(by)) => by == *u,
            _ => true,
        };
        // Kind words said too often move neither its liking nor its mood (it sees through it).
        let flattered = !gate && c.delta(event) > 0;
        let next = if (event == "apology" && !theirs) || flattered { None } else { c.next(&l.heart.mood, event, relation).map(str::to_string) };
        (c, labels, next, l.name.clone(), gate)
    };
    // What it thinks of it (see `minds`).
    crate::minds::felt(host, me, event, counts);
    if let (Some(p), Some((label, worse))) = (player, labels) {
        let line = match (label, worse) {
            ("friend", _) => format!("{name} counts you a friend now."),
            ("hates", _) => format!("{name} hates you now."),
            ("dislikes", true) => format!("{name} dislikes you now."),
            (_, true) => format!("{name} likes you less."),
            (_, false) => format!("{name} likes you more."),
        };
        let _ = p.action_bar(host, Text::literal(line));
    }
    if let Some(next) = next {
        enter(host, me, &c, &next, player, about.map_or("", |a| a.1), now);
    }
}

/// NPC `me` comes into `mood`: says so, shows it, walks as it does in it.
fn enter(host: &mut dyn ServerHost, me: EntityRef, c: &Character, mood: &str, player: Option<PlayerRef>, about: &str, now: i64) {
    let (uuid, name) = {
        let Ok(mut s) = STATE.lock() else { return };
        let Some(l) = s.live.get_mut(&me.id()) else { return };
        l.heart.mood = mood.to_string();
        l.heart.since = now;
        l.heart.by = player.map(|p| p.uuid());
        l.dirty = true;
        (l.uuid, l.name.clone())
    };
    let m = c.mood(mood).clone();
    let player_name = player.and_then(|p| p.name(&*host)).unwrap_or_default();
    // Lines for a player only when there is one.
    let lines: Vec<String> = m.say.iter().filter(|l| player.is_some() || !l.contains("@player")).cloned().collect();
    if let Some(line) = pick(&lines) {
        say(host, me, &name, &fill(line, &player_name, &name, about));
    }
    if let Some(a) = &m.animation {
        crate::talk::run_for(host, None, me, &[crate::dialog::Action::Animation(a.clone())]);
    }
    let id = crate::talk::uuid_text(uuid);
    if let Some(p) = &m.particle {
        crate::talk::queue(format!("execute at {id} run particle {p} ~ ~2.2 ~ 0.3 0.2 0.3 0 6"));
    }
    set_speed(&id, m.speed);
    crate::rulebook::notice(me.id(), &format!("mood:{mood}"), player.map(|p| p.entity().id()));
}

fn set_speed(uuid: &str, speed: f32) {
    crate::talk::queue(format!("attribute {uuid} minecraft:movement_speed modifier remove projectfelix:mood"));
    if (speed - 1.0).abs() > 0.01 {
        crate::talk::queue(format!("attribute {uuid} minecraft:movement_speed modifier add projectfelix:mood {:.2} add_multiplied_total", speed - 1.0));
    }
}

/// `living_damage_post`.
pub fn on_hurt(victim: i32, by: i32) {
    if victim > 0 && by > 0
        && let Ok(mut s) = STATE.lock()
        && s.heard.len() < 256
    {
        s.heard.push((Heard::Hurt { victim: victim as u32, by: by as u32 }, 0));
    }
}

/// `living_death`: an NPC killed (by a player, its friends will know).
pub fn on_death(host: &mut dyn ServerHost, victim: u32, killer: i32) {
    if killer <= 0 || !STATE.lock().is_ok_and(|s| s.live.contains_key(&victim)) {
        return;
    }
    let Some(e) = host.entity(victim) else { return };
    let Some(uuid) = e.uuid(host) else { return };
    let name = e.name(&*host).unwrap_or_else(|| "NPC".into());
    let at = e.position(&*host).unwrap_or([0.0; 3]);
    let dim = e.level(&*host).map_or(crate::pace::level(), |l| l.dimension_id());
    if let Ok(mut s) = STATE.lock() {
        s.heard.push((Heard::Died { victim, uuid, name, at, killer: killer as u32, dim }, 0));
    }
}

/// A game between NPCs `a` and `b` is over (`winner`, or a draw): they say so, feel it, and
/// grow friends for the time spent.
pub fn game_over(host: &mut dyn ServerHost, a: u32, b: u32, winner: Option<u32>) {
    let (Some(ea), Some(eb)) = (host.entity(a), host.entity(b)) else { return };
    let now = now_of(&*host, ea);
    let name = |e: EntityRef, host: &dyn mod_api::Host| e.name(host).unwrap_or_else(|| "NPC".into());
    let (na, nb) = (name(ea, &*host), name(eb, &*host));
    match winner {
        Some(w) => {
            let (we, le, wn, ln) = if w == a { (ea, eb, &na, &nb) } else { (eb, ea, &nb, &na) };
            say_aloud(host, we, &format!("Good game, {ln}!"));
            say_aloud(host, le, &format!("Well played, {wn}. Next time."));
            crate::talk::run_for(host, None, we, &[crate::dialog::Action::Animation("animation.felix.cheer".into())]);
            feel_at(host, we.id(), "won_game", None, 1.0, None);
            feel_at(host, le.id(), "lost_game", None, 1.0, None);
        }
        None => {
            say_aloud(host, ea, &format!("A draw, {nb}. Again tomorrow?"));
        }
    }
    for (x, y) in [(ea, eb), (eb, ea)] {
        if !load(host, x) || !load(host, y) {
            continue;
        }
        let Some(u) = y.uuid(host) else { continue };
        let yn = name(y, &*host);
        if let Ok(mut s) = STATE.lock() {
            let owner = s.live.get(&y.id()).and_then(|l| l.owner);
            if let Some(l) = s.live.get_mut(&x.id()) {
                let c = l.character();
                let base = base_for(l, &c, u, owner);
                l.heart.change(u, &yn, true, base, 3.0);
                l.dirty = true;
            }
        }
    }
    let _ = now;
}

/// NPC `npc` says `line` to the players near it.
pub fn say_aloud(host: &mut dyn ServerHost, npc: EntityRef, line: &str) {
    let name = STATE.lock().ok().and_then(|s| s.live.get(&npc.id()).map(|l| l.name.clone())).or_else(|| npc.name(&*host)).unwrap_or_else(|| "NPC".into());
    say(host, npc, &name, line);
}

/// The loaded NPC with `uuid`, by entity id.
pub fn npc_by_uuid(uuid: u128) -> Option<u32> {
    STATE.lock().ok()?.live.iter().find(|(_, l)| l.uuid == uuid).map(|(id, _)| *id)
}

/// A dialog or a rule makes NPC `npc` feel `event` (a gift, an apology), about `player`.
pub fn feel(host: &mut dyn ServerHost, npc: EntityRef, player: Option<PlayerRef>, event: &str) {
    feel_at(host, npc.id(), event, player, 1.0, None);
}

/// Moves NPC `npc`'s relation with `player` by `delta`.
pub fn befriend(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef, delta: i32) {
    if !load(host, npc) {
        return;
    }
    let name = player.name(&*host).unwrap_or_default();
    if let Ok(mut s) = STATE.lock()
        && let Some(l) = s.live.get_mut(&npc.id())
    {
        let base = base_for(l, &l.character(), player.uuid(), None);
        l.heart.change(player.uuid(), &name, false, base, delta as f32);
        l.dirty = true;
    }
}

/// A player wants to talk: false (and it says why) if the NPC won't, for it hates them.
pub fn talks_to(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) -> bool {
    if !load(host, npc) {
        return true;
    }
    let (c, value, name) = {
        let Ok(s) = STATE.lock() else { return true };
        let Some(l) = s.live.get(&npc.id()) else { return true };
        let c = l.character();
        let value = l.heart.value(player.uuid()).unwrap_or_else(|| base_for(l, &c, player.uuid(), None));
        (c, value, l.name.clone())
    };
    // In a break it won't talk (see `minds`).
    if crate::minds::shuts_out(host, npc) {
        return false;
    }
    if value > c.hate_at {
        feel(host, npc, Some(player), "talk");
        greet(host, npc, player);
        crate::minds::on_talk(host, npc, player);
        return true;
    }
    let player_name = player.name(&*host).unwrap_or_default();
    let line = pick(&c.shun).cloned().unwrap_or_else(|| "...".into());
    say(host, npc, &name, &fill(&line, &player_name, &name, ""));
    false
}

/// Ticks between greetings to one player.
const GREET_REST: i64 = 6000;

/// The NPC greets `player` by what it remembers of them, and remembers this talk.
fn greet(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) {
    let now = now_of(&*host, npc);
    let player_name = player.name(&*host).unwrap_or_default();
    let (line, name) = {
        let Ok(mut s) = STATE.lock() else { return };
        let Some(l) = s.live.get_mut(&npc.id()) else { return };
        let c = l.character();
        let u = player.uuid();
        let base = base_for(l, &c, u, None);
        let relation = l.heart.value(u).unwrap_or(base);
        // Make sure the bond is there, then read and update the memory.
        l.heart.change(u, &player_name, false, base, 0.0);
        let Some(i) = l.heart.bonds.iter().position(|b| b.who == u) else { return };
        let (met, seen) = (l.heart.bonds[i].met, l.heart.bonds[i].seen);
        let rested = met == 0 || now - seen >= GREET_REST || now < seen;
        l.heart.bonds[i].met = met + 1;
        l.heart.bonds[i].seen = now;
        l.dirty = true;
        if !rested {
            return;
        }
        let news: Vec<(&News, i64, bool)> = l.heart.news.iter().filter(|n| n.about == u).map(|n| (n, now - n.at, n.victim == l.uuid)).collect();
        let memory = character::Memory { met, since: now - seen, relation, news };
        let Some((key, victim)) = character::greeting_key(&c, &memory) else { return };
        let lines = character::greeting_lines(&c, key);
        let n = lines.len().max(1);
        let Some(line) = lines.get((now as usize / 7 + met as usize) % n).cloned() else { return };
        (fill(&line, &player_name, &l.name, &victim).replace("@victim", &victim), l.name.clone())
    };
    let _ = name;
    say_to(host, npc, player, &line);
}

/// `player` tells NPC `npc` to `mode`: it does (maybe after a while, grumbling), or refuses.
/// False if it refused.
pub fn order(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef, mode: Mode) -> bool {
    if !load(host, npc) {
        companion::set_orders(host, npc, mode);
        return true;
    }
    let (c, mood, value, name) = {
        let Ok(s) = STATE.lock() else { return true };
        let Some(l) = s.live.get(&npc.id()) else { return true };
        let c = l.character();
        let value = l.heart.value(player.uuid()).unwrap_or_else(|| base_for(l, &c, player.uuid(), None));
        (c, l.heart.mood.clone(), value, l.name.clone())
    };
    let player_name = player.name(&*host).unwrap_or_default();
    let m = c.mood(&mood).clone();
    // Hungry, it obeys worse (see `needs`).
    if random() >= c.obey_chance(&mood, value) * crate::needs::obey_factor(npc.id()) * crate::minds::obey_factor(npc.id()) {
        let line = pick(&m.refuse).cloned().unwrap_or_else(|| "No.".into());
        say(host, npc, &name, &fill(&line, &player_name, &name, ""));
        if let Some(a) = &m.animation {
            crate::talk::run_for(host, None, npc, &[crate::dialog::Action::Animation(a.clone())]);
        }
        feel(host, npc, Some(player), "refused");
        return false;
    }
    if m.delay > 0.0 {
        if let Some(line) = pick(&m.grumble) {
            say(host, npc, &name, &fill(line, &player_name, &name, ""));
        }
        let at = now_of(&*host, npc) + (m.delay * 20.0) as i64;
        if let Ok(mut s) = STATE.lock() {
            s.later.retain(|l| l.npc != npc.id());
            s.later.push(Later { npc: npc.id(), mode, at });
        }
    } else {
        companion::set_orders(host, npc, mode);
    }
    true
}

/// Gives NPC `npc` the character `id` (none: the default), starting calm again.
pub fn set_character(host: &mut dyn ServerHost, npc: EntityRef, id: Option<&str>) -> bool {
    if !load(host, npc) {
        return false;
    }
    let c = character_named(id.unwrap_or(DEFAULT));
    let now = now_of(&*host, npc);
    let uuid = {
        let Ok(mut s) = STATE.lock() else { return false };
        let Some(l) = s.live.get_mut(&npc.id()) else { return false };
        l.heart.character = id.map(str::to_string);
        l.heart.mood = c.start.clone();
        l.heart.since = now;
        l.heart.by = None;
        l.dirty = true;
        l.uuid
    };
    set_speed(&crate::talk::uuid_text(uuid), c.mood(&c.start).speed);
    true
}

// ---- the tick -----------------------------------------------------------------------------------------------

pub fn tick(host: &mut dyn ServerHost) {
    let npcs = companion::npcs();
    let Some(now) = npcs.iter().find_map(|id| host.entity(*id).map(|e| now_of(&*host, e))) else { return };
    let (heard, later, minute) = {
        let Ok(mut s) = STATE.lock() else { return };
        // Each level at its own pace (its NPCs are the ones found while it ticks).
        if !crate::pace::since("heart", now, SCAN) {
            return;
        }
        s.seed ^= (now as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let minute = crate::pace::since("heart.minute", now, MINUTE);
        if minute {
            s.kind.retain(|_, t| now - *t < GIFT_REST && now >= *t);
        }
        let (due, wait): (Vec<Later>, Vec<Later>) = std::mem::take(&mut s.later).into_iter().partition(|l| l.at <= now);
        s.later = wait;
        (std::mem::take(&mut s.heard), due, minute)
    };
    // What happened in this level is for its NPCs; the rest waits for its own (a while).
    let mut mine = Vec::new();
    let mut rest = Vec::new();
    for (h, waited) in heard {
        let ours = match &h {
            Heard::Hurt { victim, .. } => crate::pace::here(&*host, *victim),
            Heard::Died { dim, .. } => *dim == crate::pace::level(),
        };
        if ours {
            mine.push(h);
        } else if waited < 60 {
            rest.push((h, waited + 1));
        }
    }
    let heard = mine;
    let mut later_rest = Vec::new();
    for l in later {
        if crate::pace::here(&*host, l.npc) || now - l.at > 2400 {
            if let Some(e) = host.entity(l.npc) {
                companion::set_orders(host, e, l.mode);
            }
        } else {
            later_rest.push(l);
        }
    }
    if let Ok(mut s) = STATE.lock() {
        s.heard.extend(rest);
        s.later.extend(later_rest);
    }
    for id in &npcs {
        if let Some(e) = host.entity(*id)
            && load(host, e)
        {
            // Names and owners change (a name tag, a new owner).
            let name = e.name(&*host).unwrap_or_else(|| "NPC".into());
            let owner = LivingRef::new(e).owner(&*host);
            if let Ok(mut s) = STATE.lock()
                && let Some(l) = s.live.get_mut(id)
            {
                l.name = name;
                l.owner = owner;
            }
        }
    }
    for h in heard {
        match h {
            Heard::Hurt { victim, by } => hurt(host, victim, by),
            Heard::Died { victim, uuid, name, at, killer, .. } => died(host, victim, uuid, &name, at, killer),
        }
    }
    // Moods that pass with time.
    let timed: Vec<(u32, Arc<Character>, String)> = match STATE.lock() {
        Ok(s) => s
            .live
            .iter()
            .filter_map(|(id, l)| {
                let c = l.character();
                let next = c.timed(&l.heart.mood, (now - l.heart.since) as f32 / 20.0)?.to_string();
                Some((*id, c, next))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    for (id, c, next) in timed {
        if let Some(e) = host.entity(id) {
            enter(host, e, &c, &next, None, "", now);
        }
    }
    // Particles while in a mood that has them.
    if now % PUFF < SCAN {
        let puffs: Vec<(u128, String)> = match STATE.lock() {
            Ok(s) => s.live.values().filter_map(|l| Some((l.uuid, l.character().mood(&l.heart.mood).particle.clone()?))).collect(),
            Err(_) => Vec::new(),
        };
        for (u, p) in puffs {
            crate::talk::queue(format!("execute at {} run particle {p} ~ ~2.2 ~ 0.25 0.1 0.25 0 2", crate::talk::uuid_text(u)));
        }
    }
    if minute {
        a_minute(host);
        if let Ok(mut s) = STATE.lock() {
            for l in s.live.values_mut() {
                l.heart.forget_news(now);
            }
            s.pairs.retain(|_, t| now - *t < PAIR_REST);
        }
    }
    if now % GOSSIP < SCAN {
        gossip(host, now, None);
    }
    // Saved, and the unloaded let go.
    let dirty: Vec<(u32, Heart)> = match STATE.lock() {
        Ok(mut s) => {
            let dirty = s
                .live
                .iter_mut()
                .filter(|(id, l)| l.dirty && crate::pace::here(&*host, **id))
                .map(|(id, l)| {
                    l.dirty = false;
                    (*id, l.heart.clone())
                })
                .collect();
            s.live.retain(|id, _| npcs.contains(id));
            dirty
        }
        Err(_) => Vec::new(),
    };
    for (id, h) in dirty {
        let _ = HEART.set(host, Holder::Entity(id), Some(h));
    }
}

/// Whether NPC `id` noticed what happened to `victim` (it saw it, or stood near).
fn noticed(host: &dyn mod_api::Host, id: u32, victim: EntityRef, at: [f64; 3]) -> bool {
    let Some(me) = host.entity(id) else { return false };
    let Some(mine) = me.position(host) else { return false };
    let d = dist(mine, at);
    d <= CLOSE || (d <= NOTICE && LivingRef::new(me).has_line_of_sight(host, victim) == Some(true))
}

/// The NPCs that count `friend` (by UUID; `friend_owner` theirs) a friend, and how close.
fn friends_of(friend: u128, friend_owner: Option<u128>, except: u32) -> Vec<(u32, i32)> {
    let Ok(s) = STATE.lock() else { return Vec::new() };
    s.live
        .iter()
        .filter(|(id, _)| **id != except)
        .filter_map(|(id, l)| {
            let c = l.character();
            let v = l.heart.value(friend).unwrap_or_else(|| base_for(l, &c, friend, friend_owner));
            (v >= c.friend_at).then_some((*id, v))
        })
        .collect()
}

fn hurt(host: &mut dyn ServerHost, victim: u32, by: u32) {
    let (Some(v), Some(b)) = (host.entity(victim), host.entity(by)) else { return };
    let attacker = player_from(&*host, b);
    let npc = STATE.lock().ok().and_then(|s| s.live.get(&victim).map(|l| (l.uuid, l.owner, l.name.clone())));
    match (npc, attacker) {
        // An NPC hit by a player: it and its friends who saw.
        (Some((uuid, owner, name)), Some(p)) => {
            feel_at(host, victim, "hurt_by_player", Some(p), 1.0, None);
            let at = v.position(&*host).unwrap_or([0.0; 3]);
            for (friend, closeness) in friends_of(uuid, owner, victim) {
                if noticed(&*host, friend, v, at) {
                    feel_at(host, friend, "friend_hurt", Some(p), character::friend_scale(closeness), Some((uuid, &name)));
                    crate::rulebook::notice(friend, "friend_hurt", Some(by));
                }
            }
        }
        (Some(_), None) => feel_at(host, victim, "hurt", None, 1.0, None),
        // A player hit by another: the NPCs whose owner it is.
        (None, Some(p)) => {
            let Some(owner) = player_from(&*host, v).map(|o| o.uuid()) else { return };
            if owner == p.uuid() {
                return;
            }
            let at = v.position(&*host).unwrap_or([0.0; 3]);
            let theirs: Vec<u32> = STATE.lock().map(|s| s.live.iter().filter(|(_, l)| l.owner == Some(owner)).map(|(id, _)| *id).collect()).unwrap_or_default();
            let owner_name = v.name(&*host).unwrap_or_default();
            for id in theirs {
                if noticed(&*host, id, v, at) {
                    feel_at(host, id, "owner_hurt", Some(p), 1.0, Some((owner, &owner_name)));
                }
            }
        }
        (None, None) => {}
    }
}

fn died(host: &mut dyn ServerHost, victim: u32, uuid: u128, name: &str, at: [f64; 3], killer: u32) {
    let owner = STATE.lock().ok().and_then(|s| s.live.get(&victim).and_then(|l| l.owner));
    if let Ok(mut s) = STATE.lock() {
        s.live.remove(&victim);
    }
    let Some(p) = host.entity(killer).and_then(|k| player_from(&*host, k)) else { return };
    for (friend, closeness) in friends_of(uuid, owner, victim) {
        let near = host.entity(friend).and_then(|e| e.position(&*host)).is_some_and(|x| dist(x, at) <= NOTICE + 8.0);
        if near {
            feel_at(host, friend, "friend_killed", Some(p), character::friend_scale(closeness), Some((uuid, name)));
            crate::rulebook::notice(friend, "friend_killed", Some(killer));
        }
    }
}

/// Once a minute: hurts are forgiven a little, and NPCs near each other grow friends.
fn a_minute(host: &mut dyn ServerHost) {
    let ids: Vec<u32> = STATE.lock().map(|s| s.live.keys().copied().collect()).unwrap_or_default();
    let at: Vec<(u32, [f64; 3])> = ids.iter().filter_map(|id| Some((*id, host.entity(*id)?.position(&*host)?))).take(64).collect();
    let Ok(mut s) = STATE.lock() else { return };
    let info: HashMap<u32, (u128, String, Option<u128>)> = s.live.iter().map(|(id, l)| (*id, (l.uuid, l.name.clone(), l.owner))).collect();
    for (id, l) in s.live.iter_mut() {
        let c = l.character();
        l.heart.forgive(1.0, c.forgive);
        let Some(mine) = at.iter().find(|x| x.0 == *id).map(|x| x.1) else { continue };
        for (other, there) in &at {
            if other == id || dist(mine, *there) > TOGETHER {
                continue;
            }
            let Some((u, name, owner)) = info.get(other) else { continue };
            let base = base_for(l, &c, *u, *owner);
            let now = l.heart.value(*u).unwrap_or(base);
            if now < c.bond_max {
                l.heart.change(*u, name, true, base, c.bond.min((c.bond_max - now) as f32));
            }
        }
        l.dirty = true;
    }
}

// ---- gossip ---------------------------------------------------------------------------------------------------

/// What a pair says: one's news to the other, or chat.
enum Talk {
    News { teller: u32, hearer: u32, news: News },
    Chat { speaker: u32, hearer: u32 },
    /// Words of any kind, friends or not (see `social`).
    Social,
}

fn online(host: &dyn mod_api::Host, near: EntityRef, uuid: u128) -> Option<PlayerRef> {
    near.level(host)?.online_players(host)?.into_iter().find(|p| p.uuid() == uuid)
}

/// Friend NPCs standing together talk: each pair now and then (`only`: these, now). How many did.
fn gossip(host: &mut dyn ServerHost, now: i64, only: Option<&[u32]>) -> usize {
    let ids: Vec<u32> = STATE.lock().map(|s| s.live.keys().copied().filter(|id| only.is_none_or(|o| o.contains(id))).collect()).unwrap_or_default();
    let at: Vec<(u32, [f64; 3])> = ids.iter().filter_map(|id| Some((*id, host.entity(*id)?.position(&*host)?))).take(64).collect();
    let mut talks = 0;
    for i in 0..at.len() {
        for j in i + 1..at.len() {
            let ((a, pa), (b, pb)) = (at[i], at[j]);
            if dist(pa, pb) > TOGETHER {
                continue;
            }
            let key = (a.min(b), a.max(b));
            if only.is_none() {
                if random() >= GOSSIP_CHANCE {
                    continue;
                }
                let rested = STATE.lock().map(|s| s.pairs.get(&key).is_none_or(|t| now - t >= PAIR_REST || now < *t)).unwrap_or(false);
                if !rested {
                    continue;
                }
            }
            if talk(host, a, b, now) {
                talks += 1;
                if let Ok(mut s) = STATE.lock() {
                    s.pairs.insert(key, now);
                }
            }
        }
    }
    talks
}

/// NPCs `a` and `b` talk: friends tell their news or chat; anyone passes words (see `social`).
fn talk(host: &mut dyn ServerHost, a: u32, b: u32, now: i64) -> bool {
    // Sleepers don't talk.
    if crate::routines::asleep(a) || crate::routines::asleep(b) {
        return false;
    }
    let coin = random();
    let plan = {
        let Ok(s) = STATE.lock() else { return false };
        let (Some(la), Some(lb)) = (s.live.get(&a), s.live.get(&b)) else { return false };
        let likes = |x: &Live, y: &Live| {
            let c = x.character();
            x.heart.value(y.uuid).unwrap_or_else(|| base_for(x, &c, y.uuid, y.owner)) >= c.friend_at
        };
        if !likes(la, lb) && !likes(lb, la) {
            Talk::Social
        } else {
        let from_a = la.heart.to_tell(&lb.heart, lb.uuid).cloned();
        let from_b = lb.heart.to_tell(&la.heart, la.uuid).cloned();
        match (from_a, from_b) {
            (Some(x), Some(y)) if y.at > x.at => Talk::News { teller: b, hearer: a, news: y },
            (Some(x), _) => Talk::News { teller: a, hearer: b, news: x },
            (None, Some(y)) => Talk::News { teller: b, hearer: a, news: y },
            (None, None) if coin < 0.3 => Talk::Social,
            (None, None) if coin < 0.65 => Talk::Chat { speaker: a, hearer: b },
            (None, None) => Talk::Chat { speaker: b, hearer: a },
        }
        }
    };
    if let Talk::Social = plan {
        return social_talk(host, a, b, now);
    }
    let (Some(ea), Some(eb)) = (host.entity(a), host.entity(b)) else { return false };
    // They turn to each other.
    for (me, other) in [(ea, eb), (eb, ea)] {
        if let Some(p) = other.position(&*host) {
            let _ = MobRef::new(me).look_at(host, [p[0], p[1] + 1.6, p[2]]);
        }
        // A chat is fun for both (see `minds`).
        crate::minds::enjoy(host, me, crate::mind::TALK, 8.0);
    }
    let name_of = |id: u32| STATE.lock().ok().and_then(|s| s.live.get(&id).map(|l| l.name.clone())).unwrap_or_else(|| "NPC".into());
    match plan {
        Talk::Chat { speaker, hearer } => {
            let (sn, hn) = (name_of(speaker), name_of(hearer));
            let line = STATE.lock().ok().and_then(|s| {
                let l = s.live.get(&speaker)?;
                Some(l.character().mood(&l.heart.mood).chatter.clone())
            });
            let Some(line) = line.as_deref().and_then(pick).cloned() else { return false };
            let e = if speaker == a { ea } else { eb };
            say(host, e, &sn, &line.replace("@listener", &hn));
            true
        }
        Talk::Social => false,
        Talk::News { teller, hearer, news } => {
            let (tn, hn) = (name_of(teller), name_of(hearer));
            let (te, he) = if teller == a { (ea, eb) } else { (eb, ea) };
            say(host, te, &tn, &news.line(&tn, &hn));
            let reply = news.reply((random() * 16.0) as usize);
            say(host, he, &hn, &reply);
            hear(host, hearer, he, news.retold(&tn), now);
            true
        }
    }
}

/// How NPC `id` thinks of the one with `uuid` moves by `delta`.
fn shift(id: u32, uuid: u128, name: &str, delta: f32) {
    if let Ok(mut s) = STATE.lock()
        && let Some(l) = s.live.get_mut(&id)
    {
        let c = l.character();
        let base = base_for(l, &c, uuid, None);
        l.heart.change(uuid, name, true, base, delta);
        l.dirty = true;
    }
}

/// NPCs `a` and `b` pass words (see `social`): one speaks, the other answers; it moves how they
/// think of each other, may stay with the hearer, and an insult may come to blows.
fn social_talk(host: &mut dyn ServerHost, a: u32, b: u32, now: i64) -> bool {
    let (Some(ea), Some(eb)) = (host.entity(a), host.entity(b)) else { return false };
    let (speaker, hearer, es, eh) = if random() < 0.5 { (a, b, ea, eb) } else { (b, a, eb, ea) };
    let who = |id: u32| STATE.lock().ok().and_then(|s| s.live.get(&id).map(|l| (l.uuid, l.name.clone())));
    let (Some((us, ns)), Some((uh, nh))) = (who(speaker), who(hearer)) else { return false };
    let traits = crate::mind::traits(us);
    let mood = crate::minds::mood(speaker).unwrap_or(50.0);
    let act = crate::social::pick(&crate::social::weights(&traits, mood, crate::social::fit(us, uh)), random());
    for (me, other) in [(es, eh), (eh, es)] {
        if let Some(p) = other.position(&*host) {
            let _ = MobRef::new(me).look_at(host, [p[0], p[1] + 1.6, p[2]]);
        }
    }
    say(host, es, &ns, &crate::social::line(act, random()).replace("@n", &nh));
    say(host, eh, &nh, crate::social::reply(act, random()));
    let (to_speaker, to_hearer) = crate::social::opinion(act);
    shift(hearer, us, &ns, to_speaker);
    shift(speaker, uh, &nh, to_hearer);
    if let Some(t) = crate::social::thought(act) {
        crate::minds::remember(host, eh, t);
    }
    if matches!(act, crate::social::Act::Chat | crate::social::Act::HeartToHeart | crate::social::Act::KindWords) {
        crate::minds::enjoy(host, es, crate::mind::TALK, 6.0);
        crate::minds::enjoy(host, eh, crate::mind::TALK, 6.0);
    }
    // An insult may come to blows: both get hurt, and after it they think better or worse of each other.
    if random() < crate::social::fight_chance(act, &crate::mind::traits(uh)) {
        for (e, n) in [(es, &ns), (eh, &nh)] {
            if let Some(u) = e.uuid(host) {
                crate::talk::queue(format!("damage {} 2 minecraft:generic", crate::talk::uuid_text(u)));
            }
            crate::routines::play_animation(host, e, "animation.felix.angry");
            crate::minds::remember(host, e, "fought");
            let _ = n;
        }
        let after = crate::social::after_fight(random());
        shift(hearer, us, &ns, after);
        shift(speaker, uh, &nh, after);
        say(host, eh, &nh, "Right, that's it!");
        let _ = now;
    }
    true
}

/// NPC `id` hears `news`: thinks of the player as it was told, and keeps it to tell on.
fn hear(host: &mut dyn ServerHost, id: u32, me: EntityRef, news: News, now: i64) {
    let player = online(&*host, me, news.about);
    let (c, next, victim_name) = {
        let Ok(mut s) = STATE.lock() else { return };
        // The one it happened to may be an NPC of the same owner: a friend from the start.
        let victim_owner = s.live.values().find(|x| x.uuid == news.victim).and_then(|x| x.owner);
        let Some(l) = s.live.get_mut(&id) else { return };
        let c = l.character();
        let friend = l.heart.value(news.victim).unwrap_or_else(|| base_for(l, &c, news.victim, victim_owner)) >= c.friend_at;
        let delta = news.rumor_delta(friend);
        let base = base_for(l, &c, news.about, None);
        let after = l.heart.change(news.about, &news.name, false, base, delta);
        let event = if delta < 0.0 { "rumor_bad" } else { "rumor_good" };
        let next = c.next(&l.heart.mood, event, after).map(str::to_string);
        let victim_name = news.victim_name.clone();
        l.heart.learn(news);
        l.dirty = true;
        (c, next, victim_name)
    };
    crate::rulebook::notice(id, "rumor", player.map(|p| p.entity().id()));
    if let Some(next) = next {
        enter(host, me, &c, &next, player, &victim_name, now);
    }
}

/// A player asks NPC `npc` for news: it tells its newest, aloud.
pub fn tell_news(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) {
    if !load(host, npc) {
        return;
    }
    let (name, news) = match STATE.lock() {
        Ok(s) => match s.live.get(&npc.id()) {
            Some(l) => (l.name.clone(), l.heart.news.last().cloned()),
            None => return,
        },
        Err(_) => return,
    };
    let player_name = player.name(&*host).unwrap_or_default();
    let line = match news {
        Some(n) => n.line(&name, &player_name),
        None => format!("Nothing new, {player_name}."),
    };
    let _ = name;
    say_to(host, npc, player, &line);
}

// ---- commands -------------------------------------------------------------------------------------------------

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix character <targets:entities> <character:text>", 2));
    e.commands.push(Command::op("felix character edit <id:text>", 2));
    e.commands.push(Command::op("felix characters", 2));
    e.commands.push(Command::op("felix feel <targets:entities> <event:word>", 2));
    e.commands.push(Command::op("felix relation <target:entity> <who:entities> <value:integer>", 2));
    e.commands.push(Command::op("felix friends <targets:entities>", 2));
    e.commands.push(Command::op("felix heart <target:entity>", 2));
    e.commands.push(Command::op("felix gossip <targets:entities>", 2));
}

fn entities(e: &RunModCommandEvent<'_>, name: &str) -> Vec<EntityRef> {
    e.command
        .arguments
        .iter()
        .find(|a| a.name == name)
        .map(|a| match &a.value {
            CommandArgumentValue::Entities(list) => list.clone(),
            CommandArgumentValue::Entity(one) => vec![*one],
            _ => Vec::new(),
        })
        .unwrap_or_default()
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let feedback = |host: &mut dyn ServerHost, line: String| {
        let _ = host.commands().feedback(Text::literal(line));
    };
    let source = match e.source {
        CommandSource::Player { player, .. } => Some(*player),
        _ => None,
    };
    match lit.as_slice() {
        ["felix", "character", "edit"] => {
            let Some(p) = source else {
                feedback(host, "Only a player can edit a character.".into());
                return Flow::Cancel;
            };
            let id = companion::text_argument(e, "id").unwrap_or_default().trim().to_string();
            let id = if id.contains(':') { id } else { format!("{MADE_PREFIX}{id}") };
            open_editor(host, p, &id);
        }
        ["felix", "characters"] => {
            let list = all_characters();
            feedback(host, format!("{} characters: {}", list.len(), list.join(", ")));
        }
        ["felix", "character"] => {
            let id = companion::text_argument(e, "character").unwrap_or_default().trim().to_string();
            let npcs: Vec<EntityRef> = entities(e, "targets").into_iter().filter(|t| companion::is_npc(&*host, *t)).collect();
            let n = npcs.iter().filter(|t| set_character(host, **t, Some(&id))).count();
            let note = if character_exists(&id) { "" } else { " (no such character is loaded: they act as projectfelix:kind)" };
            feedback(host, format!("{n} NPCs now have the character {id}{note}"));
        }
        ["felix", "feel"] => {
            let event = companion::text_argument(e, "event").unwrap_or_default();
            let npcs: Vec<EntityRef> = entities(e, "targets").into_iter().filter(|t| companion::is_npc(&*host, *t)).collect();
            for t in &npcs {
                feel(host, *t, source, &event);
            }
            feedback(host, format!("{} NPCs felt {event}", npcs.len()));
        }
        ["felix", "relation"] => {
            let value = e.command.arguments.iter().find(|a| a.name == "value").and_then(|a| match a.value {
                CommandArgumentValue::Integer(v) => Some(v.clamp(character::LOWEST, character::HIGHEST)),
                _ => None,
            });
            let Some(npc) = entities(e, "target").into_iter().find(|t| companion::is_npc(&*host, *t)) else {
                feedback(host, "That is not an NPC.".into());
                return Flow::Cancel;
            };
            let (Some(value), true) = (value, load(host, npc)) else { return Flow::Cancel };
            let mut n = 0;
            for w in entities(e, "who") {
                if w.id() == npc.id() {
                    continue;
                }
                let Some(u) = w.uuid(host) else { continue };
                let name = w.name(&*host).unwrap_or_default();
                let is_npc = companion::is_npc(&*host, w);
                if let Ok(mut s) = STATE.lock()
                    && let Some(l) = s.live.get_mut(&npc.id())
                {
                    l.heart.set(u, &name, is_npc, value);
                    l.dirty = true;
                    n += 1;
                }
            }
            feedback(host, format!("Set {n} relations to {value}"));
        }
        ["felix", "friends"] => {
            let npcs: Vec<EntityRef> = entities(e, "targets").into_iter().filter(|t| companion::is_npc(&*host, *t)).collect();
            let mut who = Vec::new();
            for t in &npcs {
                if load(host, *t) {
                    who.push((t.id(), t.uuid(host).unwrap_or(0), t.name(&*host).unwrap_or_default()));
                }
            }
            if let Ok(mut s) = STATE.lock() {
                for (a, _, _) in &who {
                    let Some(l) = s.live.get_mut(a) else { continue };
                    let value = l.character().bond_max.max(l.character().friend_at);
                    for (b, u, name) in &who {
                        if a != b {
                            let now = l.heart.value(*u).unwrap_or(0);
                            l.heart.set(*u, name, true, value.max(now));
                        }
                    }
                    l.dirty = true;
                }
            }
            feedback(host, format!("{} NPCs are friends now", who.len()));
        }
        ["felix", "gossip"] => {
            let mut ids = Vec::new();
            for t in entities(e, "targets") {
                if companion::is_npc(&*host, t) && load(host, t) {
                    ids.push(t.id());
                }
            }
            let now = ids.first().and_then(|id| host.entity(*id)).map_or(0, |x| now_of(&*host, x));
            let n = gossip(host, now, Some(&ids));
            feedback(host, format!("{n} talks among {} NPCs (friends within {TOGETHER} blocks)", ids.len()));
        }
        ["felix", "heart"] => {
            let Some(npc) = entities(e, "target").into_iter().find(|t| companion::is_npc(&*host, *t)) else {
                feedback(host, "That is not an NPC.".into());
                return Flow::Cancel;
            };
            if !load(host, npc) {
                return Flow::Cancel;
            }
            let now = now_of(&*host, npc);
            let lines = {
                let Ok(s) = STATE.lock() else { return Flow::Cancel };
                let Some(l) = s.live.get(&npc.id()) else { return Flow::Cancel };
                let c = l.character();
                let mood = &l.heart.mood;
                let m = c.mood(mood);
                let mut lines = vec![
                    format!("{}: {} ({})", l.name, mood, l.heart.character.as_deref().unwrap_or(DEFAULT)),
                    format!("  for {} s; obeys {:.0}%, walks at {:.0}%", (now - l.heart.since) / 20, m.obey * 100.0, m.speed * 100.0),
                ];
                let next: Vec<String> = c.leads_to(mood).iter().map(|(on, to)| format!("{on} > {to}")).collect();
                if !next.is_empty() {
                    lines.push(format!("  next: {}", next.join(", ")));
                }
                let mut bonds = l.heart.bonds.clone();
                bonds.sort_by(|a, b| b.value.total_cmp(&a.value));
                for b in bonds.iter().take(12) {
                    let v = b.value.round() as i32;
                    let what = if b.npc { "NPC" } else { "player" };
                    let met = if b.met > 0 { format!(", met {} times, last {} min ago", b.met, (now - b.seen).max(0) / 1200) } else { String::new() };
                    lines.push(format!("  {} ({what}): {v}, {}{met}", if b.name.is_empty() { "?" } else { &b.name }, c.label(v)));
                }
                for n in l.heart.news.iter().rev().take(6) {
                    lines.push(format!("  news: {} ({} min ago)", n.text(&l.name, ""), (now - n.at).max(0) / 1200));
                }
                if bonds.is_empty() {
                    lines.push("  knows nobody yet".into());
                }
                lines
            };
            for line in lines {
                feedback(host, line);
            }
        }
        _ => return Flow::Continue,
    }
    Flow::Cancel
}
