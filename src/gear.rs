//! Companions' gear and growth.
//!
//! Gear: its owner hands an NPC what they hold ("Take this."): a weapon or tool goes to its hand, a
//! shield to the other, armour where it is worn, swapped with what it had. They are the mob's own
//! slots, so they count as they do for any mob (a sword hits harder, armour takes blows), drop when
//! it dies, and come back with "Give me back your gear.". Clients draw the held item in its hand
//! and armour as a coloured shell on its bones.
//!
//! Growth: an NPC gets experience for what it kills (a quarter of the victim's health), and with
//! each level (up to 20) two more health and half a heart more of hitting, a cheer and a line.
use crate::companion;
use mod_api::{
    AttachmentCodec, AttachmentType, ClientHost, Command, CommandArgumentValue, Compound, EntityRef, EquipmentSlot,
    Flow, Holder, ItemStack, LivingRef, PlayerRef, RegisterCommandsEvent, RunModCommandEvent, ServerHost, Tag, Text,
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

pub const MAX_LEVEL: u32 = 20;

/// Experience it has towards the next level, and its level.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Growth {
    pub xp: u32,
    pub level: u32,
}

/// Experience from level `level` to the next.
pub fn need(level: u32) -> u32 {
    10 + 10 * level
}

impl Growth {
    /// Adds `xp`; how many levels it rose.
    pub fn add(&mut self, xp: u32) -> u32 {
        let before = self.level;
        self.xp += xp;
        while self.level < MAX_LEVEL && self.xp >= need(self.level) {
            self.xp -= need(self.level);
            self.level += 1;
        }
        if self.level >= MAX_LEVEL {
            self.xp = 0;
        }
        self.level - before
    }
}

