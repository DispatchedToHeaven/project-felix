//! A player's squad: up to three NPCs of theirs following them at once. Keys (Options >
//! Controls, Project Felix): attack what the player looks at (G), fall back to the player and stop
//! fighting (H), hold where they are (J); `/felix squad attack|back|hold|list` too. An NPC with a
//! bow in hand shoots from where it is instead of running in; one with a shield takes less from
//! each hit (see `gear`).
use crate::companion::{self, Mode};
use mod_api::{
    ClientHost, Command, CommandSource, EntityRef, EquipmentSlot, Flow, IPayloadContext, IPayloadHandler, KeyRegistration,
    LivingRef, MobRef, PayloadBytes, PayloadId, PlayerRef, RegisterCommandsEvent, Registrar,
    RunModCommandEvent, ServerHost, SoundSource, Text,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

const MOD: &str = "projectfelix";
/// Client to server: a squad order (one byte: 0 attack, 1 fall back, 2 hold).
pub const ORDER: &str = "projectfelix:squad";
pub const MAX: usize = 3;
const KEYS: [(&str, &str); 3] = [("squad_attack", "key.keyboard.g"), ("squad_back", "key.keyboard.h"), ("squad_hold", "key.keyboard.j")];
/// How far archers shoot, and ticks between shots.
const RANGE: f64 = 24.0;
const RELOAD: i64 = 25;

#[derive(Default)]
struct State {
    /// What each archer shoots at, and when it last did.
    aims: HashMap<u32, (u32, i64)>,
    /// Members told to hold, by player.
    holding: HashMap<u32, u128>,
    /// No fighting before this, after falling back.
    calm: HashMap<u32, i64>,
    ticks: u32,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);
/// Every loaded NPC's owner, for hits between friends (no game host there).
static OWNERS: LazyLock<Mutex<HashMap<u32, u128>>> = LazyLock::new(Default::default);

/// Whether entities `a` and `b` are NPCs of the same owner (they don't hurt each other).
pub fn friendly(a: u32, b: u32) -> bool {
    OWNERS.lock().is_ok_and(|o| o.get(&a).is_some_and(|x| o.get(&b) == Some(x)))
}

pub fn register(r: &mut dyn Registrar) {
    for (id, key) in KEYS {
        r.register_key_mapping(KeyRegistration::new(id, key).label(&format!("key.projectfelix.{id}")).category("key.categories.projectfelix"));
    }
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_server(PayloadId(ORDER.into()), Arc::new(OrderHandler));
}

/// The client tick: the squad keys pressed go to the server.
pub fn client_tick(host: &mut dyn ClientHost) {
    for (i, (id, _)) in KEYS.iter().enumerate() {
        while host.consume_click(MOD, id) {
            if host.mod_screen().is_none() {
                let _ = host.send_payload_to_server(&PayloadId(ORDER.into()), PayloadBytes(vec![i as u8]));
            }
        }
    }
}

struct OrderHandler;

impl IPayloadHandler for OrderHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        match payload.0.first() {
            Some(0) => attack(host, player),
            Some(1) => fall_back(host, player),
            Some(2) => hold(host, player),
            _ => {}
        }
        Ok(())
    }
}

/// Player `p`'s squad: their NPCs following them, or holding for them.
pub fn members(host: &dyn mod_api::Host, p: PlayerRef) -> Vec<EntityRef> {
    let holding: HashSet<u32> = STATE.lock().map(|s| s.holding.iter().filter(|(_, u)| **u == p.uuid()).map(|(id, _)| *id).collect()).unwrap_or_default();
    let level = p.entity().level(host);
    companion::npcs()
        .into_iter()
        .filter_map(|id| host.entity(id))
        .filter(|e| e.level(host) == level && LivingRef::new(*e).owner(host) == Some(p.uuid()))
        .filter(|e| companion::mode_of(e.id()) == Mode::Follow || holding.contains(&e.id()))
        .collect()
}

/// Whether player `p` has a full squad without NPC `npc`.
pub fn full(host: &dyn mod_api::Host, p: PlayerRef, npc: EntityRef) -> bool {
    members(host, p).iter().filter(|e| e.id() != npc.id()).count() >= MAX
}

fn tell(host: &mut dyn ServerHost, p: PlayerRef, line: &str) {
    let _ = p.action_bar(host, Text::literal(line.to_string()));
}

fn has_bow(host: &dyn mod_api::Host, e: EntityRef) -> bool {
    LivingRef::new(e).equipment(host, EquipmentSlot::MainHand).is_some_and(|s| matches!(s.item().id(host).as_str(), "minecraft:bow" | "minecraft:crossbow"))
}

