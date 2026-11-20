//! Companions on the way (see `remark`): an NPC following its player says something when they
//! come into a new kind of land, a cave or another world, when the day or the weather turns, and
//! when the player is badly hurt (a close one heals them a little), at most every half minute;
//! the places stay in its memories of them (see `bonds`).
use crate::companion::{self, Mode};
use crate::remark;
use mod_api::{LivingRef, PlayerRef, ServerHost};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

#[derive(Default)]
struct Along {
    land: Option<&'static str>,
    time: &'static str,
    weather: &'static str,
    said: i64,
    helped: i64,
    started: bool,
}

#[derive(Default)]
struct State {
    along: HashMap<u32, Along>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// The server stopped: nothing of its world is kept.
pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

/// Ticks between remarks, and between two helps.
const QUIET: i64 = 600;
const HELP_AGAIN: i64 = 2400;

fn fill(line: &str, p: &str) -> String {
    line.replace("@player", p)
}

/// The server tick: each companion following its player looks about it.
pub fn tick(host: &mut dyn ServerHost) {
    // Each level at its own pace (its NPCs are the ones found while it ticks).
    if !crate::pace::every("companions", 20) {
        return;
    }
    let npcs = companion::npcs();
    for id in &npcs {
        if companion::mode_of(*id) != Mode::Follow {
            continue;
        }
        let Some(me) = host.entity(*id) else { continue };
        let Some(level) = me.level(&*host) else { continue };
        let Some(owner) = LivingRef::new(me).owner(&*host) else { continue };
        let Some(p) = level.online_players(&*host).unwrap_or_default().into_iter().find(|p| p.uuid() == owner) else { continue };
        let (Some(at), Some(pat)) = (me.position(&*host), p.entity().position(&*host)) else { continue };
        if (at[0] - pat[0]).powi(2) + (at[2] - pat[2]).powi(2) > 24.0 * 24.0 {
            continue;
        }
        let now = level.game_time(&*host).unwrap_or(0);
        look(host, me, p, level, pat, now);
    }
    if let Ok(mut s) = STATE.lock() {
        s.along.retain(|id, _| npcs.contains(id));
    }
}

fn look(host: &mut dyn ServerHost, me: mod_api::EntityRef, p: PlayerRef, level: mod_api::LevelRef, pat: [f64; 3], now: i64) {
    let id = me.id();
    let (biome, time, weather) = crate::flags::surroundings(&*host, p);
    let head = [pat[0].floor() as i32, pat[1].floor() as i32 + 1, pat[2].floor() as i32];
    let under = pat[1] < 50.0 && level.can_see_sky(&*host, head) == Some(false);
    let land = if under { Some("caves") } else { remark::land(&biome) };
    let stage = crate::bonds::stage(host, me, p);
    let name = p.name(&*host).unwrap_or_default();
    let seed = (now as u64 / 20).wrapping_mul(0x9e37_79b9) ^ u64::from(id);
    let (said, line) = {
        let Ok(mut s) = STATE.lock() else { return };
        let a = s.along.entry(id).or_default();
        let first = !a.started;
        a.started = true;
        let quiet = now - a.said >= QUIET || now < a.said;
        let mut line = None;
        let mut place = None;
        if land.is_some() && land != a.land {
            if !first && quiet {
                line = land.map(|l| remark::on_land(l, stage, seed));
            }
            place = land;
            a.land = land;
        } else if land.is_some() {
            a.land = land;
        }
        let turn = |was: &str, is: &'static str| (!first && !was.is_empty() && was != is).then_some(is);
        if line.is_none() && quiet {
            let t = match (a.time, time) {
                ("day", "dusk") | ("dawn", "dusk") => turn(a.time, "dusk"),
                ("night", "dawn") => turn(a.time, "dawn"),
                _ => None,
            };
            let w = turn(a.weather, weather);
            line = t.or(w).map(|what| remark::on_turn(what, stage, seed));
        }
        a.time = time;
        a.weather = weather;
        if line.is_some() {
            a.said = now;
        }
        (place, line)
    };
    if let Some(l) = said {
        crate::bonds::note_place(host, me, p, l);
    }
    if let Some(l) = line {
        crate::heart::say_aloud(host, me, &fill(&l, &name));
    }
    // Its player badly hurt: it says so, and a close one helps.
    let player = LivingRef::new(p.entity());
    let hurt = player.health(&*host).zip(player.max_health(&*host)).is_some_and(|(h, m)| m > 0.0 && h / m < 0.35);
    let due = STATE.lock().ok().and_then(|s| s.along.get(&id).map(|a| now - a.helped >= HELP_AGAIN || now < a.helped)).unwrap_or(true);
    if hurt && due {
        if let Ok(mut s) = STATE.lock() {
            s.along.entry(id).or_default().helped = now;
        }
        crate::heart::say_aloud(host, me, &fill(&remark::on_hurt(stage, seed), &name));
        if remark::heals(stage) {
            crate::talk::queue(format!("effect give {name} minecraft:regeneration 6 1 true"));
            crate::talk::queue(format!("execute in {} run particle minecraft:happy_villager {:.2} {:.2} {:.2} 0.4 0.4 0.4 0 8", level.dimension_id(), pat[0], pat[1] + 1.0, pat[2]));
        }
    }
}
