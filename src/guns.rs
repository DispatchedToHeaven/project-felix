//! Firearms on the server and in players' hands (see `gun`). A pull of the trigger (the use key)
//! on an empty gun loads it from the inventory (it is busy for as long as that takes); the reload
//! key (R) loads or tops it up at any time. A loaded gun fires: a shot a pull, or shot after shot
//! while the trigger is held. The bullet goes where they look, a little off (less crouching,
//! more as a burst goes on), to the first living thing in its way that no block hides, harder in
//! the head, with a bang, a flash, a streak and smoke. What is in a gun is kept in the gun: a
//! loaded flintlock glints, a modern gun shows its magazine as its bar. Crouching with a sniper
//! rifle looks through its scope. NPCs holding a gun shoot what they are after instead of running
//! at it (`squad` has them keep their distance), as well as their fighting lets them. Villagers
//! don't like gunfire about.
use crate::gun::{self, Arm, Gun, Pull};
use crate::skill::Skill;
use mod_api::{
    ClientHost, ClipBlock, ClipFluid, Compound, ComputeFovModifierEvent, EntityRef, Hand, HostSide, IPayloadContext, IPayloadHandler, InteractionResult, ItemStack, KeyRegistration, LevelRef,
    LivingRef, ParticleKey, ParticleOptions, PayloadBytes, PayloadId, PlayerRef, Registrar, ServerHost, SoundSource, Tag, Text, UseItemEvent,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

const MOD: &str = "projectfelix";
/// Client to server: the reload key was pressed.
pub const RELOAD: &str = "projectfelix:reload";
const RELOAD_KEY: (&str, &str) = ("reload", "key.keyboard.r");
/// What is in a gun (in its custom data): its shots, and when it is ready.
const ROUNDS: &str = "felix_rounds";
const READY: &str = "felix_ready";
/// (How 0.57 marked a loaded flintlock: with the time it is ready.)
const OLD: &str = "felix_loaded";
/// What a bullet's harm is: a shot, as an arrow's (armour and shields stop some of it, and the
/// one shot is "shot by" the shooter).
const BULLET: &str = "minecraft:arrow";
/// Ticks a held trigger keeps firing after the last sign of it.
const HELD: i64 = 6;

/// A trigger held: a gun firing by itself.
#[derive(Clone, Copy)]
struct Firing {
    gun: &'static Gun,
    hand: Hand,
    /// What the gun held when the burst began (how its stack is found again), and holds now.
    start: Arm,
    arm: Arm,
    until: i64,
    shots: u32,
}

#[derive(Default)]
struct State {
    /// The guns in NPCs' hands.
    arms: HashMap<u32, Arm>,
    firing: HashMap<u128, Firing>,
    seed: u64,
    /// When villagers were last frightened by a shot.
    scared: i64,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

struct Sparks {
    crit: ParticleKey,
    smoke: ParticleKey,
    flame: ParticleKey,
    cloud: ParticleKey,
}

static SPARKS: OnceLock<Sparks> = OnceLock::new();

pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        s.arms.clear();
        s.firing.clear();
        s.scared = 0;
    }
}

pub fn forget(id: u32) {
    if let Ok(mut s) = STATE.lock() {
        s.arms.remove(&id);
    }
}

