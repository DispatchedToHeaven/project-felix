//! Routines at work on the server (see `routine`): an NPC living by its day (its orders are
//! `live`) goes where the hour says, sleeps in its home at night, wakes when someone talks to it or
//! hits it (not pleased), and stays awake a minute after.
//!
//! Its places (`home`, `work`, `meet`…) are set by `/felix place` or by its owner in its dialog;
//! one it lacks is its home, and its home is where it stood when it first needed one.
use crate::companion::{self, LiveDo, Mode};
use crate::routine::{self, Doing, Routine};
use crate::sync::{ACTION, Action as Play};
use mod_api::{
    AddServerReloadListenersEvent, AttachmentCodec, AttachmentError, AttachmentType, Command, CommandArgumentValue,
    Compound, EntityRef, FileToIdConverter, Flow, Holder, Identifier, PlayerRef, RegisterCommandsEvent, ReloadApply,
    ReloadResourceError, RunModCommandEvent, ServerHost, ServerReloadContext, ServerReloadListener, Tag, Text,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

const SCAN: i64 = 20;
/// How long it stays awake when woken (ticks), and how near its home it must be to sleep.
const AWAKE: i64 = 1200;
const IN_BED: f64 = 2.0;
/// Ticks between its animation at work.
const BUSY: i64 = 240;
/// The entry an NPC is in while at an event (see `events`), and out of the rain.
const EVENT: usize = usize::MAX;
const SHELTER: usize = usize::MAX - 1;
pub const SLEEP: &str = "animation.felix.sleep";

pub static ROUTINE: AttachmentType<Option<String>> = AttachmentType {
    namespace: "projectfelix",
    name: "routine",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |r| {
            let mut c = Compound::new();
            if let Some(r) = r {
                c.insert("id".into(), Tag::String(r.as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| match c.get(&"id".into()) {
            Some(Tag::String(s)) => Ok(Some(s.to_string_lossy())),
            None => Ok(None),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// An NPC's places by name.
pub static PLACES: AttachmentType<Vec<(String, [f64; 3])>> = AttachmentType {
    namespace: "projectfelix",
    name: "places",
    default: Vec::new,
    codec: Some(AttachmentCodec {
        encode: |places| {
            let mut c = Compound::new();
            for (name, p) in places {
                c.insert(name.as_str().into(), Tag::List(p.iter().map(|v| Tag::Double(v.to_bits())).collect()));
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            let mut out = Vec::new();
            for (k, v) in c.iter() {
                let Tag::List(list) = v else { return Err(AttachmentError::Decode) };
                let mut p = [0.0; 3];
                for (x, t) in p.iter_mut().zip(list) {
                    if let Tag::Double(b) = t {
                        *x = f64::from_bits(*b);
                    }
                }
                out.push((k.to_string_lossy(), p));
            }
            Ok(out)
        },
    }),
    copy_on_death: false,
    sync: false,
};

static LOADED: LazyLock<Mutex<HashMap<String, Arc<Routine>>>> = LazyLock::new(Default::default);

fn routine_named(id: &str) -> Option<Arc<Routine>> {
    LOADED.lock().ok()?.get(id).cloned()
}

struct Listener;

impl ServerReloadListener for Listener {
    fn prepare(&self, cx: &ServerReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        for id in cx.list() {
            let Some(bytes) = cx.read(id) else { continue };
            match routine::parse(bytes) {
                Ok(r) => {
                    next.insert(id.to_string(), Arc::new(r));
                }
                Err(e) => eprintln!("[projectfelix] routine {id} not loaded: {e}"),
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
    let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:routines"), FileToIdConverter::json("felix_routines")) else {
        return;
    };
    if let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener)) {
        eprintln!("[projectfelix] routines listener not added: {err:?}");
    }
}

// ---- live days ------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Day {
    routine: Option<String>,
    read: i64,
    entry: Option<usize>,
    doing: String,
    asleep: bool,
    woke_until: i64,
    busy: i64,
    /// No new game before this.
    rest_until: i64,
    /// The entry's job and place (see `jobs`).
    job: String,
    place: String,
    /// The bed it sleeps in.
    bed: Option<[i32; 3]>,
    /// When it last looked for a bed and found none.
    no_bed: i64,
}

#[derive(Default)]
struct State {
    last: i64,
    days: HashMap<u32, Day>,
    hurt: Vec<u32>,
    /// Beds taken, by the NPC in them.
    beds: HashMap<[i32; 3], u32>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// The server stopped: nothing of its world is kept.
pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn now_of(host: &dyn mod_api::Host, e: EntityRef) -> i64 {
    e.level(host).and_then(|l| l.game_time(host)).unwrap_or(0)
}

/// The NPCs at work by their day: each with its entry's job, if it names one, and its place.
pub fn at_work() -> Vec<(u32, Option<String>, String)> {
    let Ok(s) = STATE.lock() else { return Vec::new() };
    s.days
        .iter()
        .filter(|(id, d)| d.entry.is_some() && (d.doing == "work" || !d.job.is_empty()) && companion::mode_of(**id) == Mode::Live)
        .map(|(id, d)| (*id, Some(d.job.clone()).filter(|j| !j.is_empty()), d.place.clone()))
        .collect()
}

/// Place `name` of NPC `npc`, if it has one.
pub fn place_named(host: &mut dyn ServerHost, npc: EntityRef, name: &str) -> Option<[f64; 3]> {
    PLACES.get(host, Holder::Entity(npc.id())).unwrap_or_default().into_iter().find(|p| p.0 == name).map(|p| p.1)
}

/// What NPC `id` is doing by its day (`work`, `sleep`…; empty: it isn't living by one).
pub fn doing(id: u32) -> String {
    if companion::mode_of(id) != Mode::Live {
        return String::new();
    }
    STATE.lock().ok().and_then(|s| s.days.get(&id).map(|d| d.doing.clone())).unwrap_or_default()
}

/// Whether NPC `id` sleeps now.
pub fn asleep(id: u32) -> bool {
    STATE.lock().ok().and_then(|s| s.days.get(&id).map(|d| d.asleep)).unwrap_or(false)
}

/// Whether NPC `id` has a routine (as last read).
pub fn has_routine(id: u32) -> bool {
    STATE.lock().ok().and_then(|s| s.days.get(&id).map(|d| d.routine.is_some())).unwrap_or(false)
}

/// Place `name` of NPC `npc`: its own, else its home, else (made its home) where it stands.
fn place(host: &mut dyn ServerHost, npc: EntityRef, name: &str) -> [f64; 3] {
    let places = PLACES.get(host, Holder::Entity(npc.id())).unwrap_or_default();
    let found = places.iter().find(|p| p.0 == name).or_else(|| places.iter().find(|p| p.0 == "home")).map(|p| p.1);
    match found {
        Some(p) => p,
        None => {
            let here = npc.position(&*host).unwrap_or([0.0; 3]);
            set_place(host, npc, "home", here);
            here
        }
    }
}

/// Sets place `name` of NPC `npc`; it goes there now if the hour sends it there.
pub fn set_place(host: &mut dyn ServerHost, npc: EntityRef, name: &str, at: [f64; 3]) {
    let mut places = PLACES.get(host, Holder::Entity(npc.id())).unwrap_or_default();
    places.retain(|p| p.0 != name);
    places.push((name.to_string(), at));
    let _ = PLACES.set(host, Holder::Entity(npc.id()), places);
    if let Ok(mut s) = STATE.lock()
        && let Some(d) = s.days.get_mut(&npc.id())
    {
        d.entry = None;
    }
}

/// Gives NPC `npc` routine `id` (none: takes it away), and has it live by it (or follow).
pub fn set_routine(host: &mut dyn ServerHost, npc: EntityRef, id: Option<&str>) -> bool {
    if ROUTINE.set(host, Holder::Entity(npc.id()), id.map(str::to_string)).is_err() {
        return false;
    }
    if let Ok(mut s) = STATE.lock() {
        s.days.remove(&npc.id());
    }
    stop_sleep(host, npc);
    companion::set_orders(host, npc, if id.is_some() { Mode::Live } else { Mode::Follow });
    true
}

fn stop_sleep(host: &mut dyn ServerHost, npc: EntityRef) {
    if ACTION.get(host, Holder::Entity(npc.id())).ok().flatten().is_some_and(|p| p.animation == SLEEP) {
        let _ = ACTION.set(host, Holder::Entity(npc.id()), None);
    }
    // Out of its bed, which is free again.
    let had = STATE.lock().map(|mut s| {
        let before = s.beds.len();
        s.beds.retain(|_, n| *n != npc.id());
        if let Some(d) = s.days.get_mut(&npc.id()) {
            d.bed = None;
        }
        before != s.beds.len()
    });
    if matches!(had, Ok(true)) {
        let _ = mod_api::LivingRef::new(npc).stop_sleeping(host);
    }
}

/// A free bed (its head) within a few blocks of `at`, for NPC `id`.
fn free_bed(host: &dyn mod_api::Host, npc: EntityRef, at: [f64; 3]) -> Option<[i32; 3]> {
    let level = npc.level(host)?;
    let taken: Vec<[i32; 3]> = STATE.lock().ok()?.beds.iter().filter(|(_, n)| **n != npc.id()).map(|(b, _)| *b).collect();
    let (bx, by, bz) = (at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32);
    let mut found = Vec::new();
    for x in bx - 6..=bx + 6 {
        for z in bz - 6..=bz + 6 {
            for y in by - 2..=by + 2 {
                let p = [x, y, z];
                let id = level.block(host, p).map(|k| k.id(host)).unwrap_or_default();
                if id.ends_with("_bed") && level.block_property(host, p, "part").as_deref() == Some("head") && !taken.contains(&p) {
                    found.push(p);
                }
            }
        }
    }
    crate::job::nearest(at, &found)
}

/// Plays `animation` on NPC `npc` (its work, say).
pub fn play_animation(host: &mut dyn ServerHost, npc: EntityRef, animation: &str) {
    play(host, npc, animation);
}

fn play(host: &mut dyn ServerHost, npc: EntityRef, animation: &str) {
    let start = now_of(&*host, npc);
    let _ = ACTION.set(host, Holder::Entity(npc.id()), Some(Play { animation: animation.to_string(), start }));
}

/// Wakes NPC `npc` if it sleeps (`player` woke it): it says so, and stays up a while. True if it slept.
pub fn wake(host: &mut dyn ServerHost, npc: EntityRef, player: Option<PlayerRef>) -> bool {
    let now = now_of(&*host, npc);
    let slept = match STATE.lock() {
        Ok(mut s) => match s.days.get_mut(&npc.id()) {
            Some(d) if d.asleep => {
                d.asleep = false;
                d.woke_until = now + AWAKE;
                true
            }
            _ => false,
        },
        Err(_) => false,
    };
    if slept {
        stop_sleep(host, npc);
        let name = player.and_then(|p| p.name(&*host)).unwrap_or_default();
        let line = if name.is_empty() { "Huh? What was that?".to_string() } else { format!("Mm? {name}? Can't a person sleep?") };
        crate::heart::say_aloud(host, npc, &line);
        crate::heart::feel(host, npc, player, "woken");
    }
    slept
}

/// `living_damage_post`: a sleeper hit wakes up.
pub fn on_hurt(victim: i32) {
    if victim > 0
        && let Ok(mut s) = STATE.lock()
        && s.days.get(&(victim as u32)).is_some_and(|d| d.asleep)
    {
        s.hurt.push(victim as u32);
    }
}

pub fn tick(host: &mut dyn ServerHost) {
    let npcs = companion::npcs();
    let Some(now) = npcs.iter().find_map(|id| host.entity(*id).map(|e| now_of(&*host, e))) else { return };
    let hurt = {
        let Ok(mut s) = STATE.lock() else { return };
        if now - s.last < SCAN && now >= s.last {
            return;
        }
        s.last = now;
        s.days.retain(|id, _| npcs.contains(id));
        s.beds.retain(|_, n| npcs.contains(n));
        std::mem::take(&mut s.hurt)
    };
    for id in hurt {
        if let Some(e) = host.entity(id) {
            wake(host, e, None);
        }
    }
    for id in npcs {
        let Some(me) = host.entity(id) else { continue };
        let living = companion::mode_of(id) == Mode::Live;
        // Read its routine now and then (a command may change it).
        let stale = STATE.lock().map(|s| s.days.get(&id).is_none_or(|d| now - d.read >= 100 || now < d.read)).unwrap_or(false);
        if stale {
            let r = ROUTINE.get(host, Holder::Entity(id)).ok().flatten();
            if let Ok(mut s) = STATE.lock() {
                let d = s.days.entry(id).or_default();
                if d.routine != r {
                    d.entry = None;
                }
                d.routine = r;
                d.read = now;
            }
        }
        let (rid, entry, asleep, woke_until, busy) = match STATE.lock() {
            Ok(s) => match s.days.get(&id) {
                Some(d) => (d.routine.clone(), d.entry, d.asleep, d.woke_until, d.busy),
                None => continue,
            },
            Err(_) => continue,
        };
        if !living {
            // A game at its play time: the day goes on after it.
            if crate::games::busy(id) || crate::matches::busy(id) {
                continue;
            }
            if asleep {
                stop_sleep(host, me);
            }
            if let Ok(mut s) = STATE.lock()
                && let Some(d) = s.days.get_mut(&id)
            {
                d.entry = None;
                d.asleep = false;
                d.doing.clear();
            }
            continue;
        }
        let Some(r) = rid.as_deref().and_then(routine_named) else { continue };
        let Some(t) = me.level(&*host).and_then(|l| l.day_time(&*host)) else { continue };
        // An event on: it goes there, whatever its day has it do.
        if let Some(g) = crate::events::gathering(host, me) {
            if entry != Some(EVENT) {
                if asleep {
                    stop_sleep(host, me);
                }
                companion::set_live(host, me, g.spot, LiveDo::Stay);
                if let Some(line) = &g.say {
                    crate::heart::say_aloud(host, me, line);
                }
                crate::rulebook::notice(id, "starts:event", None);
                if let Ok(mut s) = STATE.lock()
                    && let Some(d) = s.days.get_mut(&id)
                {
                    d.entry = Some(EVENT);
                    d.doing = "event".into();
                    d.job.clear();
                    d.place.clear();
                    d.asleep = false;
                }
            } else if now - busy >= BUSY && !crate::scenes::busy(id) && me.position(&*host).is_some_and(|p| dist(p, g.spot) <= 5.0) {
                if let Some(a) = &g.animation {
                    play(host, me, a);
                }
                if let Ok(mut s) = STATE.lock()
                    && let Some(d) = s.days.get_mut(&id)
                {
                    d.busy = now;
                }
            }
            continue;
        }
        let (i, e) = r.now(t);
        // Rain: it goes home, unless it sleeps, follows or guards, or is home already.
        // (Rain that falls on it: one at work under a roof stays there. Once inside it waits the rain out.)
        let raining = me.level(&*host).is_some_and(|l| {
            l.is_raining(&*host) == Some(true)
                && (entry == Some(SHELTER)
                    || me.position(&*host).is_some_and(|p| l.is_raining_at(&*host, [p[0].floor() as i32, p[1].floor() as i32 + 1, p[2].floor() as i32]) != Some(false)))
        });
        if raining
            && matches!(e.doing, Doing::Be | Doing::Play)
            && e.place != "home"
            && let Some(home) = place_named(host, me, "home")
        {
            if entry != Some(SHELTER) {
                companion::set_live(host, me, home, LiveDo::Stay);
                crate::heart::say_aloud(host, me, ["It's raining! I'm going inside.", "Rain! Home I go.", "Brr, wet. Let's get under a roof."][(id as usize) % 3]);
                if let Ok(mut s) = STATE.lock()
                    && let Some(d) = s.days.get_mut(&id)
                {
                    d.entry = Some(SHELTER);
                    d.doing = "shelter".into();
                    d.job.clear();
                    d.place = "home".into();
                }
            }
            continue;
        }
        let mut at = place(host, me, &e.place);
        // To sleep: in a free bed near its home, if there is one.
        if e.doing == Doing::Sleep {
            let (kept, looked) = STATE.lock().ok().and_then(|s| s.days.get(&id).map(|d| (d.bed, d.no_bed))).unwrap_or((None, 0));
            let bed = match kept {
                Some(b) if entry == Some(i) => Some(b),
                // None about when it last looked: it looks again in half a minute.
                _ if looked != 0 && now >= looked && now - looked < 600 => None,
                _ => {
                    let b = free_bed(&*host, me, at);
                    if let Ok(mut s) = STATE.lock()
                        && let Some(d) = s.days.get_mut(&id)
                    {
                        d.no_bed = if b.is_none() { now.max(1) } else { 0 };
                    }
                    b
                }
            };
            if let Some(b) = bed {
                if let Ok(mut s) = STATE.lock() {
                    s.beds.insert(b, id);
                    if let Some(d) = s.days.get_mut(&id) {
                        d.bed = Some(b);
                    }
                }
                at = [f64::from(b[0]) + 0.5, f64::from(b[1]), f64::from(b[2]) + 0.5];
            }
        }
        if entry != Some(i) {
            if asleep {
                stop_sleep(host, me);
            }
            let what = match e.doing {
                Doing::Follow => LiveDo::Follow,
                Doing::Sleep => LiveDo::Rest,
                Doing::Guard => LiveDo::Guard,
                Doing::Be | Doing::Play => LiveDo::Stay,
            };
            companion::set_live(host, me, at, what);
            if let Some(line) = e.say.get((now as usize / SCAN as usize) % e.say.len().max(1)) {
                crate::heart::say_aloud(host, me, line);
            }
            crate::rulebook::notice(id, &format!("starts:{}", e.name), None);
            if let Ok(mut s) = STATE.lock()
                && let Some(d) = s.days.get_mut(&id)
            {
                d.entry = Some(i);
                d.doing = e.name.clone();
                d.job = e.job.clone();
                d.place = e.place.clone();
                d.asleep = false;
            }
            continue;
        }
        let near = me.position(&*host).is_some_and(|p| dist(p, at) <= IN_BED);
        match e.doing {
            Doing::Sleep if near && !asleep && now >= woke_until => {
                if let Some(b) = STATE.lock().ok().and_then(|s| s.days.get(&id).and_then(|d| d.bed)) {
                    let _ = mod_api::LivingRef::new(me).start_sleeping(host, b);
                }
                play(host, me, SLEEP);
                if let Ok(mut s) = STATE.lock()
                    && let Some(d) = s.days.get_mut(&id)
                {
                    d.asleep = true;
                }
            }
            Doing::Play if me.position(&*host).is_some_and(|p| dist(p, at) <= 4.0) => {
                find_partner(host, me, &e.game, now);
            }
            Doing::Be if now - busy >= BUSY && me.position(&*host).is_some_and(|p| dist(p, at) <= 4.0) => {
                if let Some(a) = &e.animation {
                    play(host, me, a);
                }
                if let Ok(mut s) = STATE.lock()
                    && let Some(d) = s.days.get_mut(&id)
                {
                    d.busy = now;
                }
            }
            _ => {}
        }
    }
}

/// NPC `me`, at its play time, looks for another NPC playing near it, and they start `game`.
fn find_partner(host: &mut dyn ServerHost, me: EntityRef, game: &str, now: i64) {
    if crate::games::busy(me.id()) || crate::matches::busy(me.id()) {
        return;
    }
    let Some(at) = me.position(&*host) else { return };
    let others: Vec<u32> = STATE
        .lock()
        .map(|s| s.days.iter().filter(|(id, d)| **id > me.id() && d.doing == "play" && now >= d.rest_until).map(|(id, _)| *id).collect())
        .unwrap_or_default();
    // The lower id asks; one rests a while after a game.
    if STATE.lock().ok().and_then(|s| s.days.get(&me.id()).map(|d| now < d.rest_until)).unwrap_or(true) {
        return;
    }
    for o in others {
        let Some(other) = host.entity(o) else { continue };
        if crate::games::busy(o) || crate::matches::busy(o) || other.position(&*host).is_none_or(|p| dist(p, at) > 8.0) {
            continue;
        }
        let choice = if game == "any" || game.is_empty() {
            ["chess", "checkers", "table_tennis", "tennis", "basketball"][(now as usize / 20 + me.id() as usize) % 5]
        } else {
            game
        };
        let name = other.name(&*host).unwrap_or_else(|| "friend".into());
        let pretty = choice.replace('_', " ");
        let started = if let Some(k) = crate::games::GameKind::parse(choice) {
            crate::games::start_npcs(host, me, other, k)
        } else if let Some(s) = crate::sports::Sport::parse(choice) {
            crate::matches::start_npcs(host, me, other, s)
        } else {
            Err(format!("no game {choice}"))
        };
        if started.is_ok() {
            crate::heart::say_aloud(host, me, &format!("Fancy a game of {pretty}, {name}?"));
            crate::heart::say_aloud(host, other, "You're on!");
            if let Ok(mut s) = STATE.lock() {
                for id in [me.id(), o] {
                    if let Some(d) = s.days.get_mut(&id) {
                        d.rest_until = now + 600;
                    }
                }
            }
        }
        return;
    }
}

// ---- commands -------------------------------------------------------------------------------------------------

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix routine <targets:entities> <routine:text>", 2));
    e.commands.push(Command::op("felix routine clear <targets:entities>", 2));
    e.commands.push(Command::op("felix place <targets:entities> <name:word>", 2));
    e.commands.push(Command::op("felix day <target:entity>", 2));
    e.commands.push(Command::op("felix play <first:entity> <second:entity> <game:word>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let feedback = |host: &mut dyn ServerHost, line: String| {
        let _ = host.commands().feedback(Text::literal(line));
    };
    let npcs = |host: &dyn mod_api::Host, e: &RunModCommandEvent<'_>| -> Vec<EntityRef> {
        let mut list = companion::targets(e);
        if let Some(CommandArgumentValue::Entity(one)) = e.command.arguments.iter().find(|a| a.name == "target").map(|a| a.value.clone()) {
            list.push(one);
        }
        list.into_iter().filter(|t| companion::is_npc(host, *t)).collect()
    };
    match lit.as_slice() {
        ["felix", "routine"] => {
            let id = companion::text_argument(e, "routine").unwrap_or_default().trim().to_string();
            let n = npcs(&*host, e).into_iter().filter(|t| set_routine(host, *t, Some(&id))).count();
            let note = if routine_named(&id).is_some() { "" } else { " (no such routine is loaded yet)" };
            feedback(host, format!("{n} NPCs now live by {id}{note}"));
        }
        ["felix", "routine", "clear"] => {
            let n = npcs(&*host, e).into_iter().filter(|t| set_routine(host, *t, None)).count();
            feedback(host, format!("{n} NPCs have no routine now"));
        }
        ["felix", "place"] => {
            let name = companion::text_argument(e, "name").unwrap_or_default();
            let at = e.command.position;
            let list = npcs(&*host, e);
            for t in &list {
                set_place(host, *t, &name, at);
            }
            feedback(host, format!("{} NPCs: {name} is at {:.0} {:.0} {:.0}", list.len(), at[0], at[1], at[2]));
        }
        ["felix", "play"] => {
            let one = |n: &str| e.command.arguments.iter().find(|a| a.name == n).and_then(|a| match &a.value {
                CommandArgumentValue::Entity(x) => Some(*x),
                CommandArgumentValue::Entities(l) => l.first().copied(),
                _ => None,
            });
            let (Some(a), Some(b)) = (one("first"), one("second")) else { return Flow::Cancel };
            if !companion::is_npc(&*host, a) || !companion::is_npc(&*host, b) || a.id() == b.id() {
                feedback(host, "Two NPCs play.".into());
                return Flow::Cancel;
            }
            let game = companion::text_argument(e, "game").unwrap_or_default();
            let r = if let Some(k) = crate::games::GameKind::parse(&game) {
                crate::games::start_npcs(host, a, b, k)
            } else if let Some(s) = crate::sports::Sport::parse(&game) {
                crate::matches::start_npcs(host, a, b, s)
            } else {
                Err("games: chess, checkers, table_tennis, tennis, basketball".into())
            };
            match r {
                Ok(()) => feedback(host, format!("They play {game}.")),
                Err(err) => feedback(host, err),
            }
        }
        ["felix", "day"] => {
            let Some(npc) = npcs(&*host, e).into_iter().next() else {
                feedback(host, "That is not an NPC.".into());
                return Flow::Cancel;
            };
            let name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
            let id = ROUTINE.get(host, Holder::Entity(npc.id())).ok().flatten();
            let t = npc.level(&*host).and_then(|l| l.day_time(&*host)).unwrap_or(0);
            let mut lines = vec![format!("{name}: {} (now {}, orders {:?})", id.as_deref().unwrap_or("no routine"), routine::hour(t), companion::mode_of(npc.id()))];
            if let Some(r) = id.as_deref().and_then(routine_named) {
                lines.push(format!("  {}", r.describe()));
                let d = doing(npc.id());
                if !d.is_empty() {
                    let asleep = STATE.lock().ok().and_then(|s| s.days.get(&npc.id()).map(|d| d.asleep)).unwrap_or(false);
                    lines.push(format!("  doing: {d}{}", if asleep { " (asleep)" } else { "" }));
                }
            }
            for (p, at) in PLACES.get(host, Holder::Entity(npc.id())).unwrap_or_default() {
                lines.push(format!("  {p}: {:.0} {:.0} {:.0}", at[0], at[1], at[2]));
            }
            for line in lines {
                feedback(host, line);
            }
        }
        _ => return Flow::Continue,
    }
    Flow::Cancel
}
