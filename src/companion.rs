//! The NPC as a companion, on the server: its owner, its orders (follow, wait, guard) and how it
//! looks, saved with the world and synced to Horizon clients.
//!
//! The entity type copies a tame wolf (`data/projectfelix/horizon/entity_type/npc.json`), so it
//! follows its owner, teleports to them and fights for them as a tame wolf does. Waiting and
//! guarding are goals of our own that take the movement away from the wolf's following.
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, Command, CommandArgumentValue, CommandSource, Compound,
    EntityRef, EntityPart, EntitySizeEvent, EntityTypeKey, Flow, Goal, GoalContext, GoalFlags, GoalRegistrar, Holder, Host,
    LivingRef, MobCategory, MobRef, RegisterCommandsEvent,
    Registrar, RunModCommandEvent, ServerHost, Tag, Text,
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, OnceLock};

pub const NPC: &str = "projectfelix:npc";
pub const DEFAULT_MODEL: &str = "projectfelix:felix";

/// How far a guard looks for monsters around its post, and how far it chases them.
const GUARD_SIGHT: f64 = 10.0;
const GUARD_LEASH: f64 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Follow,
    Wait,
    Guard,
    /// Lives by its routine (see `routines`).
    Live,
}

impl Mode {
    pub fn byte(self) -> u8 {
        match self {
            Mode::Follow => 0,
            Mode::Wait => 1,
            Mode::Guard => 2,
            Mode::Live => 3,
        }
    }
    pub fn from_byte(b: u8) -> Mode {
        match b {
            1 => Mode::Wait,
            2 => Mode::Guard,
            3 => Mode::Live,
            _ => Mode::Follow,
        }
    }
}

/// An NPC's orders: what it does and, waiting or guarding, where.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Orders {
    pub mode: Mode,
    pub post: [f64; 3],
}

pub static ORDERS: AttachmentType<Orders> = AttachmentType {
    namespace: "projectfelix",
    name: "orders",
    default: Orders::default,
    codec: Some(AttachmentCodec {
        encode: |o| {
            let mut c = Compound::new();
            c.insert("mode".into(), Tag::Byte(o.mode.byte() as i8));
            c.insert("post".into(), Tag::List(o.post.iter().map(|v| Tag::Double(v.to_bits())).collect()));
            Ok(c)
        },
        decode: |c: &Compound| {
            let mode = match c.get(&"mode".into()) {
                Some(Tag::Byte(b)) => Mode::from_byte(*b as u8),
                _ => Mode::Follow,
            };
            let mut post = [0.0; 3];
            if let Some(Tag::List(list)) = c.get(&"post".into()) {
                for (p, t) in post.iter_mut().zip(list) {
                    if let Tag::Double(v) = t {
                        *p = f64::from_bits(*v);
                    }
                }
            }
            Ok(Orders { mode, post })
        },
    }),
    copy_on_death: false,
    sync: true,
};

