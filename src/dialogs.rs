//! Dialogs made in the game (see `dialog_editor`): kept as `dialogs/<name>.json` in the world's
//! mod data, spoken as `felixdialog:<name>`. `/felix dialog edit <name>` opens the editor (a data
//! pack's dialog opens as a copy to save under a name of its own), `/felix dialogs` lists them.
use crate::dialog;
use crate::serde_json::{Value, json};
use mod_api::{
    Command, CommandSource, ConfigPath, Flow, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId, PlayerRef,
    RegisterCommandsEvent, RunModCommandEvent, ServerHost, Text,
};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};


/// The NPC nearest player `p`, within 10 blocks.
pub fn nearest_npc(host: &dyn mod_api::Host, p: mod_api::PlayerRef) -> Option<mod_api::EntityRef> {
    let at = p.entity().position(host)?;
    crate::companion::npcs()
        .into_iter()
        .filter_map(|id| host.entity(id))
        .filter_map(|e| Some(((e.position(host)?[0] - at[0]).powi(2) + (e.position(host)?[2] - at[2]).powi(2), e)))
        .filter(|(d, _)| *d <= 100.0)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, e)| e)
}

pub const FOLDER: &str = "dialogs";
pub const PREFIX: &str = "felixdialog:";

#[derive(Default)]
struct Made {
    read: bool,
    files: HashMap<String, Value>,
    editors: HashSet<u128>,
}

static MADE: LazyLock<Mutex<Made>> = LazyLock::new(Default::default);

fn keep(id: &str, file: Value) -> Result<(), String> {
    let d = dialog::parse(&crate::serde_json::to_vec(&file).unwrap_or_default())?;
    crate::talk::set_extra_dialog(id, d);
    if let Ok(mut m) = MADE.lock() {
        m.files.insert(id.to_string(), file);
    }
    Ok(())
}

/// Reads the dialogs made in the game, the first time the level ticks.
pub fn level_tick(host: &mut dyn ServerHost) {
    if !crate::saves::new_world(host, "dialogs") {
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
            eprintln!("[projectfelix] dialog {name} not loaded: {e}");
        }
    }
}

/// Every dialog made in the game: its name and file.
pub fn made_all() -> Vec<(String, Value)> {
    MADE.lock().map(|m| m.files.iter().map(|(id, v)| (id.trim_start_matches(PREFIX).to_string(), v.clone())).collect()).unwrap_or_default()
}

/// Keeps a dialog a village brought as `felixdialog:<name>`, unless this world has one.
pub fn keep_brought(host: &mut dyn ServerHost, name: &str, file: Value) -> Result<String, String> {
    let id = format!("{PREFIX}{name}");
    if MADE.lock().is_ok_and(|m| m.files.contains_key(&id)) {
        return Ok(id);
    }
    dialog::parse(&crate::serde_json::to_vec(&file).unwrap_or_default())?;
    let p = ConfigPath::new(&format!("{FOLDER}/{name}.json")).map_err(|e| format!("{e:?}"))?;
    host.world_data_files().write(&p, &crate::serde_json::to_vec_pretty(&file).unwrap_or_default()).map_err(|e| format!("{e:?}"))?;
    keep(&id, file)?;
    Ok(id)
}

/// Opens the dialog editor for `player` on dialog `id` (one not there yet: a small sample).
fn open_editor(host: &mut dyn ServerHost, player: PlayerRef, id: &str) {
    let made = MADE.lock().ok().and_then(|m| m.files.get(id).cloned());
    let file = made.clone().or_else(|| crate::talk::raw_dialog(id));
    let name = id.rsplit(':').next().unwrap_or(id).to_string();
    if let Ok(mut m) = MADE.lock() {
        m.editors.insert(player.uuid());
    }
    let body = json!({ "id": if made.is_some() { id } else { "" }, "name": name, "file": file.unwrap_or(Value::Null) });
    let _ = player.send_payload(host, &PayloadId(crate::dialog_editor::OPEN.into()), PayloadBytes(crate::serde_json::to_vec(&body).unwrap_or_default()));
}

/// The editor's Save: checked, written to `dialogs/<name>.json`, and spoken at once.
pub struct SaveDialog;

impl IPayloadHandler for SaveDialog {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        let tell = |host: &mut dyn ServerHost, line: String| {
            let _ = player.tell(host, Text::literal(line));
        };
        if !MADE.lock().is_ok_and(|m| m.editors.contains(&player.uuid())) {
            tell(host, "Open the editor with /felix dialog edit first".into());
            return Ok(());
        }
        let Ok(v) = crate::serde_json::from_slice::<Value>(&payload.0) else { return Ok(()) };
        let name = crate::profile::make_id(v.get("name").and_then(Value::as_str).unwrap_or("mine"));
        let Some(file) = v.get("file").cloned() else { return Ok(()) };
        let id = format!("{PREFIX}{name}");
        if let Err(e) = dialog::parse(&crate::serde_json::to_vec(&file).unwrap_or_default()) {
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
            match nearest_npc(&*host, player) {
                Some(npc) => {
                    let _ = crate::talk::DIALOG.set(host, mod_api::Holder::Entity(npc.id()), Some(id.clone()));
                    crate::talk::start_dialog(host, player, npc, &id);
                    return Ok(());
                }
                None => tell(host, "No NPC near to try it on.".into()),
            }
        }
        tell(host, format!("Kept as {id}. Give it to an NPC: /felix dialog <npc> {id}"));
        Ok(())
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix dialog edit <id:text>", 2));
    e.commands.push(Command::op("felix dialogs", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    match lit.as_slice() {
        ["felix", "dialog", "edit"] => {
            let CommandSource::Player { player, .. } = e.source else {
                let _ = host.commands().feedback(Text::literal("Only a player can edit a dialog."));
                return Flow::Cancel;
            };
            let player = *player;
            let id = crate::companion::text_argument(e, "id").unwrap_or_default().trim().to_string();
            let id = if id.contains(':') { id } else { format!("{PREFIX}{}", crate::profile::make_id(&id)) };
            open_editor(host, player, &id);
            Flow::Cancel
        }
        ["felix", "dialogs"] => {
            let mut made: Vec<String> = MADE.lock().map(|m| m.files.keys().cloned().collect()).unwrap_or_default();
            made.sort();
            let line = if made.is_empty() { "No dialogs made in the game yet: /felix dialog edit <name>".to_string() } else { format!("Made in the game: {}", made.join(", ")) };
            let _ = host.commands().feedback(Text::literal(line));
            Flow::Cancel
        }
        _ => Flow::Continue,
    }
}
