//! Scenes on the server: the scenes of every data pack, starting one on an NPC, and ending it.
//!
//! A running scene is synced to Horizon clients as the NPC's `projectfelix:scene` attachment,
//! with the scene's file inside, so clients draw its props and play its animations from the
//! same moment. The NPC waits where it stands meanwhile and gets its orders back at the end.
//! The NPCs nearest it take the scene's roles: they are put in their places, say and play what
//! the timeline has them do, and get their orders back too.
//!
//! Scenes made in the scene editor (`/felix scene edit <name>`) are kept on the server in
//! `horizon-mod-data/projectfelix/scenes/<name>.json`, as `felixscene:<name>`.
use crate::companion::{self, Mode, ORDERS};
use crate::scene::{self, Cue, Scene};
use crate::serde_json::{Value, json};
use crate::sync::{ACTION, Action as Play};
use mod_api::{
    AddServerReloadListenersEvent, AttachmentCodec, AttachmentError, AttachmentType, Compound, EntityRef,
    FileToIdConverter, Holder, Identifier, PlayerRef, ReloadApply, Command, ConfigPath, Flow, IPayloadContext,
    IPayloadHandler, PayloadBytes, PayloadId, RegisterCommandsEvent, RunModCommandEvent, Text, ReloadResourceError, ServerHost, ServerReloadContext,
    ServerReloadListener, Tag,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// A scene playing on an NPC, as clients get it.
#[derive(Clone, Debug, PartialEq)]
pub struct Playing {
    pub id: String,
    /// The level's game time it started at.
    pub start: i64,
    /// The way the NPC faces for it (degrees).
    pub yaw: f32,
    /// The scene file.
    pub json: String,
}

pub static SCENE: AttachmentType<Option<Playing>> = AttachmentType {
    namespace: "projectfelix",
    name: "scene",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |p| {
            let mut c = Compound::new();
            if let Some(p) = p {
                c.insert("id".into(), Tag::String(p.id.as_str().into()));
                c.insert("start".into(), Tag::Long(p.start));
                c.insert("yaw".into(), Tag::Float(p.yaw.to_bits()));
                c.insert("json".into(), Tag::String(p.json.as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            let s = |k: &str| match c.get(&k.into()) {
                Some(Tag::String(v)) => Some(v.to_string_lossy()),
                _ => None,
            };
            let (Some(id), Some(json)) = (s("id"), s("json")) else { return Ok(None) };
            let start = match c.get(&"start".into()) {
                Some(Tag::Long(t)) => *t,
                _ => return Err(AttachmentError::Decode),
            };
            let yaw = match c.get(&"yaw".into()) {
                Some(Tag::Float(b)) => f32::from_bits(*b),
                _ => 0.0,
            };
            Ok(Some(Playing { id, start, yaw, json }))
        },
    }),
    copy_on_death: false,
    sync: true,
};

/// Every loaded scene: its parsed form and its file as read.
static SCENES: LazyLock<Mutex<HashMap<String, (Arc<Scene>, String)>>> = LazyLock::new(Default::default);

struct Running {
    npc: u32,
    ends: i64,
    restore: Mode,
    end: Vec<crate::dialog::Action>,
    player: Option<PlayerRef>,
    start: i64,
    /// The other NPCs: their roles, and the orders to give back.
    cast: Vec<(String, u32, Mode)>,
    timeline: Vec<Cue>,
    fired: usize,
}

/// Scenes made in the game, and the players the editor was opened for.
#[derive(Default)]
struct Made {
    read: bool,
    scenes: HashMap<String, (Arc<Scene>, String)>,
    editors: std::collections::HashSet<u128>,
}

static MADE: LazyLock<Mutex<Made>> = LazyLock::new(Default::default);
const MADE_FOLDER: &str = "scenes";
const MADE_PREFIX: &str = "felixscene:";

fn scene_named(id: &str) -> Option<(Arc<Scene>, String)> {
    MADE.lock().ok().and_then(|m| m.scenes.get(id).cloned()).or_else(|| SCENES.lock().ok().and_then(|s| s.get(id).cloned()))
}

/// Whether NPC `id` plays in a scene (the lead or a role).
/// Props left from a scene no longer played go.
pub fn forget_stale(host: &mut dyn ServerHost, id: u32) {
    if !busy(id) && SCENE.get(host, Holder::Entity(id)).ok().flatten().is_some() {
        let _ = SCENE.set(host, Holder::Entity(id), None);
    }
}

pub fn busy(id: u32) -> bool {
    RUNNING.lock().is_ok_and(|r| r.iter().any(|x| x.npc == id || x.cast.iter().any(|c| c.1 == id)))
}

static RUNNING: LazyLock<Mutex<Vec<Running>>> = LazyLock::new(Default::default);

/// The server stopped: no scenes play.
pub fn clear() {
    if let Ok(mut r) = RUNNING.lock() {
        r.clear();
    }
}

/// NPC `id` left its level (unloaded, dead): the scene it led is over (its cast goes back to what
/// it did); one it only played in goes on without it.
pub fn gone(host: &mut dyn ServerHost, id: u32) {
    let ended: Vec<Running> = {
        let Ok(mut r) = RUNNING.lock() else { return };
        for s in r.iter_mut() {
            s.cast.retain(|c| c.1 != id);
        }
        let (ended, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut *r).into_iter().partition(|s| s.npc == id);
        *r = keep;
        ended
    };
    for s in ended {
        for (_, c, mode) in &s.cast {
            if let Some(e) = host.entity(*c) {
                companion::release(host, e, *mode);
            }
        }
    }
}

/// Player `uuid` left the game: the scene editor is not theirs to save from until they open it again.
pub fn left(uuid: u128) {
    if let Ok(mut m) = MADE.lock() {
        m.editors.remove(&uuid);
    }
}

struct Listener;

impl ServerReloadListener for Listener {
    fn prepare(&self, cx: &ServerReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        for id in cx.list() {
            let Some(bytes) = cx.read(id) else { continue };
            match scene::parse(bytes) {
                Ok(s) => {
                    next.insert(id.to_string(), (Arc::new(s), String::from_utf8_lossy(bytes).into_owned()));
                }
                Err(e) => eprintln!("[projectfelix] scene {id} not loaded: {e}"),
            }
        }
        Ok(<dyn ReloadApply>::new(move || {
            if let Ok(mut live) = SCENES.lock() {
                *live = next;
            }
        }))
    }
}

pub fn add_listener(e: &mut AddServerReloadListenersEvent<'_>) {
    let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:scenes"), FileToIdConverter::json("felix_scenes")) else {
        return;
    };
    if let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener)) {
        eprintln!("[projectfelix] scene listener not added: {err:?}");
    }
}

