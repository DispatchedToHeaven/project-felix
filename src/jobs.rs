//! Jobs on the server (see `job`): NPCs at their day's work farm, fell trees or fish about their
//! work place, and keep what they get in their chest, which their shop sells from.
use crate::companion::{self, LiveDo};
use crate::job::{self, DEPTH, Job, MAX_TRUNK, RADIUS};
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, Command, CommandArgumentValue, Compound, EntityRef, Flow, Hand,
    Holder, HostTransaction, ItemResource, ItemStack, LevelRef, LivingRef, RegisterCommandsEvent, RunModCommandEvent,
    STORAGE, ServerHost, SoundSource, Tag, Text,
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// An NPC's own job (`farm`, `wood`, `fish`).
pub static JOB: AttachmentType<Option<String>> = AttachmentType {
    namespace: "projectfelix",
    name: "job",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |v| {
            let mut c = Compound::new();
            if let Some(v) = v {
                c.insert("name".into(), Tag::String(v.as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| match c.get(&"name".into()) {
            Some(Tag::String(s)) => Ok(Some(s.to_string_lossy())),
            None => Ok(None),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// How one NPC's work goes.
#[derive(Clone, Default)]
struct Working {
    job: Option<Job>,
    /// Work found about its place, not done yet, and when it last looked.
    queue: Vec<[i32; 3]>,
    scanned: i64,
    /// What it goes to now, since when.
    target: Option<[i32; 3]>,
    since: i64,
    /// No new work before this.
    next: i64,
    /// Pieces of work done (it says something now and then).
    done: u32,
    /// When it last said it has nowhere to put things.
    complained: i64,
    idle: bool,
}

#[derive(Default)]
struct State {
    working: HashMap<u32, Working>,
    /// Each NPC's chest as last found: where, and when it looked.
    chests: HashMap<u32, (Option<[i32; 3]>, i64)>,
    ticks: u32,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// The server stopped: nothing of its world is kept.
pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

/// Ticks between two looks at work, and between two looks for a chest.
const STEP: u32 = 5;
const LOOK_AGAIN: i64 = 100;
const CHEST_AGAIN: i64 = 600;
/// It gives up on what it can't reach after this.
const GIVE_UP: i64 = 300;

fn center(p: [i32; 3]) -> [f64; 3] {
    [f64::from(p[0]) + 0.5, f64::from(p[1]), f64::from(p[2]) + 0.5]
}

fn block_at(host: &dyn mod_api::Host, level: LevelRef, p: [i32; 3]) -> String {
    level.block(host, p).map(|k| k.id(host)).unwrap_or_default()
}

/// NPC `npc`'s own job, if it has one.
pub fn job_of(host: &mut dyn ServerHost, npc: EntityRef) -> Option<Job> {
    JOB.get(host, Holder::Entity(npc.id())).ok().flatten().as_deref().and_then(Job::parse)
}

/// Gives NPC `npc` job `name` (`none`: takes it away).
pub fn set_job(host: &mut dyn ServerHost, npc: EntityRef, name: &str) {
    let v = Job::parse(name).map(|j| j.name().to_string());
    let _ = JOB.set(host, Holder::Entity(npc.id()), v);
    if let Ok(mut s) = STATE.lock() {
        s.working.remove(&npc.id());
        s.chests.remove(&npc.id());
    }
}

/// Whether the trunk at `p` (of `log`, on the ground) is a tree's: it has leaves by its top. A
/// post of a house, or a pillar someone set, is left alone.
fn is_tree(host: &dyn mod_api::Host, level: LevelRef, p: [i32; 3], log: &str) -> bool {
    let mut top = p;
    for dy in 1..MAX_TRUNK {
        let q = [p[0], p[1] + dy, p[2]];
        if block_at(host, level, q) != log {
            break;
        }
        top = q;
    }
    const ABOUT: [[i32; 3]; 9] = [[0, 1, 0], [1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1], [1, 1, 0], [-1, 1, 0], [0, 1, 1], [0, 1, -1]];
    ABOUT.iter().any(|d| job::is_crown(&block_at(host, level, [top[0] + d[0], top[1] + d[1], top[2] + d[2]])))
}

/// The work about `at` for `job`: ripe crops, trees' trunks standing on the ground, or water.
fn scan(host: &dyn mod_api::Host, level: LevelRef, job: Job, at: [f64; 3]) -> Vec<[i32; 3]> {
    let (bx, by, bz) = (at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32);
    let mut out = Vec::new();
    let r = if job == Job::Fish { 6 } else { RADIUS };
    for x in bx - r..=bx + r {
        for z in bz - r..=bz + r {
            for y in by - DEPTH..=by + DEPTH {
                let p = [x, y, z];
                let id = block_at(host, level, p);
                let found = match job {
                    Job::Farm => job::is_crop(&id) && job::ripe(&id, level.block_property(host, p, "age").and_then(|a| a.parse().ok())).is_some(),
                    Job::Wood => job::is_log(&id) && job::is_soil(&block_at(host, level, [x, y - 1, z])) && is_tree(host, level, p, &id),
                    Job::Fish => id == "minecraft:water" && block_at(host, level, [x, y + 1, z]) == "minecraft:air",
                    Job::Build | Job::Bake => false,
                };
                if found {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Whether `p` is still work for `job`.
fn still(host: &dyn mod_api::Host, level: LevelRef, job: Job, p: [i32; 3]) -> bool {
    let id = block_at(host, level, p);
    match job {
        Job::Farm => job::ripe(&id, level.block_property(host, p, "age").and_then(|a| a.parse().ok())).is_some(),
        Job::Wood => job::is_log(&id),
        Job::Fish => id == "minecraft:water",
        Job::Build | Job::Bake => false,
    }
}

fn run_in(level: LevelRef, line: String) {
    crate::talk::queue(format!("execute in {} run {line}", level.dimension_id()));
}

/// Does the piece of work at `t`: what it got.
fn act(host: &mut dyn ServerHost, me: EntityRef, level: LevelRef, job: Job, t: [i32; 3]) -> Vec<ItemStack> {
    let _ = LivingRef::new(me).swing(host, Hand::Main, false);
    crate::routines::play_animation(host, me, "animation.felix.swing");
    match job {
        Job::Farm => {
            let crop = block_at(&*host, level, t);
            let Some(seed) = job::ripe(&crop, level.block_property(&*host, t, "age").and_then(|a| a.parse().ok())) else { return Vec::new() };
            let mut got = level.destroy_block(host, t, true, None, true).ok().flatten().unwrap_or_default();
            // One seed goes back in the ground.
            if let Some(i) = got.iter().position(|s| s.item().id(&*host) == seed) {
                let n = got[i].count();
                if n > 1 {
                    let _ = got[i].set_count(n - 1);
                } else {
                    got.remove(i);
                }
            }
            run_in(level, format!("setblock {} {} {} {crop}", t[0], t[1], t[2]));
            got
        }
        Job::Wood => {
            let log = block_at(&*host, level, t);
            let mut got = Vec::new();
            for dy in 0..MAX_TRUNK {
                let p = [t[0], t[1] + dy, t[2]];
                if block_at(&*host, level, p) != log {
                    break;
                }
                got.extend(level.destroy_block(host, p, true, None, true).ok().flatten().unwrap_or_default());
            }
            if let Some(s) = job::sapling(&log)
                && job::is_soil(&block_at(&*host, level, [t[0], t[1] - 1, t[2]]))
            {
                run_in(level, format!("setblock {} {} {} {s}", t[0], t[1], t[2]));
            }
            got
        }
        Job::Build | Job::Bake => Vec::new(),
        Job::Fish => {
            let at = center(t);
            let _ = level.play_sound(host, "minecraft:entity.fishing_bobber.splash", [at[0], at[1] + 1.0, at[2]], SoundSource::Neutral, 0.6, 1.0);
            let rod = stack(&*host, "minecraft:fishing_rod", 1);
            level.roll_loot(host, "minecraft:gameplay/fishing", [at[0], at[1] + 1.0, at[2]], rod.as_ref(), Some(me)).ok().flatten().unwrap_or_default()
        }
    }
}

/// Where NPC `npc` keeps what it gets: the chest or barrel nearest its `chest` place, else its
/// `work` place.
fn find_chest(host: &mut dyn ServerHost, npc: EntityRef, level: LevelRef) -> Option<[i32; 3]> {
    let (at, r) = match crate::routines::place_named(host, npc, "chest") {
        Some(p) => (p, 3),
        None => (crate::routines::place_named(host, npc, "work")?, 6),
    };
    chest_near(&*host, level, at, r)
}

/// The chest or barrel nearest `at`, within `r` blocks sideways (two up and down).
pub fn chest_near(host: &dyn mod_api::Host, level: LevelRef, at: [f64; 3], r: i32) -> Option<[i32; 3]> {
    let (bx, by, bz) = (at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32);
    let mut found = Vec::new();
    for x in bx - r..=bx + r {
        for z in bz - r..=bz + r {
            for y in by - 2..=by + 2 {
                if matches!(block_at(host, level, [x, y, z]).as_str(), "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel") {
                    found.push([x, y, z]);
                }
            }
        }
    }
    job::nearest(at, &found)
}

/// NPC `npc`'s chest, if it has a job: Some(None) if it has none (as last found, looked for
/// again now and then).
pub fn chest_of(host: &mut dyn ServerHost, npc: EntityRef) -> Option<Option<(LevelRef, [i32; 3])>> {
    let level = npc.level(&*host)?;
    let now = level.game_time(&*host)?;
    let working = STATE.lock().ok().is_some_and(|s| s.working.get(&npc.id()).is_some_and(|w| w.job.is_some()));
    if !working && job_of(host, npc).is_none() {
        return None;
    }
    let known = STATE.lock().ok().and_then(|s| s.chests.get(&npc.id()).copied()).filter(|(_, t)| now - *t < CHEST_AGAIN && now >= *t);
    let chest = match known {
        Some((c, _)) => c,
        None => {
            let c = find_chest(host, npc, level);
            if let Ok(mut s) = STATE.lock() {
                s.chests.insert(npc.id(), (c, now));
            }
            c
        }
    };
    Some(chest.map(|c| (level, c)))
}

fn stack(host: &dyn mod_api::Host, item: &str, count: u32) -> Option<ItemStack> {
    ItemStack::new(host.item_key(&mod_api::Identifier::parse(item).ok()?)?, count)
}

/// How many of `item` chest `c` holds.
pub fn amount(host: &mut dyn ServerHost, c: (LevelRef, [i32; 3]), item: &str) -> u64 {
    let (Some(cap), Some(s)) = (c.0.capability(host, &STORAGE, c.1, None), stack(&*host, item, 1)) else { return 0 };
    cap.amount(host, &ItemResource::of(&s)).unwrap_or(0)
}

/// Takes `count` of `item` out of chest `c`: all of them, or none.
pub fn take(host: &mut dyn ServerHost, c: (LevelRef, [i32; 3]), item: &str, count: u32) -> bool {
    let (Some(cap), Some(s)) = (c.0.capability(host, &STORAGE, c.1, None), stack(&*host, item, 1)) else { return false };
    let r = ItemResource::of(&s);
    host.transaction(|host, tx| if cap.extract(host, tx, &r, u64::from(count)) == u64::from(count) { Ok(()) } else { Err(()) }).is_ok()
}

/// Puts `s` in chest `c`: how many went in.
pub fn put(host: &mut dyn ServerHost, c: (LevelRef, [i32; 3]), s: &ItemStack) -> u32 {
    let Some(cap) = c.0.capability(host, &STORAGE, c.1, None) else { return 0 };
    let r = ItemResource::of(s);
    host.transaction(|host, tx| Ok::<u64, ()>(cap.insert(host, tx, &r, u64::from(s.count())))).unwrap_or(0) as u32
}

/// Puts `count` of `item` in chest `c` (a shop bought them): how many went in.
pub fn put_item(host: &mut dyn ServerHost, c: (LevelRef, [i32; 3]), item: &str, count: u32) -> u32 {
    stack(&*host, item, count).map_or(0, |s| put(host, c, &s))
}

/// What NPC `me` got goes in its chest; what doesn't fit drops by it.
fn keep(host: &mut dyn ServerHost, me: EntityRef, level: LevelRef, got: Vec<ItemStack>, now: i64) {
    let chest = chest_of(host, me).flatten();
    let mut spilled = false;
    for mut s in got {
        let n = s.count();
        let went = chest.map_or(0, |c| put(host, c, &s));
        if went < n {
            spilled = true;
            let _ = s.set_count(n - went);
            let at = chest.map(|c| center(c.1)).or_else(|| me.position(&*host)).unwrap_or([0.0; 3]);
            let _ = level.spawn_item(host, at[0], at[1] + 1.0, at[2], s, 10);
        }
    }
    if spilled {
        let say = STATE.lock().ok().and_then(|mut s| {
            let w = s.working.get_mut(&me.id())?;
            (now - w.complained >= 2400 || now < w.complained).then(|| w.complained = now)
        });
        if say.is_some() {
            let line = if chest.is_some() { "My chest is full!" } else { "I need a chest for all this." };
            crate::heart::say_aloud(host, me, line);
        }
    }
}

fn dist_xz(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// One NPC's work this step.
fn work(host: &mut dyn ServerHost, me: EntityRef, job: Job, place: &str) {
    // A builder builds its site (see `builds`).
    if job == Job::Build {
        crate::builds::work(host, me);
        return;
    }
    // A baker bakes at its place, from its chest (see `markets`).
    if job == Job::Bake {
        let at = crate::routines::place_named(host, me, place);
        crate::markets::bake(host, me, at);
        return;
    }
    let id = me.id();
    let Some(level) = me.level(&*host) else { return };
    let (Some(now), Some(pos)) = (level.game_time(&*host), me.position(&*host)) else { return };
    let work_at = crate::routines::place_named(host, me, place).unwrap_or(pos);
    let mut w = STATE.lock().ok().and_then(|mut s| s.working.remove(&id)).unwrap_or_default();
    if w.job != Some(job) {
        w = Working { job: Some(job), ..Working::default() };
    }
    // Fishing, it stands at its place and casts to the water near.
    if let Some(t) = w.target {
        let stand = if job == Job::Fish { work_at } else { center(t) };
        let reach = if job == Job::Fish { 2.5 } else { 2.8 };
        if dist_xz(pos, stand) <= reach && (pos[1] - stand[1]).abs() <= 4.0 {
            w.target = None;
            w.next = now + job.rest() + if job == Job::Fish { now % 400 + i64::from(id) % 300 } else { 0 };
            if still(&*host, level, job, t) {
                let got = act(host, me, level, job, t);
                w.done += 1;
                if w.done % 6 == 1 && !got.is_empty() {
                    let lines = job.lines();
                    crate::heart::say_aloud(host, me, lines[(now as usize / 20) % lines.len()]);
                }
                if let Ok(mut s) = STATE.lock() {
                    s.working.insert(id, w);
                }
                keep(host, me, level, got, now);
                return;
            }
        } else if now - w.since > GIVE_UP || now < w.since {
            w.queue.retain(|q| *q != t);
            w.target = None;
            w.next = now + 40;
        }
    } else if now >= w.next {
        // With nothing to do last time, it looks about less often.
        let again = if w.idle { LOOK_AGAIN * 4 } else { LOOK_AGAIN };
        if w.queue.is_empty() && (now - w.scanned >= again || now < w.scanned) {
            w.queue = scan(&*host, level, job, work_at);
            w.scanned = now;
        }
        let from = if job == Job::Fish { work_at } else { pos };
        while let Some(t) = job::nearest(from, &w.queue) {
            // Water stays: it fishes the same spot again.
            if job != Job::Fish {
                w.queue.retain(|q| *q != t);
            }
            if still(&*host, level, job, t) {
                w.target = Some(t);
                w.since = now;
                w.idle = false;
                let stand = if job == Job::Fish { work_at } else { center(t) };
                companion::set_live(host, me, stand, LiveDo::Work);
                break;
            }
            if job == Job::Fish {
                w.queue.retain(|q| *q != t);
            }
        }
        if w.target.is_none() {
            w.next = now + LOOK_AGAIN;
            // Nothing to do: it waits about its place.
            if !w.idle {
                w.idle = true;
                companion::set_live(host, me, work_at, LiveDo::Stay);
            }
        }
    }
    if let Ok(mut s) = STATE.lock() {
        s.working.insert(id, w);
    }
}

/// The server tick: every NPC at work with a job works.
pub fn tick(host: &mut dyn ServerHost) {
    let step = STATE.lock().map(|mut s| {
        s.ticks = s.ticks.wrapping_add(1);
        s.ticks % STEP == 0
    });
    if !matches!(step, Ok(true)) {
        return;
    }
    let at_work = crate::routines::at_work();
    let mut busy = Vec::new();
    for (id, entry_job, place) in at_work {
        let Some(me) = host.entity(id) else { continue };
        let Some(job) = entry_job.as_deref().and_then(Job::parse).or_else(|| job_of(host, me)) else { continue };
        busy.push(id);
        work(host, me, job, &place);
    }
    if let Ok(mut s) = STATE.lock() {
        s.working.retain(|id, _| busy.contains(id));
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix job <targets:entities> <job:word>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if lit.as_slice() != ["felix", "job"] {
        return Flow::Continue;
    }
    let name = companion::text_argument(e, "job").unwrap_or_default();
    let line = if name != "none" && Job::parse(&name).is_none() {
        "Jobs: farm, wood, fish, build, bake, none. An NPC works at its day's work, about its work place.".to_string()
    } else {
        let npcs: Vec<EntityRef> = e
            .command
            .arguments
            .iter()
            .find(|a| a.name == "targets")
            .map(|a| match &a.value {
                CommandArgumentValue::Entities(list) => list.clone(),
                CommandArgumentValue::Entity(one) => vec![*one],
                _ => Vec::new(),
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|t| companion::is_npc(&*host, *t))
            .collect();
        for n in &npcs {
            set_job(host, *n, &name);
        }
        format!("{} NPCs now have the job {name}", npcs.len())
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
