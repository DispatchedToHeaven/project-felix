//! Firearms on the server (see `gun`): a player's pull of the trigger loads the gun in hand with
//! a cartridge from the inventory (it is busy for as long as that takes), the next one fires it:
//! the ball goes where they look, a little off, to the first living thing in its way that no
//! block hides, with a bang, a flash and smoke. A loaded gun glints. NPCs holding a gun shoot what
//! they are after instead of running at it (`squad` has them keep their distance), as well as
//! their fighting lets them.
use crate::gun::{self, Arm, Gun, Pull};
use crate::skill::Skill;
use mod_api::{ClipBlock, ClipFluid, Compound, EntityRef, Hand, HostSide, InteractionResult, ItemStack, LevelRef, LivingRef, PlayerRef, Registrar, ServerHost, SoundSource, Tag, Text, UseItemEvent};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// The mark on a loaded gun (in its custom data).
const LOADED: &str = "felix_loaded";

#[derive(Default)]
struct State {
    /// The guns in NPCs' hands.
    arms: HashMap<u32, Arm>,
    seed: u64,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        s.arms.clear();
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
        // The client swings; the server loads and fires.
        let HostSide::Server(host) = host else { return Some(InteractionResult::Success) };
        let player = host.player(e.player)?;
        pulled(host, player, e.hand, e.stack);
        Some(InteractionResult::SuccessServer)
    }
}

pub fn register(r: &mut dyn Registrar) {
    for g in gun::GUNS {
        let key = r.item_key(g.id);
        r.item_part(key, Firearm);
    }
}

fn get<'a>(c: &'a Compound, key: &str) -> Option<&'a Tag> {
    c.iter().find(|(k, _)| k.to_string().is_ok_and(|s| s == key)).map(|(_, v)| v)
}

/// When gun `stack`'s charge is (or was) rammed home, if it holds one.
fn loaded(stack: &ItemStack) -> Option<i64> {
    let Some(Tag::Compound(data)) = get(stack.components()?.as_nbt(), "minecraft:custom_data") else { return None };
    match get(data, LOADED) {
        Some(Tag::Long(t)) => Some(*t),
        _ => None,
    }
}

/// Gun `stack` charged (ready at `ready`), or not: a loaded one glints.
fn with_charge(mut stack: ItemStack, ready: Option<i64>) -> ItemStack {
    let mut patch = Compound::new();
    if let Some(t) = ready {
        let mut data = Compound::new();
        data.insert(LOADED.into(), Tag::Long(t));
        patch.insert("minecraft:custom_data".into(), Tag::Compound(data));
        patch.insert("minecraft:enchantment_glint_override".into(), Tag::Byte(1));
    } else {
        patch.insert("!minecraft:custom_data".into(), Tag::Compound(Compound::new()));
        patch.insert("!minecraft:enchantment_glint_override".into(), Tag::Compound(Compound::new()));
    }
    let _ = stack.apply_components(&patch);
    stack
}

/// Takes one cartridge from the player's inventory: false if they have none.
fn take_cartridge(host: &mut dyn ServerHost, player: PlayerRef) -> bool {
    for (slot, stack) in player.inventory_slots(&*host).unwrap_or_default() {
        if stack.item().id(&*host) == gun::CARTRIDGE {
            return player.set_inventory_slot(host, slot, stack.shrink(1)).is_ok();
        }
    }
    false
}

fn say(host: &mut dyn ServerHost, player: PlayerRef, line: &str) {
    let _ = player.action_bar(host, Text::literal(line.to_string()));
}

fn wet(host: &dyn mod_api::Host, level: LevelRef, at: [f64; 3]) -> bool {
    level.is_raining_at(host, [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32]) == Some(true)
}

/// A player pulled the trigger of the gun `stack` in `hand`.
fn pulled(host: &mut dyn ServerHost, player: PlayerRef, hand: Hand, stack: &ItemStack) {
    let id = stack.item().id(&*host);
    let Some(g) = gun::gun(&id) else { return };
    let me = player.entity();
    let (Some(level), Some(eye), Some(aim)) = (me.level(&*host), me.eye_position(&*host), me.view_vector(&*host)) else { return };
    let now = level.game_time(&*host).unwrap_or(0);
    let Some(ready) = loaded(stack) else {
        if player.is_creative(&*host) != Some(true) && !take_cartridge(host, player) {
            let _ = level.play_sound(host, "minecraft:block.dispenser.fail", eye, SoundSource::Players, 0.6, 1.4);
            say(host, player, "No cartridges.");
            return;
        }
        let _ = LivingRef::new(me).set_hand_item(host, hand, Some(with_charge(stack.clone(), Some(now + g.reload))));
        player.set_item_cooldown(host, g.id, g.reload as i32);
        let _ = level.play_sound(host, "minecraft:item.crossbow.loading_middle", eye, SoundSource::Players, 0.8, 0.8);
        say(host, player, &format!("Loading the {}...", g.name));
        return;
    };
    // Not rammed home yet (a time from another world's clock does not count).
    if now < ready && ready - now <= g.reload {
        say(host, player, &format!("Still loading the {}...", g.name));
        return;
    }
    let _ = LivingRef::new(me).set_hand_item(host, hand, Some(with_charge(stack.clone(), None)));
    player.set_item_cooldown(host, g.id, gun::AFTER_SHOT as i32);
    let damp = wet(&*host, level, eye);
    if gun::misfires(damp, roll() as f32) {
        let _ = level.play_sound(host, "minecraft:block.fire.extinguish", eye, SoundSource::Players, 0.5, 1.6);
        say(host, player, if damp { "The powder is damp: it didn't catch." } else { "A flash in the pan: it didn't catch." });
        return;
    }
    fire(host, me, level, eye, aim, g, "minecraft:player_attack");
}

