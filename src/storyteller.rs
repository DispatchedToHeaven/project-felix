//! The storyteller on the server (see `story`): once a day, after its head's morning, each Felix
//! village may have something come to it. A wanderer settles (sometimes hurt); a caravan stops
//! for a day, sells to players and swaps with the village what it lacks; stray animals wander in;
//! raiders come at the village: its guards and those who can fight take up arms, the rest run
//! home, a friendly village sends help, friendly players hear of it; raiders who lose half their
//! number run, raiders not driven off in time plunder the stores (see `defence`), and those who
//! fight them are thanked; a blight sets the crops back; a sickness goes round; rot gets
//! into the stores; it falls out with another village, or a friendly one sends it food in a hungry
//! time (see `factions`). Everyone remembers it, the head tells of it ("What's been happening?").
//! `/felix story`, `/felix story <incident>` (at the nearest village).
use crate::companion;
use crate::serde_json::{Value, json};
use crate::story::{self, Incident, Teller, World};
use crate::village::NpcSpec;
use mod_api::{Command, ConfigPath, EntityRef, Flow, Heightmap, LivingRef, MobRef, RegisterCommandsEvent, RunModCommandEvent, ServerHost, Text};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

const OVERWORLD: &str = "minecraft:overworld";
const FILE: &str = "story.json";
const DAY: i64 = story::DAY;
/// How far about a village raiders are looked for.
const RAID_AREA: f64 = 72.0;

#[derive(Clone, Debug, Default)]
struct Tale {
    teller: Teller,
    /// The last day it was told.
    told: i64,
    /// Raiders about until then, how many came, and the village that sent help.
    raid: Option<i64>,
    raiders: u32,
    helpers: Option<String>,
    /// A caravan here until then.
    caravan: Option<i64>,
    /// The last day someone died.
    death: Option<i64>,
}