/// What player `p` looks at, within 32 blocks: a living thing, not theirs.
fn looked_at(host: &dyn mod_api::Host, p: PlayerRef, mine: &[EntityRef]) -> Option<EntityRef> {
    let me = p.entity();
    let (eye, look, level) = (me.eye_position(host)?, me.view_vector(host)?, me.level(host)?);
    let to = [eye[0] + look[0] * 32.0, eye[1] + look[1] * 32.0, eye[2] + look[2] * 32.0];
    let mut hits = level.ray_trace_entities(host, eye, to, 0.3, Some(me))?;
    hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    hits.into_iter().map(|h| h.entity).find(|e| !mine.iter().any(|m| m.id() == e.id()) && LivingRef::new(*e).health(host).is_some())
}

pub fn attack(host: &mut dyn ServerHost, p: PlayerRef) {
    let mine = members(&*host, p);
    if mine.is_empty() {
        tell(host, p, "No one is with you.");
        return;
    }
    let Some(target) = looked_at(&*host, p, &mine) else {
        tell(host, p, "Look at what they should attack.");
        return;
    };
    let now = p.entity().level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
    for m in &mine {
        if let Ok(mut s) = STATE.lock() {
            s.calm.remove(&m.id());
        }
        if has_bow(&*host, *m) {
            if let Ok(mut s) = STATE.lock() {
                s.aims.insert(m.id(), (target.id(), now - RELOAD));
            }
        } else {
            MobRef::new(*m).set_target(host, Some(LivingRef::new(target)));
        }
    }
    let name = target.name(&*host).unwrap_or_else(|| "that".into());
    crate::heart::say_aloud(host, mine[0], &format!("On it! Get the {name}!"));
}

pub fn fall_back(host: &mut dyn ServerHost, p: PlayerRef) {
    let mine = members(&*host, p);
    let now = p.entity().level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
    for m in &mine {
        MobRef::new(*m).set_target(host, None);
        if let Ok(mut s) = STATE.lock() {
            s.aims.remove(&m.id());
            s.holding.remove(&m.id());
            s.calm.insert(m.id(), now + 300);
        }
        companion::set_orders(host, *m, Mode::Follow);
    }
    match mine.first() {
        Some(m) => crate::heart::say_aloud(host, *m, "Falling back!"),
        None => tell(host, p, "No one is with you."),
    }
}

pub fn hold(host: &mut dyn ServerHost, p: PlayerRef) {
    let mine = members(&*host, p);
    for m in &mine {
        companion::set_orders(host, *m, Mode::Wait);
        if let Ok(mut s) = STATE.lock() {
            s.holding.insert(m.id(), p.uuid());
        }
    }
    match mine.first() {
        Some(m) => crate::heart::say_aloud(host, *m, "Holding here."),
        None => tell(host, p, "No one is with you."),
    }
}

/// The server tick: archers shoot at what they aim at (and don't run in), squads fallen back keep
/// out of fights for a while.
pub fn tick(host: &mut dyn ServerHost) {
    let step = STATE.lock().map(|mut s| {
        s.ticks = s.ticks.wrapping_add(1);
        s.ticks % 2 == 0
    });
    if !matches!(step, Ok(true)) {
        return;
    }
    let npcs = companion::npcs();
    let owners: HashMap<u32, u128> = npcs.iter().filter_map(|id| Some((*id, LivingRef::new(host.entity(*id)?).owner(&*host)?))).collect();
    if let Ok(mut o) = OWNERS.lock() {
        *o = owners;
    }
    for id in &npcs {
        let Some(me) = host.entity(*id) else { continue };
        // Friends don't fight each other, whoever hit whom.
        if let Some(t) = MobRef::new(me).target(&*host)
            && friendly(*id, t.entity().id())
        {
            MobRef::new(me).set_target(host, None);
        }
        let Some(level) = me.level(&*host) else { continue };
        let Some(now) = level.game_time(&*host) else { continue };
        let calm = STATE.lock().ok().and_then(|s| s.calm.get(id).copied()).is_some_and(|t| now < t);
        let mob = MobRef::new(me);
        if calm {
            if mob.target(&*host).is_some() {
                mob.set_target(host, None);
            }
            continue;
        }
        if !has_bow(&*host, me) {
            continue;
        }
        // An archer keeps its distance: what it would run at, it shoots at.
        if let Some(t) = mob.target(&*host) {
            mob.set_target(host, None);
            if let Ok(mut s) = STATE.lock() {
                let last = s.aims.get(id).map_or(now - RELOAD, |a| a.1);
                s.aims.insert(*id, (t.entity().id(), last));
            }
        }
        let Some((tid, last)) = STATE.lock().ok().and_then(|s| s.aims.get(id).copied()) else { continue };
        let target = host.entity(tid).filter(|t| LivingRef::new(*t).alive(&*host) == Some(true));
        let (Some(target), Some(at)) = (target, me.position(&*host)) else {
            if let Ok(mut s) = STATE.lock() {
                s.aims.remove(id);
            }
            continue;
        };
        let Some(to) = target.position(&*host) else { continue };
        let (dx, dz) = (to[0] - at[0], to[2] - at[2]);
        let flat = (dx * dx + dz * dz).sqrt();
        if flat > RANGE * 1.5 {
            if let Ok(mut s) = STATE.lock() {
                s.aims.remove(id);
            }
            continue;
        }
        let _ = mob.look_at(host, [to[0], to[1] + 1.2, to[2]]);
        if flat > RANGE || now - last < RELOAD || LivingRef::new(me).has_line_of_sight(&*host, target) != Some(true) {
            continue;
        }
        if let Ok(mut s) = STATE.lock() {
            s.aims.insert(*id, (tid, now));
        }
        shoot(host, me, level, at, to, target);
    }
    if let Ok(mut s) = STATE.lock() {
        s.aims.retain(|id, _| npcs.contains(id));
        s.calm.retain(|id, t| npcs.contains(id) && *t > 0);
        s.holding.retain(|id, _| npcs.contains(id) && companion::mode_of(*id) == Mode::Wait);
    }
}

