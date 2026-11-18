//! Talking to NPCs, on the server: the dialogs of every data pack, each player's conversation,
//! the conditions checked and the actions run. The client only shows what it is sent.
//!
//! Client to server: `talk` (an NPC's id), `choose` (its id and the shown option's number), `bye`.
//! Server to client: `view` (JSON: the NPC's id, name, lines and options) and `close`.
use crate::companion::{self, Mode, ORDERS};
use crate::dialog::{self, Action, Dialog, Facts};
use crate::sync::{ACTION, Action as Play};
use mod_api::{
    AddServerReloadListenersEvent, AttachmentCodec, AttachmentError, AttachmentType, Compound, EntityRef,
    FileToIdConverter, Holder, IPayloadContext, IPayloadHandler, Identifier, LivingRef, PayloadBytes, PayloadId,
    PlayerRef, Registrar, ReloadApply, ReloadResourceError, ServerHost, ServerReloadContext, ServerReloadListener, Tag,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

pub const TALK: &str = "projectfelix:talk";
pub const CHOOSE: &str = "projectfelix:choose";
pub const BYE: &str = "projectfelix:bye";
pub const VIEW: &str = "projectfelix:view";
pub const CLOSE: &str = "projectfelix:close";

/// The dialog an NPC has when none was given to it.
pub const DEFAULT_DIALOG: &str = "projectfelix:companion";
/// How far a player may stand from the NPC they talk to.
const REACH: f64 = 8.0;

/// The dialog an NPC speaks (`/felix dialog`); none means [`DEFAULT_DIALOG`].
pub static DIALOG: AttachmentType<Option<String>> = AttachmentType {
    namespace: "projectfelix",
    name: "dialog",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |d| {
            let mut c = Compound::new();
            if let Some(d) = d {
                c.insert("id".into(), Tag::String(d.as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| match c.get(&"id".into()) {
            Some(Tag::String(s)) => Ok(Some(s.to_string_lossy())),
            None => Ok(None),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

static DIALOGS: LazyLock<Mutex<HashMap<String, Arc<Dialog>>>> = LazyLock::new(Default::default);
/// Dialogs of NPCs made in the game (`felixnpc:<id>`), kept across data reloads.
static EXTRA: LazyLock<Mutex<HashMap<String, Arc<Dialog>>>> = LazyLock::new(Default::default);

pub fn set_extra_dialog(name: &str, d: Dialog) {
    if let Ok(mut e) = EXTRA.lock() {
        e.insert(name.to_string(), Arc::new(d));
    }
}

/// A player's conversation: with whom, which dialog, where in it, and which options they see.
struct Session {
    npc: u32,
    dialog: Arc<Dialog>,
    node: String,
    shown: Vec<usize>,
}

static SESSIONS: LazyLock<Mutex<HashMap<u128, Session>>> = LazyLock::new(Default::default);
/// Commands from dialog actions, run at the next level tick (not inside a player's packet).
static PENDING: LazyLock<Mutex<Vec<String>>> = LazyLock::new(Default::default);

struct Listener;

impl ServerReloadListener for Listener {
    fn prepare(&self, cx: &ServerReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        for id in cx.list() {
            let Some(bytes) = cx.read(id) else { continue };
            match dialog::parse(bytes) {
                Ok(d) => {
                    next.insert(id.to_string(), Arc::new(d));
                }
                Err(e) => eprintln!("[projectfelix] dialog {id} not loaded: {e}"),
            }
        }
        Ok(<dyn ReloadApply>::new(move || {
            if let Ok(mut live) = DIALOGS.lock() {
                *live = next;
            }
        }))
    }
}

pub fn add_listener(e: &mut AddServerReloadListenersEvent<'_>) {
    let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:dialogs"), FileToIdConverter::json("felix_dialogs")) else {
        return;
    };
    if let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener)) {
        eprintln!("[projectfelix] dialog listener not added: {err:?}");
    }
}

pub fn dialog_exists(id: &str) -> bool {
    DIALOGS.lock().is_ok_and(|d| d.contains_key(id)) || EXTRA.lock().is_ok_and(|d| d.contains_key(id))
}

struct OnServer(fn(&mut dyn ServerHost, PlayerRef, &[u8]));

impl IPayloadHandler for OnServer {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        (self.0)(host, player, &payload.0);
        Ok(())
    }
}

/// The payloads: the server's handlers here, the client's (`view`, `close`) from the menu.
pub fn register(r: &mut dyn Registrar, view: Arc<dyn IPayloadHandler>, close: Arc<dyn IPayloadHandler>) {
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_server(PayloadId(TALK.into()), Arc::new(OnServer(talk)));
    reg.play_to_server(PayloadId(CHOOSE.into()), Arc::new(OnServer(choose)));
    reg.play_to_server(PayloadId(BYE.into()), Arc::new(OnServer(|_, p, _| end(p))));
    reg.play_to_client(PayloadId(VIEW.into()), Some(view));
    reg.play_to_client(PayloadId(CLOSE.into()), Some(close));
}

/// The level tick: runs the commands dialog actions queued.
pub fn level_tick(host: &mut dyn ServerHost) {
    let lines = PENDING.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
    for line in lines {
        if !host.commands().run_command(&line) {
            eprintln!("[projectfelix] dialog command failed: {line}");
        }
    }
}

/// Runs `line` as the server console at the next level tick.
pub fn queue(line: String) {
    if let Ok(mut p) = PENDING.lock() {
        p.push(line);
    }
}

fn end(player: PlayerRef) {
    if let Ok(mut s) = SESSIONS.lock() {
        s.remove(&player.uuid());
    }
}

fn read_id(b: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(0..4)?.try_into().ok()?))
}

struct PlayerFacts {
    game: &'static str,
    in_game: bool,
    tags: Vec<String>,
    items: HashMap<String, u32>,
    owner: bool,
    mode: &'static str,
    mood: String,
    relation: i32,
}

impl Facts for PlayerFacts {
    fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
    fn item_count(&self, item: &str) -> u32 {
        self.items.get(item).copied().unwrap_or(0)
    }
    fn is_owner(&self) -> bool {
        self.owner
    }
    fn mode(&self) -> &str {
        self.mode
    }
    fn game(&self) -> &str {
        self.game
    }
    fn in_game(&self) -> bool {
        self.in_game
    }
    fn mood(&self) -> &str {
        &self.mood
    }
    fn relation(&self) -> i32 {
        self.relation
    }
}

fn facts(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef) -> PlayerFacts {
    let me = player.entity();
    let tags = me.tags(&*host).unwrap_or_default();
    let mut items = HashMap::new();
    for stack in player.inventory(&*host).unwrap_or_default() {
        *items.entry(stack.item().id(&*host)).or_insert(0) += stack.count();
    }
    let owner = LivingRef::new(npc).owner(&*host) == Some(player.uuid());
    let mode = match ORDERS.get(host, Holder::Entity(npc.id())).map(|o| o.mode) {
        Ok(Mode::Wait) => "wait",
        Ok(Mode::Guard) => "guard",
        _ => "follow",
    };
    let (game, in_game) = crate::games::state_for(npc.id(), player);
    let mood = crate::heart::mood_of(npc.id());
    let relation = crate::heart::relation_of(npc.id(), player.uuid());
    PlayerFacts { game, in_game, tags, items, owner, mode, mood, relation }
}

/// A UUID written with dashes, as commands take it.
pub fn uuid_text(u: u128) -> String {
    let h = format!("{u:032x}");
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// The NPC `id`, if the player is near enough to talk to it.
fn npc_near(host: &mut dyn ServerHost, player: PlayerRef, id: u32) -> Option<EntityRef> {
    let npc = host.entity(id)?;
    if !companion::is_npc(&*host, npc) {
        return None;
    }
    let (a, b) = (npc.position(&*host)?, player.entity().position(&*host)?);
    let d = (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
    (d <= REACH * REACH).then_some(npc)
}

/// Starts NPC `npc`'s dialog with `player` (if they are near), as a right-click does.
pub fn start(host: &mut dyn ServerHost, player: PlayerRef, npc: u32) {
    talk(host, player, &npc.to_le_bytes());
}

fn talk(host: &mut dyn ServerHost, player: PlayerRef, b: &[u8]) {
    let Some(id) = read_id(b) else { return };
    let Some(npc) = npc_near(host, player, id) else { return };
    // One that hates the player won't talk at all.
    if !crate::heart::talks_to(host, npc, player) {
        return;
    }
    let chosen = DIALOG.get(host, Holder::Entity(id)).ok().flatten().unwrap_or_else(|| DEFAULT_DIALOG.into());
    let extra = EXTRA.lock().ok().and_then(|e| e.get(&chosen).cloned());
    let found = extra.or_else(|| DIALOGS.lock().ok().and_then(|d| d.get(&chosen).or_else(|| d.get(DEFAULT_DIALOG)).cloned()));
    let Some(dialog) = found else {
        eprintln!("[projectfelix] no dialog {chosen} and no {DEFAULT_DIALOG}");
        return;
    };
    let start = dialog.start.clone();
    enter(host, player, npc, dialog, &start);
}

fn choose(host: &mut dyn ServerHost, player: PlayerRef, b: &[u8]) {
    let (Some(id), Some(&pick)) = (read_id(b), b.get(4)) else { return };
    let session = SESSIONS.lock().ok().and_then(|mut s| s.remove(&player.uuid()));
    let Some(session) = session.filter(|s| s.npc == id) else { return };
    let Some(npc) = npc_near(host, player, id) else {
        send_close(host, player);
        return;
    };
    let Some(&index) = session.shown.get(pick as usize) else { return };
    let Some(choice) = session.dialog.nodes.get(&session.node).and_then(|n| n.options.get(index)).cloned() else { return };
    // Things may have changed since the options were shown (an item given away): check again.
    let f = facts(host, player, npc);
    if !choice.shown(&f) {
        let node = session.node.clone();
        enter(host, player, npc, session.dialog, &node);
        return;
    }
    // Orders go through its character: it may take its time, or refuse.
    let mut actions = choice.actions.clone();
    let mut refused = false;
    actions.retain(|a| match a {
        Action::Order(o) => {
            refused |= !crate::heart::order(host, npc, player, mode_named(o));
            false
        }
        _ => true,
    });
    if refused {
        send_close(host, player);
        return;
    }
    run_for(host, Some(player), npc, &actions);
    match &choice.goto {
        Some(next) => enter(host, player, npc, session.dialog, next),
        None => send_close(host, player),
    }
}

fn enter(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef, dialog: Arc<Dialog>, node_name: &str) {
    let Some(node) = dialog.nodes.get(node_name).cloned() else { return };
    if let Some(anim) = &node.animation {
        run_for(host, Some(player), npc, &[Action::Animation(anim.clone())]);
    }
    run_for(host, Some(player), npc, &node.actions);
    let f = facts(host, player, npc);
    let shown: Vec<usize> = (0..node.options.len()).filter(|i| node.options[*i].shown(&f)).collect();
    let name = player.name(&*host).unwrap_or_default();
    let title = npc.name(&*host).unwrap_or_else(|| "NPC".into());
    let view = crate::serde_json::json!({
        "npc": npc.id(),
        "title": title,
        "lines": node.text.iter().map(|t| dialog::fill(t, &name, &title)).collect::<Vec<_>>(),
        "options": shown.iter().map(|i| dialog::fill(&node.options[*i].text, &name, &title)).collect::<Vec<_>>(),
    });
    if let Ok(mut s) = SESSIONS.lock() {
        s.insert(player.uuid(), Session { npc: npc.id(), dialog, node: node_name.to_string(), shown });
    }
    let bytes = crate::serde_json::to_vec(&view).unwrap_or_default();
    let _ = player.send_payload(host, &PayloadId(VIEW.into()), PayloadBytes(bytes));
}

fn send_close(host: &mut dyn ServerHost, player: PlayerRef) {
    end(player);
    let _ = player.send_payload(host, &PayloadId(CLOSE.into()), PayloadBytes(Vec::new()));
}

fn mode_named(o: &str) -> Mode {
    match o {
        "wait" => Mode::Wait,
        "guard" => Mode::Guard,
        _ => Mode::Follow,
    }
}

/// Runs dialog actions for `npc`; `player` (the one talking, if any) fills `@player`.
pub fn run_for(host: &mut dyn ServerHost, player: Option<PlayerRef>, npc: EntityRef, actions: &[Action]) {
    let name = player.and_then(|p| p.name(&*host)).unwrap_or_default();
    let npc_uuid = npc.uuid(host).map(uuid_text).unwrap_or_default();
    for a in actions {
        match a {
            Action::Animation(anim) => {
                let time = npc.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
                let _ = ACTION.set(host, Holder::Entity(npc.id()), Some(Play { animation: anim.clone(), start: time }));
            }
            Action::Order(o) => {
                companion::set_orders(host, npc, mode_named(o));
            }
            Action::Feel(event) => crate::heart::feel(host, npc, player, event),
            Action::Relation(delta) => {
                if let Some(p) = player {
                    crate::heart::befriend(host, npc, p, *delta);
                }
            }
            Action::Game(kind, vs_npc) => {
                let (Some(p), Some(kind)) = (player, crate::games::GameKind::parse(kind)) else { continue };
                if let Err(e) = crate::games::start(host, npc, kind, p, *vs_npc) {
                    eprintln!("[projectfelix] game: {e}");
                }
            }
            Action::Sport(name) => {
                let (Some(p), Some(sport)) = (player, crate::sports::Sport::parse(name)) else { continue };
                if let Err(e) = crate::matches::start(host, npc, sport, p) {
                    eprintln!("[projectfelix] sport: {e}");
                }
            }
            Action::EditAnimation(name) => {
                if let Some(p) = player {
                    crate::companion::open_editor(host, npc, p, name.clone());
                }
            }
            Action::JoinGame => {
                if let Some(p) = player
                    && let Err(e) = crate::games::join(host, npc, p)
                {
                    eprintln!("[projectfelix] join: {e}");
                }
            }
            Action::OpenGame => {
                if let Some(p) = player {
                    crate::games::open(host, npc, p);
                }
            }
            Action::Scene(id) => {
                if let Err(e) = crate::scenes::start(host, npc, id, player) {
                    eprintln!("[projectfelix] scene {id}: {e}");
                }
            }
            other => {
                if let Ok(mut p) = PENDING.lock() {
                    p.extend(dialog::commands(other, &name, &npc_uuid));
                }
            }
        }
    }
}