/// The orders an NPC goes back to after a game, a match or a scene that holds it still. Kept with
/// it, so a world closed during one doesn't leave it waiting for good.
pub static HELD: AttachmentType<Option<Mode>> = AttachmentType {
    namespace: "projectfelix",
    name: "held",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |m| {
            let mut c = Compound::new();
            if let Some(m) = m {
                c.insert("mode".into(), Tag::Byte(m.byte() as i8));
            }
            Ok(c)
        },
        decode: |c: &Compound| match c.get(&"mode".into()) {
            Some(Tag::Byte(b)) => Ok(Some(Mode::from_byte(*b as u8))),
            _ => Ok(None),
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// Holds NPC `npc` still (for a game, a match, a scene): its orders are kept to go back to.
pub fn hold(host: &mut dyn ServerHost, npc: EntityRef, restore: Mode) {
    if HELD.get(host, Holder::Entity(npc.id())).ok().flatten().is_none() {
        let _ = HELD.set(host, Holder::Entity(npc.id()), Some(restore));
    }
    set_orders(host, npc, Mode::Wait);
}

/// Lets NPC `npc` go back to the orders it had (else `restore`).
pub fn release(host: &mut dyn ServerHost, npc: EntityRef, restore: Mode) {
    let restore = HELD.get(host, Holder::Entity(npc.id())).ok().flatten().unwrap_or(restore);
    let _ = HELD.set(host, Holder::Entity(npc.id()), None);
    set_orders(host, npc, restore);
}

/// How an NPC looks: its model (`projectfelix:felix`) and texture (`projectfelix:entity/felix`).
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub model: String,
    pub texture: String,
}

impl Look {
    /// A model's usual texture: `ns:name` → `ns:entity/name`.
    pub fn of_model(model: &str) -> Look {
        let (ns, name) = model.split_once(':').unwrap_or(("projectfelix", model));
        Look { model: format!("{ns}:{name}"), texture: format!("{ns}:entity/{name}") }
    }
}

pub static LOOK: AttachmentType<Option<Look>> = AttachmentType {
    namespace: "projectfelix",
    name: "look",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |l| {
            let mut c = Compound::new();
            if let Some(l) = l {
                c.insert("model".into(), Tag::String(l.model.as_str().into()));
                c.insert("texture".into(), Tag::String(l.texture.as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            let get = |k: &str| match c.get(&k.into()) {
                Some(Tag::String(s)) => Some(s.to_string_lossy()),
                _ => None,
            };
            match (get("model"), get("texture")) {
                (Some(model), Some(texture)) => Ok(Some(Look { model, texture })),
                (None, None) => Ok(None),
                _ => Err(AttachmentError::Decode),
            }
        },
    }),
    copy_on_death: false,
    sync: true,
};

/// The server's copy of every loaded NPC's orders, for the goals (which can't read attachments).
/// Entities missing here are looked up at the next level tick.
#[derive(Default)]
struct Server {
    orders: HashMap<u32, Orders>,
    unknown: Vec<u32>,
    /// What NPCs living by their day do at their post now.
    live: HashMap<u32, LiveDo>,
    /// Where NPCs are sent to walk now, over their orders (to a player, say).
    go: HashMap<u32, [f64; 3]>,
}

/// What a living NPC does at its post: goes with its owner instead, stays and walks about, rests
/// still (sleeps), or guards it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LiveDo {
    Follow,
    #[default]
    Stay,
    Rest,
    Guard,
    /// At its job (see `jobs`): goes where the work is, and stays.
    Work,
}

fn live_of(id: u32) -> LiveDo {
    SERVER.lock().ok().and_then(|s| s.live.get(&id).copied()).unwrap_or_default()
}

/// Has NPC `npc` live by its day: at `post`, doing `what`.
pub fn set_live(host: &mut dyn ServerHost, npc: EntityRef, post: [f64; 3], what: LiveDo) -> bool {
    let o = Orders { mode: Mode::Live, post };
    if ORDERS.set(host, Holder::Entity(npc.id()), o).is_err() {
        return false;
    }
    if let Ok(mut s) = SERVER.lock() {
        s.orders.insert(npc.id(), o);
        s.live.insert(npc.id(), what);
    }
    true
}

static SERVER: LazyLock<Mutex<Server>> = LazyLock::new(Default::default);
static NPC_KEY: OnceLock<EntityTypeKey> = OnceLock::new();

/// The NPCs whose goals ran lately (every loaded NPC), by entity id.
pub fn npcs() -> Vec<u32> {
    SERVER.lock().map(|s| s.orders.keys().copied().collect()).unwrap_or_default()
}

/// An NPC's orders as last read.
pub fn mode_of(id: u32) -> Mode {
    orders_of(id).map(|o| o.mode).unwrap_or_default()
}

fn orders_of(id: u32) -> Option<Orders> {
    let mut s = SERVER.lock().ok()?;
    let o = s.orders.get(&id).copied();
    if o.is_none() && !s.unknown.contains(&id) {
        s.unknown.push(id);
    }
    o
}

pub fn register(r: &mut dyn Registrar) {
    let _ = NPC_KEY.set(r.entity_type_key(NPC));
    r.entity_part(NPC, CompanionGoals);
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_client(mod_api::PayloadId(EDIT.into()), Some(std::sync::Arc::new(crate::editor::OpenHandler)));
    reg.play_to_server(mod_api::PayloadId(crate::editor::SET_LOOK.into()), std::sync::Arc::new(SetLook));
}

/// Server level tick: reads the orders of NPCs the goals asked about, and points guards at the
/// monsters near their posts (each tick); the wolf's own attack goals do the fighting.
pub fn level_tick(host: &mut dyn ServerHost) {
    let guards: Vec<(u32, [f64; 3])> = match SERVER.lock() {
        Ok(s) => s
            .orders
            .iter()
            .filter(|(id, o)| o.mode == Mode::Guard || (o.mode == Mode::Live && s.live.get(id) == Some(&LiveDo::Guard)))
            .map(|(id, o)| (*id, o.post))
            .collect(),
        Err(_) => Vec::new(),
    };
    for (id, post) in guards {
        let Some(me) = host.entity(id) else { continue };
        let mob = MobRef::new(me);
        let current = mob.target(&*host);
        let keep = current.and_then(|t| t.entity().position(&*host)).is_some_and(|at| distance_sq(at, post) < GUARD_LEASH * GUARD_LEASH);
        if keep {
            continue;
        }
        let next = nearest_monster(&*host, me, post);
        if next.is_some() || current.is_some() {
            mob.set_target(host, next.map(LivingRef::new));
        }
    }
    let unknown = match SERVER.lock() {
        Ok(mut s) => std::mem::take(&mut s.unknown),
        Err(_) => return,
    };
    for id in unknown {
        if host.entity(id).is_none() {
            continue;
        }
        let Ok(mut o) = ORDERS.get(host, Holder::Entity(id)) else { continue };
        // A table, court or scene props left from before its world closed go.
        crate::games::forget_stale(host, id);
        crate::matches::forget_stale(host, id);
        crate::scenes::forget_stale(host, id);
        // Held when its world was closed: what held it is gone, so it goes back.
        if let Some(m) = HELD.get(host, Holder::Entity(id)).ok().flatten() {
            o.mode = m;
            let _ = ORDERS.set(host, Holder::Entity(id), o);
            let _ = HELD.set(host, Holder::Entity(id), None);
        }
        if let Ok(mut s) = SERVER.lock() {
            s.orders.insert(id, o);
        }
    }
}

/// A person's height and eyes instead of a wolf's.
pub fn entity_size(host: &dyn Host, e: &mut EntitySizeEvent<'_>) {
    if e.entity.kind(host).is_some_and(|k| Some(&k) == NPC_KEY.get()) {
        *e.size = [0.6, 1.95, 1.62];
    }
}

/// Whether `entity` is an NPC of this mod.
pub fn is_npc(host: &dyn Host, entity: EntityRef) -> bool {
    entity.kind(host).is_some_and(|k| Some(&k) == NPC_KEY.get())
}

/// Gives the NPC new orders; waiting and guarding hold where it stands now.
pub fn set_orders(host: &mut dyn ServerHost, entity: EntityRef, mode: Mode) -> bool {
    let post = entity.position(&*host).unwrap_or([0.0; 3]);
    let o = Orders { mode, post };
    if ORDERS.set(host, Holder::Entity(entity.id()), o).is_err() {
        return false;
    }
    if let Ok(mut s) = SERVER.lock() {
        s.orders.insert(entity.id(), o);
    }
    true
}

// ---- goals --------------------------------------------------------------------------------------

struct CompanionGoals;

#[mod_api::horizon_part]
impl EntityPart for CompanionGoals {
    fn register_goals(&self, goals: &mut GoalRegistrar<'_>) {
        goals.add_goal(0, Present);
        goals.add_goal(0, Go { t: 0 });
        goals.add_goal(1, Wait);
        goals.add_goal(1, KeepPost);
        goals.add_goal(1, Live { t: 0 });
    }
}

fn distance_sq(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Never runs: asking for its orders each tick makes every loaded NPC known here (for its rules
/// too), whichever of its goals runs.
struct Present;

impl Goal for Present {
    fn flags(&self) -> GoalFlags {
        GoalFlags::NONE
    }
    fn can_use(&mut self, cx: &mut GoalContext<'_>) -> bool {
        let _ = orders_of(cx.mob().entity().id());
        false
    }
}

/// Sends NPC `id` walking to `to` over its orders (none: back to them).
pub fn go_to(id: u32, to: Option<[f64; 3]>) {
    if let Ok(mut s) = SERVER.lock() {
        match to {
            Some(p) => s.go.insert(id, p),
            None => s.go.remove(&id),
        };
    }
}

fn going(id: u32) -> Option<[f64; 3]> {
    SERVER.lock().ok()?.go.get(&id).copied()
}

/// Going somewhere it was sent (see [`go_to`]), whatever its orders.
struct Go {
    t: u32,
}

impl Goal for Go {
    fn flags(&self) -> GoalFlags {
        GoalFlags::MOVE.union(GoalFlags::JUMP)
    }
    fn can_use(&mut self, cx: &mut GoalContext<'_>) -> bool {
        going(cx.mob().entity().id()).is_some()
    }
    fn can_continue(&mut self, cx: &mut GoalContext<'_>) -> bool {
        self.can_use(cx)
    }
    fn tick(&mut self, cx: &mut GoalContext<'_>) {
        self.t = self.t.wrapping_add(1);
        let id = cx.mob().entity().id();
        let (Some(to), Some(at)) = (going(id), cx.mob().entity().position(cx.host())) else { return };
        if distance_sq(at, to) <= 2.0 * 2.0 {
            cx.stop_navigation();
        } else if self.t % 10 == 1 {
            cx.move_to(to, 1.05);
        }
    }
    fn stop(&mut self, cx: &mut GoalContext<'_>) {
        cx.stop_navigation();
    }
    fn name(&self) -> &'static str {
        "projectfelix:go"
    }
}

/// Waiting: holds the movement so the wolf's following never runs.
struct Wait;

impl Goal for Wait {
    fn flags(&self) -> GoalFlags {
        GoalFlags::MOVE.union(GoalFlags::JUMP)
    }
    fn can_use(&mut self, cx: &mut GoalContext<'_>) -> bool {
        orders_of(cx.mob().entity().id()).is_some_and(|o| o.mode == Mode::Wait)
    }
    fn start(&mut self, cx: &mut GoalContext<'_>) {
        cx.stop_navigation();
    }
    fn name(&self) -> &'static str {
        "projectfelix:wait"
    }
}

/// The monster nearest a guard's post, within sight of it.
fn nearest_monster(host: &dyn Host, me: EntityRef, post: [f64; 3]) -> Option<EntityRef> {
    let level = me.level(host)?;
    let r = GUARD_SIGHT;
    let around = level.entities_in(host, [post[0] - r, post[1] - 4.0, post[2] - r], [post[0] + r, post[1] + 4.0, post[2] + r])?;
    around
        .into_iter()
        .filter(|e| e.id() != me.id() && e.classification(host) == Some(MobCategory::Monster))
        .filter_map(|e| Some((distance_sq(e.position(host)?, post), e)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, e)| e)
}

/// Guarding: walks back to the post when nothing is there to fight.
struct KeepPost;

impl Goal for KeepPost {
    fn flags(&self) -> GoalFlags {
        GoalFlags::MOVE
    }
    fn can_use(&mut self, cx: &mut GoalContext<'_>) -> bool {
        let Some(o) = orders_of(cx.mob().entity().id()).filter(|o| o.mode == Mode::Guard) else { return false };
        let Some(at) = cx.mob().entity().position(cx.host()) else { return false };
        distance_sq(at, o.post) > 2.5 * 2.5 && cx.mob().target(cx.host()).is_none()
    }
    fn can_continue(&mut self, cx: &mut GoalContext<'_>) -> bool {
        self.can_use(cx)
    }
    fn tick(&mut self, cx: &mut GoalContext<'_>) {
        if let Some(o) = orders_of(cx.mob().entity().id()) {
            cx.move_to(o.post, 1.0);
        }
    }
    fn stop(&mut self, cx: &mut GoalContext<'_>) {
        cx.stop_navigation();
    }
    fn name(&self) -> &'static str {
        "projectfelix:keep_post"
    }
}

