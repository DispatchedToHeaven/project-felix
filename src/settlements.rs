//! Felix settlements on the server (see `settle`): a settlement's chunk, first made, is noted; once
//! the land about it is there it is built — cabins with beds and chests, a campfire, paths, a
//! wheat field with the farmer's chest, the woodcutter's trees and chest — and its people come,
//! living by their days, at their jobs, the farmer and the baker a couple (sometimes with a child).
//! What it knows is kept with the world (its `horizon-mod-data`). `/felix settlements` lists those made; `/felix settlement here` makes one where you stand.
use crate::companion::{DEFAULT_MODEL, Look};
use crate::settle::{self, House, Role, Site};
use crate::village::NpcSpec;
use crate::serde_json::{Value, json};
use mod_api::{Command, CommandSource, ConfigPath, Flow, Heightmap, LevelRef, PlayerRef, RegisterCommandsEvent, RunModCommandEvent, ServerHost, Text};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

const OVERWORLD: &str = "minecraft:overworld";
const FILE: &str = "settlements.json";
const CABIN: &str = "projectfelix:cabin";
/// Within this of its middle, a player is in a settlement.
const NEAR: f64 = 40.0;
/// Two settlements are no nearer than this (blocks, each way).
const APART: i32 = 96;
/// A place waiting to be built is tried while a player is within this of it (blocks).
const TRY_WITHIN: f64 = 192.0;

struct Couple {
    a: u32,
    b: u32,
    child: bool,
    wait: u32,
    /// Steps it was looked for and one of them was not there.
    missed: u8,
}

#[derive(Default)]
struct State {
    read: bool,
    /// The world it was read for.
    world: Option<std::path::PathBuf>,
    pending: Vec<[i32; 2]>,
    built: Vec<([i32; 2], String)>,
    couples: Vec<Couple>,
    /// The last places passed over, and why.
    passed: Vec<([i32; 2], String)>,
    /// The settlement each player was last in.
    in_one: HashMap<u128, String>,
    ticks: u32,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// The server stopped: nothing of its world is kept.
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
    let xz = |x: &Value| Some([x.get(0)?.as_i64()? as i32, x.get(1)?.as_i64()? as i32]);
    let pending = v.get("pending").and_then(Value::as_array).map(|l| l.iter().filter_map(xz).collect()).unwrap_or_default();
    let built = v.get("built").and_then(Value::as_array).map(|l| l.iter().filter_map(|x| Some((xz(x)?, x.get(2)?.as_str()?.to_string()))).collect()).unwrap_or_default();
    if let Ok(mut s) = STATE.lock() {
        s.read = true;
        s.world = world;
        s.in_one.clear();
        s.couples.clear();
        s.pending = pending;
        s.built = built;
    }
}

fn write(host: &mut dyn ServerHost) {
    let v = STATE.lock().map(|s| json!({ "pending": s.pending, "built": s.built.iter().map(|(p, n)| json!([p[0], p[1], n])).collect::<Vec<_>>() })).unwrap_or(Value::Null);
    if let Ok(p) = ConfigPath::new(FILE)
        && let Err(e) = host.world_data_files().write(&p, &crate::serde_json::to_vec_pretty(&v).unwrap_or_default())
    {
        eprintln!("[projectfelix] settlements not saved: {e:?}");
    }
}

/// `chunk_load`: a settlement's chunk just made is noted, to build once the land about it is there.
pub fn chunk_load(host: &mut dyn ServerHost, dimension: &str, chunk: [i32; 2], new: bool) {
    if !new || dimension != OVERWORLD {
        return;
    }
    let Some(seed) = host.level(OVERWORLD).and_then(|l| l.seed(&*host)) else { return };
    if !settle::is_site(seed, chunk) {
        return;
    }
    read(host);
    let center = [chunk[0] * 16 + 8, chunk[1] * 16 + 8];
    if let Ok(mut s) = STATE.lock() {
        if s.pending.contains(&center) || s.built.iter().any(|b| b.0 == center) {
            return;
        }
        s.pending.push(center);
    }
    write(host);
}

fn q(line: String) {
    crate::talk::queue(format!("execute in {OVERWORLD} run {line}"));
}

/// The settlement made at `center` in this world.
fn site_at(host: &dyn mod_api::Host, level: LevelRef, center: [i32; 2]) -> Site {
    let seed = level.seed(host).unwrap_or(0);
    settle::site(settle::hash(seed, center[0], center[1], 0x5E77_1E))
}

