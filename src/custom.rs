//! NPCs made in the game, on the server: their files, making and saving them, sending their skins
//! and models to clients, summoning them, and moving them in and out of the game.
//!
//! Each lives in `horizon-mod-data/projectfelix/npcs/<id>.felixnpc` in the world's save (see
//! `saves`); `exports/` and `imports/` are in the game folder's `horizon-mod-data/projectfelix/`. `/felix npc export <id>` copies one to `exports/`, `/felix npc import`
//! takes every `.felixnpc` or `.zip` in `imports/`. A summoned one is drawn with the texture
//! `felixnpc:<id>` (its skin, sent to each client that asks) and speaks the dialog `felixnpc:<id>`.
use crate::companion::{LOOK, Look, NPC};
use crate::profile::Profile;
use crate::talk::DIALOG;
use mod_api::{
    Command, CommandArgumentValue, CommandSource, Compound, Flow, Holder, IPayloadContext, IPayloadHandler,
    PayloadBytes, PayloadId, PlayerRef, RegisterCommandsEvent, Registrar, RunModCommandEvent, ServerHost, Tag, Text,
    ConfigPath,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

/// Client to server: a `.felixnpc` to keep (first byte 1: and summon it).
pub const SAVE: &str = "projectfelix:npc_save";
/// Client to server: the id of an NPC whose skin and model the client lacks.
pub const ASK: &str = "projectfelix:npc_ask";
/// Server to client: an NPC's file, for its skin and model.
pub const FILE: &str = "projectfelix:npc_file";
/// Server to client: open the NPC maker.
pub const OPEN: &str = "projectfelix:npc_open";

const NPCS: &str = "npcs";
const EXPORTS: &str = "exports";
const IMPORTS: &str = "imports";
const EXT: &str = ".felixnpc";

#[derive(Default)]
struct Server {
    loaded: bool,
    profiles: HashMap<String, (Arc<Profile>, Arc<Vec<u8>>)>,
    /// Summoned NPCs to dress at the next tick (they join the level after the tick that made them).
    dress: Vec<(u32, String)>,
    /// Players the maker was opened for (by a command they may use): only they may save.
    makers: HashSet<u128>,
}

static SERVER: LazyLock<Mutex<Server>> = LazyLock::new(Default::default);

pub fn texture_of(id: &str) -> String {
    format!("felixnpc:{id}")
}

/// The dialog made in the game NPC file `felixnpc:<id>` speaks instead of its own, if one.
pub fn dialog_override(dialog: &str) -> Option<String> {
    let id = dialog.strip_prefix("felixnpc:")?;
    SERVER.lock().ok()?.profiles.get(id).map(|x| x.0.dialog_set.clone()).filter(|d| !d.is_empty())
}

/// The rules made in the game NPC file `felixnpc:<id>` follows instead of its own, if any.
pub fn rules_override(dialog: &str) -> Option<String> {
    let id = dialog.strip_prefix("felixnpc:")?;
    SERVER.lock().ok()?.profiles.get(id).map(|x| x.0.rules_set.clone()).filter(|d| !d.is_empty())
}

pub fn dialog_of(id: &str) -> String {
    format!("felixnpc:{id}")
}

fn inflate(d: &[u8]) -> Result<Vec<u8>, String> {
    crate::image::inflate(d)
}

/// Checks and keeps a profile from its file: its skin must be a PNG and its dialog must read.
fn admit(file: Vec<u8>) -> Result<Arc<Profile>, String> {
    let p = Profile::unpack(&file, &inflate)?;
    if !p.skin.is_empty() {
        let (w, h, _) = crate::image::decode_png(&p.skin)?;
        if w > 256 || h > 256 {
            return Err("the skin is over 256 x 256".into());
        }
    }
    if let Some(m) = &p.model_file {
        crate::geo::parse(m).map_err(|e| format!("model.geo.json: {e}"))?;
    }
    let dialog = crate::dialog::parse(p.dialog_json().as_bytes()).map_err(|e| format!("dialog: {e}"))?;
    let rules = match &p.rules {
        Some(r) => crate::rules::parse(r.as_bytes()).map_err(|e| format!("rules.json: {e}"))?,
        None => Vec::new(),
    };
    crate::talk::set_extra_dialog(&dialog_of(&p.id), dialog);
    crate::rulebook::set_extra(&dialog_of(&p.id), rules);
    let quests = match &p.quests {
        Some(q) => crate::serde_json::from_str::<crate::serde_json::Value>(q).map_err(|e| format!("quests.json: {e}")).and_then(|v| crate::quest::read_list(&v))?,
        None => Vec::new(),
    };
    crate::quests::set_extra(&dialog_of(&p.id), quests);
    let shop = match &p.shop {
        Some(s) => Some(crate::shop::parse(s.as_bytes()).map_err(|e| format!("shop.json: {e}"))?),
        None => None,
    };
    crate::shops::set_extra(&dialog_of(&p.id), shop);
    if let Some(c) = &p.character_file {
        crate::heart::adopt(&p.character, c);
    }
    let p = Arc::new(p);
    if let Ok(mut s) = SERVER.lock() {
        s.profiles.insert(p.id.clone(), (p.clone(), Arc::new(file)));
    }
    Ok(p)
}

fn path(folder: &str, name: &str) -> Option<ConfigPath> {
    ConfigPath::new(&format!("{folder}/{name}")).ok()
}

/// A folder's files: the NPCs kept with the world, the rest (exports, imports) in the game folder.
fn list(host: &dyn ServerHost, folder: &str) -> Vec<String> {
    let files = if folder == NPCS { host.world_data_files() } else { host.data_files() };
    let Ok(dir) = ConfigPath::new(folder) else { return Vec::new() };
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    while let Ok(page) = files.list(&dir, after.as_deref(), 256) {
        if page.is_empty() {
            break;
        }
        out.extend(page.iter().map(|e| e.name.clone()));
        after = page.last().map(|e| e.name.clone());
    }
    out
}

/// Reads every kept NPC (once, and again on `/felix npc reload`).
fn load_all(host: &dyn ServerHost) -> Vec<String> {
    let mut problems = Vec::new();
    if let Ok(mut s) = SERVER.lock() {
        s.profiles.clear();
    }
    for name in list(host, NPCS).into_iter().filter(|n| n.ends_with(EXT)) {
        let Some(p) = path(NPCS, &name) else { continue };
        match host.world_data_files().read(&p).map_err(|e| format!("{e:?}")).and_then(admit) {
            Ok(_) => {}
            Err(e) => problems.push(format!("{name}: {e}")),
        }
    }
    if let Ok(mut s) = SERVER.lock() {
        s.loaded = true;
    }
    problems
}

pub fn register(r: &mut dyn Registrar, file: Arc<dyn IPayloadHandler>, open: Arc<dyn IPayloadHandler>) {
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_server(PayloadId(SAVE.into()), Arc::new(OnSave));
    reg.play_to_server(PayloadId(ASK.into()), Arc::new(OnAsk));
    reg.play_to_client(PayloadId(FILE.into()), Some(file));
    reg.play_to_client(PayloadId(OPEN.into()), Some(open));
}

/// The level tick: reads the kept NPCs the first time, dresses NPCs summoned last tick.
pub fn level_tick(host: &mut dyn ServerHost) {
    if crate::saves::new_world(host, "npcs") {
        for p in load_all(&*host) {
            eprintln!("[projectfelix] NPC not loaded: {p}");
        }
    }
    let dress = SERVER.lock().map(|mut s| std::mem::take(&mut s.dress)).unwrap_or_default();
    let mut later = Vec::new();
    for (entity, id) in dress {
        let Some(p) = SERVER.lock().ok().and_then(|s| s.profiles.get(&id).map(|x| x.0.clone())) else { continue };
        if host.entity(entity).is_none() {
            later.push((entity, id));
            continue;
        }
        // No skin: the model's own texture.
        let texture = if p.skin.is_empty() { Look::of_model(&p.model_id()).texture } else { texture_of(&p.id) };
        let _ = LOOK.set(host, Holder::Entity(entity), Some(Look { model: p.model_id(), texture }));
        let _ = DIALOG.set(host, Holder::Entity(entity), Some(dialog_of(&p.id)));
        if !p.character.is_empty()
            && let Some(e) = host.entity(entity)
        {
            crate::heart::set_character(host, e, Some(&p.character));
        }
        if !p.routine.is_empty()
            && let Some(e) = host.entity(entity)
        {
            crate::routines::set_routine(host, e, Some(&p.routine));
        }
        if !p.voice.is_empty()
            && let Some(e) = host.entity(entity)
        {
            crate::voices::set_voice(host, e, &p.voice);
        }
        if !p.job.is_empty()
            && let Some(e) = host.entity(entity)
        {
            crate::jobs::set_job(host, e, &p.job);
        }
    }
    if let Ok(mut s) = SERVER.lock() {
        s.dress.extend(later.into_iter().take(64));
    }
}

/// NPC file `id`, as kept.
pub fn profile_file(id: &str) -> Option<Vec<u8>> {
    SERVER.lock().ok()?.profiles.get(id).map(|x| x.1.to_vec())
}

/// Takes in an NPC file (a village brought it): kept, unless one of that id is here already.
pub fn import_profile(host: &mut dyn ServerHost, bytes: &[u8]) -> Result<String, String> {
    let p = Profile::unpack(bytes, &inflate)?;
    if SERVER.lock().is_ok_and(|s| s.profiles.contains_key(&p.id)) {
        return Ok(p.id);
    }
    let p = admit(bytes.to_vec())?;
    let to = path(NPCS, &format!("{}{EXT}", p.id)).ok_or("bad id")?;
    host.world_data_files().write(&to, bytes).map_err(|e| format!("{e:?}"))?;
    Ok(p.id.clone())
}

/// Summons NPC `id` by `player`, theirs.
fn summon(host: &mut dyn ServerHost, player: PlayerRef, id: &str) -> Result<u32, String> {
    let p = SERVER.lock().ok().and_then(|s| s.profiles.get(id).map(|x| x.0.clone())).ok_or_else(|| format!("no NPC {id}"))?;
    let at = player.entity().position(&*host).ok_or("no position")?;
    let dim = player.dimension(&*host).map(|d| d.to_string()).unwrap_or_else(|| "minecraft:overworld".into());
    let mut nbt = Compound::new();
    nbt.insert("Owner".into(), Tag::IntArray(uuid_ints(player.uuid())));
    nbt.insert("PersistenceRequired".into(), Tag::Byte(1));
    nbt.insert("CustomName".into(), Tag::String(p.name.as_str().into()));
    let e = host.summon(&dim, NPC, at, nbt).ok_or("could not summon here")?;
    if let Ok(mut s) = SERVER.lock() {
        s.dress.push((e, p.id.clone()));
    }
    Ok(e)
}

fn uuid_ints(u: u128) -> Vec<i32> {
    vec![(u >> 96) as i32, (u >> 64) as i32, (u >> 32) as i32, u as i32]
}

fn tell(host: &mut dyn ServerHost, player: PlayerRef, line: String) {
    let _ = player.action_bar(host, Text::literal(line));
}

struct OnSave;

impl IPayloadHandler for OnSave {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        let Some((&spawn, file)) = payload.0.split_first() else { return Ok(()) };
        if !SERVER.lock().is_ok_and(|s| s.makers.contains(&player.uuid())) {
            tell(host, player, "Open the maker with /felix create first".into());
            return Ok(());
        }
        match admit(file.to_vec()) {
            Ok(mut p) => {
                // A character made here goes into the file, so the NPC takes it along.
                let mut file = file.to_vec();
                if p.character.starts_with("felixchar:")
                    && p.character_file.is_none()
                    && let Some(v) = crate::heart::made_json(&p.character)
                {
                    let mut q = (*p).clone();
                    q.character_file = Some(crate::serde_json::to_string_pretty(&v).unwrap_or_default());
                    let packed = q.pack(&crate::image::crc32);
                    if let Ok(again) = admit(packed.clone()) {
                        p = again;
                        file = packed;
                    }
                }
                let written = path(NPCS, &format!("{}{EXT}", p.id)).map(|f| host.world_data_files().write(&f, &file));
                match written {
                    Some(Ok(())) => {
                        tell(host, player, format!("{} is kept as npcs/{}{EXT}", p.name, p.id));
                        if spawn == 1 {
                            if let Err(e) = summon(host, player, &p.id) {
                                tell(host, player, format!("Kept, but not summoned: {e}"));
                            }
                        }
                    }
                    _ => tell(host, player, "The NPC could not be written".into()),
                }
            }
            Err(e) => tell(host, player, format!("Not kept: {e}")),
        }
        Ok(())
    }
}