/// Starts scene `id` on `npc`, facing `player` if there is one.
pub fn start(host: &mut dyn ServerHost, npc: EntityRef, id: &str, player: Option<PlayerRef>) -> Result<(), String> {
    let Some((scene, json)) = scene_named(id) else {
        return Err(format!("no scene {id}"));
    };
    if busy(npc.id()) {
        return Err("that NPC is already in a scene".into());
    }
    if crate::games::busy(npc.id()) || crate::matches::busy(npc.id()) {
        return Err("that NPC is playing a game".into());
    }
    let time = npc.level(&*host).and_then(|l| l.game_time(&*host)).ok_or("the NPC is not in a level")?;
    let at = npc.position(&*host).ok_or("the NPC has no position")?;
    let yaw = match player.and_then(|p| p.entity().position(&*host)) {
        Some(p) => scene::yaw_towards(at, p),
        None => npc.rotation(&*host).map(|r| r[0]).unwrap_or(0.0),
    };
    // The NPCs free to play take the roles: not asleep, not out with another player; nearest first,
    // those of no one (or of the lead's owner, or of the player) before others'.
    let lead_owner = mod_api::LivingRef::new(npc).owner(&*host);
    let ours = |o: Option<u128>| o.is_none() || o == lead_owner || o == player.map(|p| p.uuid());
    let mut free: Vec<(bool, f64, EntityRef)> = Vec::new();
    for o in companion::npcs() {
        if o == npc.id() || busy(o) || crate::games::busy(o) || crate::matches::busy(o) || crate::routines::asleep(o) {
            continue;
        }
        let Some(e) = host.entity(o) else { continue };
        let owner = mod_api::LivingRef::new(e).owner(&*host);
        if !ours(owner) && companion::mode_of(o) == companion::Mode::Follow {
            continue;
        }
        let Some(d) = e.position(&*host).map(|p| (p[0] - at[0]).powi(2) + (p[2] - at[2]).powi(2)) else { continue };
        if d <= 24.0 * 24.0 {
            free.push((!ours(owner), d, e));
        }
    }
    free.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let free: Vec<(f64, EntityRef)> = free.into_iter().map(|(_, d, e)| (d, e)).collect();
    if free.len() < scene.cast.len() {
        return Err(format!("it needs {} more NPCs near, awake and free", scene.cast.len() - free.len()));
    }
    let mut cast = Vec::new();
    for (role, (_, e)) in scene.cast.iter().zip(free) {
        let mode = ORDERS.get(host, Holder::Entity(e.id())).map(|o| o.mode).unwrap_or_default();
        companion::hold(host, e, mode);
        let p = scene::place_of(at, yaw, role.offset);
        let p = npc.level(&*host).map_or(p, |l| crate::villages::stand_at(&*host, l, p));
        if let Some(u) = e.uuid(host) {
            crate::talk::queue(format!("tp {} {:.2} {:.2} {:.2} {:.1} 0", crate::talk::uuid_text(u), p[0], p[1], p[2], yaw + role.turn));
        }
        cast.push((role.name.clone(), e.id(), mode));
    }
    let restore = ORDERS.get(host, Holder::Entity(npc.id())).map(|o| o.mode).unwrap_or_default();
    companion::hold(host, npc, restore);
    let playing = Playing { id: id.to_string(), start: time, yaw, json };
    SCENE.set(host, Holder::Entity(npc.id()), Some(playing)).map_err(|e| format!("{e:?}"))?;
    let ends = time + (scene.length * 20.0).ceil() as i64;
    if let Ok(mut r) = RUNNING.lock() {
        r.push(Running { npc: npc.id(), ends, restore, end: scene.end.clone(), player, start: time, cast, timeline: scene.timeline.clone(), fired: 0 });
    }
    Ok(())
}