enum Outcome {
    Wait,
    Built(String),
    Not(String),
}

/// Builds the settlement noted at `center` there, else as near it as it can (the first spot about
/// it that will do): where, or why not.
fn try_near(host: &mut dyn ServerHost, center: [i32; 2]) -> (Outcome, [i32; 2]) {
    const AROUND: [[i32; 2]; 9] = [[0, 0], [32, 0], [-32, 0], [0, 32], [0, -32], [40, 40], [-40, 40], [40, -40], [-40, -40]];
    let mut first = None;
    let mut waiting = false;
    for d in AROUND {
        let at = [center[0] + d[0], center[1] + d[1]];
        match try_build(host, at, false) {
            Outcome::Built(n) => return (Outcome::Built(n), at),
            Outcome::Wait => waiting = true,
            Outcome::Not(why) => {
                first.get_or_insert(why);
            }
        }
    }
    if waiting { (Outcome::Wait, center) } else { (Outcome::Not(first.unwrap_or_default()), center) }
}

/// Builds the settlement at `center` if it can (`forced`: wherever, if the land is there).
fn try_build(host: &mut dyn ServerHost, center: [i32; 2], forced: bool) -> Outcome {
    let Some(level) = host.level(OVERWORLD) else { return Outcome::Wait };
    let site = site_at(&*host, level, center);
    let spots = settle::samples(&site);
    let cols: Vec<(i32, i32)> = spots.iter().map(|s| (center[0] + s[0], center[1] + s[1])).collect();
    let Ok(heights) = host.surface_heights(OVERWORLD, &cols, Heightmap::MotionBlockingNoLeaves) else { return Outcome::Wait };
    if heights.iter().any(Option::is_none) {
        return Outcome::Wait;
    }
    // The ground itself: a tree's trunk (a giant mushroom, bamboo) standing on it is not it.
    let top = |x: i32, y: i32, z: i32| level.block(&*host, [x, y - 1, z]).map(|k| k.id(&*host)).unwrap_or_default();
    let heights: Vec<Option<i32>> = cols
        .iter()
        .zip(&heights)
        .map(|((x, z), h)| {
            let mut y = h.unwrap_or(64);
            for _ in 0..32 {
                let id = top(*x, y, *z);
                let grown = id.ends_with("_log") || id.ends_with("_stem") || id.ends_with("_wood") || id.ends_with("_leaves") || id.ends_with("mushroom_block") || id.contains("bamboo");
                if !grown {
                    break;
                }
                y -= 1;
            }
            Some(y)
        })
        .collect();
    let hs: HashMap<(i32, i32), i32> = cols.iter().copied().zip(heights.iter().map(|h| h.unwrap_or(64))).collect();
    let h_of = |x: i32, z: i32| hs.get(&(center[0] + x, center[1] + z)).copied().unwrap_or(64);
    let hc = h_of(0, 0);
    if !forced {
        let biome = level.biome_at(&*host, [center[0], hc, center[1]]).unwrap_or_default();
        if !settle::biome_ok(&biome) {
            return Outcome::Not(format!("biome {biome}"));
        }
        let wet = cols.iter().zip(&heights).filter(|((x, z), h)| {
            let id = level.block(&*host, [*x, h.unwrap_or(64) - 1, *z]).map(|k| k.id(&*host)).unwrap_or_default();
            id.contains("water") || id.contains("lava") || id.contains("ice")
        });
        if wet.count() > 2 {
            return Outcome::Not("water".into());
        }
        // Not over what someone built (a village, a player's house), nor next to another settlement.
        let made = cols.iter().zip(&heights).any(|((x, z), h)| {
            let id = top(*x, h.unwrap_or(64), *z);
            ["planks", "cobblestone", "bricks", "dirt_path", "farmland", "glass", "wool", "_stairs", "_slab", "fence", "concrete", "carpet"].iter().any(|w| id.contains(w))
        });
        if made {
            return Outcome::Not("something is built there".into());
        }
        let near = STATE.lock().is_ok_and(|s| s.built.iter().any(|(c, _)| (c[0] - center[0]).abs() < APART && (c[1] - center[1]).abs() < APART));
        if near {
            return Outcome::Not("another settlement is near".into());
        }
        let ys: Vec<i32> = heights.iter().map(|h| h.unwrap_or(64)).collect();
        if ys.iter().max().unwrap_or(&0) - ys.iter().min().unwrap_or(&0) > 7 {
            return Outcome::Not("too steep".into());
        }
    }
    let Some(plan) = crate::builds::plan_named(CABIN) else { return Outcome::Not("no cabin plan".into()) };
    let [cx, cz] = center;
    // The cabins: cleared, set on a footing, built, furnished.
    let mut bases = Vec::new();
    for h in &site.houses {
        let base = h_of(h.front[0], h.front[1]);
        bases.push(base);
        let a = settle::place(h, [0, 0, 0]);
        let b = settle::place(h, [4, 0, 4]);
        let (x0, x1, z0, z1) = (cx + a[0].min(b[0]), cx + a[0].max(b[0]), cz + a[2].min(b[2]), cz + a[2].max(b[2]));
        q(format!("fill {x0} {} {z0} {x1} {} {z1} minecraft:air", base, base + 6));
        // What is left of a tree that stood there, above.
        for grown in ["#minecraft:logs", "#minecraft:leaves"] {
            q(format!("fill {x0} {} {z0} {x1} {} {z1} minecraft:air replace {grown}", base + 7, base + 16));
        }
        for fill in ["air", "water", "short_grass", "tall_grass", "snow"] {
            q(format!("fill {x0} {} {z0} {x1} {} {z1} minecraft:cobblestone replace minecraft:{fill}", base - 7, base - 2));
        }
        for c in plan.cells() {
            let Some(state) = c.state else { continue };
            let p = settle::place(h, c.at);
            q(format!("setblock {} {} {} {}", cx + p[0], base - 1 + p[1], cz + p[2], crate::plan::turn_state(&state, h.rot)));
        }
        for (at, state) in settle::furniture() {
            let p = settle::place(h, at);
            q(format!("setblock {} {} {} {}", cx + p[0], base - 1 + p[1], cz + p[2], crate::plan::turn_state(state, h.rot)));
        }
        // Paths to the middle.
        let steps = h.front[0].abs().max(h.front[1].abs());
        for i in 0..=steps {
            let (x, z) = (cx + h.front[0] * i / steps.max(1), cz + h.front[1] * i / steps.max(1));
            let y = hc + (base - hc) * i / steps.max(1);
            q(format!("fill {x} {} {z} {x} {} {z} minecraft:dirt_path replace minecraft:grass_block", y - 3, y + 2));
        }
    }
    // The middle: a campfire and seats.
    q(format!("fill {} {hc} {} {} {} {} minecraft:air", cx - 2, cz - 2, cx + 2, hc + 3, cz + 2));
    q(format!("setblock {cx} {hc} {cz} minecraft:campfire[lit=true]"));
    q(format!("setblock {} {hc} {} minecraft:oak_stairs[facing=east]", cx - 2, cz));
    q(format!("setblock {} {hc} {} minecraft:oak_stairs[facing=west]", cx + 2, cz));
    // The field: farmland, water in the middle, wheat (some ripe), the farmer's chest.
    let [fx, fz] = [cx + site.farm[0], cz + site.farm[1]];
    let n = settle::FARM - 1;
    let fb = h_of(site.farm[0] + settle::FARM / 2, site.farm[1] + settle::FARM / 2);
    q(format!("fill {fx} {fb} {fz} {} {} {} minecraft:air", fx + n, fb + 4, fz + n));
    q(format!("fill {fx} {} {fz} {} {} {} minecraft:dirt replace minecraft:air", fb - 5, fx + n, fb - 2, fz + n));
    q(format!("fill {fx} {} {fz} {} {} {} minecraft:farmland[moisture=7]", fb - 1, fx + n, fb - 1, fz + n));
    for z in 0..=n {
        q(format!("fill {fx} {fb} {} {} {fb} {} minecraft:wheat[age={}]", fz + z, fx + n, fz + z, if z % 2 == 0 { 7 } else { 4 }));
    }
    let (mx, mz) = (fx + settle::FARM / 2, fz + settle::FARM / 2);
    q(format!("setblock {mx} {fb} {mz} minecraft:air"));
    q(format!("setblock {mx} {} {mz} minecraft:water", fb - 1));
    let farm_chest = [fx - 1, fb, fz + settle::FARM / 2];
    q(format!("setblock {} {} {} minecraft:chest[facing=east]", farm_chest[0], farm_chest[1], farm_chest[2]));
    q(format!("item replace block {} {} {} container.0 with minecraft:wheat_seeds 8", farm_chest[0], farm_chest[1], farm_chest[2]));
    // The woodcutter's spot: its chest, and trees about it.
    let [wx, wz] = [cx + site.wood[0], cz + site.wood[1]];
    let wb = h_of(site.wood[0], site.wood[1]);
    q(format!("setblock {wx} {wb} {wz} minecraft:chest"));
    for (dx, dz) in [(4, 0), (0, 4), (-4, -1), (-1, -4)] {
        q(format!("place feature minecraft:oak {} {wb} {}", wx + dx, wz + dz));
    }
    // A start for the baker.
    let baker_home = site.people.iter().find(|p| p.role == Role::Baker).map_or(0, |p| p.home);
    let bh = site.houses[baker_home];
    let chest = settle::place(&bh, [1, 1, 3]);
    q(format!("item replace block {} {} {} container.0 with minecraft:wheat 9", cx + chest[0], bases[baker_home] - 1 + chest[1], cz + chest[2]));
    // Its people.
    let anchor = [f64::from(cx) + 0.5, f64::from(hc), f64::from(cz) + 0.5];
    let rel = |p: [f64; 3]| [p[0] - anchor[0], p[1] - anchor[1], p[2] - anchor[2]];
    let block = |x: i32, y: i32, z: i32| [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5];
    let mut ids = Vec::new();
    let tag = format!("felix_{}", site.name.to_lowercase());
    for p in &site.people {
        let house: &House = &site.houses[p.home];
        let inside = settle::place(house, settle::INSIDE);
        let home = block(cx + inside[0], bases[p.home] - 1 + inside[1], cz + inside[2]);
        let front = block(cx + house.front[0], bases[p.home], cz + house.front[1]);
        let work = match p.role {
            Role::Farmer => block(mx, fb, mz + 1),
            Role::Baker => front,
            Role::Woodcutter => block(wx + 1, wb, wz),
            _ => block(cx + 1, hc, cz + 2),
        };
        let meet = block(cx - 1, hc, cz - 2);
        let mut places = vec![("home".to_string(), rel(home)), ("work".to_string(), rel(work)), ("meet".to_string(), rel(meet))];
        if p.role == Role::Guard {
            places.push(("guard".to_string(), rel(block(cx, hc, cz + 3))));
        }
        let look = Look::of_model(DEFAULT_MODEL);
        let spec = NpcSpec {
            name: p.name.clone(),
            at: rel(home),
            model: look.model,
            texture: look.texture,
            routine: Some(p.role.routine().to_string()),
            shop: p.role.shop().map(str::to_string),
            character: Some(p.character.to_string()),
            places,
            tags: vec!["felix_settler".into(), tag.clone()],
            orders: "live".into(),
            job: p.role.job().map(str::to_string),
            ..NpcSpec::default()
        };
        ids.push(crate::villages::settle(host, OVERWORLD, anchor, spec));
    }
    let (a, b) = site.couple;
    if let (Some(Some(a)), Some(Some(b))) = (ids.get(a), ids.get(b))
        && let Ok(mut s) = STATE.lock()
    {
        s.couples.push(Couple { a: *a, b: *b, child: site.child, wait: 3, missed: 0 });
    }
    Outcome::Built(site.name)
}

