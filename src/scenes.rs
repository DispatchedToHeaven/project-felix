//! Scenes on the server: the scenes of every data pack, starting one on an NPC, and ending it.
//!
//! A running scene is synced to Horizon clients as the NPC's `projectfelix:scene` attachment,
//! with the scene's file inside, so clients draw its props and play its animations from the
//! same moment. The NPC waits where it stands meanwhile and gets its orders back at the end.
use crate::companion::{self, Mode, ORDERS};
use crate::scene::{self, Scene};
use mod_api::{
    AddServerReloadListenersEvent, AttachmentCodec, AttachmentError, AttachmentType, Compound, EntityRef,
    FileToIdConverter, Holder, Identifier, PlayerRef, ReloadApply, ReloadResourceError, ServerHost, ServerReloadContext,
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
}

static RUNNING: LazyLock<Mutex<Vec<Running>>> = LazyLock::new(Default::default);

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
    let Some((scene, json)) = SCENES.lock().ok().and_then(|s| s.get(id).cloned()) else {
        return Err(format!("no scene {id}"));
    };
    if RUNNING.lock().is_ok_and(|r| r.iter().any(|x| x.npc == npc.id())) {
        return Err("that NPC is already in a scene".into());
    }
    let time = npc.level(&*host).and_then(|l| l.game_time(&*host)).ok_or("the NPC is not in a level")?;
    let at = npc.position(&*host).ok_or("the NPC has no position")?;
    let yaw = match player.and_then(|p| p.entity().position(&*host)) {
        Some(p) => scene::yaw_towards(at, p),
        None => npc.rotation(&*host).map(|r| r[0]).unwrap_or(0.0),
    };
    let restore = ORDERS.get(host, Holder::Entity(npc.id())).map(|o| o.mode).unwrap_or_default();
    companion::set_orders(host, npc, Mode::Wait);
    let playing = Playing { id: id.to_string(), start: time, yaw, json };
    SCENE.set(host, Holder::Entity(npc.id()), Some(playing)).map_err(|e| format!("{e:?}"))?;
    let ends = time + (scene.length * 20.0).ceil() as i64;
    if let Ok(mut r) = RUNNING.lock() {
        r.push(Running { npc: npc.id(), ends, restore, end: scene.end.clone(), player });
    }
    Ok(())
}

/// The level tick: ends the scenes whose time is up.
pub fn level_tick(host: &mut dyn ServerHost) {
    let running = RUNNING.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default();
    let mut keep = Vec::new();
    for s in running {
        let Some(npc) = host.entity(s.npc) else {
            // Not in this level (or gone): another level's tick may have it.
            keep.push(s);
            continue;
        };
        let now = npc.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(i64::MAX);
        if now < s.ends {
            keep.push(s);
            continue;
        }
        let _ = SCENE.set(host, Holder::Entity(s.npc), None);
        companion::set_orders(host, npc, s.restore);
        crate::talk::run_for(host, s.player, npc, &s.end);
    }
    if let Ok(mut r) = RUNNING.lock() {
        r.extend(keep);
    }
}

pub fn scene_exists(id: &str) -> bool {
    SCENES.lock().is_ok_and(|s| s.contains_key(id))
}