/// A number from 0 up to 1.
fn roll() -> f64 {
    let Ok(mut s) = STATE.lock() else { return 0.5 };
    s.seed = s.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut x = s.seed;
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

struct Firearm;

#[mod_api::horizon_part]
impl mod_api::ItemPart for Firearm {
    fn use_item(&self, host: HostSide, e: &mut UseItemEvent<'_>) -> Option<InteractionResult> {
        // The client only knows the trigger was pulled; the server loads and fires. (No swing: a
        // gun is not waved about.)
        let HostSide::Server(host) = host else { return Some(InteractionResult::Consume) };
        let player = host.player(e.player)?;
        pulled(host, player, e.hand, e.stack);
        Some(InteractionResult::Consume)
    }
}

pub fn register(r: &mut dyn Registrar) {
    for g in gun::GUNS {
        let key = r.item_key(g.id);
        r.item_part(key, Firearm);
    }
    let _ = SPARKS.set(Sparks {
        crit: r.particle_key("minecraft:crit"),
        smoke: r.particle_key("minecraft:smoke"),
        flame: r.particle_key("minecraft:flame"),
        cloud: r.particle_key("minecraft:campfire_cosy_smoke"),
    });
    r.register_key_mapping(KeyRegistration::new(RELOAD_KEY.0, RELOAD_KEY.1).label("key.projectfelix.reload").category("key.categories.projectfelix"));
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_server(PayloadId(RELOAD.into()), Arc::new(ReloadHandler));
}

/// The client tick: the reload key pressed goes to the server.
pub fn client_tick(host: &mut dyn ClientHost) {
    while host.consume_click(MOD, RELOAD_KEY.0) {
        if host.mod_screen().is_none() {
            let _ = host.send_payload_to_server(&PayloadId(RELOAD.into()), PayloadBytes(Vec::new()));
        }
    }
}

struct ReloadHandler;

impl IPayloadHandler for ReloadHandler {
    fn handle(&self, _payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        let me = LivingRef::new(player.entity());
        // The gun in the main hand, else the one in the other.
        for hand in [Hand::Main, Hand::Off] {
            let Some(stack) = me.hand_item(&*host, hand) else { continue };
            let Some(g) = gun::gun(&stack.item().id(&*host)) else { continue };
            let Some(now) = player.entity().level(&*host).and_then(|l| l.game_time(&*host)) else { return Ok(()) };
            stop(host, player);
            let stack = me.hand_item(&*host, hand).unwrap_or(stack);
            let arm = arm_of(&stack);
            if !arm.busy(g, now) {
                reload(host, player, hand, &stack, g, arm, now);
            }
            break;
        }
        Ok(())
    }
}

/// `compute_fov_modifier`: crouching with a scoped gun in hand looks through its scope.
pub fn fov(host: &dyn ClientHost, e: &mut ComputeFovModifierEvent<'_>) {
    let Some(p) = host.local_player() else { return };
    let me = p.entity();
    if me.is_shift_key_down(host) != Some(true) {
        return;
    }
    let scoped = LivingRef::new(me).hand_item(host, Hand::Main).is_some_and(|s| gun::gun(&s.item().id(host)).is_some_and(|g| g.scope));
    if scoped {
        e.modifier *= 0.2;
    }
}

fn get<'a>(c: &'a Compound, key: &str) -> Option<&'a Tag> {
    c.iter().find(|(k, _)| k.to_string().is_ok_and(|s| s == key)).map(|(_, v)| v)
}

/// What gun `stack` holds: its shots, and when it is ready.
fn arm_of(stack: &ItemStack) -> Arm {
    let Some(Tag::Compound(data)) = stack.components().and_then(|c| get(c.as_nbt(), "minecraft:custom_data")) else { return Arm::default() };
    if let Some(Tag::Long(t)) = get(data, OLD) {
        return Arm { rounds: 1, busy_until: *t };
    }
    let rounds = match get(data, ROUNDS) {
        Some(Tag::Int(n)) => (*n).max(0) as u32,
        _ => 0,
    };
    let busy_until = match get(data, READY) {
        Some(Tag::Long(t)) => *t,
        _ => 0,
    };
    Arm { rounds, busy_until }
}

/// Gun `stack` holding `arm`: a loaded flintlock glints, a modern gun shows its magazine as its bar.
fn with_arm(mut stack: ItemStack, g: &Gun, arm: Arm) -> ItemStack {
    let mut patch = Compound::new();
    let mut data = Compound::new();
    data.insert(ROUNDS.into(), Tag::Int(arm.rounds as i32));
    data.insert(READY.into(), Tag::Long(arm.busy_until));
    patch.insert("minecraft:custom_data".into(), Tag::Compound(data));
    if g.flintlock {
        if arm.rounds > 0 {
            patch.insert("minecraft:enchantment_glint_override".into(), Tag::Byte(1));
        } else {
            patch.insert("!minecraft:enchantment_glint_override".into(), Tag::Compound(Compound::new()));
        }
    } else {
        patch.insert("minecraft:max_damage".into(), Tag::Int(g.magazine as i32));
        patch.insert("minecraft:damage".into(), Tag::Int(g.magazine.saturating_sub(arm.rounds) as i32));
    }
    let _ = stack.apply_components(&patch);
    stack
}

/// How many of `item` the player carries.
fn carried(host: &dyn mod_api::Host, player: PlayerRef, item: &str) -> u32 {
    player.inventory_slots(host).unwrap_or_default().iter().filter(|(_, s)| s.item().id(host) == item).map(|(_, s)| s.count()).sum()
}

