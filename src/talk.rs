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
use std::collections::{HashMap, HashSet};
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

/// A button of the talk: an answer, the next line, going on, or the end.
#[derive(Clone)]
enum Btn {
    Answer(usize),
    Page(usize),
    Next(String),
    End,
}

/// A player's conversation: with whom, which dialog (its id: answers are remembered by it),
/// where in it, and which buttons they see.
struct Session {
    npc: u32,
    dialog: Arc<Dialog>,
    id: String,
    node: String,
    buttons: Vec<Btn>,
    /// The roll its options were shown with (`chance`): the same when one is picked.
    roll: u32,
}

static SESSIONS: LazyLock<Mutex<HashMap<u128, Session>>> = LazyLock::new(Default::default);
/// Commands from dialog actions, run at the next level tick (not inside a player's packet).
static PENDING: LazyLock<Mutex<Vec<String>>> = LazyLock::new(Default::default);
/// The players who answered since the queued commands last ran: one answer each until they have
/// (what an answer takes is taken by a queued command, so a second one would see it still there).
static ACTED: LazyLock<Mutex<HashSet<u128>>> = LazyLock::new(Default::default);

/// The server stopped: no talks, nothing queued, no dialogs but the data packs'.
pub fn clear() {
    if let Ok(mut s) = SESSIONS.lock() {
        s.clear();
    }
    if let Ok(mut p) = PENDING.lock() {
        p.clear();
    }
    if let Ok(mut a) = ACTED.lock() {
        a.clear();
    }
    if let Ok(mut e) = EXTRA.lock() {
        e.clear();
    }
}

/// Player `uuid` left the game (or the talk): it is over, and the NPC goes on with its day.
pub fn left(uuid: u128) {
    let had = SESSIONS.lock().ok().and_then(|mut s| s.remove(&uuid));
    if let Some(s) = had {
        companion::go_to(s.npc, None);
    }
}

struct Listener;

