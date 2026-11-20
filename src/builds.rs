//! Building on the server (see `plan`): a builder NPC (job `build`) at work builds the plan it
//! was given, a block at a time, from its chest; says what it is short of; clears what is in the
//! way into its chest. `/felix build <npc> <plan>` sets the site before the player (turned their
//! way), `/felix build status|stop <npc>`, `/felix plans`, `/felix plan save <name> <dx> <dy> <dz>`
//! keeps what stands from the player's feet as `felixplan:<name>`.
use crate::companion::{self, LiveDo};
use crate::plan::{self, Plan};
use mod_api::{
    AddServerReloadListenersEvent, AttachmentCodec, AttachmentType, Command, CommandArgumentValue, CommandSource,
    Compound, ConfigPath, EntityRef, FileToIdConverter, Flow, Hand, Holder, Identifier, LevelRef, LivingRef,
    RegisterCommandsEvent, ReloadApply, ReloadResourceError, RunModCommandEvent, ServerHost, ServerReloadContext,
    ServerReloadListener, SoundSource, Tag, Text,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Mutex};

/// What a builder builds: its plan, the site's corner, its turn, and how far it got.
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    pub plan: String,
    pub origin: [i32; 3],
    pub rot: u8,
    pub step: u32,
}

pub static BUILD: AttachmentType<Option<Site>> = AttachmentType {
    namespace: "projectfelix",
    name: "build",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |s| {
            let mut c = Compound::new();
            if let Some(s) = s {
                c.insert("plan".into(), Tag::String(s.plan.as_str().into()));
                c.insert("origin".into(), Tag::IntArray(s.origin.to_vec()));
                c.insert("rot".into(), Tag::Byte(s.rot as i8));
                c.insert("step".into(), Tag::Int(s.step as i32));
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            let (Some(Tag::String(p)), Some(Tag::IntArray(o))) = (c.get(&"plan".into()), c.get(&"origin".into())) else { return Ok(None) };
            let rot = match c.get(&"rot".into()) {
                Some(Tag::Byte(b)) => *b as u8,
                _ => 0,
            };
            let step = match c.get(&"step".into()) {
                Some(Tag::Int(i)) => (*i).max(0) as u32,
                _ => 0,
            };
            Ok(Some(Site { plan: p.to_string_lossy(), origin: [o.first().copied().unwrap_or(0), o.get(1).copied().unwrap_or(0), o.get(2).copied().unwrap_or(0)], rot, step }))
        },
    }),
    copy_on_death: false,
    sync: false,
};

static LOADED: LazyLock<Mutex<HashMap<String, Arc<Plan>>>> = LazyLock::new(Default::default);

#[derive(Default)]
struct Made {
    read: bool,
    plans: HashMap<String, Arc<Plan>>,
}

static MADE: LazyLock<Mutex<Made>> = LazyLock::new(Default::default);
const FOLDER: &str = "plans";
const PREFIX: &str = "felixplan:";

struct Listener;

impl ServerReloadListener for Listener {
    fn prepare(&self, cx: &ServerReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        for id in cx.list() {
            let Some(bytes) = cx.read(id) else { continue };
            match plan::parse(bytes) {
                Ok(p) => {
                    next.insert(id.to_string(), Arc::new(p));
                }
                Err(e) => eprintln!("[projectfelix] plan {id} not loaded: {e}"),
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
    let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:plans"), FileToIdConverter::json("felix_plans")) else {
        return;
    };
    if let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener)) {
        eprintln!("[projectfelix] plans listener not added: {err:?}");
    }
}

fn full_id(id: &str) -> String {
    let id = id.trim();
    if id.contains(':') { id.to_string() } else { format!("projectfelix:{id}") }
}

pub fn plan_named(id: &str) -> Option<Arc<Plan>> {
    MADE.lock().ok().and_then(|m| m.plans.get(id).cloned()).or_else(|| LOADED.lock().ok().and_then(|l| l.get(id).cloned()))
}