/// Living by its day: holds the movement (no following), keeps to its post and walks about it
/// now and then; guarding, it leaves the fighting to the wolf's goals.
struct Live {
    t: u32,
}

impl Goal for Live {
    fn flags(&self) -> GoalFlags {
        GoalFlags::MOVE.union(GoalFlags::JUMP)
    }
    fn can_use(&mut self, cx: &mut GoalContext<'_>) -> bool {
        let id = cx.mob().entity().id();
        let Some(o) = orders_of(id).filter(|o| o.mode == Mode::Live) else { return false };
        match live_of(id) {
            LiveDo::Follow => false,
            LiveDo::Guard => {
                cx.mob().target(cx.host()).is_none() && cx.mob().entity().position(cx.host()).is_some_and(|at| distance_sq(at, o.post) > 2.5 * 2.5)
            }
            LiveDo::Stay | LiveDo::Rest | LiveDo::Work => true,
        }
    }
    fn can_continue(&mut self, cx: &mut GoalContext<'_>) -> bool {
        self.can_use(cx)
    }
    fn start(&mut self, cx: &mut GoalContext<'_>) {
        cx.stop_navigation();
    }
    fn tick(&mut self, cx: &mut GoalContext<'_>) {
        self.t = self.t.wrapping_add(1);
        let id = cx.mob().entity().id();
        let Some(o) = orders_of(id) else { return };
        let Some(at) = cx.mob().entity().position(cx.host()) else { return };
        let far = distance_sq(at, o.post) > if live_of(id) == LiveDo::Work { 1.2 * 1.2 } else { 2.5 * 2.5 };
        if far && self.t % 10 == 1 {
            cx.move_to(o.post, 1.0);
        } else if !far && live_of(id) == LiveDo::Stay && self.t % 200 == 0 {
            // A few steps about its post.
            let h = (id as u64).wrapping_mul(0x9E37_79B9).wrapping_add(self.t as u64 * 7919);
            let off = |k: u64| ((h >> k) % 7) as f64 - 3.0;
            cx.move_to([o.post[0] + off(3), o.post[1], o.post[2] + off(11)], 0.6);
        } else if !far && live_of(id) == LiveDo::Rest && self.t % 20 == 0 {
            cx.stop_navigation();
        }
    }
    fn stop(&mut self, cx: &mut GoalContext<'_>) {
        cx.stop_navigation();
    }
    fn name(&self) -> &'static str {
        "projectfelix:live"
    }
}