fn players(host: &dyn mod_api::Host) -> Vec<PlayerRef> {
    host.level(OVERWORLD).and_then(|l| l.online_players(host)).unwrap_or_default()
}

/// The level tick: settlements whose land is there get built; couples settle in; players coming
/// into one are told its name.
pub fn level_tick(host: &mut dyn ServerHost, dimension: &str) {
    // Settlements are the overworld's: its tick only (each level has its own).
    if dimension != OVERWORLD {
        return;
    }
    let step = STATE.lock().map(|mut s| {
        s.ticks = s.ticks.wrapping_add(1);
        s.ticks % 40 == 0
    });
    if !matches!(step, Ok(true)) {
        return;
    }
    read(host);
    // Couples: once both are set up.
    let couples = STATE.lock().map(|mut s| std::mem::take(&mut s.couples)).unwrap_or_default();
    let mut later = Vec::new();
    for mut c in couples {
        if c.wait > 0 {
            c.wait -= 1;
            later.push(c);
            continue;
        }
        let (Some(a), Some(b)) = (host.entity(c.a), host.entity(c.b)) else {
            // Not there yet (or unloaded for now): a few more looks.
            if c.missed < 30 {
                c.missed += 1;
                later.push(c);
            }
            continue;
        };
        crate::families::bind(host, a, b);
        if c.child {
            let _ = crate::families::birth(host, a, b);
        }
    }
    if let Ok(mut s) = STATE.lock() {
        s.couples.extend(later);
    }
    // One settlement a step at most.
    let pending = STATE.lock().map(|s| s.pending.clone()).unwrap_or_default();
    let about: Vec<[f64; 3]> = players(&*host).into_iter().filter_map(|p| p.entity().position(&*host)).collect();
    for center in pending {
        // Far from everyone its land may not be there, and no one would see it: later.
        if !about.iter().any(|p| (p[0] - f64::from(center[0])).hypot(p[2] - f64::from(center[1])) <= TRY_WITHIN) {
            continue;
        }
        let (outcome, at) = try_near(host, center);
        match outcome {
            Outcome::Wait => continue,
            Outcome::Built(name) => {
                if let Ok(mut s) = STATE.lock() {
                    s.pending.retain(|c| *c != center);
                    s.built.push((at, name));
                }
                write(host);
                break;
            }
            Outcome::Not(why) => {
                eprintln!("[projectfelix] no settlement at {} {}: {why}", center[0], center[1]);
                if let Ok(mut s) = STATE.lock() {
                    s.pending.retain(|c| *c != center);
                    s.passed.push((center, why));
                    let n = s.passed.len();
                    s.passed.drain(..n.saturating_sub(4));
                }
                write(host);
            }
        }
    }
    // Players coming into one.
    let built = STATE.lock().map(|s| s.built.clone()).unwrap_or_default();
    for p in players(&*host) {
        let Some(at) = p.entity().position(&*host) else { continue };
        let here = built.iter().find(|(c, _)| (f64::from(c[0]) - at[0]).hypot(f64::from(c[1]) - at[2]) <= NEAR).map(|b| b.1.clone());
        let before = STATE.lock().ok().and_then(|s| s.in_one.get(&p.uuid()).cloned());
        if here != before {
            if let Some(name) = &here {
                let _ = p.action_bar(host, Text::literal(format!("{name} - a Felix settlement")));
            }
            if let Ok(mut s) = STATE.lock() {
                match here {
                    Some(n) => s.in_one.insert(p.uuid(), n),
                    None => s.in_one.remove(&p.uuid()),
                };
            }
        }
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::new("felix settlements"));
    e.commands.push(Command::op("felix settlement here", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let line = match lit.as_slice() {
        ["felix", "settlements"] => {
            read(host);
            let at = match e.source {
                CommandSource::Player { player, .. } => player.entity().position(&*host),
                _ => None,
            };
            let (built, pending, passed) = STATE.lock().map(|s| (s.built.clone(), s.pending.len(), s.passed.clone())).unwrap_or_default();
            let op = matches!(e.source, CommandSource::Player { player, .. } if player.permission_level(&*host).unwrap_or(0) >= 2);
            let passed = if op && !passed.is_empty() { format!(" Passed over: {}.", passed.iter().map(|(c, w)| format!("{} {} ({w})", c[0], c[1])).collect::<Vec<_>>().join(", ")) } else { String::new() };
            if built.is_empty() {
                format!("No Felix settlement yet ({pending} to come as the land is made).{passed}")
            } else {
                let mut list: Vec<(f64, String)> = built
                    .iter()
                    .map(|(c, n)| {
                        let d = at.map_or(0.0, |a| (f64::from(c[0]) - a[0]).hypot(f64::from(c[1]) - a[2]));
                        (d, format!("{n} at {} {} ({:.0} blocks)", c[0], c[1], d))
                    })
                    .collect();
                list.sort_by(|a, b| a.0.total_cmp(&b.0));
                format!("Felix settlements: {}.{passed}", list.into_iter().map(|x| x.1).take(8).collect::<Vec<_>>().join("; "))
            }
        }
        ["felix", "settlement", "here"] => {
            let CommandSource::Player { player, .. } = e.source else { return Flow::Cancel };
            let Some(at) = player.entity().position(&*host) else { return Flow::Cancel };
            read(host);
            let center = [at[0].floor() as i32, at[2].floor() as i32];
            match try_build(host, center, true) {
                Outcome::Built(name) => {
                    if let Ok(mut s) = STATE.lock() {
                        s.built.push((center, name.clone()));
                    }
                    write(host);
                    format!("{name} is made here.")
                }
                Outcome::Wait => "The land about here isn't all there yet.".to_string(),
                Outcome::Not(why) => format!("Not here: {why}"),
            }
        }
        _ => return Flow::Continue,
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
