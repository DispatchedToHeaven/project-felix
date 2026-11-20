//! Village events on the server (see `event`): on when the calendar says (or `/felix event
//! start`), the NPCs living by a day gather at the event's spot, the players are told, the
//! scene plays, shops sell for less and the event's quests are offered.
use crate::event::{self, Event};
use crate::serde_json::{Map, Value, json};
use mod_api::{
    AddServerReloadListenersEvent, Command, ConfigPath, EntityRef, FileToIdConverter, Flow, Identifier, LevelRef,
    PlayerRef, RegisterCommandsEvent, ReloadApply, ReloadResourceError, RunModCommandEvent, ServerHost,
    ServerReloadContext, ServerReloadListener, Text,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

static LOADED: LazyLock<Mutex<HashMap<String, Arc<Event>>>> = LazyLock::new(Default::default);

struct Listener;

impl ServerReloadListener for Listener {
    fn prepare(&self, cx: &ServerReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        for id in cx.list() {
            let Some(bytes) = cx.read(id) else { continue };
            match event::parse(bytes) {
                Ok(e) => {
                    next.insert(id.to_string(), Arc::new(e));
                }
                Err(e) => eprintln!("[projectfelix] event {id} not loaded: {e}"),
            }
        }
        Ok(<dyn ReloadApply>::new(move || {
            if let Ok(mut live) = LOADED.lock() {
                *live = next;
            }
        }))
    }
}

pub fn add_listener(e: &mut AddServerReloadListenersEvent<'_>) {
    let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:events"), FileToIdConverter::json("felix_events")) else {
        return;
    };
    if let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener)) {
        eprintln!("[projectfelix] events listener not added: {err:?}");
    }
}

fn event_named(id: &str) -> Option<Arc<Event>> {
    LOADED.lock().ok()?.get(id).cloned()
}

/// An event that is on.
struct On {
    event: Arc<Event>,
    started: i64,
    scene_played: bool,
}

