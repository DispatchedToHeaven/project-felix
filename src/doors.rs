//! Doors on the server: NPCs can't open doors as players do, so an NPC next to a shut wooden door
//! or fence gate opens it, and it shuts again a little after no NPC is near it.
use crate::companion;
use mod_api::{LevelRef, ServerHost};
use std::sync::{LazyLock, Mutex};

/// Doors and gates NPCs opened: level, the lower block, when.
static OPEN: LazyLock<Mutex<Vec<(&'static str, [i32; 3], i64)>>> = LazyLock::new(Default::default);

pub fn clear() {
    if let Ok(mut o) = OPEN.lock() {
        o.clear();
    }
}

fn openable(id: &str) -> bool {
    (id.ends_with("_door") && id != "minecraft:iron_door") || id.ends_with("_fence_gate")
}

/// Sets door or gate `p` (its lower block) of `level` open or shut, as a player would.
fn set(host: &mut dyn ServerHost, level: LevelRef, p: [i32; 3], open: bool) {
    let Some(id) = level.block(&*host, p).map(|k| k.id(&*host)) else { return };
    if !openable(&id) {
        return;
    }
    let prop = |k: &str| level.block_property(&*host, p, k).unwrap_or_default();
    let dim = level.dimension_id();
    let facing = prop("facing");
    if id.ends_with("_fence_gate") {
        let wall = prop("in_wall");
        crate::talk::queue_try(format!("execute in {dim} run setblock {} {} {} {id}[facing={facing},in_wall={wall},open={open},powered=false]", p[0], p[1], p[2]));
    } else {
        let hinge = prop("hinge");
        for (half, y) in [("upper", p[1] + 1), ("lower", p[1])] {
            crate::talk::queue_try(format!("execute in {dim} run setblock {} {y} {} {id}[facing={facing},half={half},hinge={hinge},open={open},powered=false]", p[0], p[2]));
        }
    }
    let sound = match (id.ends_with("_fence_gate"), open) {
        (true, true) => "minecraft:block.fence_gate.open",
        (true, false) => "minecraft:block.fence_gate.close",
        (false, true) => "minecraft:block.wooden_door.open",
        (false, false) => "minecraft:block.wooden_door.close",
    };
    // (Nobody near to hear it is no failure.)
    crate::talk::queue_try(format!("execute in {dim} run playsound {sound} block @a {} {} {} 0.8", p[0], p[1], p[2]));
}

/// The tick of each level: doors next to its NPCs open, those left alone shut.
pub fn tick(host: &mut dyn ServerHost) {
    if !crate::pace::every("doors", 5) {
        return;
    }
    let mut near: Vec<[i32; 3]> = Vec::new();
    let mut level_now = None;
    for id in companion::npcs() {
        let Some(e) = host.entity(id) else { continue };
        let (Some(level), Some(pos)) = (e.level(&*host), e.position(&*host)) else { continue };
        level_now = Some(level);
        let f = [pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32];
        for dx in -1..=1 {
            for dz in -1..=1 {
                for dy in 0..=1 {
                    let mut p = [f[0] + dx, f[1] + dy, f[2] + dz];
                    let Some(bid) = level.block(&*host, p).map(|k| k.id(&*host)) else { continue };
                    if !openable(&bid) {
                        continue;
                    }
                    if level.block_property(&*host, p, "half").as_deref() == Some("upper") {
                        p[1] -= 1;
                    }
                    if near.contains(&p) {
                        continue;
                    }
                    near.push(p);
                    if level.block_property(&*host, p, "open").as_deref() == Some("false") {
                        set(host, level, p, true);
                        let now = level.game_time(&*host).unwrap_or(0);
                        if let Ok(mut o) = OPEN.lock() {
                            o.retain(|d| !(d.0 == level.dimension_id() && d.1 == p));
                            o.push((level.dimension_id(), p, now));
                        }
                    }
                }
            }
        }
    }
    // Shut what NPCs opened in this level, once none is next to it (a second after).
    let Some(level) = level_now.or_else(|| host.level(crate::pace::level())) else { return };
    let dim = level.dimension_id();
    let now = level.game_time(&*host).unwrap_or(0);
    let shut: Vec<[i32; 3]> = match OPEN.lock() {
        Ok(mut o) => {
            let (go, stay): (Vec<_>, Vec<_>) = o.drain(..).partition(|d| d.0 == dim && !near.contains(&d.1) && (now - d.2 >= 20 || now < d.2));
            *o = stay;
            go.into_iter().map(|d| d.1).collect()
        }
        Err(_) => Vec::new(),
    };
    for p in shut {
        if level.block_property(&*host, p, "open").as_deref() == Some("true") {
            set(host, level, p, false);
        }
    }
}