/// Takes `n` of `item` from the player's inventory.
fn take(host: &mut dyn ServerHost, player: PlayerRef, item: &str, mut n: u32) {
    for (slot, stack) in player.inventory_slots(&*host).unwrap_or_default() {
        if n == 0 {
            break;
        }
        if stack.item().id(&*host) == item {
            let k = stack.count().min(n);
            if player.set_inventory_slot(host, slot, stack.shrink(k)).is_ok() {
                n -= k;
            }
        }
    }
}

fn say(host: &mut dyn ServerHost, player: PlayerRef, line: &str) {
    let _ = player.action_bar(host, Text::literal(line.to_string()));
}

fn wet(host: &dyn mod_api::Host, level: LevelRef, at: [f64; 3]) -> bool {
    level.is_raining_at(host, [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32]) == Some(true)
}

/// The gun in `hand` (`stack`, holding `arm`) is loaded, or topped up, from the player's inventory.
fn reload(host: &mut dyn ServerHost, player: PlayerRef, hand: Hand, stack: &ItemStack, g: &'static Gun, mut arm: Arm, now: i64) {
    let me = player.entity();
    let at = me.eye_position(&*host).unwrap_or([0.0; 3]);
    let Some(level) = me.level(&*host) else { return };
    let free = player.is_creative(&*host) == Some(true);
    let have = if free { g.magazine } else { carried(&*host, player, g.ammo) };
    match arm.load(g, now, have) {
        Pull::Load { taken, .. } => {
            if !free {
                take(host, player, g.ammo, taken);
            }
            let _ = LivingRef::new(me).set_hand_item(host, hand, Some(with_arm(stack.clone(), g, arm)));
            player.set_item_cooldown(host, g.id, g.reload as i32);
            let sound = if g.flintlock { "minecraft:item.crossbow.loading_middle" } else { "minecraft:block.iron_door.close" };
            let _ = level.play_sound(host, sound, at, SoundSource::Players, 0.8, if g.flintlock { 0.8 } else { 1.6 });
            let line = if g.flintlock { format!("Loading the {}...", g.name) } else { format!("Reloading the {}... {}/{}", g.name, arm.rounds, g.magazine) };
            say(host, player, &line);
        }
        Pull::NoAmmo => {
            let _ = level.play_sound(host, "minecraft:block.dispenser.fail", at, SoundSource::Players, 0.6, 1.4);
            say(host, player, &format!("No {}.", gun::ammo_name(g.ammo)));
        }
        _ => say(host, player, &format!("The {} is full.", g.name)),
    }
}

/// A player pulled the trigger of the gun `stack` in `hand`.
fn pulled(host: &mut dyn ServerHost, player: PlayerRef, hand: Hand, stack: &ItemStack) {
    let id = stack.item().id(&*host);
    let Some(g) = gun::gun(&id) else { return };
    let me = player.entity();
    let (Some(level), Some(eye), Some(aim)) = (me.level(&*host), me.eye_position(&*host), me.view_vector(&*host)) else { return };
    let now = level.game_time(&*host).unwrap_or(0);
    // A trigger still held: the burst goes on (see `tick`).
    if let Ok(mut s) = STATE.lock()
        && let Some(f) = s.firing.get_mut(&player.uuid()).filter(|f| f.gun.id == g.id && f.hand == hand)
    {
        f.until = now + HELD;
        return;
    }
    let mut arm = arm_of(stack);
    if arm.busy(g, now) {
        if arm.busy_until - now > 10 {
            say(host, player, &format!("Still loading the {}...", g.name));
        }
        return;
    }
    if arm.rounds == 0 {
        reload(host, player, hand, stack, g, arm, now);
        return;
    }
    if g.auto {
        if let Ok(mut s) = STATE.lock() {
            s.firing.insert(player.uuid(), Firing { gun: g, hand, start: arm, arm, until: now + HELD, shots: 0 });
        }
        return;
    }
    let damp = g.flintlock && wet(&*host, level, eye);
    let pull = arm.pull(g, now, 0, g.flintlock && gun::misfires(damp, roll() as f32));
    let _ = LivingRef::new(me).set_hand_item(host, hand, Some(with_arm(stack.clone(), g, arm)));
    player.set_item_cooldown(host, g.id, g.cycle as i32);
    match pull {
        Pull::Fire => {
            let steady = me.is_shift_key_down(&*host) == Some(true);
            let head = fire(host, me, level, eye, aim, g, gun::spread_now(g, 0, steady));
            told(host, player, g, arm, head);
        }
        Pull::Misfire => {
            let _ = level.play_sound(host, "minecraft:block.fire.extinguish", eye, SoundSource::Players, 0.5, 1.6);
            say(host, player, if damp { "The powder is damp: it didn't catch." } else { "A flash in the pan: it didn't catch." });
        }
        _ => {}
    }
}

