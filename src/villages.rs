//! Villages on the server (see `village`): `/felix village export <name> [radius]` saves the NPCs
//! around you, and what the game made that they need, to `exports/<name>.felixvillage`;
//! `/felix village import <name>` (from `imports/`, else `exports/`) puts them around you, yours,
//! each with its look, dialog, rules, day, shop, quests, character, voice, job, places, tags and level.
use crate::companion::{self, LOOK, Look, Mode, NPC};
use crate::village::{NpcSpec, Village};
use crate::zipfile::{self, Entry};
use mod_api::{
    Command, CommandArgumentValue, CommandSource, Compound, ConfigPath, Flow, Holder, Host, LevelRef, PlayerRef,
    RegisterCommandsEvent, RunModCommandEvent, ServerHost, Tag, Text,
};
use std::sync::{LazyLock, Mutex};

const EXT: &str = ".felixvillage";

/// Summoned NPCs to set up once they are in the level: (entity, what it is, where the village is).
static PENDING: LazyLock<Mutex<Vec<(u32, NpcSpec, [f64; 3])>>> = LazyLock::new(Default::default);

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Follow => "follow",
        Mode::Wait => "wait",
        Mode::Guard => "guard",
        Mode::Live => "live",
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// How far up and down, and to the sides, a spot is looked for.
const REACH: i32 = 24;
const SIDES: i32 = 8;

/// Where an NPC can stand nearest `p`: in the middle of a block, on ground that is not a tree
/// (leaves, logs), room for its body, out of water; its own column first, then ever wider rings
/// (out of a lake). `p` itself where there is none (or the chunks are not there).
fn ground(host: &dyn Host, level: LevelRef, p: [f64; 3]) -> [f64; 3] {
    let (bx, bz) = (p[0].floor() as i32, p[2].floor() as i32);
    for ring in 0..=SIDES {
        for dx in -ring..=ring {
            for dz in -ring..=ring {
                if dx.abs().max(dz.abs()) != ring {
                    continue;
                }
                let (x, z) = (f64::from(bx + dx) + 0.5, f64::from(bz + dz) + 0.5);
                if let Some(y) = column(host, level, x, z, p[1]) {
                    return [x, y, z];
                }
            }
        }
    }
    p
}

/// The height nearest `y` an NPC can stand at in the column of `x`, `z` (a block's middle).
fn column(host: &dyn Host, level: LevelRef, x: f64, z: f64, y: f64) -> Option<f64> {
    let r = 0.3;
    let stand = |cell: i32| -> Option<f64> {
        let y = f64::from(cell);
        // The top of what is under the feet in this cell (a full block, a slab, a path).
        let boxes = level.collision_boxes(host, [x - r, y - 1.0, z - r, x + r, y + 0.6, z + r])?;
        let top = boxes.iter().map(|b| b[4]).filter(|t| *t <= y + 0.6).fold(f64::NAN, f64::max);
        if top.is_nan() {
            return None;
        }
        let below = [x.floor() as i32, (top - 0.5).floor() as i32, z.floor() as i32];
        let id = level.block(host, below).map(|k| k.id(host)).unwrap_or_default();
        if id.contains("leaves") || id.ends_with("_log") || id.ends_with("_wood") || id.ends_with("_stem") {
            return None;
        }
        if level.has_collisions(host, [x - r, top + 0.01, z - r, x + r, top + 1.8, z + r])? {
            return None;
        }
        let feet = [x.floor() as i32, (top + 0.01).floor() as i32, z.floor() as i32];
        if level.fluid_state(host, feet).is_some_and(|f| !f.is_empty()) {
            return None;
        }
        Some(top)
    };
    let base = y.round() as i32;
    (0..=REACH).flat_map(|d| if d == 0 { vec![base] } else { vec![base + d, base - d] }).find_map(stand)
}