/// An arrow from NPC `me` at `at` towards `target` at `to`, aimed over the drop.
fn shoot(host: &mut dyn ServerHost, me: EntityRef, level: mod_api::LevelRef, at: [f64; 3], to: [f64; 3], target: EntityRef) {
    let from = [at[0], at[1] + 1.5, at[2]];
    let aim = target.eye_position(&*host).map_or([to[0], to[1] + 1.0, to[2]], |e| [e[0], e[1] - 0.3, e[2]]);
    let (dx, dy, dz) = (aim[0] - from[0], aim[1] - from[1], aim[2] - from[2]);
    let flat = (dx * dx + dz * dz).sqrt().max(0.01);
    let speed = 2.4;
    let drop = 0.0045 * flat * flat;
    let len = (dx * dx + (dy + drop).powi(2) + dz * dz).sqrt().max(0.01);
    let m = [dx / len * speed, (dy + drop) / len * speed, dz / len * speed];
    let start = [from[0] + dx / flat * 0.7, from[1], from[2] + dz / flat * 0.7];
    let owner = me.uuid(host).map(|u| format!(",Owner:[I;{},{},{},{}]", (u >> 96) as i32, (u >> 64) as i32, (u >> 32) as i32, u as i32)).unwrap_or_default();
    crate::talk::queue(format!(
        "execute in {} run summon minecraft:arrow {:.3} {:.3} {:.3} {{Motion:[{:.3},{:.3},{:.3}],pickup:0b{owner}}}",
        level.dimension_id(),
        start[0],
        start[1],
        start[2],
        m[0],
        m[1],
        m[2]
    ));
    let _ = LivingRef::new(me).swing(host, mod_api::Hand::Main, false);
    let _ = level.play_sound(host, "minecraft:entity.arrow.shoot", from, SoundSource::Neutral, 0.8, 1.1);
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix squad <order:word>", 0));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if lit.as_slice() != ["felix", "squad"] {
        return Flow::Continue;
    }
    let CommandSource::Player { player, .. } = e.source else { return Flow::Cancel };
    let p = *player;
    match companion::text_argument(e, "order").as_deref() {
        Some("attack") => attack(host, p),
        Some("back") => fall_back(host, p),
        Some("hold") => hold(host, p),
        _ => {
            let names: Vec<String> = members(&*host, p).iter().map(|m| {
                let name = m.name(&*host).unwrap_or_else(|| "NPC".into());
                let what = if has_bow(&*host, *m) { "bow" } else { "melee" };
                format!("{name} ({what}{})", if companion::mode_of(m.id()) == Mode::Wait { ", holding" } else { "" })
            }).collect();
            let line = format!("Squad {}/{MAX}: {}. Orders: attack (G), back (H), hold (J)", names.len(), if names.is_empty() { "no one".into() } else { names.join(", ") });
            let _ = host.commands().feedback(Text::literal(line));
        }
    }
    Flow::Cancel
}