/// Reads the plans saved in the game, the first time the level ticks.
pub fn level_tick(host: &mut dyn ServerHost) {
    if !crate::saves::new_world(host, "plans") {
        return;
    }
    if let Ok(mut m) = MADE.lock() {
        *m = Default::default();
    }
    if let Ok(mut m) = MADE.lock() {
        m.read = true;
    }
    let files = host.world_data_files();
    let mut names = Vec::new();
    if let Ok(dir) = ConfigPath::new(FOLDER) {
        let mut after: Option<String> = None;
        while let Ok(page) = files.list(&dir, after.as_deref(), 256) {
            if page.is_empty() {
                break;
            }
            names.extend(page.iter().map(|e| e.name.clone()));
            after = page.last().map(|e| e.name.clone());
        }
    }
    for name in names.into_iter().filter(|n| n.ends_with(".json")) {
        let Ok(p) = ConfigPath::new(&format!("{FOLDER}/{name}")) else { continue };
        match files.read(&p).map_err(|e| format!("{e:?}")).and_then(|b| plan::parse(&b)) {
            Ok(plan) => {
                if let Ok(mut m) = MADE.lock() {
                    m.plans.insert(format!("{PREFIX}{}", name.trim_end_matches(".json")), Arc::new(plan));
                }
            }
            Err(e) => eprintln!("[projectfelix] plan {name} not loaded: {e}"),
        }
    }
}

/// Where a plan's cells are in the world, and what each is to be.
fn placed(p: &Plan, site: &Site) -> Vec<([i32; 3], Option<String>)> {
    p.cells()
        .into_iter()
        .map(|c| {
            let t = plan::turn(c.at, site.rot);
            ([site.origin[0] + t[0], site.origin[1] + t[1], site.origin[2] + t[2]], c.state.map(|s| plan::turn_state(&s, site.rot)))
        })
        .collect()
}

fn block_at(host: &dyn mod_api::Host, level: LevelRef, p: [i32; 3]) -> String {
    level.block(host, p).map(|k| k.id(host)).unwrap_or_default()
}

/// Whether the block at `p` already is what the plan wants there.
fn done_at(host: &dyn mod_api::Host, level: LevelRef, p: [i32; 3], want: &Option<String>) -> bool {
    let id = block_at(host, level, p);
    match want {
        None => id == "minecraft:air" || id.is_empty(),
        Some(s) => id == plan::block_id(s),
    }
}

/// What the cells from `from` on still need, by item.
fn needs(cells: &[([i32; 3], Option<String>)], from: usize) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for (_, s) in cells.iter().skip(from) {
        if let Some(item) = s.as_deref().and_then(plan::item_for) {
            *out.entry(item).or_insert(0) += 1;
        }
    }
    out
}

fn short(item: &str) -> String {
    item.trim_start_matches("minecraft:").replace('_', " ")
}

#[derive(Default)]
struct Pace {
    next: i64,
    said: i64,
}

static PACE: LazyLock<Mutex<HashMap<u32, Pace>>> = LazyLock::new(Default::default);