/// The level tick: ends the scenes whose time is up.
pub fn level_tick(host: &mut dyn ServerHost) {
    let running = RUNNING.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default();
    let mut keep = Vec::new();
    for mut s in running {
        let Some(npc) = host.entity(s.npc) else {
            // Not in this level (or gone): another level's tick may have it.
            keep.push(s);
            continue;
        };
        let now = npc.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(i64::MAX);
        // The timeline's cues that are due.
        while let Some(c) = s.timeline.get(s.fired).cloned() {
            if now < s.start + (c.at * 20.0) as i64 {
                break;
            }
            s.fired += 1;
            let who = if c.who == "lead" { Some(s.npc) } else { s.cast.iter().find(|x| x.0 == c.who).map(|x| x.1) };
            let Some(e) = who.and_then(|w| host.entity(w)) else { continue };
            if let Some(line) = &c.say {
                let player = s.player.and_then(|p| p.name(&*host)).unwrap_or_default();
                let name = e.name(&*host).unwrap_or_else(|| "NPC".into());
                crate::heart::say_aloud(host, e, &crate::dialog::fill(line, &player, &name));
            }
            if let Some(a) = &c.animation {
                let _ = ACTION.set(host, Holder::Entity(e.id()), Some(Play { animation: a.clone(), start: now }));
            }
        }
        if now < s.ends {
            keep.push(s);
            continue;
        }
        let _ = SCENE.set(host, Holder::Entity(s.npc), None);
        companion::release(host, npc, s.restore);
        for (_, id, mode) in &s.cast {
            if let Some(e) = host.entity(*id) {
                companion::release(host, e, *mode);
            }
        }
        crate::talk::run_for(host, s.player, npc, &s.end);
    }
    read_made(host);
    if let Ok(mut r) = RUNNING.lock() {
        r.extend(keep);
    }
}

