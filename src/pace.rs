//! Each level ticks on its own, and while it does the game resolves only its entities (and the
//! players). So what looks at NPCs runs from each level's tick, at its own pace in each: a count
//! of that level's ticks, or the game time since it last ran there.
use mod_api::Host;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// The level ticking now (its dimension), as `level_tick_pre` said.
static LEVEL: Mutex<&'static str> = Mutex::new("minecraft:overworld");
/// For each thing in each level: its ticks counted, and when it last ran.
static PACE: LazyLock<Mutex<HashMap<(&'static str, &'static str), (u32, i64)>>> = LazyLock::new(Default::default);

/// Level `dimension` is ticking now.
pub fn enter(dimension: &'static str) {
    if let Ok(mut l) = LEVEL.lock() {
        *l = dimension;
    }
}

/// The level ticking now (or that ticked last).
pub fn level() -> &'static str {
    LEVEL.lock().map(|l| *l).unwrap_or("minecraft:overworld")
}

/// Whether `what` runs at this tick of the level ticking now: every `n`th.
pub fn every(what: &'static str, n: u32) -> bool {
    let Ok(mut p) = PACE.lock() else { return false };
    let e = p.entry((what, level())).or_default();
    e.0 = e.0.wrapping_add(1);
    e.0 % n.max(1) == 0
}

/// Whether `step` game ticks went by since `what` last ran in the level ticking now (the clock
/// going back counts too); if so, it runs now.
pub fn since(what: &'static str, now: i64, step: i64) -> bool {
    let Ok(mut p) = PACE.lock() else { return false };
    let e = p.entry((what, level())).or_insert((0, i64::MIN / 2));
    if now - e.1 < step && now >= e.1 {
        return false;
    }
    e.1 = now;
    true
}

/// Whether entity `id` is in the level ticking now (a player too: the game finds them anywhere).
pub fn here(host: &dyn Host, id: u32) -> bool {
    host.entity(id).and_then(|e| e.level(host)).is_some_and(|l| l.dimension_id() == level())
}

/// The server stopped.
pub fn clear() {
    if let Ok(mut p) = PACE.lock() {
        p.clear();
    }
}