impl ServerReloadListener for Listener {
    fn prepare(&self, cx: &ServerReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        let mut raw = HashMap::new();
        for id in cx.list() {
            let Some(bytes) = cx.read(id) else { continue };
            match dialog::parse(bytes) {
                Ok(d) => {
                    next.insert(id.to_string(), Arc::new(d));
                    if let Ok(v) = crate::serde_json::from_slice::<crate::serde_json::Value>(bytes) {
                        raw.insert(id.to_string(), v);
                    }
                }
                Err(e) => eprintln!("[projectfelix] dialog {id} not loaded: {e}"),
            }
        }
        Ok(<dyn ReloadApply>::new(move || {
            if let Ok(mut live) = DIALOGS.lock() {
                *live = next;
            }
            if let Ok(mut r) = RAW.lock() {
                *r = raw;
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

/// The data packs' dialog files as they were, for the dialog editor.
static RAW: LazyLock<Mutex<HashMap<String, crate::serde_json::Value>>> = LazyLock::new(Default::default);

/// Data pack dialog `id`'s file.
pub fn raw_dialog(id: &str) -> Option<crate::serde_json::Value> {
    RAW.lock().ok()?.get(id).cloned()
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
pub fn level_tick(host: &mut dyn ServerHost, dimension: &str) {
    close_far(host, dimension);
    let lines = PENDING.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
    for line in lines {
        // A line that may fail (see `queue_try`) says nothing of it.
        let (line, quiet) = match line.strip_prefix(TRY) {
            Some(l) => (l, true),
            None => (line.as_str(), false),
        };
        if !host.commands().run_command(line) && !quiet {
            eprintln!("[projectfelix] dialog command failed: {line}");
        }
    }
    if let Ok(mut a) = ACTED.lock() {
        a.clear();
    }
}

/// What marks a queued line that may fail.
const TRY: char = '\u{1}';

/// As [`queue`], for a line that may well fail (removing what may not be there): nothing is logged.
pub fn queue_try(line: String) {
    queue(format!("{TRY}{line}"));
}

/// Runs `line` as the server console at the next level tick.
pub fn queue(line: String) {
    if let Ok(mut p) = PENDING.lock() {
        p.push(line);
    }
}

fn end(player: PlayerRef) {
    left(player.uuid());
}

/// Ticks between two looks at the talks open.
const LOOK_EVERY: u32 = 20;
static LOOKS: Mutex<u32> = Mutex::new(0);

/// Each second: a talk whose player walked off (or whose NPC is gone) is over.
fn close_far(host: &mut dyn ServerHost, dimension: &str) {
    let due = LOOKS.lock().is_ok_and(|mut n| {
        *n = n.wrapping_add(1);
        *n % LOOK_EVERY == 0
    });
    if !due {
        return;
    }
    let open: Vec<(u128, u32)> = SESSIONS.lock().map(|s| s.iter().map(|(u, x)| (*u, x.npc)).collect()).unwrap_or_default();
    for (uuid, npc) in open {
        let Some(p) = host.player(uuid) else { continue };
        // Players of the level ticking now: its NPCs are the ones that can be asked after.
        if p.entity().level(&*host).is_none_or(|l| l.dimension_id() != dimension) {
            continue;
        }
        let near = host.entity(npc).and_then(|e| e.position(&*host)).zip(p.entity().position(&*host)).is_some_and(|(a, b)| {
            (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2) <= (REACH + 3.0) * (REACH + 3.0)
        });
        if !near {
            send_close(host, p);
        }
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
    doing: String,
    routine: bool,
    met: bool,
    remembers: Vec<String>,
    fault: bool,
    event: String,
    flags: HashMap<String, String>,
    dialog: String,
    biome: String,
    time: &'static str,
    weather: &'static str,
    roll: u32,
    hungry: bool,
    family: bool,
    stage: u8,
    romance: &'static str,
    quests_done: Vec<String>,
    quests_active: Vec<String>,
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
    fn doing(&self) -> &str {
        &self.doing
    }
    fn has_routine(&self) -> bool {
        self.routine
    }
    fn met(&self) -> bool {
        self.met
    }
    fn remembers(&self, kind: &str) -> bool {
        self.remembers.iter().any(|k| k == kind)
    }
    fn fault(&self) -> bool {
        self.fault
    }
    fn event(&self) -> &str {
        &self.event
    }
    fn flag(&self, key: &str) -> Option<&str> {
        self.flags.get(key).map(String::as_str)
    }
    fn answer(&self, node: &str) -> Option<&str> {
        let key = if node.contains(':') { node.to_string() } else { format!("{}:{node}", self.dialog) };
        self.flags.get(&key).map(String::as_str)
    }
    fn biome(&self) -> &str {
        &self.biome
    }
    fn time_of_day(&self) -> &str {
        self.time
    }
    fn weather(&self) -> &str {
        self.weather
    }
    fn roll(&self) -> u32 {
        self.roll
    }
    fn hungry(&self) -> bool {
        self.hungry
    }
    fn has_family(&self) -> bool {
        self.family
    }
    fn stage(&self) -> u8 {
        self.stage
    }
    fn romance(&self) -> &str {
        self.romance
    }
    fn quest_done(&self, id: &str) -> bool {
        self.quests_done.iter().any(|q| q == id)
    }
    fn quest_active(&self, id: &str) -> bool {
        self.quests_active.iter().any(|q| q == id)
    }
}

fn facts(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef, dialog: &str) -> PlayerFacts {
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
        Ok(Mode::Live) => "live",
        _ => "follow",
    };
    let (game, in_game) = crate::games::state_for(npc.id(), player);
    let mood = crate::heart::mood_of(npc.id());
    let relation = crate::heart::relation_of(npc.id(), player.uuid());
    let doing = crate::routines::doing(npc.id());
    let routine = crate::routines::has_routine(npc.id());
    let (met, remembers) = crate::heart::memory_of(npc.id(), player.uuid());
    let fault = crate::heart::fault_of(npc.id(), player.uuid());
    let event = crate::events::id_at(host, npc).unwrap_or_default();
    let flags = crate::flags::all(host, player);
    let (quests_done, quests_active) = crate::quests::log_ids(host, player);
    let (biome, time, weather) = crate::flags::surroundings(&*host, player);
    let now = npc.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0) as u64;
    let roll = (now.wrapping_mul(2_654_435_761).wrapping_add(u64::from(npc.id()).wrapping_mul(40_503)) >> 7) as u32 % 100;
    PlayerFacts { game, in_game, tags, items, owner, mode, mood, relation, doing, routine, met, remembers, fault, event, flags, dialog: dialog.to_string(), biome, time, weather, roll, hungry: crate::needs::food_of(npc.id()) <= crate::need::HUNGRY, family: crate::families::has_family(host, npc), stage: crate::bonds::stage(host, npc, player) as u8, romance: match crate::bonds::stage(host, npc, player) {
        crate::bond::Stage::Sweetheart => "sweetheart",
        crate::bond::Stage::Partner => "partner",
        _ => "none",
    }, quests_done, quests_active }
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
    // A sleeper wakes first; one that hates the player won't talk at all.
    crate::routines::wake(host, npc, Some(player));
    if !crate::heart::talks_to(host, npc, player) {
        return;
    }
    crate::quests::on_talk(host, npc, player);
    crate::bonds::on_talk(host, npc, player);
    let chosen = DIALOG.get(host, Holder::Entity(id)).ok().flatten().unwrap_or_else(|| DEFAULT_DIALOG.into());
    // An NPC file may speak a dialog made in the game.
    let chosen = crate::custom::dialog_override(&chosen).unwrap_or(chosen);
    let extra = EXTRA.lock().ok().and_then(|e| e.get(&chosen).cloned());
    let found = extra.or_else(|| DIALOGS.lock().ok().and_then(|d| d.get(&chosen).or_else(|| d.get(DEFAULT_DIALOG)).cloned()));
    let Some(dialog) = found else {
        eprintln!("[projectfelix] no dialog {chosen} and no {DEFAULT_DIALOG}");
        return;
    };
    let start = dialog.start.clone();
    enter(host, player, npc, dialog, &chosen, &start);
}

/// The dialog of id `id`: made in the game, an NPC's own, or a data pack's.
pub fn dialog_named(id: &str) -> Option<Arc<Dialog>> {
    EXTRA.lock().ok().and_then(|e| e.get(id).cloned()).or_else(|| DIALOGS.lock().ok().and_then(|d| d.get(id).cloned()))
}

/// Starts dialog `id` (its start) between `player` and NPC `npc`, as if the player had spoken.
pub fn start_dialog(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef, id: &str) -> bool {
    let Some(d) = dialog_named(id) else { return false };
    let start = d.start.clone();
    enter(host, player, npc, d, id, &start);
    true
}

fn choose(host: &mut dyn ServerHost, player: PlayerRef, b: &[u8]) {
    let (Some(id), Some(&pick)) = (read_id(b), b.get(4)) else { return };
    // One answer until what the last one queued has run.
    if !ACTED.lock().is_ok_and(|mut a| a.insert(player.uuid())) {
        return;
    }
    let session = SESSIONS.lock().ok().and_then(|mut s| s.remove(&player.uuid()));
    let Some(session) = session.filter(|s| s.npc == id) else { return };
    let Some(npc) = npc_near(host, player, id) else {
        send_close(host, player);
        return;
    };
    let Some(btn) = session.buttons.get(pick as usize).cloned() else { return };
    let index = match btn {
        Btn::Page(p) => {
            show(host, player, npc, session.dialog, &session.id, &session.node, p, 0, true);
            return;
        }
        Btn::Next(n) => {
            show(host, player, npc, session.dialog, &session.id, &n, 0, 0, true);
            return;
        }
        Btn::End => {
            send_close(host, player);
            return;
        }
        Btn::Answer(i) => i,
    };
    let Some(node) = session.dialog.nodes.get(&session.node).cloned() else { return };
    let Some(choice) = node.options.get(index).cloned() else { return };
    // Things may have changed since the options were shown (an item given away): check again.
    let mut f = facts(host, player, npc, &session.id);
    f.roll = session.roll;
    if !choice.shown(&f) {
        // Shown again as it is now; what the node does was done when they came to it.
        show(host, player, npc, session.dialog, &session.id, &session.node, 0, 0, false);
        return;
    }
    // What the player answered is remembered, for the talks after.
    if node.options.len() >= 2 && !session.id.is_empty() {
        crate::flags::set(host, player, &format!("{}:{}", session.id, session.node), &dialog::letter(index));
    }
    // Orders go through its character: it may take its time, or refuse.
    let mut actions = choice.actions.clone();
    let mut refused = false;
    actions.retain(|a| match a {
        Action::Order(o) => {
            // Three follow a player at most (see `squad`).
            if mode_named(o) == Mode::Follow && crate::squad::full(&*host, player, npc) {
                let name = player.name(&*host).unwrap_or_default();
                crate::heart::say_to(host, npc, player, &format!("You have enough company already, {name}."));
                refused = true;
            } else {
                refused |= !crate::heart::order(host, npc, player, mode_named(o));
            }
            false
        }
        _ => true,
    });
    if refused {
        send_close(host, player);
        return;
    }
    // A quest board opens in place of this dialog.
    let board = actions.iter().any(|a| matches!(a, Action::Quests(_) | Action::Shop(_) | Action::Trade(_)));
    run_for(host, Some(player), npc, &actions);
    // Another dialog takes over.
    if let Some(Action::Dialog(other)) = actions.iter().find(|a| matches!(a, Action::Dialog(_))) {
        if !start_dialog(host, player, npc, other) {
            send_close(host, player);
        }
        return;
    }
    match &choice.goto {
        Some(next) => enter(host, player, npc, session.dialog, &session.id, next),
        None if board => {}
        None => send_close(host, player),
    }
}

/// Shows `player` node `node` of a dialog built on the spot (a quest board), as NPC `npc`'s.
pub fn open(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef, dialog: Arc<Dialog>, node: &str) {
    enter(host, player, npc, dialog, "", node);
}

fn enter(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef, dialog: Arc<Dialog>, id: &str, node_name: &str) {
    show(host, player, npc, dialog, id, node_name, 0, 0, true);
}

/// Shows `player` page `page` of node `node_name` (a branch node sends it on, at most so deep).
/// `act`: the node's actions run (coming to it; not when it is only shown again).
#[allow(clippy::too_many_arguments)]
fn show(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef, dialog: Arc<Dialog>, id: &str, node_name: &str, page: usize, depth: u8, act: bool) {
    let Some(node) = dialog.nodes.get(node_name).cloned() else {
        send_close(host, player);
        return;
    };
    if page == 0 && act {
        if let Some(anim) = &node.animation {
            run_for(host, Some(player), npc, &[Action::Animation(anim.clone())]);
        }
        run_for(host, Some(player), npc, &node.actions);
    }
    let f = facts(host, player, npc, id);
    if !node.branch.is_empty() {
        match dialog::route(&node, &f) {
            Some(next) if depth < 16 => show(host, player, npc, dialog, id, &next, 0, depth + 1, true),
            _ => send_close(host, player),
        }
        return;
    }
    let extended = crate::flags::extended(&*host, player);
    let name = player.name(&*host).unwrap_or_default();
    let fill_name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
    let paged = node.paged && node.text.len() > 1;
    let last = !paged || page + 1 >= node.text.len();
    let lines: Vec<String> = if paged { node.text.get(page).cloned().into_iter().collect() } else { node.text.clone() };
    let more = node.more.clone().unwrap_or_else(|| "...".into());
    let mut buttons = Vec::new();
    let mut labels = Vec::new();
    if !last {
        buttons.push(Btn::Page(page + 1));
        labels.push(more.clone());
    } else {
        for (i, o) in node.options.iter().enumerate().filter(|(_, o)| o.shown(&f)) {
            let mut label = dialog::fill(&o.text, &name, &fill_name);
            if extended {
                let d = dialog::describe(o);
                if !d.is_empty() {
                    label = format!("{label}  [{d}]");
                }
            }
            buttons.push(Btn::Answer(i));
            labels.push(label);
        }
        if buttons.is_empty() {
            match &node.next {
                Some(n) => {
                    buttons.push(Btn::Next(n.clone()));
                    labels.push(if extended { format!("{more}  [> {n}]") } else { more });
                }
                None => {
                    buttons.push(Btn::End);
                    labels.push("Bye.".into());
                }
            }
        }
    }
    // Who it is, and roughly how it stands with the player: no numbers but in the extended view.
    let mut title = fill_name.clone();
    let level = crate::gear::level_of(host, npc);
    if level > 0 {
        title = format!("{title} (level {level})");
    }
    let food = crate::needs::food_of(npc.id());
    // Past friends, where they stand (a close friend, a sweetheart…) says more than liking.
    let standing = crate::bonds::stage_word(host, npc, player).unwrap_or_else(|| crate::flags::standing_word(npc.id(), player.uuid()));
    let words: Vec<&str> = [standing, crate::need::word(food)].into_iter().filter(|w| !w.is_empty()).collect();
    if !words.is_empty() {
        title = format!("{title} - {}", words.join(", "));
    }
    if extended {
        let rel = crate::heart::relation_of(npc.id(), player.uuid());
        let mood = crate::heart::mood_of(npc.id());
        let close = crate::bonds::closeness(host, npc, player);
        title = format!("{title}  [rel {rel}, close {close:.0}, {mood}, food {food}, {}{node_name}{}]", if id.is_empty() { String::new() } else { format!("{id} ") }, if paged { format!(" {}/{}", page + 1, node.text.len()) } else { String::new() });
    }
    let event_name = crate::events::name_at(host, npc).unwrap_or_else(|| "festival".into());
    let upset_by = crate::heart::upset_by_name(npc.id());
    let view = crate::serde_json::json!({
        "npc": npc.id(),
        "title": title,
        "lines": lines.iter().map(|t| dialog::fill(t, &name, &fill_name).replace("@upset_by", &upset_by).replace("@event", &event_name)).collect::<Vec<_>>(),
        "options": labels,
    });
    if let Ok(mut s) = SESSIONS.lock() {
        s.insert(player.uuid(), Session { npc: npc.id(), dialog, id: id.to_string(), node: node_name.to_string(), buttons, roll: f.roll });
    }
    // While they talk it stays where it is, turned to the player.
    if let Some(at) = npc.position(&*host) {
        companion::go_to(npc.id(), Some(at));
    }
    if let Some(to) = player.entity().position(&*host) {
        let _ = mod_api::MobRef::new(npc).look_at(host, [to[0], to[1] + 1.5, to[2]]);
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
        "live" => Mode::Live,
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
            Action::Feel(event) => {
                let before = player.map(|p| crate::heart::relation_of(npc.id(), p.uuid()));
                crate::heart::feel(host, npc, player, event);
                if let (Some(p), Some(b)) = (player, before)
                    && crate::flags::extended(&*host, p)
                {
                    crate::flags::tell_change(host, npc, p, b, &format!("feels {event}"));
                }
            }
            Action::Remember(key, value) => {
                if let Some(p) = player {
                    crate::flags::set(host, p, key, value);
                }
            }
            Action::Forget(key) => {
                if let Some(p) = player {
                    crate::flags::forget(host, p, key);
                }
            }
            Action::Dialog(_) => {}
            Action::Feed => {
                if let Some(p) = player {
                    crate::needs::feed(host, npc, p);
                }
            }
            Action::Family => {
                if let Some(p) = player {
                    crate::families::tell(host, npc, p);
                }
            }
            Action::Gift | Action::Tastes | Action::AskOut | Action::Propose | Action::Part | Action::Reminisce => {
                if let Some(p) = player {
                    match a {
                        Action::Gift => crate::bonds::give_held(host, npc, p),
                        Action::Tastes => crate::bonds::tell_tastes(host, npc, p),
                        Action::AskOut => crate::bonds::ask_out(host, npc, p),
                        Action::Propose => crate::bonds::propose(host, npc, p),
                        Action::Reminisce => crate::bonds::reminisce(host, npc, p),
                        _ => crate::bonds::part(host, npc, p, false),
                    }
                }
            }
            Action::News => {
                if let Some(p) = player {
                    crate::heart::tell_news(host, npc, p);
                }
            }
            Action::Place(name) => {
                let at = player.and_then(|p| p.entity().position(&*host)).or_else(|| npc.position(&*host));
                if let Some(at) = at {
                    crate::routines::set_place(host, npc, name, at);
                }
            }
            Action::Gear(what) => {
                if let Some(p) = player {
                    if what == "return" {
                        crate::gear::give_back(host, npc, p);
                    } else {
                        crate::gear::take(host, npc, p);
                    }
                }
            }
            Action::Stats => crate::gear::tell_stats(host, npc, player),
            Action::Mind => {
                if let Some(p) = player {
                    crate::minds::tell(host, npc, p);
                }
            }
            Action::Skills => {
                if let Some(p) = player {
                    crate::skills::tell(host, npc, p);
                }
            }
            Action::Village(what) => {
                if let Some(p) = player {
                    crate::chiefs::talk(host, npc, p, what);
                }
            }
            Action::Shop(id) => {
                if let Some(p) = player {
                    crate::shops::board(host, p, npc, id);
                }
            }
            Action::Trade(n) => {
                if let Some(p) = player {
                    crate::shops::trade(host, p, npc, *n);
                }
            }
            Action::Quests(list) => {
                if let Some(p) = player {
                    crate::quests::board(host, p, npc, list);
                }
            }
            Action::Quest(id) => {
                if let Some(p) = player {
                    crate::quests::take(host, p, npc, id);
                }
            }
            Action::HandIn => {
                if let Some(p) = player {
                    crate::quests::hand_in(host, p, npc);
                }
            }
            Action::GiveUp => {
                if let Some(p) = player {
                    crate::quests::give_up(host, p, npc);
                }
            }
            Action::Relation(delta) => {
                if let Some(p) = player {
                    let before = crate::heart::relation_of(npc.id(), p.uuid());
                    crate::heart::befriend(host, npc, p, *delta);
                    crate::flags::tell_change(host, npc, p, before, &format!("relation {delta:+}"));
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