/// A builder's work this step (the job's tick calls it).
pub fn work(host: &mut dyn ServerHost, me: EntityRef) {
    let id = me.id();
    let Some(level) = me.level(&*host) else { return };
    let (Some(now), Some(pos)) = (level.game_time(&*host), me.position(&*host)) else { return };
    if PACE.lock().ok().and_then(|p| p.get(&id).map(|x| x.next)).is_some_and(|n| now < n) {
        return;
    }
    let Some(mut site) = BUILD.get(host, Holder::Entity(id)).ok().flatten() else { return };
    let Some(p) = plan_named(&site.plan) else {
        let _ = BUILD.set(host, Holder::Entity(id), None);
        return;
    };
    let cells = placed(&p, &site);
    // Skip what is right already.
    let mut i = site.step as usize;
    while i < cells.len() && done_at(&*host, level, cells[i].0, &cells[i].1) {
        i += 1;
    }
    let say_every = |host: &mut dyn ServerHost, line: &str| {
        let due = PACE.lock().ok().is_none_or(|p| p.get(&id).is_none_or(|x| now - x.said >= 2400 || now < x.said));
        if due {
            crate::heart::say_aloud(host, me, line);
            if let Ok(mut pc) = PACE.lock() {
                pc.entry(id).or_default().said = now;
            }
        }
    };
    if i >= cells.len() {
        let _ = BUILD.set(host, Holder::Entity(id), None);
        crate::heart::say_aloud(host, me, &format!("Done! The {} is finished.", p.name));
        companion::set_live(host, me, pos, LiveDo::Stay);
        return;
    }
    let (at, want) = cells[i].clone();
    let centre = [f64::from(at[0]) + 0.5, f64::from(at[1]), f64::from(at[2]) + 0.5];
    let d = ((pos[0] - centre[0]).powi(2) + (pos[1] - centre[1]).powi(2) + (pos[2] - centre[2]).powi(2)).sqrt();
    if d > 5.5 {
        // To the site's edge nearest the block: never inside what it builds.
        let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
        for (c, _) in &cells {
            for k in 0..3 {
                lo[k] = lo[k].min(c[k]);
                hi[k] = hi[k].max(c[k]);
            }
        }
        let mut sx = at[0].clamp(lo[0] - 1, hi[0] + 1);
        let mut sz = at[2].clamp(lo[2] - 1, hi[2] + 1);
        if (lo[0]..=hi[0]).contains(&sx) && (lo[2]..=hi[2]).contains(&sz) {
            let options = [(sx - lo[0] + 1, 0), (hi[0] + 1 - sx, 1), (sz - lo[2] + 1, 2), (hi[2] + 1 - sz, 3)];
            match options.iter().min_by_key(|o| o.0).map(|o| o.1) {
                Some(0) => sx = lo[0] - 1,
                Some(1) => sx = hi[0] + 1,
                Some(2) => sz = lo[2] - 1,
                _ => sz = hi[2] + 1,
            }
        }
        let stand = crate::villages::stand_at(&*host, level, [f64::from(sx) + 0.5, f64::from(site.origin[1]), f64::from(sz) + 0.5]);
        companion::set_live(host, me, stand, LiveDo::Work);
        if let Ok(mut pc) = PACE.lock() {
            pc.entry(id).or_default().next = now + 20;
        }
        return;
    }
    let chest = crate::jobs::chest_of(host, me).flatten();
    match &want {
        None => {
            let got = level.destroy_block(host, at, true, None, true).ok().flatten().unwrap_or_default();
            for s in got {
                let went = chest.map_or(0, |c| crate::jobs::put(host, c, &s));
                if went < s.count() {
                    let mut rest = s;
                    let n = rest.count() - went;
                    let _ = rest.set_count(n);
                    let _ = level.spawn_item(host, pos[0], pos[1] + 0.5, pos[2], rest, 10);
                }
            }
        }
        Some(state) => {
            if let Some(item) = plan::item_for(state) {
                let ok = chest.is_some_and(|c| crate::jobs::take(host, c, &item, 1));
                if !ok {
                    // All it is short of, to finish.
                    let mut short_of = Vec::new();
                    if let Some(c) = chest {
                        for (it, n) in needs(&cells, i) {
                            let have = crate::jobs::amount(host, c, &it).min(u64::from(n)) as u32;
                            if have < n {
                                short_of.push(format!("{} {}", n - have, short(&it)));
                            }
                        }
                    }
                    let line = if chest.is_none() { "I need a chest with my building things.".to_string() } else { format!("To go on I need {}.", short_of.join(", ")) };
                    say_every(host, &line);
                    if let Ok(mut pc) = PACE.lock() {
                        pc.entry(id).or_default().next = now + 100;
                    }
                    site.step = i as u32;
                    let _ = BUILD.set(host, Holder::Entity(id), Some(site));
                    return;
                }
            }
            crate::talk::queue(format!("execute in {} run setblock {} {} {} {state}", level.dimension_id(), at[0], at[1], at[2]));
            let _ = level.play_sound(host, "minecraft:block.wood.place", centre, SoundSource::Blocks, 0.6, 1.0);
        }
    }
    let _ = LivingRef::new(me).swing(host, Hand::Main, false);
    let _ = mod_api::MobRef::new(me).look_at(host, [centre[0], centre[1] + 0.5, centre[2]]);
    site.step = i as u32 + 1;
    let _ = BUILD.set(host, Holder::Entity(id), Some(site));
    if let Ok(mut pc) = PACE.lock() {
        pc.entry(id).or_default().next = now + 10;
    }
}