/// A shot from `shooter`'s gun `g`, from `from` along `aim`: what it hit, and how hard.
fn fire(host: &mut dyn ServerHost, shooter: EntityRef, level: LevelRef, from: [f64; 3], aim: [f64; 3], g: &Gun, kind: &str) -> Vec<(EntityRef, f32)> {
    let mut hits: Vec<(EntityRef, f32)> = Vec::new();
    for _ in 0..g.pellets {
        let d = gun::scatter(aim, g.spread, roll(), roll());
        let to = [from[0] + d[0] * g.range, from[1] + d[1] * g.range, from[2] + d[2] * g.range];
        // As far as the first block in the way.
        let wall = level
            .clip(&*host, from, to, ClipBlock::Collider, ClipFluid::None)
            .filter(|c| !c.miss)
            .map_or(g.range, |c| ((c.location[0] - from[0]).powi(2) + (c.location[1] - from[1]).powi(2) + (c.location[2] - from[2]).powi(2)).sqrt());
        let mut found = level.ray_trace_entities(&*host, from, to, 0.2, Some(shooter)).unwrap_or_default();
        found.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        let Some(h) = found.into_iter().find(|h| h.distance <= wall && LivingRef::new(h.entity).alive(&*host) == Some(true)) else { continue };
        let harm = gun::damage_at(g, h.distance);
        match hits.iter_mut().find(|x| x.0.id() == h.entity.id()) {
            Some(x) => x.1 += harm,
            None => hits.push((h.entity, harm)),
        }
    }
    bang(host, level, from, aim, g);
    // (All the shot that found one body is one blow: a body is hurt once a moment.)
    for (e, harm) in &hits {
        let _ = e.hurt(host, kind, *harm, Some(shooter), Some(shooter));
    }
    hits
}

/// The bang, the flash and the smoke of gun `g` fired from `from` along `aim`.
fn bang(host: &mut dyn ServerHost, level: LevelRef, from: [f64; 3], aim: [f64; 3], g: &Gun) {
    // A sound carries 16 blocks for each point of its volume.
    let loud = (g.heard / 16.0) as f32;
    let _ = level.play_sound(host, "minecraft:entity.generic.explode", from, SoundSource::Players, loud, 1.7 + roll() as f32 * 0.2);
    let _ = level.play_sound(host, "minecraft:entity.firework_rocket.blast", from, SoundSource::Players, loud, 0.7);
    let m = [from[0] + aim[0] * 1.2, from[1] + aim[1] * 1.2 - 0.1, from[2] + aim[2] * 1.2];
    let dim = level.dimension_id();
    // (Nobody near to see it is no failure.)
    crate::talk::queue_try(format!("execute in {dim} run particle minecraft:flame {:.2} {:.2} {:.2} 0.05 0.05 0.05 0.02 4", m[0], m[1], m[2]));
    crate::talk::queue_try(format!("execute in {dim} run particle minecraft:campfire_cosy_smoke {:.2} {:.2} {:.2} 0.15 0.15 0.15 0.01 8", m[0], m[1], m[2]));
}

/// The gun NPC `e` holds, if it holds one.
pub fn held(host: &dyn mod_api::Host, e: EntityRef) -> Option<&'static Gun> {
    let s = LivingRef::new(e).equipment(host, mod_api::EquipmentSlot::MainHand)?;
    gun::gun(&s.item().id(host))
}

/// NPC `me`, with gun `g` in hand and `target` in sight `dist` away, pulls the trigger at `now`:
/// it loads (it never runs out), or it fires, and hits as its fighting lets it.
pub fn npc_pull(host: &mut dyn ServerHost, me: EntityRef, target: EntityRef, g: &'static Gun, now: i64, dist: f64) {
    let (Some(level), Some(from)) = (me.level(&*host), me.eye_position(&*host)) else { return };
    let Some(to) = target.position(&*host) else { return };
    let mut arm = STATE.lock().ok().and_then(|s| s.arms.get(&me.id()).copied()).unwrap_or_default();
    let damp = wet(&*host, level, from);
    let pull = arm.pull(g, now, true, gun::misfires(damp, roll() as f32));
    if let Ok(mut s) = STATE.lock() {
        s.arms.insert(me.id(), arm);
    }
    match pull {
        Pull::Load { .. } => {
            let _ = level.play_sound(host, "minecraft:item.crossbow.loading_middle", from, SoundSource::Neutral, 0.8, 0.8);
        }
        Pull::Misfire => {
            let _ = level.play_sound(host, "minecraft:block.fire.extinguish", from, SoundSource::Neutral, 0.5, 1.6);
        }
        Pull::Fire => {
            let aim = [to[0] - from[0], to[1] + 1.0 - from[1], to[2] - from[2]];
            let l = (aim[0] * aim[0] + aim[1] * aim[1] + aim[2] * aim[2]).sqrt().max(1e-6);
            bang(host, level, from, [aim[0] / l, aim[1] / l, aim[2] / l], g);
            let fighting = crate::skills::level(host, me, Skill::Fighting);
            if gun::npc_hits(fighting, dist, g, roll() as f32) {
                // Of a handful of shot, most of it finds the mark.
                let share = if g.pellets > 1 { g.pellets as f32 * 0.6 } else { 1.0 };
                let _ = target.hurt(host, "minecraft:mob_attack", gun::damage_at(g, dist) * share, Some(me), Some(me));
            } else {
                // A miss still teaches a little.
                crate::skills::practice(host, me, Skill::Fighting, crate::defence::BLOW_TAKEN);
            }
        }
        Pull::NoAmmo | Pull::Busy { .. } => {}
    }
}