/// What the shooter is told after a shot: a hit in the head, and what is left in a magazine.
fn told(host: &mut dyn ServerHost, player: PlayerRef, g: &Gun, arm: Arm, head: bool) {
    let left = if g.magazine > 1 { format!("{} {}/{}", g.name, arm.rounds, g.magazine) } else { String::new() };
    let line = match (head, left.is_empty()) {
        (true, true) => "Headshot!".to_string(),
        (true, false) => format!("Headshot! {left}"),
        (false, _) => left,
    };
    if !line.is_empty() {
        say(host, player, &line);
    }
}

/// A burst ends: what is left goes back into the gun it came from, wherever the player put it.
fn stop(host: &mut dyn ServerHost, player: PlayerRef) {
    let Some(f) = STATE.lock().ok().and_then(|mut s| s.firing.remove(&player.uuid())) else { return };
    if f.arm == f.start {
        return;
    }
    for (slot, stack) in player.inventory_slots(&*host).unwrap_or_default() {
        if stack.item().id(&*host) == f.gun.id && arm_of(&stack) == f.start {
            let _ = player.set_inventory_slot(host, slot, Some(with_arm(stack, f.gun, f.arm)));
            return;
        }
    }
}

/// The tick of each level: the guns of its players whose triggers are held fire; a trigger let
/// go, a gun put away or run dry ends the burst.
pub fn tick(host: &mut dyn ServerHost) {
    let firing: Vec<(u128, Firing)> = STATE.lock().map(|s| s.firing.iter().map(|(u, f)| (*u, *f)).collect()).unwrap_or_default();
    for (uuid, mut f) in firing {
        let Some(player) = host.player(uuid) else {
            if let Ok(mut s) = STATE.lock() {
                s.firing.remove(&uuid);
            }
            continue;
        };
        let me = player.entity();
        if !crate::pace::here(&*host, me.id()) {
            continue;
        }
        let Some(level) = me.level(&*host) else { continue };
        let now = level.game_time(&*host).unwrap_or(0);
        let held = LivingRef::new(me).hand_item(&*host, f.hand).is_some_and(|s| s.item().id(&*host) == f.gun.id);
        if now > f.until || !held {
            stop(host, player);
            continue;
        }
        if f.arm.busy(f.gun, now) {
            continue;
        }
        if f.arm.rounds == 0 {
            stop(host, player);
            if let Some(at) = me.eye_position(&*host) {
                let _ = level.play_sound(host, "minecraft:block.dispenser.fail", at, SoundSource::Players, 0.6, 1.8);
            }
            say(host, player, &format!("The {} is empty. Reload (R).", f.gun.name));
            continue;
        }
        let (Some(eye), Some(aim)) = (me.eye_position(&*host), me.view_vector(&*host)) else { continue };
        if f.arm.pull(f.gun, now, 0, false) != Pull::Fire {
            continue;
        }
        let steady = me.is_shift_key_down(&*host) == Some(true);
        let head = fire(host, me, level, eye, aim, f.gun, gun::spread_now(f.gun, f.shots, steady));
        f.shots += 1;
        if let Ok(mut s) = STATE.lock()
            && let Some(x) = s.firing.get_mut(&uuid)
        {
            // (The trigger may have been pulled again meanwhile: its time is kept.)
            x.arm = f.arm;
            x.shots = f.shots;
        }
        told(host, player, f.gun, f.arm, head);
    }
}

fn spark(host: &mut dyn ServerHost, level: LevelRef, key: ParticleKey, at: [f64; 3], spread: f64, speed: f64, count: i32) {
    let _ = level.send_particles(host, &ParticleOptions::simple(key), None, at, [spread; 3], speed, count);
}

