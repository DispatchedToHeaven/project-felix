//! Village heads on the server (see `chief`): each Felix settlement chooses a head among its
//! grown-ups, and each morning the head sees to the village: calls a festival when spirits are low,
//! has a cabin built (from the village's logs) when someone slept without a bed, gives work to
//! those with none, takes in a newcomer when there is room, and asks travellers for what the village
//! lacks. Players ask any villager who leads them and what has happened lately, and the head how the village is doing and what
//! it needs, and bring it. `/felix village`, `/felix village elect|morning|newcomer|build`.
use crate::chief::{self, Candidate, Decision, Report};
use crate::companion::{self, DEFAULT_MODEL, Look};
use crate::serde_json::{Value, json};
use crate::skill::Skill;
use crate::village::NpcSpec;
use mod_api::{
    Command, CommandSource, ConfigPath, EntityRef, Flow, Heightmap, Holder, PlayerRef, RegisterCommandsEvent, RunModCommandEvent, ServerHost, Text,
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

const OVERWORLD: &str = "minecraft:overworld";
const FILE: &str = "chiefs.json";
const CABIN: &str = "projectfelix:cabin";
const DAY: i64 = 24_000;

#[derive(Clone, Debug, Default)]
struct Build {
    builder: u128,
    /// Its job before (given back when the cabin stands).
    job: Option<String>,
    origin: [i32; 3],
    rot: u8,
    /// Who moves in.
    homes_for: Vec<u128>,
}

#[derive(Clone, Debug, Default)]
struct Village {
    head: Option<(u128, String)>,
    elected: i64,
    festival: i64,
    newcomer: i64,
    /// The last day its head saw to it.
    last_day: i64,
    cabins: usize,
    build: Option<Build>,
    wants: Vec<(String, u32)>,
    log: Vec<String>,
    /// Who died, to be said goodbye to the next morning.
    funerals: Vec<String>,
}

#[derive(Default)]
struct State {
    read: bool,
    world: Option<std::path::PathBuf>,
    villages: HashMap<String, Village>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

fn hex(u: u128) -> String {
    format!("{u:032x}")
}

fn unhex(s: &str) -> Option<u128> {
    u128::from_str_radix(s, 16).ok()
}

fn read(host: &mut dyn ServerHost) {
    let world = host.world_dir();
    if STATE.lock().map(|s| s.read && s.world == world).unwrap_or(true) {
        return;
    }
    let v = ConfigPath::new(FILE).ok().and_then(|p| host.world_data_files().read(&p).ok()).and_then(|b| crate::serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
    let mut villages = HashMap::new();
    if let Some(m) = v.as_object() {
        for (name, x) in m {
            let i = |k: &str| x.get(k).and_then(Value::as_i64).unwrap_or(0);
            let build = x.get("build").filter(|b| !b.is_null()).and_then(|b| {
                let o = b.get("origin")?.as_array()?;
                Some(Build {
                    builder: unhex(b.get("builder")?.as_str()?)?,
                    job: b.get("job").and_then(Value::as_str).map(str::to_string),
                    origin: [o.first()?.as_i64()? as i32, o.get(1)?.as_i64()? as i32, o.get(2)?.as_i64()? as i32],
                    rot: b.get("rot").and_then(Value::as_u64).unwrap_or(0) as u8,
                    homes_for: b.get("homes_for").and_then(Value::as_array).map(|a| a.iter().filter_map(|u| unhex(u.as_str()?)).collect()).unwrap_or_default(),
                })
            });
            villages.insert(
                name.clone(),
                Village {
                    head: x.get("head").and_then(|h| Some((unhex(h.get(0)?.as_str()?)?, h.get(1)?.as_str()?.to_string()))),
                    elected: i("elected"),
                    festival: i("festival"),
                    newcomer: i("newcomer"),
                    last_day: i("last_day"),
                    cabins: i("cabins").max(0) as usize,
                    build,
                    wants: x.get("wants").and_then(Value::as_array).map(|a| a.iter().filter_map(|w| Some((w.get(0)?.as_str()?.to_string(), w.get(1)?.as_u64()? as u32))).collect()).unwrap_or_default(),
                    log: x.get("log").and_then(Value::as_array).map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                    funerals: x.get("funerals").and_then(Value::as_array).map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                },
            );
        }
    }
    if let Ok(mut s) = STATE.lock() {
        s.read = true;
        s.world = world;
        s.villages = villages;
    }
}

fn write(host: &mut dyn ServerHost) {
    let v = STATE.lock().map(|s| {
        let mut m = crate::serde_json::Map::new();
        for (name, x) in &s.villages {
            m.insert(
                name.clone(),
                json!({
                    "head": x.head.as_ref().map(|(u, n)| json!([hex(*u), n])),
                    "elected": x.elected, "festival": x.festival, "newcomer": x.newcomer, "last_day": x.last_day, "cabins": x.cabins,
                    "build": x.build.as_ref().map(|b| json!({ "builder": hex(b.builder), "job": b.job, "origin": b.origin, "rot": b.rot,
                        "homes_for": b.homes_for.iter().map(|u| hex(*u)).collect::<Vec<_>>() })),
                    "wants": x.wants.iter().map(|(i, n)| json!([i, n])).collect::<Vec<_>>(),
                    "log": x.log,
                    "funerals": x.funerals,
                }),
            );
        }
        Value::Object(m)
    });
    if let (Ok(v), Ok(p)) = (v, ConfigPath::new(FILE))
        && let Err(e) = host.world_data_files().write(&p, &crate::serde_json::to_vec_pretty(&v).unwrap_or_default())
    {
        eprintln!("[projectfelix] village heads not saved: {e:?}");
    }
}

fn village(name: &str) -> Village {
    STATE.lock().ok().and_then(|s| s.villages.get(name).cloned()).unwrap_or_default()
}

fn set_village(name: &str, v: Village) {
    if let Ok(mut s) = STATE.lock() {
        s.villages.insert(name.to_string(), v);
    }
}

fn tag_of(name: &str) -> String {
    format!("felix_{}", name.to_lowercase())
}

/// The loaded people of village `name`.
pub fn members(host: &mut dyn ServerHost, name: &str) -> Vec<EntityRef> {
    let tag = tag_of(name);
    companion::npcs().into_iter().filter_map(|id| host.entity(id)).filter(|e| e.tags(&*host).unwrap_or_default().contains(&tag)).collect()
}

/// The village NPC `npc` belongs to, if any.
pub fn village_of(host: &mut dyn ServerHost, npc: EntityRef) -> Option<String> {
    let tags = npc.tags(&*host).unwrap_or_default();
    crate::settlements::list(host).into_iter().map(|(_, n)| n).find(|n| tags.contains(&tag_of(n)))
}

fn is_guard(host: &mut dyn ServerHost, e: EntityRef) -> bool {
    crate::routines::ROUTINE.get(host, Holder::Entity(e.id())).ok().flatten().is_some_and(|r| r.ends_with(":guard"))
}

/// One given work keeps working hours (a night owl's day has none).
fn to_work(host: &mut dyn ServerHost, e: EntityRef) {
    let r = crate::routines::ROUTINE.get(host, Holder::Entity(e.id())).ok().flatten();
    if r.is_none_or(|r| r.ends_with(":night_owl")) {
        crate::routines::set_routine(host, e, Some("projectfelix:villager"));
    }
}

fn grown(host: &mut dyn ServerHost, e: EntityRef) -> bool {
    !crate::families::is_child(host, e)
}

fn name_of(host: &dyn mod_api::Host, e: EntityRef) -> String {
    e.name(host).unwrap_or_else(|| "someone".into())
}

/// Says `line` to the players within 64 of the village middle.
pub fn tell_near(host: &mut dyn ServerHost, center: [i32; 2], line: &str) {
    let Some(level) = host.level(OVERWORLD) else { return };
    for p in level.online_players(&*host).unwrap_or_default() {
        let near = p.entity().position(&*host).is_some_and(|q| (q[0] - center[0] as f64).powi(2) + (q[2] - center[1] as f64).powi(2) < 64.0 * 64.0);
        if near {
            let _ = p.tell(host, Text::literal(line.to_string()));
        }
    }
}

fn note(v: &mut Village, day: i64, line: &str) {
    v.log.push(format!("Day {day}: {line}"));
    if v.log.len() > 8 {
        v.log.remove(0);
    }
}

/// The village's grown-ups choose a head.
fn elect(host: &mut dyn ServerHost, name: &str, people: &[EntityRef], center: [i32; 2], now: i64) {
    let mut cands = Vec::new();
    for c in people {
        if !grown(host, *c) {
            continue;
        }
        let Some(uuid) = c.uuid(host) else { continue };
        let support: f32 = people.iter().filter(|o| o.id() != c.id()).map(|o| crate::heart::relation_of(o.id(), uuid) as f32).sum();
        let talking = crate::skills::level(host, *c, Skill::Talking);
        let mood = crate::minds::mood(c.id()).unwrap_or(50.0);
        cands.push(Candidate { uuid, name: name_of(&*host, *c), support, talking, mood });
    }
    let Some(w) = chief::elect(&cands).cloned() else { return };
    let mut v = village(name);
    let again = v.head.as_ref().is_some_and(|h| h.0 == w.uuid);
    v.head = Some((w.uuid, w.name.clone()));
    v.elected = now;
    let line = if again { format!("{name} keeps {} as its head.", w.name) } else { format!("{name} has chosen {} as its head.", w.name) };
    note(&mut v, now / DAY, &line);
    set_village(name, v);
    tell_near(host, center, &line);
    if let Some(e) = people.iter().find(|e| e.uuid(host) == Some(w.uuid)).copied() {
        crate::heart::say_aloud(host, e, if again { "Thank you, all. Again." } else { "I'll do my best for all of you. I promise." });
    }
}

/// How the village stands.
pub fn report(host: &mut dyn ServerHost, name: &str, center: [i32; 2], people: &[EntityRef], now: i64) -> Report {
    read(host);
    let v = village(name);
    let head = v.head.as_ref().map(|h| h.0);
    let mut r = Report { people: people.len(), ..Report::default() };
    r.beds = 2 * (crate::settlements::houses(host, center) + v.cabins);
    let mut moods = Vec::new();
    let mut chests = Vec::new();
    for e in people {
        let job = crate::jobs::job_of(host, *e);
        if let Some(j) = job
            && !r.jobs.contains(&j.name().to_string())
            && j.name() != "build"
        {
            r.jobs.push(j.name().to_string());
        }
        let uuid = e.uuid(host);
        if job.is_none() && grown(host, *e) && uuid != head && !is_guard(host, *e)
            && let Some(u) = uuid
        {
            r.jobless.push(u);
        }
        if crate::minds::slept_rough(e.id(), now) {
            r.rough += 1;
        }
        if let Some(m) = crate::minds::mood(e.id()) {
            moods.push(m);
        }
        if let Some(c) = crate::jobs::chest_of(host, *e).flatten()
            && !chests.contains(&c)
        {
            chests.push(c);
        }
    }
    for c in &chests {
        r.food += crate::need::FOODS.iter().filter(|f| f.1 >= 3).map(|f| crate::jobs::amount(host, *c, f.0)).sum::<u64>();
        r.logs += LOGS.iter().map(|l| crate::jobs::amount(host, *c, l)).sum::<u64>();
    }
    r.mood = if moods.is_empty() { 50.0 } else { moods.iter().sum::<f32>() / moods.len() as f32 };
    r.days_since_festival = (now - v.festival) / DAY;
    r.days_since_newcomer = (now - v.newcomer) / DAY;
    r.building = v.build.is_some();
    r
}

pub const LOGS: [&str; 6] = ["minecraft:oak_log", "minecraft:spruce_log", "minecraft:birch_log", "minecraft:jungle_log", "minecraft:acacia_log", "minecraft:dark_oak_log"];

/// The village's chests (of its people with jobs).
pub fn chests(host: &mut dyn ServerHost, people: &[EntityRef]) -> Vec<(mod_api::LevelRef, [i32; 3])> {
    let mut out = Vec::new();
    for e in people {
        if let Some(c) = crate::jobs::chest_of(host, *e).flatten()
            && !out.contains(&c)
        {
            out.push(c);
        }
    }
    out
}

/// Takes `n` logs from the village's chests; false (and none taken) if it hasn't that many.
pub fn take_logs(host: &mut dyn ServerHost, people: &[EntityRef], mut n: u64) -> bool {
    let cs = chests(host, people);
    let have: u64 = cs.iter().map(|c| LOGS.iter().map(|l| crate::jobs::amount(host, *c, l)).sum::<u64>()).sum();
    if have < n {
        return false;
    }
    for c in cs {
        for l in LOGS {
            if n == 0 {
                return true;
            }
            let k = crate::jobs::amount(host, c, l).min(n);
            if k > 0 && crate::jobs::take(host, c, l, k as u32) {
                n -= k;
            }
        }
    }
    true
}

/// Where a new cabin can go about the village at `center`: dry, level ground about as high as the
/// middle, with nothing built on it; and its ground height.
fn cabin_place(host: &mut dyn ServerHost, center: [i32; 2]) -> Option<([i32; 2], i32)> {
    let level = host.level(OVERWORLD)?;
    let mid = host.surface_heights(OVERWORLD, &[(center[0], center[1])], Heightmap::MotionBlockingNoLeaves).ok()?.first().copied().flatten()?;
    'spots: for k in 0..24 {
        let s = chief::cabin_spot(k);
        let (x, z) = (center[0] + s[0], center[1] + s[1]);
        let cols: Vec<(i32, i32)> = [(0, 0), (-3, -3), (3, -3), (-3, 3), (3, 3)].iter().map(|(dx, dz)| (x + dx, z + dz)).collect();
        let Ok(hs) = host.surface_heights(OVERWORLD, &cols, Heightmap::MotionBlockingNoLeaves) else { return None };
        let mut ys = Vec::new();
        for ((cx, cz), h) in cols.iter().zip(hs) {
            let Some(h) = h else { continue 'spots };
            let ground = level.block(&*host, [*cx, h - 1, *cz]).map(|b| b.id(&*host)).unwrap_or_default();
            let wet = ground.contains("water") || ground.contains("lava") || ground.ends_with("ice") || ground.contains("kelp") || ground.contains("seagrass");
            let built = ground.ends_with("_planks") || ground.ends_with("_slab") || ground.ends_with("_log") || ground == "minecraft:dirt_path" || ground == "minecraft:farmland" || ground == "minecraft:cobblestone";
            if wet || built {
                continue 'spots;
            }
            ys.push(h);
        }
        let (lo, hi) = (ys.iter().min().copied().unwrap_or(mid), ys.iter().max().copied().unwrap_or(mid));
        if hi - lo <= 2 && (ys[0] - mid).abs() <= 4 {
            return Some((s, ys[0]));
        }
    }
    None
}

/// The head has a cabin built: a builder, a spot facing the middle, the wood from the village.
fn start_cabin(host: &mut dyn ServerHost, name: &str, center: [i32; 2], people: &[EntityRef], rough: Vec<u128>, now: i64) -> String {
    let Some(plan) = crate::builds::plan_named(CABIN) else { return String::new() };
    let mut v = village(name);
    let head = v.head.as_ref().map(|h| h.0);
    // The best builder among the grown-ups (not the head, not a guard).
    let mut best: Option<(u8, EntityRef)> = None;
    for e in people {
        if !grown(host, *e) || e.uuid(host) == head || is_guard(host, *e) {
            continue;
        }
        let l = crate::skills::level(host, *e, Skill::Building);
        if best.is_none_or(|(b, _)| l > b) {
            best = Some((l, *e));
        }
    }
    let Some((_, builder)) = best else { return String::new() };
    let [w, _, d] = plan.size();
    let Some((spot, y)) = cabin_place(host, center) else {
        let line = format!("There is no good ground left about {name} for another cabin.");
        note(&mut v, now / DAY, &line);
        set_village(name, v);
        return line;
    };
    let rot = chief::facing_turn([0, d / 2 + 1], [-spot[0], -spot[1]], crate::plan::turn);
    let (x, z) = (center[0] + spot[0], center[1] + spot[1]);
    let half = crate::plan::turn([w / 2, 0, d / 2], rot);
    let origin = [x - half[0], y - 1 - plan.sink, z - half[2]];
    let site = crate::builds::Site { plan: CABIN.into(), origin, rot, step: 0 };
    let need = crate::builds::materials(&plan, &site);
    let logs: u32 = need.iter().map(|(i, n)| chief::logs_for(i, *n)).sum();
    if !take_logs(host, people, logs as u64) {
        if !v.wants.iter().any(|w| LOGS.contains(&w.0.as_str())) {
            v.wants.push(("minecraft:oak_log".into(), logs.max(16)));
        }
        let line = format!("{name} needs {logs} logs for a new cabin.");
        note(&mut v, now / DAY, &line);
        set_village(name, v);
        return line;
    }
    let job = crate::jobs::job_of(host, builder).map(|j| j.name().to_string());
    crate::jobs::set_job(host, builder, "build");
    to_work(host, builder);
    if let Some(c) = crate::jobs::chest_of(host, builder).flatten() {
        for (item, n) in &need {
            crate::jobs::put_item(host, c, item, *n);
        }
    }
    let _ = crate::builds::BUILD.set(host, Holder::Entity(builder.id()), Some(site));
    v.build = Some(Build { builder: builder.uuid(host).unwrap_or(0), job, origin, rot, homes_for: rough });
    let line = format!("{} is building a new cabin for {name}.", name_of(&*host, builder));
    note(&mut v, now / DAY, &line);
    set_village(name, v);
    crate::heart::say_aloud(host, builder, "A new cabin? Leave it to me.");
    line
}

/// A cabin that stands: its beds and chest, its people moved in, its builder back at its work.
fn finish_cabin(host: &mut dyn ServerHost, name: &str, people: &[EntityRef], b: &Build, now: i64) -> String {
    for (at, state) in crate::settle::furniture() {
        let t = crate::plan::turn(at, b.rot);
        let p = [b.origin[0] + t[0], b.origin[1] + t[1], b.origin[2] + t[2]];
        crate::talk::queue(format!("execute in {OVERWORLD} run setblock {} {} {} {}", p[0], p[1], p[2], crate::plan::turn_state(state, b.rot)));
    }
    let inside = crate::plan::turn(crate::settle::INSIDE, b.rot);
    let home = [(b.origin[0] + inside[0]) as f64 + 0.5, (b.origin[1] + inside[1]) as f64, (b.origin[2] + inside[2]) as f64 + 0.5];
    let mut moved = Vec::new();
    for e in people {
        if e.uuid(host).is_some_and(|u| b.homes_for.contains(&u)) && moved.len() < 2 {
            crate::routines::set_place(host, *e, "home", home);
            moved.push(name_of(&*host, *e));
        }
    }
    if let Some(builder) = people.iter().find(|e| e.uuid(host) == Some(b.builder)).copied() {
        crate::jobs::set_job(host, builder, b.job.as_deref().unwrap_or(""));
    }
    let mut v = village(name);
    v.cabins += 1;
    v.build = None;
    v.wants.retain(|w| !LOGS.contains(&w.0.as_str()));
    let line = if moved.is_empty() { format!("{name} has a new cabin.") } else { format!("{name} has a new cabin; {} moved in.", moved.join(" and ")) };
    note(&mut v, now / DAY, &line);
    set_village(name, v);
    line
}

/// A newcomer comes to live in the village, to do `job`.
fn newcomer(host: &mut dyn ServerHost, name: &str, center: [i32; 2], people: &[EntityRef], job: &str, now: i64) -> String {
    newcomer_as(host, name, center, people, job, now, "a traveller").0
}

/// A newcomer (`what`: `a traveller`, `a wanderer`...) comes to live in the village, to do
/// `job`: what is said, and the entity.
pub fn newcomer_as(host: &mut dyn ServerHost, name: &str, center: [i32; 2], people: &[EntityRef], job: &str, now: i64, what: &str) -> (String, Option<u32>) {
    read(host);
    let taken: Vec<String> = people.iter().map(|e| name_of(&*host, *e)).collect();
    let who = crate::settle::newcomer_name(now as u64 ^ center[0] as u64, &taken);
    // Its places: a worker's of that job, anyone's meeting place, the newest cabin (else the middle).
    let mut work = None;
    let mut meet = None;
    for e in people {
        if work.is_none() && crate::jobs::job_of(host, *e).is_some_and(|j| j.name() == job) {
            work = crate::routines::place_named(host, *e, "work");
        }
        if meet.is_none() {
            meet = crate::routines::place_named(host, *e, "meet");
        }
    }
    let v = village(name);
    let Ok(h) = host.surface_heights(OVERWORLD, &[(center[0] + 2, center[1] + 2)], Heightmap::MotionBlockingNoLeaves) else { return (String::new(), None) };
    let Some(Some(y)) = h.first().copied() else { return (String::new(), None) };
    let anchor = [center[0] as f64 + 2.5, y as f64, center[1] as f64 + 2.5];
    let rel = |p: [f64; 3]| [p[0] - anchor[0], p[1] - anchor[1], p[2] - anchor[2]];
    // No bed of its own yet: the head has a cabin built when it sleeps without one.
    let _ = v;
    let mut places = vec![("home".to_string(), rel(anchor))];
    if let Some(w) = work {
        places.push(("work".into(), rel(w)));
    }
    if let Some(m) = meet {
        places.push(("meet".into(), rel(m)));
    }
    let look = Look::of_model(DEFAULT_MODEL);
    let characters = ["projectfelix:kind", "projectfelix:stoic", "projectfelix:touchy"];
    let shop = match job {
        "farm" => Some("projectfelix:farmer"),
        "wood" => Some("projectfelix:woodcutter"),
        "bake" => Some("projectfelix:baker"),
        _ => None,
    };
    let spec = NpcSpec {
        name: who.clone(),
        at: [0.0; 3],
        model: look.model,
        texture: look.texture,
        routine: Some("projectfelix:villager".into()),
        shop: shop.map(str::to_string),
        character: Some(characters[(now as usize / 7) % 3].into()),
        places,
        tags: vec!["felix_settler".into(), tag_of(name)],
        orders: "live".into(),
        job: Some(job.to_string()),
        ..NpcSpec::default()
    };
    let id = crate::villages::settle(host, OVERWORLD, anchor, spec);
    let mut v = village(name);
    v.newcomer = now;
    let line = format!("{who}, {what}, has come to live in {name}.");
    note(&mut v, now / DAY, &line);
    set_village(name, v);
    (line, id)
}

/// Puts `line` in village `name`'s news (what its head tells of the last days), and saves.
pub fn record(host: &mut dyn ServerHost, name: &str, now: i64, line: &str) {
    read(host);
    let mut v = village(name);
    note(&mut v, now / DAY, line);
    set_village(name, v);
    write(host);
}

/// Village `name`'s head, if it is about.
pub fn head_of(host: &mut dyn ServerHost, name: &str, people: &[EntityRef]) -> Option<EntityRef> {
    read(host);
    let (u, _) = village(name).head?;
    people.iter().find(|e| e.uuid(host) == Some(u)).copied()
}

/// NPC `victim` died: its village says goodbye the next morning.
pub fn on_death(host: &mut dyn ServerHost, victim: EntityRef) {
    read(host);
    let Some(name) = village_of(host, victim) else { return };
    let who = name_of(&*host, victim);
    let mut v = village(&name);
    v.funerals.push(who);
    set_village(&name, v);
    write(host);
}

/// NPCs `a` and `b` married: their village celebrates.
pub fn wedding(host: &mut dyn ServerHost, a: EntityRef, b: EntityRef) {
    read(host);
    let Some(name) = village_of(host, a) else { return };
    let (na, nb) = (name_of(&*host, a), name_of(&*host, b));
    let center = crate::settlements::list(host).into_iter().find(|(_, n)| *n == name).map(|(c, _)| c).unwrap_or([0, 0]);
    let now = a.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
    for e in members(host, &name) {
        if e.id() != a.id() && e.id() != b.id() {
            crate::minds::remember(host, e, "wedding");
        }
    }
    let line = format!("{na} and {nb} are married! All of {name} celebrates.");
    let mut v = village(&name);
    note(&mut v, now / DAY, &line);
    set_village(&name, v);
    write(host);
    tell_near(host, center, &line);
}

/// The head sees to the village this morning.
fn morning(host: &mut dyn ServerHost, name: &str, center: [i32; 2], people: &[EntityRef], now: i64) {
    // First, goodbyes for those who died: the head says a few words, everyone remembers.
    let gone = std::mem::take(&mut village(name).funerals);
    if !gone.is_empty() {
        let mut v = village(name);
        v.funerals.clear();
        let line = format!("{name} says goodbye to {}.", gone.join(" and "));
        note(&mut v, now / DAY, &line);
        set_village(name, v);
        for e in people {
            crate::minds::remember(host, *e, "funeral");
        }
        if let Some(h) = village(name).head.and_then(|(u, _)| people.iter().find(|e| e.uuid(host) == Some(u)).copied()) {
            crate::heart::say_aloud(host, h, &format!("We'll miss you, {}. Rest well.", gone.join(" and ")));
        }
        tell_near(host, center, &line);
    }
    let r = report(host, name, center, people, now);
    let head = village(name).head.and_then(|(u, _)| people.iter().find(|e| e.uuid(host) == Some(u)).copied());
    let head_name = head.map(|h| name_of(&*host, h)).unwrap_or_else(|| "The village".into());
    let rough: Vec<u128> = people.iter().filter(|e| crate::minds::slept_rough(e.id(), now)).filter_map(|e| e.uuid(host)).collect();
    let mut lines = Vec::new();
    let mut wants: Vec<(String, u32)> = Vec::new();
    for d in chief::decide(&r) {
        match d {
            Decision::Festival => {
                let id = if (now / DAY) % 2 == 0 { "projectfelix:harvest_fair" } else { "projectfelix:market_day" };
                if crate::events::start(host, id) {
                    let mut v = village(name);
                    v.festival = now;
                    let line = format!("{head_name} calls a festival in {name}, to lift everyone's spirits!");
                    note(&mut v, now / DAY, &line);
                    set_village(name, v);
                    lines.push(line);
                    if let Some(h) = head {
                        crate::heart::say_aloud(host, h, "Everyone! Let's have a festival. We could all use one.");
                    }
                }
            }
            Decision::BuildCabin => {
                let l = start_cabin(host, name, center, people, rough.clone(), now);
                if !l.is_empty() {
                    lines.push(l);
                }
            }
            Decision::GiveJob(u, job) => {
                let Some(e) = people.iter().find(|e| e.uuid(host) == Some(u)).copied() else { continue };
                let worker = people.iter().find(|o| crate::jobs::job_of(host, **o).is_some_and(|j| j.name() == job)).copied();
                if let Some(w) = worker.and_then(|w| crate::routines::place_named(host, w, "work")) {
                    crate::routines::set_place(host, e, "work", w);
                }
                crate::jobs::set_job(host, e, &job);
                to_work(host, e);
                let what = match job.as_str() {
                    "farm" => "the fields",
                    "wood" => "the woodcutting",
                    "fish" => "the fishing",
                    "bake" => "the baking",
                    _ => "the work",
                };
                let line = format!("{head_name} asks {} to help with {what}.", name_of(&*host, e));
                let mut v = village(name);
                note(&mut v, now / DAY, &line);
                set_village(name, v);
                crate::heart::say_aloud(host, e, "All right. I'll do my part.");
                lines.push(line);
            }
            Decision::Newcomer(job) => {
                let l = newcomer(host, name, center, people, &job, now);
                if !l.is_empty() {
                    lines.push(l);
                }
            }
            Decision::Ask(item, n) => wants.push((item, n)),
        }
    }
    let mut v = village(name);
    // What it asks for now: this morning's, and logs for a cabin still waiting.
    let logs: Vec<(String, u32)> = v.wants.iter().filter(|w| LOGS.contains(&w.0.as_str())).cloned().collect();
    for (item, n) in logs {
        if !wants.iter().any(|w| w.0 == item) {
            wants.push((item, n));
        }
    }
    if !wants.is_empty() && v.wants.is_empty() {
        let list: Vec<String> = wants.iter().map(|(i, n)| format!("{n} {}", short(i))).collect();
        lines.push(format!("{head_name} of {name} asks travellers for {}.", list.join(" and ")));
    }
    v.wants = wants;
    set_village(name, v);
    for l in lines {
        tell_near(host, center, &l);
    }
}

fn short(item: &str) -> String {
    item.trim_start_matches("minecraft:").replace('_', " ")
}

/// The overworld's tick: elections, cabins finished, the head's morning.
pub fn tick(host: &mut dyn ServerHost) {
    let Some(level) = host.level(OVERWORLD) else { return };
    let Some(now) = level.game_time(&*host) else { return };
    if !crate::pace::since("chiefs", now, 600) {
        return;
    }
    read(host);
    let time = level.day_time(&*host).unwrap_or(0).rem_euclid(DAY);
    let mut changed = false;
    for (center, name) in crate::settlements::list(host) {
        let people = members(host, &name);
        if people.is_empty() {
            continue;
        }
        let v = village(&name);
        let head_here = v.head.as_ref().is_some_and(|(u, _)| people.iter().any(|e| e.uuid(host) == Some(*u)));
        if !head_here || now - v.elected >= chief::TERM * DAY || now < v.elected {
            // A head away (unloaded) for a while is not gone; one never chosen, or a term over, is.
            if v.head.is_none() || now - v.elected >= chief::TERM * DAY || now < v.elected || (!head_here && now - v.elected > DAY) {
                elect(host, &name, &people, center, now);
                changed = true;
            }
        }
        if let Some(b) = village(&name).build
            && let Some(builder) = people.iter().find(|e| e.uuid(host) == Some(b.builder)).copied()
            && crate::builds::BUILD.get(host, Holder::Entity(builder.id())).ok().flatten().is_none()
        {
            let line = finish_cabin(host, &name, &people, &b, now);
            tell_near(host, center, &line);
            changed = true;
        }
        let day = now / DAY;
        if day > village(&name).last_day && (1_000..6_000).contains(&time) {
            let mut v = village(&name);
            v.last_day = day;
            set_village(&name, v);
            morning(host, &name, center, &people, now);
            changed = true;
        }
    }
    if changed {
        write(host);
    }
}

/// What players and villagers say of the village (the `village` dialog action): who leads it
/// (`head`), how it is doing (`report`), what it needs (`needs`), and taking what the player
/// brings for it (`give`).
pub fn talk(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef, what: &str) {
    read(host);
    let Some(name) = village_of(host, npc) else {
        crate::heart::say_to(host, npc, player, "I don't belong to any village. I go where I please.");
        return;
    };
    let v = village(&name);
    let me = npc.uuid(host);
    let is_head = v.head.as_ref().is_some_and(|h| Some(h.0) == me);
    let head_name = v.head.as_ref().map(|h| h.1.clone()).unwrap_or_else(|| "nobody yet".into());
    if what == "head" {
        let line = if is_head { format!("That would be me. I look after {name}.") } else { format!("{head_name} is our head. Ask them about {name}.") };
        crate::heart::say_to(host, npc, player, &line);
        return;
    }
    // The news anyone of the village tells: the last few things that happened.
    if what == "news" {
        let line = match v.log.len() {
            0 => format!("Nothing much happens in {name}. Which suits us fine."),
            n => v.log[n.saturating_sub(3)..].join(" "),
        };
        crate::heart::say_to(host, npc, player, &line);
        return;
    }
    if !is_head {
        crate::heart::say_to(host, npc, player, &format!("You'd better ask {head_name}, our head."));
        return;
    }
    let center = crate::settlements::list(host).into_iter().find(|(_, n)| *n == name).map(|(c, _)| c).unwrap_or([0, 0]);
    let people = members(host, &name);
    let now = npc.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
    match what {
        "report" => {
            let r = report(host, &name, center, &people, now);
            crate::heart::say_to(
                host,
                npc,
                player,
                &format!("{name}: {} of us, beds for {}, food for a while ({}), {} logs put by. Folk are {}.", r.people, r.beds, r.food, r.logs, crate::mind::mood_word(r.mood)),
            );
            if let Some(b) = &v.build
                && let Some(e) = people.iter().find(|e| e.uuid(host) == Some(b.builder)).copied()
            {
                let line = format!("{} is building us a new cabin.", name_of(&*host, e));
                crate::heart::say_to(host, npc, player, &line);
            }
            // What the village thinks of the player.
            let liked: Vec<i32> = people.iter().map(|e| crate::heart::relation_of(e.id(), player.uuid())).collect();
            let avg = if liked.is_empty() { 0 } else { liked.iter().sum::<i32>() / liked.len() as i32 };
            let word = match avg {
                i32::MIN..=-30 => "Folk here don't trust you, I'm afraid.",
                -29..=4 => "Folk here don't know you well yet.",
                5..=29 => "Folk here think well of you.",
                _ => "Folk here think the world of you.",
            };
            crate::heart::say_to(host, npc, player, word);
        }
        "needs" => {
            let line = if v.wants.is_empty() {
                "We have what we need, thank you. Come by any time.".to_string()
            } else {
                let list: Vec<String> = v.wants.iter().map(|(i, n)| format!("{n} {}", short(i))).collect();
                format!("We could use {}. Bring them to me and the village will pay you.", list.join(" and "))
            };
            crate::heart::say_to(host, npc, player, &line);
        }
        "give" => give(host, npc, player, &name, &people),
        _ => {}
    }
}

/// The player hands the head what the village asked for: it goes in the village's chests; the
/// head pays an emerald for every eight, and the village thinks better of them.
fn give(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef, name: &str, people: &[EntityRef]) {
    let mut v = village(name);
    let pname = player.name(&*host).unwrap_or_default();
    let mut have: HashMap<String, u32> = HashMap::new();
    for s in player.inventory(&*host).unwrap_or_default() {
        *have.entry(s.item().id(&*host)).or_insert(0) += s.count();
    }
    let mut given = 0u32;
    let mut what = Vec::new();
    let cs = chests(host, people);
    for w in &mut v.wants {
        // Logs of any wood do for logs.
        let items: Vec<String> = if LOGS.contains(&w.0.as_str()) { LOGS.iter().map(|s| s.to_string()).collect() } else { vec![w.0.clone()] };
        for item in items {
            let n = have.get(&item).copied().unwrap_or(0).min(w.1);
            if n == 0 {
                continue;
            }
            let mut left = n;
            for c in &cs {
                if left == 0 {
                    break;
                }
                left -= crate::jobs::put_item(host, *c, &item, left).min(left);
            }
            let put = n - left;
            if put > 0 {
                crate::talk::queue(format!("clear {pname} {item} {put}"));
                w.1 -= put;
                given += put;
                what.push(format!("{put} {}", short(&item)));
            }
        }
    }
    v.wants.retain(|w| w.1 > 0);
    if given == 0 {
        crate::heart::say_to(host, npc, player, "You don't have anything we need right now. But thank you for thinking of us.");
        return;
    }
    let pay = (given / 8).min(crate::markets::purse_of(host, npc));
    if pay > 0 {
        crate::markets::spend(host, npc, pay);
        crate::talk::queue(format!("give {pname} minecraft:emerald {pay}"));
    }
    for e in people {
        crate::heart::befriend(host, *e, player, 2);
    }
    let now = npc.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
    note(&mut v, now / DAY, &format!("{pname} brought {}.", what.join(" and ")));
    set_village(name, v);
    write(host);
    let thanks = if pay > 0 { format!("Thank you, {pname}! Here's {pay} emerald{} for it. {name} won't forget this.", if pay == 1 { "" } else { "s" }) } else { format!("Thank you, {pname}! {name} won't forget this.") };
    crate::heart::say_to(host, npc, player, &thanks);
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix village", 2));
    e.commands.push(Command::op("felix village elect", 2));
    e.commands.push(Command::op("felix village morning", 2));
    e.commands.push(Command::op("felix village newcomer", 2));
    e.commands.push(Command::op("felix village build", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let what = match lit.as_slice() {
        ["felix", "village"] => "show",
        ["felix", "village", "elect"] => "elect",
        ["felix", "village", "morning"] => "morning",
        ["felix", "village", "newcomer"] => "newcomer",
        ["felix", "village", "build"] => "build",
        _ => return Flow::Continue,
    };
    read(host);
    let at = match e.source {
        CommandSource::Player { player, .. } => player.entity().position(&*host),
        _ => None,
    };
    let Some(at) = at else { return Flow::Cancel };
    let nearest = crate::settlements::list(host)
        .into_iter()
        .min_by_key(|(c, _)| ((c[0] as f64 - at[0]).powi(2) + (c[1] as f64 - at[2]).powi(2)) as i64);
    let Some((center, name)) = nearest else {
        let _ = host.commands().feedback(Text::literal("No Felix settlement yet."));
        return Flow::Cancel;
    };
    let people = members(host, &name);
    let now = host.level(OVERWORLD).and_then(|l| l.game_time(&*host)).unwrap_or(0);
    let mut lines = Vec::new();
    match what {
        "elect" => elect(host, &name, &people, center, now),
        "morning" => morning(host, &name, center, &people, now),
        "newcomer" | "build" => {
            // A cabin under way is given up for the new one.
            if what == "build"
                && let Some(b) = village(&name).build
            {
                if let Some(e) = people.iter().find(|e| e.uuid(host) == Some(b.builder)).copied() {
                    let _ = crate::builds::BUILD.set(host, Holder::Entity(e.id()), None);
                    crate::jobs::set_job(host, e, b.job.as_deref().unwrap_or(""));
                }
                let mut v = village(&name);
                v.build = None;
                set_village(&name, v);
            }
            let r = report(host, &name, center, &people, now);
            let line = if what == "newcomer" { newcomer(host, &name, center, &people, &chief::needed_job(&r), now) } else { start_cabin(host, &name, center, &people, Vec::new(), now) };
            tell_near(host, center, &line);
        }
        _ => {}
    }
    let v = village(&name);
    let r = report(host, &name, center, &people, now);
    lines.push(format!(
        "{name} (at {} {}): head {}, {} people loaded, beds {}, food {}, logs {}, mood {:.0}, jobless {}, slept rough {}{}",
        center[0],
        center[1],
        v.head.as_ref().map(|h| h.1.as_str()).unwrap_or("none"),
        r.people,
        r.beds,
        r.food,
        r.logs,
        r.mood,
        r.jobless.len(),
        r.rough,
        if r.building { ", building a cabin" } else { "" }
    ));
    if !v.wants.is_empty() {
        lines.push(format!("Wants: {}", v.wants.iter().map(|(i, n)| format!("{n} {}", short(i))).collect::<Vec<_>>().join(", ")));
    }
    for l in &v.log {
        lines.push(format!("  {l}"));
    }
    write(host);
    for l in lines {
        let _ = host.commands().feedback(Text::literal(l));
    }
    Flow::Cancel
}