pub static GROWTH: AttachmentType<Growth> = AttachmentType {
    namespace: "projectfelix",
    name: "growth",
    default: Growth::default,
    codec: Some(AttachmentCodec {
        encode: |g| {
            let mut c = Compound::new();
            c.insert("xp".into(), Tag::Int(g.xp as i32));
            c.insert("level".into(), Tag::Int(g.level as i32));
            Ok(c)
        },
        decode: |c: &Compound| {
            let n = |k: &str| match c.get(&k.into()) {
                Some(Tag::Int(v)) => (*v).max(0) as u32,
                _ => 0,
            };
            Ok(Growth { xp: n("xp"), level: n("level").min(MAX_LEVEL) })
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// Where an item is worn or held.
pub fn slot_for(item: &str) -> EquipmentSlot {
    let id = item.rsplit(':').next().unwrap_or(item);
    if id.ends_with("_helmet") || id == "carved_pumpkin" || id.ends_with("_skull") || id.ends_with("_head") {
        EquipmentSlot::Head
    } else if id.ends_with("_chestplate") || id == "elytra" {
        EquipmentSlot::Chest
    } else if id.ends_with("_leggings") {
        EquipmentSlot::Legs
    } else if id.ends_with("_boots") {
        EquipmentSlot::Feet
    } else if id == "shield" {
        EquipmentSlot::OffHand
    } else {
        EquipmentSlot::MainHand
    }
}

/// The colour armour of `item` is drawn in.
pub fn armour_colour(item: &str) -> u32 {
    let id = item.rsplit(':').next().unwrap_or(item);
    for (k, c) in [
        ("netherite", 0x4A_3F_44),
        ("diamond", 0x5C_DB_D5),
        ("golden", 0xE8_C6_40),
        ("iron", 0xD0_D0_D0),
        ("chainmail", 0x8E_8E_8E),
        ("leather", 0x8B_5A_2B),
        ("turtle", 0x47_A0_36),
        ("copper", 0xC8_75_33),
    ] {
        if id.starts_with(k) {
            return c;
        }
    }
    0xAA_AA_AA
}

/// Attack damage the type gives (data/projectfelix/horizon/entity_type/npc.json).
const BASE_DAMAGE: f64 = 5.0;

const TIERS: [&str; 7] = ["wooden", "golden", "stone", "copper", "iron", "diamond", "netherite"];

/// What a held item adds to its hits (the game doesn't count an NPC's held item: this does).
pub fn weapon_bonus(item: &str) -> f64 {
    let id = item.rsplit(':').next().unwrap_or(item);
    let tier = TIERS.iter().position(|t| id.starts_with(t));
    let by = |list: [f64; 7]| tier.map_or(0.0, |t| list[t]);
    if id.ends_with("_sword") {
        by([3.0, 3.0, 4.0, 4.0, 5.0, 6.0, 7.0])
    } else if id.ends_with("_axe") {
        by([5.0, 5.0, 7.0, 7.0, 7.0, 7.0, 8.0])
    } else if id == "trident" {
        8.0
    } else if id == "mace" {
        5.0
    } else if id.ends_with("_pickaxe") || id.ends_with("_shovel") || id.ends_with("_hoe") {
        by([1.0, 1.0, 2.0, 2.0, 3.0, 4.0, 5.0])
    } else {
        0.0
    }
}

/// The armour points a piece gives where it is worn.
pub fn armour_points(item: &str) -> f64 {
    let id = item.rsplit(':').next().unwrap_or(item);
    let mats = ["leather", "chainmail", "golden", "copper", "iron", "diamond", "netherite", "turtle"];
    let Some(m) = mats.iter().position(|t| id.starts_with(t)) else { return 0.0 };
    let table: [f64; 8] = if id.ends_with("_helmet") {
        [1.0, 2.0, 2.0, 2.0, 2.0, 3.0, 3.0, 2.0]
    } else if id.ends_with("_chestplate") {
        [3.0, 5.0, 5.0, 4.0, 6.0, 8.0, 8.0, 0.0]
    } else if id.ends_with("_leggings") {
        [2.0, 4.0, 3.0, 3.0, 5.0, 6.0, 6.0, 0.0]
    } else if id.ends_with("_boots") {
        [1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 3.0, 0.0]
    } else {
        return 0.0;
    };
    table[m]
}

/// What its gear adds, by NPC: (hits, armour), as last applied.
static APPLIED: LazyLock<Mutex<HashMap<u32, (f64, f64)>>> = LazyLock::new(Default::default);
/// NPCs with a shield in their off hand, as last seen.
static SHIELDED: LazyLock<Mutex<std::collections::HashSet<u32>>> = LazyLock::new(Default::default);

const SLOTS: [EquipmentSlot; 6] = [EquipmentSlot::MainHand, EquipmentSlot::OffHand, EquipmentSlot::Head, EquipmentSlot::Chest, EquipmentSlot::Legs, EquipmentSlot::Feet];

fn item_id(host: &dyn mod_api::Host, s: &ItemStack) -> String {
    s.item().id(host)
}

fn short(id: &str) -> String {
    id.rsplit(':').next().unwrap_or(id).replace('_', " ")
}

// ---- gear, on the server ---------------------------------------------------------------------------------

/// `living_damage_pre`: an NPC's armour points take a blow from something as armour does
/// (no toughness): `damage * (1 - min(20, max(a / 5, a - damage / 2)) / 25)`.
pub fn on_damage_pre(e: &mut mod_api::LivingDamagePreEvent<'_>) {
    if e.kind != companion::NPC || e.source.causing <= 0 {
        return;
    }
    // An NPC of the same owner (a stray arrow) does it no harm.
    if crate::squad::friendly(e.entity as u32, e.source.causing as u32) {
        e.container.set_new_damage(0.0);
        return;
    }
    // A shield in its off hand takes a third of each hit.
    if SHIELDED.lock().is_ok_and(|s| s.contains(&(e.entity as u32))) {
        let d = e.container.new_damage();
        e.container.set_new_damage(d * 0.67);
    }
    let Some(a) = APPLIED.lock().ok().and_then(|m| m.get(&(e.entity as u32)).map(|x| x.1 as f32)).filter(|a| *a > 0.0) else { return };
    let d = e.container.new_damage();
    let cut = (a / 5.0).max(a - d / 2.0).min(20.0);
    e.container.set_new_damage(d * (1.0 - cut / 25.0));
}

/// The `gear` action `take`: the player's held item goes to the NPC (what it had there comes back).
pub fn take(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) {
    let me = LivingRef::new(player.entity());
    let it = LivingRef::new(npc);
    let name = player.name(&*host).unwrap_or_default();
    let Some(stack) = me.equipment(&*host, EquipmentSlot::MainHand).filter(|s| s.count() > 0) else {
        crate::heart::say_aloud(host, npc, &format!("Hold what you want to give me, {name}."));
        return;
    };
    let id = item_id(&*host, &stack);
    let slot = slot_for(&id);
    let old = it.equipment(&*host, slot).filter(|s| s.count() > 0);
    if it.set_equipment(host, slot, Some(stack)).is_err() {
        crate::heart::say_aloud(host, npc, "I can't take that.");
        return;
    }
    let _ = me.set_equipment(host, EquipmentSlot::MainHand, old);
    // It drops all of it when it dies, not a mob's odd piece.
    if let Some(u) = npc.uuid(host) {
        crate::talk::queue(format!(
            "data merge entity {} {{drop_chances:{{mainhand:2.0f,offhand:2.0f,head:2.0f,chest:2.0f,legs:2.0f,feet:2.0f}}}}",
            crate::talk::uuid_text(u)
        ));
    }
    crate::heart::say_aloud(host, npc, &format!("Thanks, {name}. I'll put the {} to good use.", short(&id)));
    crate::talk::run_for(host, None, npc, &[crate::dialog::Action::Animation("animation.felix.wave".into())]);
}

/// The `gear` action `return`: everything it wears and holds goes to the player.
pub fn give_back(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) {
    let it = LivingRef::new(npc);
    let mut n = 0;
    for slot in SLOTS {
        let Some(s) = it.equipment(&*host, slot).filter(|s| s.count() > 0) else { continue };
        if it.set_equipment(host, slot, None).is_ok() {
            if player.give_stack(host, s.clone()).is_err()
                && let Some(at) = player.entity().position(&*host)
                && let Some(level) = player.entity().level(&*host)
            {
                let _ = level.spawn_item(host, at[0], at[1], at[2], s, 0);
            }
            n += 1;
        }
    }
    let line = if n == 0 { "I have nothing of yours.".to_string() } else { "Here, all of it back.".to_string() };
    crate::heart::say_aloud(host, npc, &line);
}

// ---- growth --------------------------------------------------------------------------------------------------

/// `living_death`: an NPC that killed something learns from it.
pub fn on_death(host: &mut dyn ServerHost, victim: u32, killer: i32) {
    if killer <= 0 {
        return;
    }
    let (Some(v), Some(k)) = (host.entity(victim), host.entity(killer as u32)) else { return };
    if !companion::is_npc(&*host, k) || companion::is_npc(&*host, v) {
        return;
    }
    let max = LivingRef::new(v).max_health(&*host).unwrap_or(4.0);
    add_xp(host, k, ((max / 4.0).ceil() as u32).max(1));
}

/// Adds `xp` to NPC `npc`; a level up shows.
pub fn add_xp(host: &mut dyn ServerHost, npc: EntityRef, xp: u32) {
    let mut g = GROWTH.get(host, Holder::Entity(npc.id())).unwrap_or_default();
    let up = g.add(xp);
    let _ = GROWTH.set(host, Holder::Entity(npc.id()), g);
    if up > 0 {
        apply(host, npc, g.level);
        let Some(u) = npc.uuid(host).map(crate::talk::uuid_text) else { return };
        crate::talk::queue(format!("execute at {u} run particle minecraft:totem_of_undying ~ ~1.2 ~ 0.4 0.6 0.4 0.2 30"));
        crate::talk::queue(format!("execute at {u} run playsound minecraft:entity.player.levelup neutral @a ~ ~ ~"));
        LivingRef::new(npc).heal(host, 100.0);
        crate::heart::say_aloud(host, npc, &format!("I feel stronger! (level {})", g.level));
        crate::talk::run_for(host, None, npc, &[crate::dialog::Action::Animation("animation.felix.cheer".into())]);
        crate::heart::feel(host, npc, None, "level_up");
        crate::rulebook::notice(npc.id(), "level_up", None);
    }
}

/// What a level gives: more health, harder hits.
fn apply(host: &mut dyn ServerHost, npc: EntityRef, level: u32) {
    let Some(u) = npc.uuid(host).map(crate::talk::uuid_text) else { return };
    for (attr, per) in [("minecraft:max_health", 2.0), ("minecraft:attack_damage", 0.5)] {
        crate::talk::queue(format!("attribute {u} {attr} modifier remove projectfelix:level"));
        if level > 0 {
            crate::talk::queue(format!("attribute {u} {attr} modifier add projectfelix:level {:.1} add_value", per * level as f64));
        }
    }
}

/// Sets NPC `npc`'s level (a village brought it).
pub fn set_level(host: &mut dyn ServerHost, npc: EntityRef, level: u32) {
    let level = level.min(MAX_LEVEL);
    let _ = GROWTH.set(host, Holder::Entity(npc.id()), Growth { xp: 0, level });
    apply(host, npc, level);
}

/// An NPC's level, for its dialog's title.
pub fn level_of(host: &mut dyn ServerHost, npc: EntityRef) -> u32 {
    GROWTH.get(host, Holder::Entity(npc.id())).map(|g| g.level).unwrap_or(0)
}

/// The `stats` action: it says how strong it is.
/// How strong NPC `npc` is, in its words.
pub fn stats_line(host: &mut dyn ServerHost, npc: EntityRef) -> String {
    let g = GROWTH.get(host, Holder::Entity(npc.id())).unwrap_or_default();
    let l = LivingRef::new(npc);
    let health = l.health(&*host).unwrap_or(0.0);
    let max = l.max_health(&*host).unwrap_or(0.0);
    let item = |slot| l.equipment(&*host, slot).filter(|s: &ItemStack| s.count() > 0).map(|s| item_id(&*host, &s)).unwrap_or_default();
    let armour: f64 = [EquipmentSlot::Head, EquipmentSlot::Chest, EquipmentSlot::Legs, EquipmentSlot::Feet].into_iter().map(|s| armour_points(&item(s))).sum();
    let hits = BASE_DAMAGE + 0.5 * g.level as f64 + weapon_bonus(&item(EquipmentSlot::MainHand));
    let next = if g.level >= MAX_LEVEL { "as strong as I get".to_string() } else { format!("{}/{} to the next", g.xp, need(g.level)) };
    let held = l.equipment(&*host, EquipmentSlot::MainHand).filter(|s| s.count() > 0).map(|s| format!(", holding a {}", short(&item_id(&*host, &s)))).unwrap_or_default();
    format!("Level {} ({next}). Health {health:.0}/{max:.0}, armour {armour:.0}, I hit for {hits:.1}{held}.", g.level)
}

/// The `stats` action: it tells the player how strong it is.
pub fn tell_stats(host: &mut dyn ServerHost, npc: EntityRef, player: Option<PlayerRef>) {
    let line = stats_line(host, npc);
    match player {
        Some(p) => crate::heart::say_to(host, npc, p, &line),
        None => crate::heart::say_aloud(host, npc, &line),
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix level <target:entity>", 2));
    e.commands.push(Command::op("felix level set <target:entity> <level:integer>", 2));
    e.commands.push(Command::op("felix level xp <target:entity> <xp:integer>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if !matches!(lit.as_slice(), ["felix", "level", ..]) {
        return Flow::Continue;
    }
    let target = e.command.arguments.iter().find(|a| a.name == "target").and_then(|a| match &a.value {
        CommandArgumentValue::Entity(x) => Some(*x),
        CommandArgumentValue::Entities(l) => l.first().copied(),
        _ => None,
    });
    let number = |n: &str| e.command.arguments.iter().find(|a| a.name == n).and_then(|a| match a.value {
        CommandArgumentValue::Integer(v) => Some(v.max(0) as u32),
        _ => None,
    });
    let Some(npc) = target.filter(|t| companion::is_npc(&*host, *t)) else {
        let _ = host.commands().feedback(Text::literal("That is not an NPC."));
        return Flow::Cancel;
    };
    match lit.as_slice() {
        ["felix", "level", "set"] => {
            let level = number("level").unwrap_or(0).min(MAX_LEVEL);
            let _ = GROWTH.set(host, Holder::Entity(npc.id()), Growth { xp: 0, level });
            apply(host, npc, level);
            let _ = host.commands().feedback(Text::literal(format!("Level {level}")));
        }
        ["felix", "level", "xp"] => {
            let xp = number("xp").unwrap_or(0);
            add_xp(host, npc, xp);
        }
        _ => {
            let line = stats_line(host, npc);
            crate::speech::show(host, npc, &line);
            let _ = host.commands().feedback(Text::literal(line));
        }
    }
    Flow::Cancel
}

// ---- what clients see -----------------------------------------------------------------------------------------

/// What an NPC wears and holds, by item id, for clients to draw (the API gives clients no mob's
/// equipment): main hand, off hand, head, chest, legs.
pub static WORN: AttachmentType<Option<[String; 5]>> = AttachmentType {
    namespace: "projectfelix",
    name: "worn",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |w| {
            let mut c = Compound::new();
            if let Some(w) = w {
                for (k, v) in WORN_KEYS.iter().zip(w.iter()) {
                    c.insert((*k).into(), Tag::String(v.as_str().into()));
                }
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            if c.is_empty() {
                return Ok(None);
            }
            let get = |k: &str| match c.get(&k.into()) {
                Some(Tag::String(s)) => s.to_string_lossy(),
                _ => String::new(),
            };
            Ok(Some(WORN_KEYS.map(get)))
        },
    }),
    copy_on_death: false,
    sync: true,
};

const WORN_KEYS: [&str; 5] = ["mainhand", "offhand", "head", "chest", "legs"];
const WORN_SLOTS: [EquipmentSlot; 5] = [EquipmentSlot::MainHand, EquipmentSlot::OffHand, EquipmentSlot::Head, EquipmentSlot::Chest, EquipmentSlot::Legs];

/// When each level's NPCs were last looked at.
static LAST: LazyLock<Mutex<HashMap<&'static str, i64>>> = LazyLock::new(Default::default);

/// The server stopped: nothing is known of what NPCs wear.
pub fn clear() {
    if let Ok(mut a) = APPLIED.lock() {
        a.clear();
    }
    if let Ok(mut s) = SHIELDED.lock() {
        s.clear();
    }
    if let Ok(mut l) = LAST.lock() {
        l.clear();
    }
}

/// NPC `id` left its level.
pub fn forget(id: u32) {
    if let Ok(mut a) = APPLIED.lock() {
        a.remove(&id);
    }
    if let Ok(mut s) = SHIELDED.lock() {
        s.remove(&id);
    }
}

/// The level tick: every two seconds, what each NPC of the level wears goes to the clients (when
/// it changed).
pub fn level_tick(host: &mut dyn ServerHost, dimension: &'static str) {
    let npcs = companion::npcs();
    let Some(now) = npcs.iter().find_map(|id| host.entity(*id)).and_then(|e| e.level(&*host)).and_then(|l| l.game_time(&*host)) else { return };
    let due = LAST.lock().is_ok_and(|mut last| {
        let l = last.entry(dimension).or_insert(now - 40);
        let due = now - *l >= 40 || now < *l;
        if due {
            *l = now;
        }
        due
    });
    if !due {
        return;
    }
    for id in npcs {
        let Some(e) = host.entity(id) else { continue };
        let l = LivingRef::new(e);
        let worn = WORN_SLOTS.map(|slot| l.equipment(&*host, slot).filter(|s| s.count() > 0).map(|s| item_id(&*host, &s)).unwrap_or_default());
        let feet = l.equipment(&*host, EquipmentSlot::Feet).filter(|s| s.count() > 0).map(|s| item_id(&*host, &s)).unwrap_or_default();
        let stats = (weapon_bonus(&worn[0]), worn[2..].iter().chain([&feet]).map(|i| armour_points(i)).sum::<f64>());
        if let Ok(mut s) = SHIELDED.lock() {
            if worn[1] == "minecraft:shield" {
                s.insert(id);
            } else {
                s.remove(&id);
            }
        }
        let before = APPLIED.lock().ok().and_then(|a| a.get(&id).copied());
        if before != Some(stats)
            && let Some(u) = e.uuid(host).map(crate::talk::uuid_text)
        {
            // Its weapon adds to its blows; its armour counts in each hit it takes (`on_damage_pre`).
            if before.is_none_or(|b| b.0 != stats.0) {
                // What it had (maybe from before it was unloaded) goes first: no harm if there was none.
                crate::talk::queue_try(format!("attribute {u} minecraft:attack_damage modifier remove projectfelix:gear"));
                if stats.0 > 0.0 {
                    crate::talk::queue(format!("attribute {u} minecraft:attack_damage modifier add projectfelix:gear {:.1} add_value", stats.0));
                }
            }
            if let Ok(mut a) = APPLIED.lock() {
                a.insert(id, stats);
            }
        }
        let now_worn = if worn.iter().all(String::is_empty) { None } else { Some(worn) };
        if WORN.get(host, Holder::Entity(id)).ok().flatten() != now_worn {
            let _ = WORN.set(host, Holder::Entity(id), now_worn);
        }
    }
}

static SEEN: LazyLock<Mutex<HashMap<u32, crate::models::Extras>>> = LazyLock::new(Default::default);
static KEYS: LazyLock<Mutex<HashMap<String, Option<mod_api::ItemKey>>>> = LazyLock::new(Default::default);

fn stack_of(host: &dyn ClientHost, id: &str) -> Option<ItemStack> {
    if id.is_empty() {
        return None;
    }
    let key = {
        let mut keys = KEYS.lock().ok()?;
        *keys.entry(id.to_string()).or_insert_with(|| mod_api::Identifier::parse(id).ok().and_then(|i| host.item_key(&i)))
    }?;
    ItemStack::new(key, 1)
}

/// The client tick: what each NPC wears and holds, for its renderer.
pub fn client_tick(host: &dyn ClientHost) {
    use mod_api::{AttachmentHolder, AttachmentQueries};
    let mut seen = HashMap::new();
    if let Ok(query) = host.query(&WORN) {
        for row in query.iter().flatten() {
            let (AttachmentHolder::Entity(e), Some(w)) = (row.holder, row.value) else { continue };
            let mut x = crate::models::Extras { seed: e.id() as i32, ..Default::default() };
            x.hand = stack_of(host, &w[0]);
            x.off_hand = stack_of(host, &w[1]);
            for (item, bones) in [(&w[2], &["head"][..]), (&w[3], &["body", "rightarm", "leftarm"][..]), (&w[4], &["rightleg", "leftleg"][..])] {
                if !item.is_empty() {
                    let c = armour_colour(item);
                    x.armour.extend(bones.iter().map(|b| (*b, c)));
                }
            }
            seen.insert(e.id(), x);
        }
    }
    if let Ok(mut s) = SEEN.lock() {
        *s = seen;
    }
}

pub fn extras(id: u32) -> Option<crate::models::Extras> {
    SEEN.lock().ok()?.get(&id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_slots() {
        let mut g = Growth::default();
        assert_eq!(g.add(9), 0);
        assert_eq!(g.add(1), 1);
        assert_eq!((g.level, g.xp), (1, 0));
        assert_eq!(g.add(20 + 30 + 5), 2);
        assert_eq!((g.level, g.xp), (3, 5));
        assert_eq!(g.add(100_000), MAX_LEVEL - 3);
        assert_eq!(g.xp, 0);
        assert_eq!(slot_for("minecraft:iron_helmet"), EquipmentSlot::Head);
        assert_eq!(slot_for("minecraft:diamond_chestplate"), EquipmentSlot::Chest);
        assert_eq!(slot_for("minecraft:shield"), EquipmentSlot::OffHand);
        assert_eq!(slot_for("minecraft:iron_sword"), EquipmentSlot::MainHand);
        assert_eq!(armour_colour("minecraft:diamond_helmet"), 0x5C_DB_D5);
    }
}
