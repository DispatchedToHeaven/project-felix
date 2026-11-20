//! What is made in the game is kept with the world it was made in: quests, dialogs, rules,
//! characters, scenes, plans, NPCs made in the maker and event spots go in the save's
//! `horizon-mod-data/projectfelix/` (`world_data_files`), so each world has its own. The game
//! folder's `horizon-mod-data/projectfelix/` (`data_files`) keeps what is shared between worlds:
//! `exports/`, `imports/`, and the maker's models, skins and animations.
//!
//! Before 0.42.0 everything was in the game folder. A world older than a day, opened the first
//! time with 0.42.0 or later, gets a copy of what was there (what it doesn't have already); a new
//! world starts empty.
use mod_api::{ConfigPath, PlayerRef, ServerHost};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

/// The folders kept with the world.
pub const FOLDERS: &[&str] = &["npcs", "characters", "scenes", "plans", "dialogs", "rules", "quests", "events"];
const MARK: &str = "carried_over.txt";

/// For each reader, the world it last read.
static SEEN: LazyLock<Mutex<HashMap<&'static str, Option<PathBuf>>>> = LazyLock::new(Default::default);
static CARRIED: LazyLock<Mutex<Option<Option<PathBuf>>>> = LazyLock::new(Default::default);

/// Whether reader `what` has yet to read this world (true once for each world opened).
pub fn new_world(host: &mut dyn ServerHost, what: &'static str) -> bool {
    let world = host.world_dir();
    let fresh = SEEN.lock().is_ok_and(|s| s.get(what) != Some(&world));
    if !fresh {
        return false;
    }
    carry_over(host, &world);
    if let Ok(mut s) = SEEN.lock() {
        s.insert(what, world);
    }
    true
}

/// Why `player` may not save what an editor sent (`text`: its JSON, or an NPC file's texts): they
/// must be an operator still, and only one of level 4 keeps console commands (they run as the
/// server).
pub fn may_save(host: &dyn ServerHost, player: PlayerRef, text: &[u8]) -> Result<(), &'static str> {
    let level = player.permission_level(host).unwrap_or(0);
    if level < 2 {
        return Err("Not kept: only an operator saves from the editors.");
    }
    let commands = text.windows(9).any(|w| w == b"\"command");
    if commands && level < 4 {
        return Err("Not kept: only a level 4 operator saves console commands.");
    }
    Ok(())
}

/// The server stopped: the next world is read anew, were it the same one.
pub fn clear() {
    if let Ok(mut s) = SEEN.lock() {
        s.clear();
    }
    if let Ok(mut c) = CARRIED.lock() {
        *c = None;
    }
}

fn names(files: &dyn mod_api::ConfigFiles, folder: &str) -> Vec<String> {
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

/// Copies what the game folder kept before 0.42.0 into an older world, once.
fn carry_over(host: &mut dyn ServerHost, world: &Option<PathBuf>) {
    if world.is_none() || CARRIED.lock().is_ok_and(|c| c.as_ref() == Some(world)) {
        return;
    }
    if let Ok(mut c) = CARRIED.lock() {
        *c = Some(world.clone());
    }
    let Ok(mark) = ConfigPath::new(MARK) else { return };
    if host.world_data_files().read(&mark).is_ok() {
        return;
    }
    let age = host.level("minecraft:overworld").and_then(|l| l.game_time(&*host)).unwrap_or(0);
    let old = host.data_files();
    let mut copied = 0;
    if age > 24000 {
        for folder in FOLDERS {
            let have = names(host.world_data_files(), folder);
            for name in names(old, folder) {
                if have.contains(&name) {
                    continue;
                }
                let Ok(p) = ConfigPath::new(&format!("{folder}/{name}")) else { continue };
                if let Ok(bytes) = old.read(&p)
                    && host.world_data_files().write(&p, &bytes).is_ok()
                {
                    copied += 1;
                }
            }
        }
    }
    let note = if age > 24000 { format!("{copied} files copied from the game folder's horizon-mod-data/projectfelix/ (kept there before 0.42.0).\n") } else { "A new world: nothing copied.\n".to_string() };
    if let Err(e) = host.world_data_files().write(&mark, note.as_bytes()) {
        eprintln!("[projectfelix] world data not written: {e:?}");
    } else if copied > 0 {
        eprintln!("[projectfelix] {copied} files made in the game copied into this world");
    }
}
