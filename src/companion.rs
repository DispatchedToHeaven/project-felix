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
}

impl Mode {
    pub fn byte(self) -> u8 {
        match self {
            Mode::Follow => 0,
            Mode::Wait => 1,
            Mode::Guard => 2,
        }
    }
    pub fn from_byte(b: u8) -> Mode {
        match b {
            1 => Mode::Wait,
            2 => Mode::Guard,
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
}

static SERVER: LazyLock<Mutex<Server>> = LazyLock::new(Default::default);
static NPC_KEY: OnceLock<EntityTypeKey> = OnceLock::new();

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
}

/// Server level tick: reads the orders of NPCs the goals asked about, and points guards at the
/// monsters near their posts (each tick); the wolf's own attack goals do the fighting.
pub fn level_tick(host: &mut dyn ServerHost) {
    let guards: Vec<(u32, [f64; 3])> = match SERVER.lock() {
        Ok(s) => s.orders.iter().filter(|(_, o)| o.mode == Mode::Guard).map(|(id, o)| (*id, o.post)).collect(),
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
        let Ok(o) = ORDERS.get(host, Holder::Entity(id)) else { continue };
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
        goals.add_goal(1, Wait);
        goals.add_goal(1, KeepPost);
    }
}

fn distance_sq(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
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

// ---- commands -----------------------------------------------------------------------------------

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix spawn", 2));
    e.commands.push(Command::op("felix look <targets:entities> <spec:text>", 2));
    e.commands.push(Command::op("felix order <targets:entities> <order:word>", 2));
    e.commands.push(Command::op("felix dialog <targets:entities> <dialog:text>", 2));
    e.commands.push(Command::op("felix scene <targets:entities> <scene:text>", 2));
}

fn uuid_tag(uuid: u128) -> Tag {
    Tag::IntArray(vec![(uuid >> 96) as i32, (uuid >> 64) as i32, (uuid >> 32) as i32, uuid as i32])
}

fn targets(e: &RunModCommandEvent<'_>) -> Vec<EntityRef> {
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

fn text_argument(e: &RunModCommandEvent<'_>, name: &str) -> Option<String> {
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
                _ => {
                    feedback(host, "Orders: follow, wait, guard".into());
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