/// The properties a saved plan keeps of each block.
const KEPT: &[&str] = &["facing", "half", "hinge", "axis", "type", "part", "shape", "rotation", "face", "attachment"];

fn state_at(host: &dyn mod_api::Host, level: LevelRef, p: [i32; 3]) -> String {
    let id = block_at(host, level, p);
    let props: Vec<String> = KEPT.iter().filter_map(|k| level.block_property(host, p, k).map(|v| format!("{k}={v}"))).collect();
    if props.is_empty() || id == "minecraft:air" { id } else { format!("{id}[{}]", props.join(",")) }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix build <targets:entities> <plan:text>", 2));
    e.commands.push(Command::op("felix build status <targets:entities>", 2));
    e.commands.push(Command::op("felix build stop <targets:entities>", 2));
    e.commands.push(Command::op("felix plans", 2));
    e.commands.push(Command::op("felix plan save <name:word> <dx:integer> <dy:integer> <dz:integer>", 2));
}

fn int(e: &RunModCommandEvent<'_>, name: &str) -> Option<i32> {
    e.command.arguments.iter().find(|a| a.name == name).and_then(|a| match a.value {
        CommandArgumentValue::Integer(v) => Some(v),
        _ => None,
    })
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let player = match e.source {
        CommandSource::Player { player, .. } => Some(*player),
        _ => None,
    };
    let npcs = || -> Vec<EntityRef> { companion::targets(e).into_iter().collect() };
    let lines: Vec<String> = match lit.as_slice() {
        ["felix", "build"] => {
            let id = full_id(&companion::text_argument(e, "plan").unwrap_or_default());
            let (Some(p), Some(player)) = (plan_named(&id), player) else {
                let _ = host.commands().feedback(Text::literal(format!("No plan {id} (or not a player). /felix plans")));
                return Flow::Cancel;
            };
            let feet = player.entity().position(&*host).unwrap_or([0.0; 3]);
            let yaw = player.entity().rotation(&*host).map(|r| r[0]).unwrap_or(0.0);
            let rot = ((yaw / 90.0).round() as i32).rem_euclid(4) as u8;
            let [w, _, _] = p.size();
            let off = plan::turn([-(w / 2), -p.sink, 2], rot);
            let origin = [feet[0].floor() as i32 + off[0], feet[1].floor() as i32 + off[1], feet[2].floor() as i32 + off[2]];
            let targets: Vec<EntityRef> = npcs().into_iter().filter(|t| companion::is_npc(&*host, *t)).collect();
            for t in &targets {
                let _ = BUILD.set(host, Holder::Entity(t.id()), Some(Site { plan: id.clone(), origin, rot, step: 0 }));
            }
            let need = needs(&placed(&p, &Site { plan: id.clone(), origin, rot, step: 0 }), 0);
            let list: Vec<String> = need.iter().map(|(k, n)| format!("{n} {}", short(k))).collect();
            vec![format!("{} NPCs will build the {} here (they build at work, from their chest). It takes: {}", targets.len(), p.name, list.join(", "))]
        }
        ["felix", "build", "status"] => {
            let list: Vec<EntityRef> = npcs().into_iter().filter(|t| companion::is_npc(&*host, *t)).collect();
            list.into_iter()
            .map(|t| {
                let name = t.name(&*host).unwrap_or_else(|| "NPC".into());
                match BUILD.get(host, Holder::Entity(t.id())).ok().flatten() {
                    Some(site) => match plan_named(&site.plan) {
                        Some(p) => {
                            let cells = placed(&p, &site);
                            let left: Vec<String> = needs(&cells, site.step as usize).iter().map(|(k, n)| format!("{n} {}", short(k))).collect();
                            format!("{name}: the {} at {} {} {}, {}/{} done; still to place: {}", p.name, site.origin[0], site.origin[1], site.origin[2], site.step, cells.len(), if left.is_empty() { "nothing".into() } else { left.join(", ") })
                        }
                        None => format!("{name}: plan {} is gone", site.plan),
                    },
                    None => format!("{name} builds nothing."),
                }
            })
            .collect()
        }
        ["felix", "build", "stop"] => {
            let targets: Vec<EntityRef> = npcs().into_iter().filter(|t| companion::is_npc(&*host, *t)).collect();
            for t in &targets {
                let _ = BUILD.set(host, Holder::Entity(t.id()), None);
            }
            vec![format!("{} NPCs stop building", targets.len())]
        }
        ["felix", "plans"] => {
            let mut ids: Vec<String> = LOADED.lock().map(|l| l.keys().cloned().collect()).unwrap_or_default();
            ids.extend(MADE.lock().map(|m| m.plans.keys().cloned().collect::<Vec<_>>()).unwrap_or_default());
            ids.sort();
            vec![format!("Plans: {}", ids.join(", "))]
        }
        ["felix", "plan", "save"] => {
            let name = crate::profile::make_id(&companion::text_argument(e, "name").unwrap_or_default());
            let (Some(player), Some(dx), Some(dy), Some(dz)) = (player, int(e, "dx"), int(e, "dy"), int(e, "dz")) else { return Flow::Cancel };
            let (dx, dy, dz) = (dx.clamp(1, 32), dy.clamp(1, 32), dz.clamp(1, 32));
            let Some(level) = player.entity().level(&*host) else { return Flow::Cancel };
            let feet = player.entity().position(&*host).unwrap_or([0.0; 3]);
            let base = [feet[0].floor() as i32, feet[1].floor() as i32, feet[2].floor() as i32];
            let blocks: Vec<Vec<Vec<String>>> = (0..dy)
                .map(|y| (0..dz).map(|z| (0..dx).map(|x| state_at(&*host, level, [base[0] + x, base[1] + y, base[2] + z])).collect()).collect())
                .collect();
            match Plan::capture(&name, &blocks) {
                Ok(p) => {
                    let bytes = crate::serde_json::to_vec_pretty(&p.to_json()).unwrap_or_default();
                    let written = ConfigPath::new(&format!("{FOLDER}/{name}.json")).map_err(|e| format!("{e:?}")).and_then(|path| host.world_data_files().write(&path, &bytes).map_err(|e| format!("{e:?}")));
                    match written {
                        Ok(()) => {
                            let blocks = p.cells().iter().filter(|c| c.state.is_some()).count();
                            if let Ok(mut m) = MADE.lock() {
                                m.plans.insert(format!("{PREFIX}{name}"), Arc::new(p));
                            }
                            vec![format!("Kept as {PREFIX}{name}: {blocks} blocks, {dx}x{dy}x{dz} from your feet (east, up, south). /felix build <npc> {PREFIX}{name}")]
                        }
                        Err(e) => vec![format!("Not written: {e}")],
                    }
                }
                Err(err) => vec![format!("Not kept: {err}")],
            }
        }
        _ => return Flow::Continue,
    };
    for l in lines {
        let _ = host.commands().feedback(Text::literal(l));
    }
    Flow::Cancel
}