fn export(host: &mut dyn ServerHost, player: PlayerRef, name: &str, radius: f64) -> Result<String, String> {
    let anchor = player.entity().position(&*host).ok_or("no position")?;
    let level = player.entity().level(&*host).ok_or("no level")?;
    let r = radius;
    let around = level.entities_in(&*host, [anchor[0] - r, anchor[1] - r, anchor[2] - r], [anchor[0] + r, anchor[1] + r, anchor[2] + r]).unwrap_or_default();
    let mut village = Village { name: name.to_string(), npcs: Vec::new() };
    let mut profiles = Vec::new();
    let npcs: Vec<_> = around.into_iter().filter(|e| companion::is_npc(&*host, *e)).collect();
    for e in npcs {
        let Some(at) = e.position(&*host) else { continue };
        let id = e.id();
        let look = LOOK.get(host, Holder::Entity(id)).ok().flatten().unwrap_or_else(|| Look::of_model(companion::DEFAULT_MODEL));
        let dialog = crate::talk::DIALOG.get(host, Holder::Entity(id)).ok().flatten();
        let places = crate::routines::PLACES.get(host, Holder::Entity(id)).unwrap_or_default().into_iter().map(|(k, p)| (k, sub(p, anchor))).collect();
        let spec = NpcSpec {
            name: e.name(&*host).unwrap_or_else(|| "NPC".into()),
            at: sub(at, anchor),
            yaw: e.rotation(&*host).map(|r| r[0]).unwrap_or(0.0),
            model: look.model,
            texture: look.texture,
            dialog,
            rules: crate::rulebook::RULES.get(host, Holder::Entity(id)).ok().flatten(),
            routine: crate::routines::ROUTINE.get(host, Holder::Entity(id)).ok().flatten(),
            shop: crate::shops::SHOP.get(host, Holder::Entity(id)).ok().flatten(),
            quests: crate::quests::GIVES.get(host, Holder::Entity(id)).ok().flatten(),
            character: crate::heart::character_of(host, e),
            places,
            tags: e.tags(&*host).unwrap_or_default(),
            level: crate::gear::level_of(host, e),
            orders: mode_name(companion::mode_of(id)).to_string(),
            voice: crate::voices::VOICE.get(host, Holder::Entity(id)).ok().flatten(),
            job: crate::jobs::JOB.get(host, Holder::Entity(id)).ok().flatten(),
        };
        if let Some(p) = spec.profile() {
            profiles.push(p.to_string());
        }
        village.npcs.push(spec);
    }
    if village.npcs.is_empty() {
        return Err(format!("no NPCs within {radius:.0} blocks"));
    }
    let mut entries = vec![Entry { name: "village.json".into(), data: crate::serde_json::to_vec_pretty(&village.to_json()).unwrap_or_default() }];
    profiles.sort();
    profiles.dedup();
    for p in &profiles {
        if let Some(file) = crate::custom::profile_file(p) {
            entries.push(Entry { name: format!("npcs/{p}.felixnpc"), data: file });
        }
    }
    for c in village.made_characters() {
        if let Some(v) = crate::heart::made_json(&c) {
            let n = c.trim_start_matches("felixchar:");
            entries.push(Entry { name: format!("characters/{n}.json"), data: crate::serde_json::to_vec_pretty(&v).unwrap_or_default() });
        }
    }
    for (n, text) in crate::scenes::made_all() {
        entries.push(Entry { name: format!("scenes/{n}.json"), data: text.into_bytes() });
    }
    let file = zipfile::write(&entries, &crate::image::crc32);
    let to = ConfigPath::new(&format!("exports/{name}{EXT}")).map_err(|e| format!("{e:?}"))?;
    host.data_files().write(&to, &file).map_err(|e| format!("{e:?}"))?;
    Ok(format!("{} NPCs, {} files of theirs: horizon-mod-data/projectfelix/exports/{name}{EXT}", village.npcs.len(), entries.len() - 1))
}

fn import(host: &mut dyn ServerHost, player: PlayerRef, name: &str) -> Result<String, String> {
    let files = host.data_files();
    let bytes = ["imports", "exports"]
        .iter()
        .find_map(|dir| ConfigPath::new(&format!("{dir}/{name}{EXT}")).ok().and_then(|p| files.read(&p).ok()))
        .ok_or(format!("no {name}{EXT} in imports/ or exports/"))?;
    let entries = zipfile::read(&bytes, &crate::image::inflate, 32 << 20)?;
    let get = |n: &str| entries.iter().find(|e| e.name == n).map(|e| e.data.clone());
    let village = Village::parse(&get("village.json").ok_or("no village.json")?)?;
    let mut problems = Vec::new();
    for e in &entries {
        let result = if e.name.starts_with("npcs/") && e.name.ends_with(".felixnpc") {
            crate::custom::import_profile(host, &e.data).map(|_| ())
        } else if let Some(n) = e.name.strip_prefix("characters/").and_then(|n| n.strip_suffix(".json")) {
            crate::serde_json::from_slice(&e.data).map_err(|err| err.to_string()).and_then(|v| crate::heart::keep_character(host, n, v)).map(|_| ())
        } else if let Some(n) = e.name.strip_prefix("scenes/").and_then(|n| n.strip_suffix(".json")) {
            crate::scenes::keep_scene(host, n, &String::from_utf8_lossy(&e.data)).map(|_| ())
        } else {
            Ok(())
        };
        if let Err(err) = result {
            problems.push(format!("{}: {err}", e.name));
        }
    }
    let anchor = player.entity().position(&*host).ok_or("no position")?;
    let dim = player.dimension(&*host).map(|d| d.to_string()).unwrap_or_else(|| "minecraft:overworld".into());
    let mut n = 0;
    for spec in village.npcs {
        let mut nbt = Compound::new();
        let u = player.uuid();
        nbt.insert("Owner".into(), Tag::IntArray(vec![(u >> 96) as i32, (u >> 64) as i32, (u >> 32) as i32, u as i32]));
        nbt.insert("PersistenceRequired".into(), Tag::Byte(1));
        nbt.insert("CustomName".into(), Tag::String(spec.name.as_str().into()));
        if let Some(e) = host.summon(&dim, NPC, add(anchor, spec.at), nbt) {
            n += 1;
            if let Ok(mut p) = PENDING.lock() {
                p.push((e, spec, anchor));
            }
        }
    }
    let note = if problems.is_empty() { String::new() } else { format!(" ({})", problems.join("; ")) };
    Ok(format!("{n} NPCs of {} came{note}", village.name))
}

