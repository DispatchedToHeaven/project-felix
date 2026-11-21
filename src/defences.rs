//! Defence on the server (see `defence`): fighting is learned by the blows NPCs give and take,
//! and makes theirs harder; and who of a village stands when raiders come (`storyteller` has them
//! fight, `chiefs` has the head keep guards).
use crate::companion;
use crate::defence;
use crate::mind::Trait;
use crate::skill::Skill;
use mod_api::{EntityRef, Holder, LivingRef, ServerHost};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

#[derive(Default)]
struct State {
    /// Blows since the last look: who gave it, who took it.
    blows: Vec<(u32, u32)>,
    /// The fighting level each NPC's blows were last set by.
    applied: HashMap<u32, u8>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

pub fn forget(id: u32) {
    if let Ok(mut s) = STATE.lock() {
        s.applied.remove(&id);
    }
}

/// `living_damage_post`: a blow, from something.
pub fn on_hurt(victim: i32, by: i32) {
    if victim > 0 && by > 0 && victim != by
        && let Ok(mut s) = STATE.lock()
        && s.blows.len() < 256
    {
        s.blows.push((by as u32, victim as u32));
    }
}

/// The tick of each level: blows teach those of this level who gave and took them; each NPC's
/// blows are as hard as its fighting.
pub fn tick(host: &mut dyn ServerHost) {
    if !crate::pace::every("defences", 20) {
        return;
    }
    let blows = STATE.lock().map(|mut s| std::mem::take(&mut s.blows)).unwrap_or_default();
    for (by, victim) in blows {
        for (id, xp) in [(by, defence::BLOW_GIVEN), (victim, defence::BLOW_TAKEN)] {
            if let Some(e) = host.entity(id).filter(|e| crate::pace::here(&*host, id) && companion::is_npc(&*host, *e)) {
                crate::skills::practice(host, e, Skill::Fighting, xp);
            }
        }
    }
    if !crate::pace::every("defences_blows", 200) {
        return;
    }
    let npcs = companion::npcs();
    for id in &npcs {
        let Some(e) = host.entity(*id) else { continue };
        let level = crate::skills::level(host, e, Skill::Fighting);
        if STATE.lock().is_ok_and(|s| s.applied.get(id) == Some(&level)) {
            continue;
        }
        let Some(u) = e.uuid(host).map(crate::talk::uuid_text) else { continue };
        // What it had (maybe from before it was unloaded) goes first: no harm if there was none.
        crate::talk::queue_try(format!("attribute {u} minecraft:attack_damage modifier remove projectfelix:fighting"));
        if level > 0 {
            crate::talk::queue(format!("attribute {u} minecraft:attack_damage modifier add projectfelix:fighting {:.2} add_value", defence::blow(level)));
        }
        if let Ok(mut s) = STATE.lock() {
            s.applied.insert(*id, level);
        }
    }
    if let Ok(mut s) = STATE.lock() {
        s.applied.retain(|id, _| npcs.contains(id));
    }
}

/// Whether NPC `e` is one of its village's guards.
pub fn is_guard(host: &mut dyn ServerHost, e: EntityRef) -> bool {
    crate::routines::ROUTINE.get(host, Holder::Entity(e.id())).ok().flatten().is_some_and(|r| r.ends_with(":guard"))
}

/// Who of `people` stands when raiders come: the guards, and the willing among the rest.
pub fn muster(host: &mut dyn ServerHost, people: &[EntityRef]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut willing = Vec::new();
    let mut grown = 0;
    for e in people {
        if crate::families::is_child(host, *e) {
            continue;
        }
        grown += 1;
        if crate::healers::bedridden(e.id()) || crate::minds::breaking(e.id()).is_some() {
            continue;
        }
        if is_guard(host, *e) {
            out.push(e.id());
            continue;
        }
        let level = crate::skills::level(host, *e, Skill::Fighting);
        let life = LivingRef::new(*e);
        let health = life.health(&*host).zip(life.max_health(&*host)).map_or(1.0, |(h, m)| if m > 0.0 { h / m } else { 1.0 });
        let traits = crate::minds::traits_of(host, *e);
        let nerve = if traits.contains(&Trait::Steady) {
            1
        } else if traits.contains(&Trait::Nervous) {
            -1
        } else {
            0
        };
        if defence::takes_arms(level, health, nerve) {
            willing.push((e.id(), level));
        }
    }
    out.extend(defence::militia(willing, grown));
    out
}
