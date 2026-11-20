//! Quests made in the game (see `quest_editor`): kept as `quests/<name>.json` in the world's mod
//! data, given as `felixquest:<name>`. `/felix quest edit <name>` opens the editor (a data pack's
//! quest opens as a copy), `/felix quest made` lists them.
use crate::quest::{self, Quest};
use crate::serde_json::{Value, json};
use mod_api::{
    Command, CommandSource, ConfigPath, Flow, Holder, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId, PlayerRef,
    RegisterCommandsEvent, RunModCommandEvent, ServerHost, Text,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

pub const FOLDER: &str = "quests";
pub const PREFIX: &str = "felixquest:";

#[derive(Default)]
struct Made {
    read: bool,
    quests: HashMap<String, (Arc<Quest>, Value)>,
    editors: HashSet<u128>,
}

static MADE: LazyLock<Mutex<Made>> = LazyLock::new(Default::default);

/// Player `uuid` left the game: the editor is not theirs to save from until they open it again.
pub fn left(uuid: u128) {
    if let Ok(mut m) = MADE.lock() {
        m.editors.remove(&uuid);
    }
}
/// Data pack quest files as they were, for the editor.
static RAW: LazyLock<Mutex<HashMap<String, Value>>> = LazyLock::new(Default::default);

pub fn made(id: &str) -> Option<Arc<Quest>> {
    MADE.lock().ok()?.quests.get(id).map(|x| x.0.clone())
}

/// Keeps a data pack quest file as it was (the quests' reload listener tells it).
pub fn keep_raw(id: &str, v: Value) {
    if let Ok(mut r) = RAW.lock() {
        r.insert(id.to_string(), v);
    }
}

fn keep(id: &str, file: Value) -> Result<(), String> {
    let q = quest::read(&file)?;
    if let Ok(mut m) = MADE.lock() {
        m.quests.insert(id.to_string(), (Arc::new(q), file));
    }
    Ok(())
}

/// Reads the quests made in the game, the first time the level ticks.
pub fn level_tick(host: &mut dyn ServerHost) {
    if !crate::saves::new_world(host, "quests") {
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
        let read = files.read(&p).map_err(|e| format!("{e:?}")).and_then(|b| crate::serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string()));
        if let Err(e) = read.and_then(|v| keep(&format!("{PREFIX}{}", name.trim_end_matches(".json")), v)) {
            eprintln!("[projectfelix] quest {name} not loaded: {e}");
        }
    }
}

fn open_editor(host: &mut dyn ServerHost, player: PlayerRef, id: &str) {
    let made = MADE.lock().ok().and_then(|m| m.quests.get(id).map(|x| x.1.clone()));
    let file = made.clone().or_else(|| RAW.lock().ok().and_then(|r| r.get(id).cloned()));
    let name = id.rsplit(':').next().unwrap_or(id).to_string();
    if let Ok(mut m) = MADE.lock() {
        m.editors.insert(player.uuid());
    }
    let body = json!({ "id": if made.is_some() { id } else { "" }, "name": name, "file": file.unwrap_or(Value::Null) });
    let _ = player.send_payload(host, &PayloadId(crate::quest_editor::OPEN.into()), PayloadBytes(crate::serde_json::to_vec(&body).unwrap_or_default()));
}

/// The editor's Save: checked, written, and given at once; trying, the NPC nearest the player
/// gives it, its board shown.
pub struct SaveQuest;

impl IPayloadHandler for SaveQuest {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        let tell = |host: &mut dyn ServerHost, line: String| {
            let _ = player.tell(host, Text::literal(line));
        };
        if !MADE.lock().is_ok_and(|m| m.editors.contains(&player.uuid())) {
            tell(host, "Open the editor with /felix quest edit first".into());
            return Ok(());
        }
        if let Err(why) = crate::saves::may_save(&*host, player, &payload.0) {
            tell(host, why.into());
            return Ok(());
        }
        let Ok(v) = crate::serde_json::from_slice::<Value>(&payload.0) else { return Ok(()) };
        let name = crate::profile::make_id(v.get("name").and_then(Value::as_str).unwrap_or("mine"));
        let Some(file) = v.get("file").cloned() else { return Ok(()) };
        let id = format!("{PREFIX}{name}");
        if let Err(e) = quest::read(&file) {
            tell(host, format!("Not kept: {e}"));
            return Ok(());
        }
        let written = ConfigPath::new(&format!("{FOLDER}/{name}.json"))
            .map_err(|e| format!("{e:?}"))
            .and_then(|p| host.world_data_files().write(&p, &crate::serde_json::to_vec_pretty(&file).unwrap_or_default()).map_err(|e| format!("{e:?}")));
        if let Err(e) = written {
            tell(host, format!("Not written: {e}"));
            return Ok(());
        }
        if let Err(e) = keep(&id, file) {
            tell(host, format!("Not kept: {e}"));
            return Ok(());
        }
        if v.get("try").and_then(Value::as_bool) == Some(true) {
            match crate::dialogs::nearest_npc(&*host, player) {
                Some(npc) => {
                    let mut ids: Vec<String> = crate::quests::GIVES.get(host, Holder::Entity(npc.id())).ok().flatten().unwrap_or_default().split_whitespace().map(str::to_string).collect();
                    if !ids.contains(&id) {
                        ids.push(id.clone());
                    }
                    let _ = crate::quests::GIVES.set(host, Holder::Entity(npc.id()), Some(ids.join(" ")));
                    crate::quests::board(host, player, npc, std::slice::from_ref(&id));
                    return Ok(());
                }
                None => tell(host, "No NPC near to try it on.".into()),
            }
        }
        tell(host, format!("Kept as {id}. Give it: /felix quest give <npc> {id}"));
        Ok(())
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix quest edit <id:text>", 2));
    e.commands.push(Command::op("felix quest made", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    match lit.as_slice() {
        ["felix", "quest", "edit"] => {
            let CommandSource::Player { player, .. } = e.source else { return Flow::Cancel };
            let player = *player;
            let id = crate::companion::text_argument(e, "id").unwrap_or_default().trim().to_string();
            let id = if id.contains(':') { id } else { format!("{PREFIX}{}", crate::profile::make_id(&id)) };
            open_editor(host, player, &id);
            Flow::Cancel
        }
        ["felix", "quest", "made"] => {
            let mut ids: Vec<String> = MADE.lock().map(|m| m.quests.keys().cloned().collect()).unwrap_or_default();
            ids.sort();
            let line = if ids.is_empty() { "No quests made in the game yet: /felix quest edit <name>".to_string() } else { format!("Made in the game: {}", ids.join(", ")) };
            let _ = host.commands().feedback(Text::literal(line));
            Flow::Cancel
        }
        _ => Flow::Continue,
    }
}