struct OnAsk;

impl IPayloadHandler for OnAsk {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        let id = String::from_utf8_lossy(&payload.0).into_owned();
        let file = SERVER.lock().ok().and_then(|s| s.profiles.get(&id).map(|x| x.1.clone()));
        if let Some(file) = file {
            let _ = player.send_payload(host, &PayloadId(FILE.into()), PayloadBytes(file.to_vec()));
        }
        Ok(())
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix create", 2));
    e.commands.push(Command::op("felix npc spawn <id:word>", 2));
    e.commands.push(Command::op("felix npc edit <id:word>", 2));
    e.commands.push(Command::op("felix npc list", 2));
    e.commands.push(Command::op("felix npc export <id:word>", 2));
    e.commands.push(Command::op("felix npc import", 2));
    e.commands.push(Command::op("felix npc reload", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let player = match e.source {
        CommandSource::Player { player, .. } => Some(*player),
        _ => None,
    };
    let id = e.command.arguments.iter().find(|a| a.name == "id").and_then(|a| match &a.value {
        CommandArgumentValue::String(s) => Some(s.clone()),
        _ => None,
    });
    let say = |host: &mut dyn ServerHost, line: String| {
        let _ = host.commands().feedback(Text::literal(line));
    };
    match lit.as_slice() {
        ["felix", "create"] | ["felix", "npc", "edit"] => match player {
            Some(p) => {
                let file = match lit.len() {
                    2 => Some(Vec::new()),
                    _ => SERVER.lock().ok().and_then(|s| s.profiles.get(id.as_deref().unwrap_or("")).map(|x| x.1.to_vec())),
                };
                match file {
                    Some(file) => {
                        if let Ok(mut s) = SERVER.lock() {
                            s.makers.insert(p.uuid());
                        }
                        crate::heart::send_list(host, p);
                        let _ = p.send_payload(host, &PayloadId(OPEN.into()), PayloadBytes(file));
                    }
                    None => say(host, format!("No NPC {}", id.unwrap_or_default())),
                }
            }
            None => say(host, "Only a player can make an NPC.".into()),
        },
        ["felix", "npc", "spawn"] => match (player, id) {
            (Some(p), Some(id)) => match summon(host, p, &id) {
                Ok(e) => say(host, format!("{id} is here (entity {e}).")),
                Err(err) => say(host, err),
            },
            _ => say(host, "Only a player can summon an NPC.".into()),
        },
        ["felix", "npc", "list"] => {
            let mut ids: Vec<String> = SERVER.lock().map(|s| s.profiles.values().map(|(p, _)| format!("{} ({})", p.id, p.name)).collect()).unwrap_or_default();
            ids.sort();
            say(host, if ids.is_empty() { "No NPCs made yet.".into() } else { ids.join(", ") });
        }
        ["felix", "npc", "export"] => {
            let id = id.unwrap_or_default();
            let file = SERVER.lock().ok().and_then(|s| s.profiles.get(&id).map(|x| x.1.clone()));
            match (file, path(EXPORTS, &format!("{id}{EXT}"))) {
                (Some(file), Some(to)) => match host.data_files().write(&to, &file) {
                    Ok(()) => say(host, format!("Exported: horizon-mod-data/projectfelix/{EXPORTS}/{id}{EXT} (a zip: npc.json, skin.png, dialog.json, rules.json)")),
                    Err(err) => say(host, format!("Not exported: {err:?}")),
                },
                _ => say(host, format!("No NPC {id}")),
            }
        }
        ["felix", "npc", "import"] => {
            let mut done = Vec::new();
            for name in list(&*host, IMPORTS).into_iter().filter(|n| n.ends_with(EXT) || n.ends_with(".zip")) {
                let Some(from) = path(IMPORTS, &name) else { continue };
                let result = host.data_files().read(&from).map_err(|e| format!("{e:?}")).and_then(|bytes| {
                    let p = admit(bytes.clone())?;
                    let to = path(NPCS, &format!("{}{EXT}", p.id)).ok_or("bad id")?;
                    host.world_data_files().write(&to, &bytes).map_err(|e| format!("{e:?}"))?;
                    Ok(p.id.clone())
                });
                done.push(match result {
                    Ok(id) => format!("{name} → {id}"),
                    Err(err) => format!("{name}: {err}"),
                });
            }
            say(host, if done.is_empty() { format!("Nothing in horizon-mod-data/projectfelix/{IMPORTS}/") } else { done.join("; ") });
        }
        ["felix", "npc", "reload"] => {
            let problems = load_all(&*host);
            let n = SERVER.lock().map(|s| s.profiles.len()).unwrap_or(0);
            say(host, if problems.is_empty() { format!("{n} NPCs read.") } else { format!("{n} NPCs read; {}", problems.join("; ")) });
        }
        _ => return Flow::Continue,
    }
    Flow::Cancel
}