/// Every scene made in the game: its name and file.
pub fn made_all() -> Vec<(String, String)> {
    MADE.lock().map(|m| m.scenes.iter().map(|(id, (_, t))| (id.trim_start_matches(MADE_PREFIX).to_string(), t.clone())).collect()).unwrap_or_default()
}

/// Keeps a scene a village brought as `felixscene:<name>`, unless this world has one.
pub fn keep_scene(host: &mut dyn ServerHost, name: &str, text: &str) -> Result<String, String> {
    let id = format!("{MADE_PREFIX}{name}");
    if MADE.lock().is_ok_and(|m| m.scenes.contains_key(&id)) {
        return Ok(id);
    }
    let s = scene::parse(text.as_bytes())?;
    let p = ConfigPath::new(&format!("{MADE_FOLDER}/{name}.json")).map_err(|e| format!("{e:?}"))?;
    host.world_data_files().write(&p, text.as_bytes()).map_err(|e| format!("{e:?}"))?;
    if let Ok(mut m) = MADE.lock() {
        m.scenes.insert(id.clone(), (Arc::new(s), text.to_string()));
    }
    Ok(id)
}

pub fn scene_exists(id: &str) -> bool {
    scene_named(id).is_some()
}

/// Reads the scenes made in the game, the first time.
fn read_made(host: &mut dyn ServerHost) {
    if !crate::saves::new_world(host, "scenes") {
        return;
    }
    if let Ok(mut m) = MADE.lock() {
        *m = Default::default();
    }
    let files = host.world_data_files();
    let mut names = Vec::new();
    if let Ok(dir) = ConfigPath::new(MADE_FOLDER) {
        let mut after: Option<String> = None;
        while let Ok(page) = files.list(&dir, after.as_deref(), 256) {
            if page.is_empty() {
                break;
            }
            names.extend(page.iter().map(|e| e.name.clone()));
            after = page.last().map(|e| e.name.clone());
        }
    }
    let mut made = HashMap::new();
    for name in names.into_iter().filter(|n| n.ends_with(".json")) {
        let Ok(p) = ConfigPath::new(&format!("{MADE_FOLDER}/{name}")) else { continue };
        let Ok(bytes) = files.read(&p) else { continue };
        match scene::parse(&bytes) {
            Ok(s) => {
                made.insert(format!("{MADE_PREFIX}{}", name.trim_end_matches(".json")), (Arc::new(s), String::from_utf8_lossy(&bytes).into_owned()));
            }
            Err(e) => eprintln!("[projectfelix] scene {name} not loaded: {e}"),
        }
    }
    if let Ok(mut m) = MADE.lock() {
        m.read = true;
        m.scenes.extend(made);
    }
}

// ---- the scene editor -------------------------------------------------------------------------------------

/// Server to client: open the scene editor (JSON: id, name, file).
pub const EDIT_OPEN: &str = "projectfelix:scene_open";
/// Client to server: a scene to keep, and maybe play (JSON: name, file, play).
pub const EDIT_SAVE: &str = "projectfelix:scene_save";

pub fn register(r: &mut dyn mod_api::Registrar, open: Arc<dyn IPayloadHandler>) {
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_client(PayloadId(EDIT_OPEN.into()), Some(open));
    reg.play_to_server(PayloadId(EDIT_SAVE.into()), Arc::new(SaveScene));
}