/// A shot from `shooter`'s gun `g`, from `from` along `aim`, `spread` degrees off at most:
/// whether it hit something in the head.
fn fire(host: &mut dyn ServerHost, shooter: EntityRef, level: LevelRef, from: [f64; 3], aim: [f64; 3], g: &Gun, spread: f64) -> bool {
    let mut hits: Vec<(EntityRef, f32)> = Vec::new();
    let mut head = false;
    for k in 0..g.pellets {
        let d = gun::scatter(aim, spread, roll(), roll());
        let to = [from[0] + d[0] * g.range, from[1] + d[1] * g.range, from[2] + d[2] * g.range];
        // As far as the first block in the way.
        let wall = level.clip(&*host, from, to, ClipBlock::Collider, ClipFluid::None).filter(|c| !c.miss);
        let reach = wall.as_ref().map_or(g.range, |c| ((c.location[0] - from[0]).powi(2) + (c.location[1] - from[1]).powi(2) + (c.location[2] - from[2]).powi(2)).sqrt());
        let mut found = level.ray_trace_entities(&*host, from, to, 0.2, Some(shooter)).unwrap_or_default();
        found.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        let hit = found.into_iter().find(|h| h.distance <= reach && LivingRef::new(h.entity).alive(&*host) == Some(true));
        let end = hit.as_ref().map_or(reach, |h| h.distance);
        // Its streak (of a handful of shot, a few), and where it struck a block.
        if let Some(sp) = SPARKS.get() {
            if k < 3 {
                let mut at = 3.0;
                while at < end.min(48.0) {
                    spark(host, level, sp.crit, [from[0] + d[0] * at, from[1] + d[1] * at - 0.1, from[2] + d[2] * at], 0.0, 0.0, 1);
                    at += 4.0;
                }
            }
            if hit.is_none()
                && let Some(c) = &wall
            {
                spark(host, level, sp.smoke, c.location, 0.05, 0.01, 3);
            }
        }
        let Some(h) = hit else { continue };
        let mut harm = gun::damage_at(g, h.distance);
        if g.pellets == 1 && h.entity.eye_position(&*host).is_some_and(|e| gun::in_head(h.location[1], e[1])) {
            harm *= gun::HEADSHOT;
            head = true;
        }
        match hits.iter_mut().find(|x| x.0.id() == h.entity.id()) {
            Some(x) => x.1 += harm,
            None => hits.push((h.entity, harm)),
        }
    }
    bang(host, level, from, aim, g, shooter);
    // (All the shot that found one body is one blow.)
    for (e, harm) in &hits {
        strike(host, *e, *harm, shooter);
    }
    head
}

/// A bullet of `shooter`'s hits `e`: each one tells, however fast they come (a body just hurt
/// shrugs off other blows for a moment, but not bullets).
fn strike(host: &mut dyn ServerHost, e: EntityRef, harm: f32, shooter: EntityRef) {
    e.set_damage_cooldown_time(host, 0);
    let _ = e.hurt(host, BULLET, harm, Some(shooter), Some(shooter));
}

/// The bang, the flash and the smoke of gun `g` fired from `from` along `aim`; villagers about
/// don't like it.
fn bang(host: &mut dyn ServerHost, level: LevelRef, from: [f64; 3], aim: [f64; 3], g: &Gun, shooter: EntityRef) {
    // A sound carries 16 blocks for each point of its volume.
    let loud = (g.heard / 16.0) as f32;
    let vary = roll() as f32 * 0.15;
    if g.flintlock {
        let _ = level.play_sound(host, "minecraft:entity.generic.explode", from, SoundSource::Players, loud, 1.7 + vary);
        let _ = level.play_sound(host, "minecraft:entity.firework_rocket.blast", from, SoundSource::Players, loud, 0.7);
    } else if g.scope || g.pellets > 1 {
        let _ = level.play_sound(host, "minecraft:entity.firework_rocket.large_blast", from, SoundSource::Players, loud, 0.8 + vary);
        let _ = level.play_sound(host, "minecraft:entity.generic.explode", from, SoundSource::Players, loud * 0.5, 1.9);
    } else {
        let _ = level.play_sound(host, "minecraft:entity.firework_rocket.blast", from, SoundSource::Players, loud, 1.5 + vary);
        let _ = level.play_sound(host, "minecraft:entity.generic.explode", from, SoundSource::Players, loud * 0.25, 2.0);
    }
    let m = [from[0] + aim[0] * 1.2, from[1] + aim[1] * 1.2 - 0.1, from[2] + aim[2] * 1.2];
    if let Some(sp) = SPARKS.get() {
        spark(host, level, sp.flame, m, 0.04, 0.01, if g.flintlock { 4 } else { 1 });
        if g.flintlock {
            spark(host, level, sp.cloud, m, 0.15, 0.01, 8);
        } else {
            spark(host, level, sp.smoke, m, 0.05, 0.01, 2);
        }
    }
    scare(host, level, from, g, shooter);
}

