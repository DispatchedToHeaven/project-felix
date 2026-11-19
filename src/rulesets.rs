//! Rules made in the game (see `rules_editor`): kept as `rules/<name>.json` in the world's mod
//! data, followed as `felixrules:<name>`. `/felix rules edit <name>` opens the editor (a data
//! pack's rules open as a copy to save under a name of their own), `/felix rulesets` lists them.
use crate::serde_json::{Value, json};
use mod_api::{
    Command, CommandSource, ConfigPath, Flow, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId, PlayerRef,
    RegisterCommandsEvent, RunModCommandEvent, ServerHost, Text,
};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

pub const FOLDER: &str = "rules";
pub const PREFIX: &str = "felixrules:";

#[derive(Default)]
struct Made {
    read: bool,
    files: HashMap<String, Value>,
    editors: HashSet<u128>,
}

static MADE: LazyLock<Mutex<Made>> = LazyLock::new(Default::default);

fn keep(id: &str, file: Value) -> Result<(), String> {
    let r = crate::rules::read(&file)?;
    crate::rulebook::set_extra(id, r);
    if let Ok(mut m) = MADE.lock() {
        m.files.insert(id.to_string(), file);
    }
    Ok(())
}

/// Reads the rules made in the game, the first time the level ticks.
pub fn level_tick(host: &mut dyn ServerHost) {
    if MADE.lock().is_ok_and(|m| m.read) {
        return;
    }
    if let Ok(mut m) = MADE.lock() {
        m.read = true;
    }
    let files = host.data_files();
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
            eprintln!("[projectfelix] rules {name} not loaded: {e}");
        }
    }
}

/// Every set of rules made in the game: its name and file.
pub fn made_all() -> Vec<(String, Value)> {
    MADE.lock().map(|m| m.files.iter().map(|(id, v)| (id.trim_start_matches(PREFIX).to_string(), v.clone())).collect()).unwrap_or_default()
}

/// Keeps rules a village brought as `felixrules:<name>`, unless this world has them.
pub fn keep_brought(host: &mut dyn ServerHost, name: &str, file: Value) -> Result<String, String> {
    let id = format!("{PREFIX}{name}");
    if MADE.lock().is_ok_and(|m| m.files.contains_key(&id)) {
        return Ok(id);
    }
    crate::rules::read(&file)?;
    let p = ConfigPath::new(&format!("{FOLDER}/{name}.json")).map_err(|e| format!("{e:?}"))?;
    host.data_files().write(&p, &crate::serde_json::to_vec_pretty(&file).unwrap_or_default()).map_err(|e| format!("{e:?}"))?;
    keep(&id, file)?;
    Ok(id)
}

/// Opens the rules editor for `player` on rules `id` (none there yet: one rule to start from).
fn open_editor(host: &mut dyn ServerHost, player: PlayerRef, id: &str) {
    let made = MADE.lock().ok().and_then(|m| m.files.get(id).cloned());
    let file = made.clone().or_else(|| crate::rulebook::raw_rules(id));
    let name = id.rsplit(':').next().unwrap_or(id).to_string();
    if let Ok(mut m) = MADE.lock() {
        m.editors.insert(player.uuid());
    }
    let body = json!({ "id": if made.is_some() { id } else { "" }, "name": name, "file": file.unwrap_or(Value::Null) });
    let _ = player.send_payload(host, &PayloadId(crate::rules_editor::OPEN.into()), PayloadBytes(crate::serde_json::to_vec(&body).unwrap_or_default()));
}

/// The editor's Save: checked, written to `rules/<name>.json`, and followed at once.
pub struct SaveRules;

impl IPayloadHandler for SaveRules {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        let tell = |host: &mut dyn ServerHost, line: String| {
            let _ = player.tell(host, Text::literal(line));
        };
        if !MADE.lock().is_ok_and(|m| m.editors.contains(&player.uuid())) {
            tell(host, "Open the editor with /felix rules edit first".into());
            return Ok(());
        }
        let Ok(v) = crate::serde_json::from_slice::<Value>(&payload.0) else { return Ok(()) };
        let name = crate::profile::make_id(v.get("name").and_then(Value::as_str).unwrap_or("mine"));
        let Some(file) = v.get("file").cloned() else { return Ok(()) };
        let id = format!("{PREFIX}{name}");
        if let Err(e) = crate::rules::read(&file) {
            tell(host, format!("Not kept: {e}"));
            return Ok(());
        }
        let written = ConfigPath::new(&format!("{FOLDER}/{name}.json"))
            .map_err(|e| format!("{e:?}"))
            .and_then(|p| host.data_files().write(&p, &crate::serde_json::to_vec_pretty(&file).unwrap_or_default()).map_err(|e| format!("{e:?}")));
        if let Err(e) = written {
            tell(host, format!("Not written: {e}"));
            return Ok(());
        }
        if let Err(e) = keep(&id, file) {
            tell(host, format!("Not kept: {e}"));
            return Ok(());
        }
        crate::rulebook::forget_minds();
        tell(host, format!("Kept as {id}. Give them to an NPC: /felix rules <npc> {id}"));
        Ok(())
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix rules edit <id:text>", 2));
    e.commands.push(Command::op("felix rulesets", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    match lit.as_slice() {
        ["felix", "rules", "edit"] => {
            let CommandSource::Player { player, .. } = e.source else {
                let _ = host.commands().feedback(Text::literal("Only a player can edit rules."));
                return Flow::Cancel;
            };
            let player = *player;
            let id = crate::companion::text_argument(e, "id").unwrap_or_default().trim().to_string();
            let id = if id.contains(':') { id } else { format!("{PREFIX}{}", crate::profile::make_id(&id)) };
            open_editor(host, player, &id);
            Flow::Cancel
        }
        ["felix", "rulesets"] => {
            let mut made: Vec<String> = MADE.lock().map(|m| m.files.keys().cloned().collect()).unwrap_or_default();
            made.sort();
            let line = if made.is_empty() { "No rules made in the game yet: /felix rules edit <name>".to_string() } else { format!("Made in the game: {}", made.join(", ")) };
            let _ = host.commands().feedback(Text::literal(line));
            Flow::Cancel
        }
        _ => Flow::Continue,
    }
}