fn open_editor(host: &mut dyn ServerHost, player: PlayerRef, id: &str) {
    let found = scene_named(id).and_then(|(_, json)| crate::serde_json::from_str::<Value>(&json).ok());
    let name = id.rsplit(':').next().unwrap_or(id).to_string();
    if let Ok(mut m) = MADE.lock() {
        m.editors.insert(player.uuid());
    }
    let body = json!({ "id": if found.is_some() { id } else { "" }, "name": name, "file": found });
    let _ = player.send_payload(host, &PayloadId(EDIT_OPEN.into()), PayloadBytes(crate::serde_json::to_vec(&body).unwrap_or_default()));
}

struct SaveScene;

impl IPayloadHandler for SaveScene {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        let tell = |host: &mut dyn ServerHost, line: String| {
            let _ = player.tell(host, Text::literal(line));
        };
        if !MADE.lock().is_ok_and(|m| m.editors.contains(&player.uuid())) {
            tell(host, "Open the editor with /felix scene edit first".into());
            return Ok(());
        }
        if let Err(why) = crate::saves::may_save(&*host, player, &payload.0) {
            tell(host, why.into());
            return Ok(());
        }
        let Ok(v) = crate::serde_json::from_slice::<Value>(&payload.0) else { return Ok(()) };
        let name = crate::profile::make_id(v.get("name").and_then(Value::as_str).unwrap_or("mine"));
        let Some(file) = v.get("file") else { return Ok(()) };
        let text = crate::serde_json::to_string_pretty(file).unwrap_or_default();
        let s = match scene::parse(text.as_bytes()) {
            Ok(s) => s,
            Err(e) => {
                tell(host, format!("Not kept: {e}"));
                return Ok(());
            }
        };
        let written = ConfigPath::new(&format!("{MADE_FOLDER}/{name}.json")).map_err(|e| format!("{e:?}")).and_then(|p| host.world_data_files().write(&p, text.as_bytes()).map_err(|e| format!("{e:?}")));
        if let Err(e) = written {
            tell(host, format!("Not written: {e}"));
            return Ok(());
        }
        let id = format!("{MADE_PREFIX}{name}");
        if let Ok(mut m) = MADE.lock() {
            m.scenes.insert(id.clone(), (Arc::new(s), text));
        }
        if v.get("play").and_then(Value::as_bool).unwrap_or(false) {
            // The NPC nearest the player leads it.
            let at = player.entity().position(&*host).unwrap_or([0.0; 3]);
            let lead = companion::npcs()
                .into_iter()
                .filter(|o| !busy(*o))
                .filter_map(|o| host.entity(o))
                .filter_map(|e| Some(((e.position(&*host)?[0] - at[0]).powi(2) + (e.position(&*host)?[2] - at[2]).powi(2), e)))
                .filter(|(d, _)| *d <= 16.0 * 16.0)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|x| x.1);
            match lead {
                Some(npc) => {
                    if let Err(e) = start(host, npc, &id, Some(player)) {
                        tell(host, format!("Kept as {id}, but not played: {e}"));
                    }
                }
                None => tell(host, format!("Kept as {id}: no NPC near to play it")),
            }
        } else {
            tell(host, format!("Kept as {id}: /felix scene <npc> {id}, or a dialog's scene action"));
        }
        Ok(())
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix scene edit <id:text>", 2));
    e.commands.push(Command::op("felix scenes", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    match lit.as_slice() {
        ["felix", "scene", "edit"] => {
            let mod_api::CommandSource::Player { player, .. } = e.source else { return Flow::Cancel };
            let player = *player;
            let id = companion::text_argument(e, "id").unwrap_or_default().trim().to_string();
            let id = if id.contains(':') { id } else { format!("{MADE_PREFIX}{id}") };
            open_editor(host, player, &id);
        }
        ["felix", "scenes"] => {
            let mut ids: Vec<String> = SCENES.lock().map(|s| s.keys().cloned().collect()).unwrap_or_default();
            ids.extend(MADE.lock().map(|m| m.scenes.keys().cloned().collect::<Vec<_>>()).unwrap_or_default());
            ids.sort();
            let _ = host.commands().feedback(Text::literal(format!("{} scenes: {}", ids.len(), ids.join(", "))));
        }
        _ => return Flow::Continue,
    }
    Flow::Cancel
}