#[derive(Default)]
struct State {
    read: bool,
    world: Option<std::path::PathBuf>,
    tales: HashMap<String, Tale>,
    seed: u64,
    /// Who of each village stands against its raiders now (not kept: a raid met again after a
    /// restart is fought by whoever is hit).
    militia: HashMap<String, Vec<u32>>,
    /// The villages whose helpers have their blades.
    armed: Vec<String>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

fn read(host: &mut dyn ServerHost) {
    let world = host.world_dir();
    if STATE.lock().map(|s| s.read && s.world == world).unwrap_or(true) {
        return;
    }
    let v = ConfigPath::new(FILE).ok().and_then(|p| host.world_data_files().read(&p).ok()).and_then(|b| crate::serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
    let mut tales = HashMap::new();
    if let Some(m) = v.as_object() {
        for (name, x) in m {
            let i = |k: &str| x.get(k).and_then(Value::as_i64);
            let last = x.get("last").and_then(Value::as_object).map(|l| l.iter().filter_map(|(k, d)| Some((Incident::parse(k)?, d.as_i64()?))).collect()).unwrap_or_default();
            tales.insert(
                name.clone(),
                Tale {
                    teller: Teller { since: i("since").unwrap_or(0), phase: i("phase").unwrap_or(0), last },
                    told: i("told").unwrap_or(0),
                    raid: i("raid"),
                    raiders: i("raiders").unwrap_or(0).max(0) as u32,
                    helpers: x.get("helpers").and_then(Value::as_str).map(str::to_string),
                    caravan: i("caravan"),
                    death: i("death"),
                },
            );
        }
    }
    if let Ok(mut s) = STATE.lock() {
        s.read = true;
        s.world = world;
        s.tales = tales;
    }
}

fn write(host: &mut dyn ServerHost) {
    let v = STATE.lock().map(|s| {
        let mut m = crate::serde_json::Map::new();
        for (name, t) in &s.tales {
            let last: crate::serde_json::Map<String, Value> = t.teller.last.iter().map(|(i, d)| (i.id().to_string(), json!(d))).collect();
            m.insert(
                name.clone(),
                json!({ "since": t.teller.since, "phase": t.teller.phase, "last": last, "told": t.told, "raid": t.raid, "raiders": t.raiders, "helpers": t.helpers, "caravan": t.caravan, "death": t.death }),
            );
        }
        Value::Object(m)
    });
    if let (Ok(v), Ok(p)) = (v, ConfigPath::new(FILE))
        && let Err(e) = host.world_data_files().write(&p, &crate::serde_json::to_vec_pretty(&v).unwrap_or_default())
    {
        eprintln!("[projectfelix] the storyteller not saved: {e:?}");
    }
}

fn tale(name: &str, day: i64) -> Tale {
    STATE.lock().ok().and_then(|s| s.tales.get(name).cloned()).unwrap_or_else(|| Tale { teller: Teller::new(day, crate::settle::hash(0, name.len() as i32, day as i32, name.bytes().map(u64::from).sum())), told: day, ..Tale::default() })
}

fn set_tale(name: &str, t: Tale) {
    if let Ok(mut s) = STATE.lock() {
        s.tales.insert(name.to_string(), t);
    }
}

fn roll() -> u64 {
    let Ok(mut s) = STATE.lock() else { return 0 };
    s.seed = s.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut x = s.seed;
    x ^= x >> 31;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 29;
    x
}

fn tag_of(name: &str, what: &str) -> String {
    format!("felix_{what}_{}", name.to_lowercase())
}

fn name_of(host: &dyn mod_api::Host, e: EntityRef) -> String {
    e.name(host).unwrap_or_else(|| "someone".into())
}

fn ground(host: &mut dyn ServerHost, x: i32, z: i32) -> Option<i32> {
    host.surface_heights(OVERWORLD, &[(x, z)], Heightmap::MotionBlockingNoLeaves).ok()?.first().copied().flatten()
}

/// Dry ground about `dist` from the village middle (the direction from `r`).
fn spot_out(host: &mut dyn ServerHost, center: [i32; 2], dist: f64, r: u64) -> Option<[i32; 3]> {
    let level = host.level(OVERWORLD)?;
    for k in 0..12u64 {
        let a = ((r + k * 5) % 12) as f64 * std::f64::consts::TAU / 12.0;
        let (x, z) = (center[0] + (a.cos() * dist) as i32, center[1] + (a.sin() * dist) as i32);
        let Some(y) = ground(host, x, z) else { continue };
        let below = level.block(&*host, [x, y - 1, z]).map(|b| b.id(&*host)).unwrap_or_default();
        if !(below.contains("water") || below.contains("lava") || below.contains("ice") || below.contains("kelp") || below.contains("seagrass")) {
            return Some([x, y, z]);
        }
    }
    None
}

/// The entities about village `center` with tag `tag`.
fn tagged(host: &mut dyn ServerHost, center: [i32; 2], tag: &str) -> Vec<EntityRef> {
    let Some(level) = host.level(OVERWORLD) else { return Vec::new() };
    let (x, z) = (center[0] as f64, center[1] as f64);
    let around = level.entities_in(&*host, [x - RAID_AREA, -64.0, z - RAID_AREA], [x + RAID_AREA, 320.0, z + RAID_AREA]).unwrap_or_default();
    around.into_iter().filter(|e| e.tags(&*host).unwrap_or_default().iter().any(|t| t == tag)).collect()
}

fn remember_all(host: &mut dyn ServerHost, people: &[EntityRef], thought: &str) {
    for e in people {
        crate::minds::remember(host, *e, thought);
    }
}

fn run(host: &mut dyn ServerHost, line: &str) -> bool {
    let ok = host.commands().run_command(line);
    if !ok {
        eprintln!("[projectfelix] storyteller command failed: {line}");
    }
    ok
}

/// How the village stands today.
fn world(host: &mut dyn ServerHost, name: &str, center: [i32; 2], people: &[EntityRef], now: i64) -> World {
    let r = crate::chiefs::report(host, name, center, people, now);
    let farmers = people.iter().filter(|e| crate::jobs::job_of(host, **e).is_some_and(|j| j.name() == "farm")).count();
    let since_death = tale(name, now / DAY).death.map_or(99, |d| now / DAY - d);
    let neighbours = crate::factions::neighbours(host, name);
    let aid = neighbours > 0 && crate::factions::aid_for(host, name, r.people, r.food).is_some();
    World { day: now / DAY, people: r.people, food: r.food, logs: r.logs, mood: r.mood, farmers, since_death, neighbours, aid }
}

/// `i` comes to the village: what is said of it (none: it could not happen now).
fn happen(host: &mut dyn ServerHost, name: &str, center: [i32; 2], people: &[EntityRef], i: Incident, now: i64) -> Option<String> {
    let w = world(host, name, center, people, now);
    let r = roll();
    let head = crate::chiefs::head_of(host, name, people);
    let head_name = head.map(|h| name_of(&*host, h)).unwrap_or_else(|| format!("{name}'s folk"));
    let line = match i {
        Incident::Wanderer => {
            if people.len() >= story::FULL {
                return None;
            }
            let rep = crate::chiefs::report(host, name, center, people, now);
            let job = crate::chief::needed_job(&rep);
            let (mut line, id) = crate::chiefs::newcomer_as(host, name, center, people, &job, now, "a wanderer");
            if line.is_empty() {
                return None;
            }
            // Some come hurt from the road.
            if r % 10 < 3
                && let Some(id) = id
            {
                crate::healers::on_damage(id as i32, 4.0 + (r >> 8) as f32 % 4.0, true);
                line.push_str(" They came hurt from the road.");
            }
            remember_all(host, people, "new_face");
            line
        }
        Incident::Caravan => {
            let (shop, what) = story::caravan(r);
            let Some(at) = spot_out(host, center, 6.0, r) else { return None };
            let taken: Vec<String> = people.iter().map(|e| name_of(&*host, *e)).collect();
            let who = crate::settle::newcomer_name(r >> 16, &taken);
            let anchor = [at[0] as f64 + 0.5, at[1] as f64, at[2] as f64 + 0.5];
            let look = companion::Look::of_model(companion::DEFAULT_MODEL);
            let spec = NpcSpec {
                name: who.clone(),
                model: look.model,
                texture: look.texture,
                shop: Some(shop.to_string()),
                character: Some("projectfelix:kind".into()),
                places: vec![("home".into(), [0.0; 3])],
                tags: vec!["felix_caravan".into(), tag_of(name, "caravan")],
                orders: "wait".into(),
                ..NpcSpec::default()
            };
            crate::villages::settle(host, OVERWORLD, anchor, spec)?;
            let mut t = tale(name, now / DAY);
            t.caravan = Some(now + story::CARAVAN_STAYS);
            set_tale(name, t);
            // From a village it gets on with, if there is one (see `factions`).
            let from = crate::factions::caravan_from(host, name, r >> 24, now).map(|f| format!(" from {f}")).unwrap_or_default();
            let mut line = format!("{}{from} has come to {name}: {who} trades until tomorrow.", capital(what));
            // The village swaps with it what it lacks.
            let chests = crate::chiefs::chests(host, people);
            let wheat: u64 = chests.iter().map(|c| crate::jobs::amount(host, *c, "minecraft:wheat")).sum();
            if let Some((give, n, get, m)) = story::swap(w.people, w.food, w.logs, wheat)
                && let Some(first) = chests.first().copied()
            {
                let paid = if give == "logs" {
                    crate::chiefs::take_logs(host, people, n as u64)
                } else {
                    let mut left = n as u64;
                    for c in &chests {
                        let k = crate::jobs::amount(host, *c, give).min(left);
                        if k > 0 && crate::jobs::take(host, *c, give, k as u32) {
                            left -= k;
                        }
                    }
                    left == 0
                };
                if paid {
                    crate::jobs::put_item(host, first, get, m);
                    line.push_str(&format!(" {head_name} swaps {n} {} for {m} {}.", short(give), short(get)));
                }
            }
            remember_all(host, people, "caravan");
            line
        }
        Incident::Strays => {
            let (kind, n) = story::strays(r);
            let Some(at) = spot_out(host, center, 10.0, r) else { return None };
            for k in 0..n {
                run(host, &format!("summon {kind} {} {} {}", at[0] as f64 + 0.5 + k as f64 * 0.7, at[1], at[2] as f64 + 0.5));
            }
            let many = if kind.ends_with("sheep") { short(kind) } else { format!("{}s", short(kind)) };
            format!("{n} stray {many} have wandered into {name}.")
        }
        Incident::Raid => {
            let (n, ranged) = story::raiders(&w, w.day - tale(name, w.day).teller.since);
            // From out of sight if there is dry land there, else nearer.
            let Some(at) = [32.0, 24.0, 16.0].into_iter().find_map(|d| spot_out(host, center, d, r)) else { return None };
            let tags = format!("Tags:[\"felix_raider\",\"{}\"],PersistenceRequired:1b,CanJoinRaid:0b", tag_of(name, "raider"));
            let mut came = 0;
            for k in 0..n {
                let kind = if k < ranged { "minecraft:pillager" } else { "minecraft:vindicator" };
                if run(host, &format!("summon {kind} {} {} {} {{{tags}}}", at[0] as f64 + 0.5 + (k % 3) as f64, at[1], at[2] as f64 + 0.5 + (k / 3) as f64)) {
                    came += 1;
                }
            }
            if came == 0 {
                return None;
            }
            let mut t = tale(name, w.day);
            t.raid = Some(now + story::RAID_LASTS);
            t.raiders = came;
            // The guards and those who can fight stand by the middle; everyone else runs home.
            let fighters = crate::defences::muster(host, people);
            let post = [f64::from(center[0]) + 0.5, f64::from(ground(host, center[0], center[1]).unwrap_or(at[1])), f64::from(center[1]) + 0.5];
            for e in people {
                if fighters.contains(&e.id()) {
                    crate::routines::wake(host, *e, None);
                    companion::go_to(e.id(), None);
                    companion::set_live(host, *e, post, companion::LiveDo::Guard);
                } else if let Some(home) = crate::routines::place_named(host, *e, "home") {
                    companion::go_to(e.id(), Some(home));
                }
            }
            remember_all(host, people, "raided");
            if let Some(h) = head {
                crate::heart::say_aloud(host, h, if fighters.is_empty() { "Raiders! Everyone, get inside!" } else { "Raiders! To arms! Everyone else, get inside!" });
            }
            let mut line = format!("{came} raiders are coming at {name}! ");
            line.push_str(&match fighters.len() {
                0 => "Its folk run for their homes.".to_string(),
                1 => "One of its folk takes up arms, the rest run for their homes.".to_string(),
                n => format!("{n} of its folk take up arms, the rest run for their homes."),
            });
            // A friendly village sends a few of its own (see `factions`).
            if let Some((from, n)) = crate::factions::helpers_for(host, name)
                && let Some(spot) = spot_out(host, center, 10.0, r >> 5)
            {
                let taken: Vec<String> = people.iter().map(|e| name_of(&*host, *e)).collect();
                let look = companion::Look::of_model(companion::DEFAULT_MODEL);
                let mut sent = 0;
                for k in 0..n {
                    let spec = NpcSpec {
                        name: crate::settle::newcomer_name((r >> 16).wrapping_add(u64::from(k) * 31), &taken),
                        model: look.model.clone(),
                        texture: look.texture.clone(),
                        character: Some("projectfelix:stoic".into()),
                        places: vec![("home".into(), [0.0; 3])],
                        tags: vec!["felix_helper".into(), tag_of(name, "helper")],
                        orders: "guard".into(),
                        ..NpcSpec::default()
                    };
                    let anchor = [f64::from(spot[0]) + 0.5 + f64::from(k), f64::from(spot[1]), f64::from(spot[2]) + 0.5];
                    if crate::villages::settle(host, OVERWORLD, anchor, spec).is_some() {
                        sent += 1;
                    }
                }
                if sent > 0 {
                    t.helpers = Some(from.clone());
                    line.push_str(&format!(" {sent} from {from} come to stand with them."));
                }
            }
            set_tale(name, t);
            if let Ok(mut s) = STATE.lock() {
                s.militia.insert(name.to_string(), fighters);
                s.armed.retain(|v| v != name);
            }
            // Its friends among the players hear of it wherever they are.
            crate::factions::call_friends(host, name, center, &format!("{name} is under attack, and calls for its friends!"));
            line
        }
        Incident::Blight => {
            let y = ground(host, center[0], center[1])?;
            let (x0, z0, x1, z1) = (center[0] - 32, center[1] - 32, center[0] + 31, center[1] + 31);
            let mut any = false;
            for crop in ["minecraft:wheat", "minecraft:carrots", "minecraft:potatoes", "minecraft:beetroots"] {
                // (Fails when there was none to wither.)
                any |= host.commands().run_command(&format!("fill {x0} {} {z0} {x1} {} {z1} {crop}[age=0] replace {crop}", y - 4, y + 2));
            }
            if !any {
                return None;
            }
            remember_all(host, people, "crops_lost");
            format!("A blight has struck the fields of {name}: the crops wither back to sprouts.")
        }
        Incident::Outbreak => {
            let n = story::sick_count(people.len(), r);
            let names = ["the flu", "a fever", "a cough"];
            let what = names[((r >> 12) % 3) as usize];
            let mut ill = Vec::new();
            for k in 0..people.len() {
                if ill.len() >= n {
                    break;
                }
                let e = people[(k + (r >> 20) as usize) % people.len()];
                if crate::healers::make_sick(host, e, what) {
                    ill.push(name_of(&*host, e));
                }
            }
            if ill.is_empty() {
                return None;
            }
            format!("{} is going round {name}: {} {} fallen ill.", capital(what), ill.join(" and "), if ill.len() == 1 { "has" } else { "have" })
        }
        Incident::Spoilage => {
            let mut lost = 0u64;
            for c in crate::chiefs::chests(host, people) {
                for f in crate::need::FOODS {
                    let a = crate::jobs::amount(host, c, f.0);
                    let k = (a as f32 * story::SPOIL) as u64;
                    if k > 0 && crate::jobs::take(host, c, f.0, k as u32) {
                        lost += k;
                    }
                }
            }
            if lost == 0 {
                return None;
            }
            remember_all(host, people, "stores_spoiled");
            format!("Rot has got into the stores of {name}: {lost} food had to be thrown out.")
        }
        Incident::Quarrel => {
            let (other, over, bond) = crate::factions::fall_out(host, name, r, now)?;
            remember_all(host, people, "quarrel");
            let line = format!("{name} and {other} have fallen out over {over}.{}", if bond == crate::faction::Bond::Feud { " They are at feud now." } else { "" });
            crate::chiefs::record(host, &other, now, &line);
            line
        }
        Incident::Aid => {
            let (from, n) = crate::factions::aid_for(host, name, w.people, w.food)?;
            let chest = crate::chiefs::chests(host, people).first().copied()?;
            let got = crate::jobs::put_item(host, chest, "minecraft:bread", n);
            if got == 0 {
                return None;
            }
            crate::factions::send_aid(host, &from, name, got, now);
            remember_all(host, people, "aid");
            if let Some(h) = head {
                crate::heart::say_aloud(host, h, &format!("Bread from {from}! We won't forget this."));
            }
            let line = format!("Hearing {name} is short of food, its friends in {from} have sent {got} bread.");
            crate::chiefs::record(host, &from, now, &line);
            line
        }
    };
    let mut t = tale(name, now / DAY);
    t.teller.happened(i, now / DAY);
    set_tale(name, t);
    crate::chiefs::record(host, name, now, &line);
    Some(line)
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn short(item: &str) -> String {
    item.trim_start_matches("minecraft:").replace('_', " ")
}

/// Whether raiders are at village `name` now.
pub fn raided_now(name: &str) -> bool {
    STATE.lock().is_ok_and(|s| s.tales.get(name).is_some_and(|t| t.raid.is_some()))
}

/// Days since raiders last came at village `name` (many: never).
pub fn days_since_raid(host: &mut dyn ServerHost, name: &str, day: i64) -> i64 {
    read(host);
    STATE.lock().ok().and_then(|s| s.tales.get(name).and_then(|t| t.teller.last_of(Incident::Raid))).map_or(999, |d| day - d)
}

/// What raiders nobody drove off carry away from the village's chests: food, and logs.
fn plunder(host: &mut dyn ServerHost, people: &[EntityRef]) -> (u64, u64) {
    let (mut food, mut logs) = (0, 0);
    for c in crate::chiefs::chests(host, people) {
        for f in crate::need::FOODS {
            let k = crate::defence::plunder(crate::jobs::amount(host, c, f.0));
            if k > 0 && crate::jobs::take(host, c, f.0, k as u32) {
                food += k;
            }
        }
        for l in crate::chiefs::LOGS {
            let k = crate::defence::plunder(crate::jobs::amount(host, c, l));
            if k > 0 && crate::jobs::take(host, c, l, k as u32) {
                logs += k;
            }
        }
    }
    (food, logs)
}

/// Raiders about the village go for its folk (else the players about), and those who stand go
/// for them; all killed or half of them down (they run), the village has won; not driven off in
/// time, they plunder it and leave.
fn raid_tick(host: &mut dyn ServerHost, name: &str, center: [i32; 2], people: &[EntityRef], end: i64, now: i64) {
    let raiders = tagged(host, center, &tag_of(name, "raider"));
    let t = tale(name, now / DAY);
    let helpers = if t.helpers.is_some() { tagged(host, center, &tag_of(name, "helper")) } else { Vec::new() };
    // Those who came to help get their blades (once they are there to take them).
    if !helpers.is_empty()
        && STATE.lock().is_ok_and(|s| !s.armed.iter().any(|v| v == name))
        && host.commands().run_command(&format!("item replace entity @e[tag={}] weapon.mainhand with minecraft:iron_sword", tag_of(name, "helper")))
        && let Ok(mut s) = STATE.lock()
    {
        s.armed.push(name.to_string());
    }
    let routed = crate::defence::routed(t.raiders, raiders.len() as u32);
    let over = raiders.is_empty() || routed || now >= end;
    if over {
        let won = raiders.is_empty() || routed;
        for e in &raiders {
            e.discard(host);
        }
        let from = t.helpers.clone();
        let mut t = t;
        t.raid = None;
        t.raiders = 0;
        t.helpers = None;
        set_tale(name, t);
        let militia = STATE.lock().ok().and_then(|mut s| s.militia.remove(name)).unwrap_or_default();
        for e in people {
            companion::go_to(e.id(), None);
            if militia.contains(&e.id()) {
                // Back to its day, and something to remember.
                MobRef::new(*e).set_target(host, None);
                crate::routines::resume(e.id());
                crate::minds::remember(host, *e, "fought");
            }
        }
        let line = if won {
            remember_all(host, people, "raid_won");
            if let Some(h) = crate::chiefs::head_of(host, name, people) {
                crate::heart::say_aloud(host, h, "They're gone! Well done, everyone.");
            }
            if routed { format!("The raiders have had enough, and run from {name}!") } else { format!("{name} has driven off the raiders!") }
        } else {
            remember_all(host, people, "plundered");
            match plunder(host, people) {
                (0, 0) => format!("The raiders find nothing worth taking in {name}, and leave."),
                (food, 0) => format!("The raiders plunder {name}: {food} food is gone."),
                (0, logs) => format!("The raiders plunder {name}: {logs} logs are gone."),
                (food, logs) => format!("The raiders plunder {name}: {food} food and {logs} logs are gone."),
            }
        };
        crate::chiefs::record(host, name, now, &line);
        crate::chiefs::tell_near(host, center, &line);
        // Those who came to help go home; the two villages are closer for it.
        if let Some(from) = from {
            for e in &helpers {
                e.discard(host);
            }
            if !helpers.is_empty() {
                remember_all(host, people, "stood_by");
                crate::factions::thank(host, name, &from, crate::faction::CARAVAN + 1, now);
                let line = format!("The folk of {from} who stood with {name} head home.");
                crate::chiefs::record(host, name, now, &line);
                crate::chiefs::record(host, &from, now, &format!("{from} sent help to {name} against raiders."));
                crate::chiefs::tell_near(host, center, &line);
            }
        }
        return;
    }
    // Those who stand go for the nearest raider.
    let militia = STATE.lock().ok().and_then(|s| s.militia.get(name).cloned()).unwrap_or_default();
    let stand = people.iter().filter(|e| militia.contains(&e.id())).chain(helpers.iter());
    for e in stand.copied().collect::<Vec<_>>() {
        let mob = MobRef::new(e);
        if mob.target(&*host).is_some_and(|t| t.alive(&*host) == Some(true)) {
            continue;
        }
        let Some(at) = e.position(&*host) else { continue };
        let d = |p: [f64; 3]| (p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2) + (p[2] - at[2]).powi(2);
        let near = raiders.iter().filter_map(|r| Some((d(r.position(&*host)?), *r))).filter(|(x, _)| *x < 40.0 * 40.0).min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, target)) = near {
            mob.set_target(host, Some(LivingRef::new(target)));
        }
    }
    let players = host.level(OVERWORLD).and_then(|l| l.online_players(&*host)).unwrap_or_default();
    for raider in raiders {
        let mob = MobRef::new(raider);
        if mob.target(&*host).is_some_and(|t| t.alive(&*host) == Some(true)) {
            continue;
        }
        let Some(at) = raider.position(&*host) else { continue };
        let d = |p: [f64; 3]| (p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2) + (p[2] - at[2]).powi(2);
        let folk = people.iter().filter_map(|e| Some((d(e.position(&*host)?), *e)));
        let others = players.iter().filter_map(|p| Some((d(p.entity().position(&*host)?), p.entity())));
        if let Some((_, target)) = folk.chain(others).filter(|(x, _)| *x < 48.0 * 48.0).min_by(|a, b| a.0.total_cmp(&b.0)) {
            mob.set_target(host, Some(LivingRef::new(target)));
        }
    }
}

/// Something died: a raider killed by a player earns it the village's thanks; one of a village's
/// own is grieved.
pub fn on_death(host: &mut dyn ServerHost, victim: u32, killer: i32) {
    let Some(e) = host.entity(victim) else { return };
    let tags = e.tags(&*host).unwrap_or_default();
    if tags.iter().any(|t| t == "felix_raider") && killer > 0 {
        read(host);
        let Some(name) = crate::settlements::list(host).into_iter().map(|(_, n)| n).find(|n| tags.contains(&tag_of(n, "raider"))) else { return };
        let Some(level) = e.level(&*host) else { return };
        let Some(p) = level.online_players(&*host).unwrap_or_default().into_iter().find(|p| p.entity().id() == killer as u32) else { return };
        for m in crate::chiefs::members(host, &name) {
            crate::heart::befriend(host, m, p, 3);
        }
        return;
    }
    if companion::is_npc(&*host, e) {
        read(host);
        let Some(name) = crate::chiefs::village_of(host, e) else { return };
        let day = e.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0) / DAY;
        let mut t = tale(&name, day);
        t.death = Some(day);
        set_tale(&name, t);
        write(host);
    }
}