#[derive(Default)]
struct State {
    on: HashMap<String, On>,
    /// Started by command: until when (game time).
    forced: HashMap<String, i64>,
    /// Stopped by command: until when its hours are over.
    stopped: HashMap<String, i64>,
    /// Where each event gathers (`/felix event here`), kept in `events/spots.json`.
    spots: HashMap<String, [f64; 3]>,
    spots_read: bool,
    ticks: u32,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

const SPOTS: &str = "events/spots.json";
const OVERWORLD: &str = "minecraft:overworld";

fn read_spots(host: &mut dyn ServerHost) {
    if !crate::saves::new_world(host, "events") {
        return;
    }
    let mut spots = HashMap::new();
    if let Some(bytes) = ConfigPath::new(SPOTS).ok().and_then(|p| host.world_data_files().read(&p).ok())
        && let Ok(Value::Object(m)) = crate::serde_json::from_slice::<Value>(&bytes)
    {
        for (k, v) in m {
            if let Some(a) = v.as_array()
                && let [Some(x), Some(y), Some(z)] = [a.first(), a.get(1), a.get(2)].map(|c| c.and_then(Value::as_f64))
            {
                spots.insert(k, [x, y, z]);
            }
        }
    }
    if let Ok(mut s) = STATE.lock() {
        s.spots = spots;
        s.spots_read = true;
    }
}

fn write_spots(host: &mut dyn ServerHost) {
    let m: Map<String, Value> = STATE.lock().map(|s| s.spots.iter().map(|(k, p)| (k.clone(), json!(p))).collect()).unwrap_or_default();
    if let Ok(p) = ConfigPath::new(SPOTS)
        && let Err(e) = host.world_data_files().write(&p, &crate::serde_json::to_vec_pretty(&Value::Object(m)).unwrap_or_default())
    {
        eprintln!("[projectfelix] event spots not saved: {e:?}");
    }
}

fn players(host: &dyn mod_api::Host) -> Vec<PlayerRef> {
    ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"]
        .iter()
        .filter_map(|d| host.level(d))
        .flat_map(|l| l.online_players(host).unwrap_or_default())
        .collect()
}

fn tell_all(host: &mut dyn ServerHost, line: &str) {
    for p in players(&*host) {
        let _ = p.tell(host, Text::literal(line.to_string()));
    }
}

fn title(e: &Event) -> String {
    let mut c = e.name.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The server tick: events come on and go off by the calendar; a scene plays at each.
pub fn tick(host: &mut dyn ServerHost) {
    let step = STATE.lock().map(|mut s| {
        s.ticks = s.ticks.wrapping_add(1);
        s.ticks % 20 == 0
    });
    if !matches!(step, Ok(true)) {
        return;
    }
    read_spots(host);
    let Some(level) = host.level(OVERWORLD) else { return };
    let (Some(now), Some(t)) = (level.game_time(&*host), level.day_time(&*host)) else { return };
    let events: Vec<(String, Arc<Event>)> = LOADED.lock().map(|l| l.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    if let Ok(mut s) = STATE.lock() {
        s.forced.retain(|_, until| now < *until);
        s.stopped.retain(|_, until| now < *until);
        for (id, e) in &events {
            let on = (s.forced.contains_key(id) || e.on_at(t)) && !s.stopped.contains_key(id);
            if on && !s.on.contains_key(id) {
                s.on.insert(id.clone(), On { event: e.clone(), started: now, scene_played: e.scene.is_none() });
                starts.push(e.clone());
            } else if !on && let Some(o) = s.on.remove(id) {
                ends.push(o.event);
            }
        }
        // Events no longer loaded end too.
        let gone: Vec<String> = s.on.keys().filter(|k| !events.iter().any(|(id, _)| id == *k)).cloned().collect();
        for k in gone {
            if let Some(o) = s.on.remove(&k) {
                ends.push(o.event);
            }
        }
    }
    for e in starts {
        tell_all(host, &format!("[{}] It has begun! Everyone is going to the {}.", title(&e), if e.place == "meet" { "meeting place".to_string() } else { e.place.clone() }));
    }
    for e in ends {
        tell_all(host, &format!("[{}] It is over for now.", title(&e)));
    }
    play_scenes(host, level, now);
}

/// Scenes due at events on: led by the NPC nearest the spot that is there.
fn play_scenes(host: &mut dyn ServerHost, level: LevelRef, now: i64) {
    let due: Vec<(String, Arc<Event>)> = STATE
        .lock()
        .map(|s| s.on.iter().filter(|(_, o)| !o.scene_played && now - o.started >= (o.event.scene_after * 20.0) as i64).map(|(k, o)| (k.clone(), o.event.clone())).collect())
        .unwrap_or_default();
    for (id, e) in due {
        if let Ok(mut s) = STATE.lock()
            && let Some(o) = s.on.get_mut(&id)
        {
            o.scene_played = true;
        }
        let Some(scene) = &e.scene else { continue };
        let joined: Vec<EntityRef> = crate::companion::npcs().into_iter().filter_map(|n| host.entity(n)).filter(|n| n.level(&*host) == Some(level)).collect();
        let mut best: Option<(f64, EntityRef)> = None;
        for n in joined {
            let Some(g) = gathering(host, n) else { continue };
            if g.id != id {
                continue;
            }
            let Some(p) = n.position(&*host) else { continue };
            let d = (p[0] - g.spot[0]).powi(2) + (p[2] - g.spot[2]).powi(2);
            if d <= 64.0 && best.is_none_or(|b| d < b.0) {
                best = Some((d, n));
            }
        }
        match best {
            Some((_, lead)) => {
                if let Err(err) = crate::scenes::start(host, lead, scene, None) {
                    eprintln!("[projectfelix] event scene {scene}: {err}");
                }
            }
            None => eprintln!("[projectfelix] event {id}: no NPC at its spot for {scene}"),
        }
    }
}

/// Where an NPC goes for an event, and what it does there.
pub struct Gather {
    pub id: String,
    pub spot: [f64; 3],
    pub say: Option<String>,
    pub animation: Option<String>,
}

/// The event NPC `npc` goes to now, if one is on that it joins.
pub fn gathering(host: &mut dyn ServerHost, npc: EntityRef) -> Option<Gather> {
    let on: Vec<(String, Arc<Event>, Option<[f64; 3]>)> = {
        let s = STATE.lock().ok()?;
        if s.on.is_empty() {
            return None;
        }
        let mut v: Vec<_> = s.on.iter().map(|(k, o)| (k.clone(), o.event.clone(), s.spots.get(k).copied())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    let tags = npc.tags(&*host).unwrap_or_default();
    for (id, e, spot) in on {
        if e.who.as_ref().is_some_and(|w| !tags.contains(w)) {
            continue;
        }
        let spot = spot
            .or_else(|| crate::routines::place_named(host, npc, &e.place))
            .or_else(|| crate::routines::place_named(host, npc, "meet"))
            .or_else(|| crate::routines::place_named(host, npc, "home"));
        let Some(spot) = spot else { continue };
        let say = (!e.say.is_empty()).then(|| e.say[npc.id() as usize % e.say.len()].clone());
        return Some(Gather { id, spot, say, animation: e.animation.clone() });
    }
    None
}

/// The event NPC `npc` is at now, if any.
fn event_of(host: &mut dyn ServerHost, npc: EntityRef) -> Option<Arc<Event>> {
    if crate::routines::doing(npc.id()) != "event" {
        return None;
    }
    let g = gathering(host, npc)?;
    STATE.lock().ok()?.on.get(&g.id).map(|o| o.event.clone())
}

/// What NPC `npc` sells for `base` now (less at an event).
pub fn price(host: &mut dyn ServerHost, npc: EntityRef, base: u32) -> u32 {
    event_of(host, npc).map_or(base, |e| e.price(base))
}

/// The quests NPC `npc` offers for the event it is at.
pub fn quests_for(host: &mut dyn ServerHost, npc: EntityRef) -> Vec<String> {
    event_of(host, npc).map(|e| e.quests.clone()).unwrap_or_default()
}

/// The id of the event NPC `npc` is at.
pub fn id_at(host: &mut dyn ServerHost, npc: EntityRef) -> Option<String> {
    if crate::routines::doing(npc.id()) != "event" {
        return None;
    }
    gathering(host, npc).map(|g| g.id)
}

/// The name of the event NPC `npc` is at (for `@event`).
pub fn name_at(host: &mut dyn ServerHost, npc: EntityRef) -> Option<String> {
    event_of(host, npc).map(|e| e.name.clone())
}

/// The events on now (their ids).
pub fn on_now() -> Vec<String> {
    STATE.lock().map(|s| s.on.keys().cloned().collect()).unwrap_or_default()
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix event list", 2));
    e.commands.push(Command::op("felix event start <id:text>", 2));
    e.commands.push(Command::op("felix event stop <id:text>", 2));
    e.commands.push(Command::op("felix event here <id:text>", 2));
}

fn full_id(id: &str) -> String {
    let id = id.trim();
    if id.contains(':') { id.to_string() } else { format!("projectfelix:{id}") }
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let what = match lit.as_slice() {
        ["felix", "event", w] => *w,
        _ => return Flow::Continue,
    };
    read_spots(host);
    let level = host.level(OVERWORLD);
    let now = level.and_then(|l| l.game_time(&*host)).unwrap_or(0);
    let t = level.and_then(|l| l.day_time(&*host)).unwrap_or(0);
    let id = full_id(&crate::companion::text_argument(e, "id").unwrap_or_default());
    let lines: Vec<String> = match what {
        "list" => {
            let mut list: Vec<(String, Arc<Event>)> = LOADED.lock().map(|l| l.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
            list.sort_by_key(|(_, ev)| ev.next_start(t));
            let on = on_now();
            let mut out = vec![format!("Now: {}", event::when(t))];
            for (k, ev) in list {
                let state = if on.contains(&k) { "on now".to_string() } else { format!("next {}", event::when(ev.next_start(t))) };
                let spot = STATE.lock().ok().and_then(|s| s.spots.get(&k).copied()).map_or_else(|| format!("each NPC's {}", ev.place), |p| format!("{:.0} {:.0} {:.0}", p[0], p[1], p[2]));
                out.push(format!("{k} ({}): every {} days, {state}; at {spot}", ev.name, ev.every));
            }
            out
        }
        "start" => match event_named(&id) {
            Some(ev) => {
                if let Ok(mut s) = STATE.lock() {
                    s.stopped.remove(&id);
                    s.forced.insert(id.clone(), now + ev.length());
                }
                vec![format!("{id} starts, for {} minutes", ev.length() / 1200)]
            }
            None => vec![format!("No event {id}")],
        },
        "stop" => match event_named(&id) {
            Some(ev) => {
                if let Ok(mut s) = STATE.lock() {
                    s.forced.remove(&id);
                    s.stopped.insert(id.clone(), now + ev.length());
                }
                vec![format!("{id} stops")]
            }
            None => vec![format!("No event {id}")],
        },
        "here" => {
            let at = match e.source {
                mod_api::CommandSource::Player { player, .. } => player.entity().position(&*host),
                _ => None,
            };
            match (event_named(&id), at) {
                (Some(_), Some(p)) => {
                    if let Ok(mut s) = STATE.lock() {
                        s.spots.insert(id.clone(), p);
                    }
                    write_spots(host);
                    vec![format!("{id} gathers here: {:.0} {:.0} {:.0}", p[0], p[1], p[2])]
                }
                (None, _) => vec![format!("No event {id}")],
                (_, None) => vec!["Stand where it gathers.".into()],
            }
        }
        _ => return Flow::Continue,
    };
    for l in lines {
        let _ = host.commands().feedback(Text::literal(l));
    }
    Flow::Cancel
}

/// The event on now, else the next one, in words (empty: none known).
pub fn next_up(host: &mut dyn ServerHost) -> String {
    let Some(t) = host.level(OVERWORLD).and_then(|l| l.day_time(&*host)) else { return String::new() };
    let on = on_now();
    let list: Vec<(String, Arc<Event>)> = LOADED.lock().map(|l| l.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
    if let Some((_, e)) = list.iter().filter(|(k, _)| on.contains(k)).min_by(|a, b| a.0.cmp(&b.0)) {
        return format!("On now: {}", title(e));
    }
    let Some((_, e)) = list.iter().min_by_key(|(k, e)| (e.next_start(t), k.clone())) else { return String::new() };
    let start = e.next_start(t);
    let days = start.div_euclid(24000) - t.div_euclid(24000);
    let when = match days {
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        d => format!("in {d} days"),
    };
    format!("Next: {}, {when} ({})", title(e), event::when(start))
}