/// The level tick: the NPCs just summoned get all they had.
pub fn level_tick(host: &mut dyn ServerHost) {
    let pending = PENDING.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
    let mut later = Vec::new();
    for (id, spec, anchor) in pending {
        let Some(e) = host.entity(id) else {
            later.push((id, spec, anchor));
            continue;
        };
        let _ = LOOK.set(host, Holder::Entity(id), Some(Look { model: spec.model.clone(), texture: spec.texture.clone() }));
        let _ = crate::talk::DIALOG.set(host, Holder::Entity(id), spec.dialog.clone());
        let _ = crate::rulebook::RULES.set(host, Holder::Entity(id), spec.rules.clone());
        let _ = crate::shops::SHOP.set(host, Holder::Entity(id), spec.shop.clone());
        let _ = crate::quests::GIVES.set(host, Holder::Entity(id), spec.quests.clone());
        let _ = crate::voices::VOICE.set(host, Holder::Entity(id), spec.voice.clone());
        let _ = crate::jobs::JOB.set(host, Holder::Entity(id), spec.job.clone());
        if let Some(c) = &spec.character {
            crate::heart::set_character(host, e, Some(c));
        }
        let level = e.level(&*host);
        let on_ground = |host: &dyn Host, p: [f64; 3]| level.map_or(p, |l| ground(host, l, p));
        let places = spec.places.iter().map(|(k, p)| (k.clone(), on_ground(&*host, add(anchor, *p)))).collect();
        let _ = crate::routines::PLACES.set(host, Holder::Entity(id), places);
        if spec.level > 0 {
            crate::gear::set_level(host, e, spec.level);
        }
        if let Some(u) = e.uuid(host).map(crate::talk::uuid_text) {
            for t in &spec.tags {
                crate::talk::queue(format!("tag {u} add {t}"));
            }
            // Its feet on this world's ground, which is not where the village was made.
            let at = on_ground(&*host, add(anchor, spec.at));
            crate::talk::queue(format!("tp {u} {:.2} {:.2} {:.2} {:.1} 0", at[0], at[1], at[2], spec.yaw));
        }
        match (&spec.routine, spec.orders.as_str()) {
            (Some(r), "live") => {
                crate::routines::set_routine(host, e, Some(r));
            }
            (r, o) => {
                if let Some(r) = r {
                    let _ = crate::routines::ROUTINE.set(host, Holder::Entity(id), Some(r.clone()));
                }
                let mode = match o {
                    "follow" => Mode::Follow,
                    "guard" => Mode::Guard,
                    _ => Mode::Wait,
                };
                companion::set_orders(host, e, mode);
            }
        }
    }
    if !later.is_empty()
        && let Ok(mut p) = PENDING.lock()
    {
        p.extend(later.into_iter().take(128));
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix village export <name:word> [radius:integer]", 2));
    e.commands.push(Command::op("felix village import <name:word>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let export_it = match lit.as_slice() {
        ["felix", "village", "export"] => true,
        ["felix", "village", "import"] => false,
        _ => return Flow::Continue,
    };
    let CommandSource::Player { player, .. } = e.source else {
        let _ = host.commands().feedback(Text::literal("A village is saved and placed around a player."));
        return Flow::Cancel;
    };
    let player = *player;
    let name = crate::profile::make_id(&companion::text_argument(e, "name").unwrap_or_default());
    let radius = e.command.arguments.iter().find(|a| a.name == "radius").and_then(|a| match a.value {
        CommandArgumentValue::Integer(v) => Some(v.clamp(4, 128) as f64),
        _ => None,
    });
    let result = if export_it { export(host, player, &name, radius.unwrap_or(48.0)) } else { import(host, player, &name) };
    let line = match result {
        Ok(l) => l,
        Err(err) => format!("Village {name}: {err}"),
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