/// The overworld's tick: each village's day, raids and caravans.
pub fn tick(host: &mut dyn ServerHost) {
    let Some(level) = host.level(OVERWORLD) else { return };
    let Some(now) = level.game_time(&*host) else { return };
    let raids = STATE.lock().is_ok_and(|s| s.tales.values().any(|t| t.raid.is_some()));
    // Raids are looked at each second, the rest each half minute.
    if !crate::pace::since("story_raid", now, 20) {
        return;
    }
    let slow = crate::pace::since("story", now, 600);
    if !raids && !slow {
        return;
    }
    read(host);
    if STATE.lock().is_ok_and(|s| s.seed == 0)
        && let Ok(mut s) = STATE.lock()
    {
        s.seed = now as u64 ^ 0x5DEE_CE66_D;
    }
    let time = level.day_time(&*host).unwrap_or(0).rem_euclid(DAY);
    let mut changed = false;
    for (center, name) in crate::settlements::list(host) {
        let t = tale(&name, now / DAY);
        if t.raid.is_none() && !slow {
            continue;
        }
        let people = crate::chiefs::members(host, &name);
        if people.is_empty() {
            continue;
        }
        if let Some(end) = t.raid {
            raid_tick(host, &name, center, &people, end, now);
            changed |= tale(&name, now / DAY).raid.is_none();
        }
        if !slow {
            continue;
        }
        if let Some(leave) = t.caravan
            && now >= leave
        {
            for e in tagged(host, center, &tag_of(&name, "caravan")) {
                crate::heart::say_aloud(host, e, "Safe travels, all. Until next time!");
                e.discard(host);
            }
            let mut t = tale(&name, now / DAY);
            t.caravan = None;
            set_tale(&name, t);
            crate::chiefs::tell_near(host, center, &format!("The caravan moves on from {name}."));
            changed = true;
        }
        let day = now / DAY;
        let t = tale(&name, day);
        if day > t.told && (6_000..9_000).contains(&time) {
            let mut t2 = t.clone();
            t2.told = day;
            set_tale(&name, t2);
            changed = true;
            if t.raid.is_some() {
                continue;
            }
            let w = world(host, &name, center, &people, now);
            if let Some(i) = story::pick(&tale(&name, day).teller, &w, roll())
                && let Some(line) = happen(host, &name, center, &people, i, now)
            {
                crate::chiefs::tell_near(host, center, &line);
            }
        }
    }
    if changed {
        write(host);
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix story", 2));
    e.commands.push(Command::op("felix story <incident:word>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if lit.first() != Some(&"felix") || lit.get(1) != Some(&"story") {
        return Flow::Continue;
    }
    read(host);
    let Some(now) = host.level(OVERWORLD).and_then(|l| l.game_time(&*host)) else { return Flow::Cancel };
    let day = now / DAY;
    let at = e.command.position;
    let mut villages = crate::settlements::list(host);
    villages.sort_by(|a, b| {
        let d = |c: [i32; 2]| (c[0] as f64 - at[0]).powi(2) + (c[1] as f64 - at[2]).powi(2);
        d(a.0).total_cmp(&d(b.0))
    });
    let word = e.command.arguments.iter().find(|a| a.name == "incident").and_then(|a| match &a.value {
        mod_api::CommandArgumentValue::String(s) => Some(s.clone()),
        _ => None,
    });
    let line = match word {
        None => {
            let lines: Vec<String> = villages
                .iter()
                .map(|(_, n)| {
                    let t = tale(n, day);
                    let last: Vec<String> = t.teller.last.iter().map(|(i, d)| format!("{} day {d}", i.id())).collect();
                    format!(
                        "{n}: day {day}, {} days in, {}{}{}; last: {}",
                        day - t.teller.since,
                        if t.teller.restless(day) { "restless" } else { "quiet" },
                        if t.raid.is_some() { ", raid on" } else { "" },
                        if t.caravan.is_some() { ", caravan here" } else { "" },
                        if last.is_empty() { "nothing yet".into() } else { last.join(", ") }
                    )
                })
                .collect();
            if lines.is_empty() { "No villages yet.".into() } else { lines.join("\n") }
        }
        Some(w) => {
            let Some(i) = Incident::parse(&w) else {
                let all: Vec<&str> = story::ALL.iter().map(|i| i.id()).collect();
                let _ = host.commands().feedback(Text::literal(format!("Incidents: {}.", all.join(", "))));
                return Flow::Cancel;
            };
            let Some((center, name)) = villages.first().cloned() else {
                let _ = host.commands().feedback(Text::literal("No villages yet."));
                return Flow::Cancel;
            };
            let people = crate::chiefs::members(host, &name);
            if people.is_empty() {
                "Nobody of that village is about.".to_string()
            } else {
                match happen(host, &name, center, &people, i, now) {
                    Some(l) => {
                        crate::chiefs::tell_near(host, center, &l);
                        write(host);
                        format!("Told {} at {name}.", i.id())
                    }
                    None => format!("{} could not happen at {name} now.", i.id()),
                }
            }
        }
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