/// Client to server: nothing; server to client: open the editor on an NPC (JSON: npc, name).
pub const EDIT: &str = "projectfelix:edit";

/// Opens the animation editor for `player` on `npc`, with `name` or the model's `custom`.
pub fn open_editor(host: &mut dyn ServerHost, npc: EntityRef, player: mod_api::PlayerRef, name: Option<String>) {
    let model = LOOK.get(host, Holder::Entity(npc.id())).ok().flatten().map(|l| l.model).unwrap_or(DEFAULT_MODEL.into());
    let short = model.rsplit(':').next().unwrap_or("felix").to_string();
    let name = name.unwrap_or_else(|| format!("animation.{short}.custom"));
    let body = crate::serde_json::json!({ "npc": npc.id(), "name": name }).to_string();
    let _ = player.send_payload(host, &mod_api::PayloadId(EDIT.into()), mod_api::PayloadBytes(body.into_bytes()));
}

/// The editor's "Use on NPC": its owner (or anyone, for an NPC without one) gives it a model and texture.
struct SetLook;

impl mod_api::IPayloadHandler for SetLook {
    fn handle(&self, payload: mod_api::PayloadBytes, context: &mut dyn mod_api::IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        let Ok(v) = crate::serde_json::from_slice::<crate::serde_json::Value>(&payload.0) else { return Ok(()) };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let (Some(model), Some(texture)) = (s("model"), s("texture")) else { return Ok(()) };
        let id = v.get("npc").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        let Some(npc) = host.entity(id).filter(|e| is_npc(&*host, *e)) else { return Ok(()) };
        let owner = LivingRef::new(npc).owner(&*host);
        if owner.is_some() && owner != Some(player.uuid()) {
            let _ = player.action_bar(host, Text::literal("Only its owner can change how this NPC looks."));
            return Ok(());
        }
        let _ = LOOK.set(host, Holder::Entity(id), Some(Look { model: model.clone(), texture }));
        let _ = player.action_bar(host, Text::literal(format!("The NPC now looks like {model}.")));
        Ok(())
    }
}