/// Villagers within half the hearing of a shot don't like it (told now and then, not each shot;
/// not those with a gun of their own, nor while raiders are about: then it is their defence).
fn scare(host: &mut dyn ServerHost, level: LevelRef, from: [f64; 3], g: &Gun, shooter: EntityRef) {
    let now = level.game_time(&*host).unwrap_or(0);
    let due = STATE.lock().is_ok_and(|mut s| {
        let due = now - s.scared >= 200 || now < s.scared;
        if due {
            s.scared = now;
        }
        due
    });
    if !due {
        return;
    }
    let near = (g.heard / 2.0).powi(2);
    for id in crate::companion::npcs() {
        let Some(e) = host.entity(id).filter(|e| e.id() != shooter.id() && e.level(&*host) == Some(level)) else { continue };
        let close = e.position(&*host).is_some_and(|p| (p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2) + (p[2] - from[2]).powi(2) <= near);
        if !close || held(&*host, e).is_some() || LivingRef::new(e).owner(&*host).is_some() {
            continue;
        }
        if crate::chiefs::village_of(host, e).is_some_and(|v| !crate::storyteller::raided_now(&v)) {
            crate::minds::remember(host, e, "gunfire");
        }
    }
}

/// The gun NPC `e` holds, if it holds one.
pub fn held(host: &dyn mod_api::Host, e: EntityRef) -> Option<&'static Gun> {
    let s = LivingRef::new(e).equipment(host, mod_api::EquipmentSlot::MainHand)?;
    gun::gun(&s.item().id(host))
}

/// NPC `me`, with gun `g` in hand and `target` in sight `dist` away, pulls the trigger at `now`:
/// it loads (it never runs out), or it fires (a burst, from a gun that fires by itself), and hits
/// as its fighting lets it.
pub fn npc_pull(host: &mut dyn ServerHost, me: EntityRef, target: EntityRef, g: &'static Gun, now: i64, dist: f64) {
    let (Some(level), Some(from)) = (me.level(&*host), me.eye_position(&*host)) else { return };
    let Some(to) = target.position(&*host) else { return };
    let mut arm = STATE.lock().ok().and_then(|s| s.arms.get(&me.id()).copied()).unwrap_or_default();
    let damp = g.flintlock && wet(&*host, level, from);
    let aim = [to[0] - from[0], to[1] + 1.0 - from[1], to[2] - from[2]];
    let l = (aim[0] * aim[0] + aim[1] * aim[1] + aim[2] * aim[2]).sqrt().max(1e-6);
    let aim = [aim[0] / l, aim[1] / l, aim[2] / l];
    let fighting = crate::skills::level(host, me, Skill::Fighting);
    let mut harm = 0.0;
    for k in 0..gun::npc_burst(g) {
        match arm.pull(g, now + i64::from(k) * g.cycle, g.magazine, g.flintlock && gun::misfires(damp, roll() as f32)) {
            Pull::Load { .. } => {
                let sound = if g.flintlock { "minecraft:item.crossbow.loading_middle" } else { "minecraft:block.iron_door.close" };
                let _ = level.play_sound(host, sound, from, SoundSource::Neutral, 0.8, if g.flintlock { 0.8 } else { 1.6 });
                break;
            }
            Pull::Misfire => {
                let _ = level.play_sound(host, "minecraft:block.fire.extinguish", from, SoundSource::Neutral, 0.5, 1.6);
            }
            Pull::Fire => {
                bang(host, level, from, aim, g, me);
                if let Some(sp) = SPARKS.get() {
                    let mut at = 3.0;
                    while at < dist.min(48.0) {
                        spark(host, level, sp.crit, [from[0] + aim[0] * at, from[1] + aim[1] * at - 0.1, from[2] + aim[2] * at], 0.0, 0.0, 1);
                        at += 4.0;
                    }
                }
                if gun::npc_hits(fighting, dist, g, roll() as f32) {
                    // Of a handful of shot, most of it finds the mark.
                    let share = if g.pellets > 1 { g.pellets as f32 * 0.6 } else { 1.0 };
                    harm += gun::damage_at(g, dist) * share;
                } else {
                    // A miss still teaches a little.
                    crate::skills::practice(host, me, Skill::Fighting, crate::defence::BLOW_TAKEN);
                }
            }
            Pull::NoAmmo | Pull::Busy { .. } => break,
        }
    }
    if let Ok(mut s) = STATE.lock() {
        s.arms.insert(me.id(), arm);
    }
    if harm > 0.0 {
        strike(host, target, harm, me);
    }
}