// ---- commands -----------------------------------------------------------------------------------

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix spawn", 2));
    e.commands.push(Command::op("felix look <targets:entities> <spec:text>", 2));
    e.commands.push(Command::op("felix order <targets:entities> <order:word>", 2));
    e.commands.push(Command::op("felix dialog <targets:entities> <dialog:text>", 2));
    e.commands.push(Command::op("felix scene <targets:entities> <scene:text>", 2));
    e.commands.push(Command::op("felix edit <target:entity> [name:word]", 2));
}

fn uuid_tag(uuid: u128) -> Tag {
    Tag::IntArray(vec![(uuid >> 96) as i32, (uuid >> 64) as i32, (uuid >> 32) as i32, uuid as i32])
}

pub(crate) fn targets(e: &RunModCommandEvent<'_>) -> Vec<EntityRef> {
    e.command
        .arguments
        .iter()
        .find(|a| a.name == "targets")
        .map(|a| match &a.value {
            CommandArgumentValue::Entities(list) => list.clone(),
            CommandArgumentValue::Entity(one) => vec![*one],
            _ => Vec::new(),
        })
        .unwrap_or_default()
}

pub(crate) fn text_argument(e: &RunModCommandEvent<'_>, name: &str) -> Option<String> {
    e.command.arguments.iter().find(|a| a.name == name).and_then(|a| match &a.value {
        CommandArgumentValue::String(s) => Some(s.clone()),
        _ => None,
    })
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let feedback = |host: &mut dyn ServerHost, line: String| {
        let _ = host.commands().feedback(Text::literal(line));
    };
    match lit.as_slice() {
        ["felix", "spawn"] => {
            let CommandSource::Player { player, .. } = e.source else {
                feedback(host, "Only a player can have a companion.".into());
                return Flow::Cancel;
            };
            let player = *player;
            let at = e.command.position;
            let mut nbt = Compound::new();
            nbt.insert("Owner".into(), uuid_tag(player.uuid()));
            nbt.insert("PersistenceRequired".into(), Tag::Byte(1));
            let dim = e.command.dimension.id(&*host).unwrap_or_else(|| "minecraft:overworld".into());
            match host.summon(&dim, NPC, at, nbt) {
                Some(id) => feedback(host, format!("Your companion is here (entity {id}).")),
                None => feedback(host, "The companion could not be summoned here.".into()),
            }
        }
        ["felix", "look"] => {
            let spec = text_argument(e, "spec").unwrap_or_default();
            let mut words = spec.split_whitespace();
            let Some(model) = words.next() else { return Flow::Cancel };
            let mut look = Look::of_model(model);
            if let Some(texture) = words.next() {
                look.texture = texture.to_string();
            }
            let mut n = 0;
            for t in targets(e) {
                if LOOK.set(host, Holder::Entity(t.id()), Some(look.clone())).is_ok() {
                    n += 1;
                }
            }
            feedback(host, format!("{n} entities now look like {} with {}", look.model, look.texture));
        }
        ["felix", "order"] => {
            let mode = match text_argument(e, "order").as_deref() {
                Some("follow") => Mode::Follow,
                Some("wait") => Mode::Wait,
                Some("guard") => Mode::Guard,
                Some("live") => Mode::Live,
                _ => {
                    feedback(host, "Orders: follow, wait, guard, live".into());
                    return Flow::Cancel;
                }
            };
            let mut n = 0;
            for t in targets(e) {
                if set_orders(host, t, mode) {
                    n += 1;
                }
            }
            feedback(host, format!("{n} companions: {mode:?}"));
        }
        ["felix", "dialog"] => {
            let id = text_argument(e, "dialog").unwrap_or_default().trim().to_string();
            let mut n = 0;
            for t in targets(e) {
                if crate::talk::DIALOG.set(host, Holder::Entity(t.id()), Some(id.clone())).is_ok() {
                    n += 1;
                }
            }
            let note = if crate::talk::dialog_exists(&id) { "" } else { " (no such dialog is loaded yet)" };
            feedback(host, format!("{n} NPCs now speak {id}{note}"));
        }
        ["felix", "edit"] => {
            let CommandSource::Player { player, .. } = e.source else {
                feedback(host, "Only a player can edit.".into());
                return Flow::Cancel;
            };
            let player = *player;
            let target = e.command.arguments.iter().find(|a| a.name == "target").and_then(|a| match &a.value {
                CommandArgumentValue::Entity(t) => Some(*t),
                CommandArgumentValue::Entities(l) => l.first().copied(),
                _ => None,
            });
            let Some(target) = target.filter(|t| is_npc(&*host, *t)) else {
                feedback(host, "That is not an NPC.".into());
                return Flow::Cancel;
            };
            open_editor(host, target, player, text_argument(e, "name"));
        }
        ["felix", "scene"] => {
            let id = text_argument(e, "scene").unwrap_or_default().trim().to_string();
            let player = match e.source {
                CommandSource::Player { player, .. } => Some(*player),
                _ => None,
            };
            let mut n = 0;
            let mut why = String::new();
            for t in targets(e) {
                match crate::scenes::start(host, t, &id, player) {
                    Ok(()) => n += 1,
                    Err(err) => why = err,
                }
            }
            let note = if why.is_empty() { String::new() } else { format!(" ({why})") };
            feedback(host, format!("{n} NPCs play {id}{note}"));
        }
        _ => return Flow::Continue,
    }
    Flow::Cancel
}
