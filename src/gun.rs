//! Firearms, old and new. No mod API types here.
//!
//! Flintlocks, as a village smith could make them: a pistol (quick, short), a musket (slow, far,
//! hard) and a blunderbuss (a handful of shot, close up). Each holds one charge: a paper cartridge
//! is rammed home (which takes a while), then the next pull of the trigger fires. The priming
//! powder sometimes fails to catch, and often does in the rain.
//!
//! Modern guns hold a magazine of rounds and never misfire: a pistol and a shotgun fire a shot a
//! pull, a submachine gun and an assault rifle fire for as long as the trigger is held (and spray
//! wider the longer they do), a sniper rifle is slow, carries very far and hits very hard, but
//! only shoots straight through its scope (crouching). Each takes its own ammunition.
//!
//! Every ball or bullet flies straight, a little off the aim, hits the first thing in its way,
//! hits less hard far off, and harder in the head. A steady (crouching) aim is truer. A shot is
//! heard a long way.

/// What the flintlocks are loaded with.
pub const CARTRIDGE: &str = "projectfelix:cartridge";
pub const PISTOL_ROUNDS: &str = "projectfelix:pistol_rounds";
pub const RIFLE_ROUNDS: &str = "projectfelix:rifle_rounds";
pub const SHELLS: &str = "projectfelix:shotgun_shells";
/// What a hit in the head does beside one in the body.
pub const HEADSHOT: f32 = 1.5;
/// How far off a scoped gun shoots without its scope (degrees).
pub const UNSCOPED: f64 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gun {
    pub id: &'static str,
    pub name: &'static str,
    /// The item it is loaded with.
    pub ammo: &'static str,
    /// What one ball (or pellet) does, close up.
    pub damage: f32,
    pub pellets: u32,
    /// How far it carries (blocks).
    pub range: f64,
    /// How far off the aim a ball may go (degrees).
    pub spread: f64,
    /// How many shots a loading gives.
    pub magazine: u32,
    /// Ticks to load it, and between two shots.
    pub reload: i64,
    pub cycle: i64,
    /// Fires for as long as the trigger is held.
    pub auto: bool,
    /// Its powder may fail to catch.
    pub flintlock: bool,
    /// Shoots straight only through its scope.
    pub scope: bool,
    /// How far the shot is heard (blocks).
    pub heard: f64,
}

const fn flintlock(id: &'static str, name: &'static str, damage: f32, pellets: u32, range: f64, spread: f64, reload: i64, heard: f64) -> Gun {
    Gun { id, name, ammo: CARTRIDGE, damage, pellets, range, spread, magazine: 1, reload, cycle: 8, auto: false, flintlock: true, scope: false, heard }
}

pub const GUNS: [Gun; 8] = [
    flintlock("projectfelix:flintlock_pistol", "flintlock pistol", 9.0, 1, 24.0, 2.5, 50, 48.0),
    flintlock("projectfelix:musket", "musket", 16.0, 1, 56.0, 1.0, 80, 64.0),
    flintlock("projectfelix:blunderbuss", "blunderbuss", 4.0, 7, 14.0, 9.0, 90, 64.0),
    Gun { id: "projectfelix:pistol", name: "pistol", ammo: PISTOL_ROUNDS, damage: 6.0, pellets: 1, range: 40.0, spread: 1.5, magazine: 12, reload: 30, cycle: 4, auto: false, flintlock: false, scope: false, heard: 48.0 },
    Gun { id: "projectfelix:submachine_gun", name: "submachine gun", ammo: PISTOL_ROUNDS, damage: 4.0, pellets: 1, range: 32.0, spread: 2.5, magazine: 30, reload: 40, cycle: 2, auto: true, flintlock: false, scope: false, heard: 48.0 },
    Gun { id: "projectfelix:assault_rifle", name: "assault rifle", ammo: RIFLE_ROUNDS, damage: 7.0, pellets: 1, range: 72.0, spread: 1.2, magazine: 30, reload: 50, cycle: 3, auto: true, flintlock: false, scope: false, heard: 80.0 },
    Gun { id: "projectfelix:shotgun", name: "shotgun", ammo: SHELLS, damage: 3.5, pellets: 8, range: 18.0, spread: 7.0, magazine: 6, reload: 60, cycle: 14, auto: false, flintlock: false, scope: false, heard: 64.0 },
    Gun { id: "projectfelix:sniper_rifle", name: "sniper rifle", ammo: RIFLE_ROUNDS, damage: 30.0, pellets: 1, range: 128.0, spread: 0.15, magazine: 5, reload: 60, cycle: 25, auto: false, flintlock: false, scope: true, heard: 96.0 },
];

/// The gun item `item` is, if it is one.
pub fn gun(item: &str) -> Option<&'static Gun> {
    GUNS.iter().find(|g| g.id == item)
}

/// How ammunition `item` is spoken of.
pub fn ammo_name(item: &str) -> &'static str {
    match item {
        CARTRIDGE => "cartridges",
        PISTOL_ROUNDS => "pistol rounds",
        RIFLE_ROUNDS => "rifle rounds",
        SHELLS => "shells",
        _ => "ammunition",
    }
}

/// What one ball does at `dist`: all of it up to half the gun's range, falling to three fifths at
/// the end of it, nothing beyond.
pub fn damage_at(g: &Gun, dist: f64) -> f32 {
    if dist > g.range {
        return 0.0;
    }
    let half = g.range / 2.0;
    if dist <= half {
        return g.damage;
    }
    g.damage * (1.0 - 0.4 * ((dist - half) / half) as f32)
}

/// Whether the powder fails to catch: now and then, and often in the rain (`roll` 0..1).
pub fn misfires(raining: bool, roll: f32) -> bool {
    roll < if raining { 0.3 } else { 0.04 }
}

/// How far off the aim gun `g` shoots now (degrees): truer from a steady (crouching) aim, a scoped
/// gun wild without its scope, a held trigger wider with each of the `shots` already in the burst.
pub fn spread_now(g: &Gun, shots: u32, steady: bool) -> f64 {
    let base = match (g.scope, steady) {
        (true, false) => UNSCOPED,
        (_, true) => g.spread * 0.6,
        (false, false) => g.spread,
    };
    if g.auto { base * (1.0 + 0.2 * f64::from(shots.min(10))) } else { base }
}

/// Whether a hit at height `hit_y` on something whose eyes are at `eye_y` is one in the head.
pub fn in_head(hit_y: f64, eye_y: f64) -> bool {
    hit_y >= eye_y - 0.2
}

/// The way a player (or anything) looks, from its yaw and pitch in degrees.
pub fn look(yaw: f32, pitch: f32) -> [f64; 3] {
    let (y, p) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-9 { [0.0, 0.0, 1.0] } else { [v[0] / l, v[1] / l, v[2] / l] }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Where a ball aimed along `dir` really goes: off by up to `spread` degrees, any way round
/// (`r1`, `r2` 0..1: how far off, and which way).
pub fn scatter(dir: [f64; 3], spread: f64, r1: f64, r2: f64) -> [f64; 3] {
    let d = unit(dir);
    // Two directions across the aim.
    let side = unit(cross(d, if d[1].abs() > 0.99 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }));
    let up = cross(side, d);
    let off = (spread * r1.clamp(0.0, 1.0)).to_radians().tan();
    let turn = r2 * std::f64::consts::TAU;
    let (a, b) = (off * turn.cos(), off * turn.sin());
    unit([d[0] + side[0] * a + up[0] * b, d[1] + side[1] * a + up[1] * b, d[2] + side[2] * a + up[2] * b])
}

/// How far along the ray from `from` going `dir` (a unit vector) it enters the box `min`..`max`,
/// if it does within `reach`.
pub fn ray_box(from: [f64; 3], dir: [f64; 3], min: [f64; 3], max: [f64; 3], reach: f64) -> Option<f64> {
    let (mut near, mut far) = (0.0f64, reach);
    for i in 0..3 {
        if dir[i].abs() < 1e-9 {
            if from[i] < min[i] || from[i] > max[i] {
                return None;
            }
            continue;
        }
        let (a, b) = ((min[i] - from[i]) / dir[i], (max[i] - from[i]) / dir[i]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
        if near > far {
            return None;
        }
    }
    Some(near)
}

/// A gun in someone's hands: the shots left in it, and busy (being loaded, or between two shots)
/// until a time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Arm {
    pub rounds: u32,
    pub busy_until: i64,
}

/// What a pull of the trigger did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pull {
    Fire,
    /// The powder didn't catch: the charge is wasted.
    Misfire,
    /// It is loaded with `taken` of its ammunition: ready at `until`.
    Load { until: i64, taken: u32 },
    NoAmmo,
    /// Still loading (or between shots): `left` ticks to go.
    Busy { left: i64 },
}

impl Arm {
    /// Whether it is busy at `now` (a time from another world's clock does not keep it so).
    pub fn busy(&self, g: &Gun, now: i64) -> bool {
        now < self.busy_until && self.busy_until - now <= g.reload.max(g.cycle)
    }

    /// The trigger is pulled at `now`: a loaded gun fires (a flintlock may `misfire`), an empty
    /// one is loaded with what there is of its ammunition (`ammo` of it at hand).
    pub fn pull(&mut self, g: &Gun, now: i64, ammo: u32, misfire: bool) -> Pull {
        if self.busy(g, now) {
            return Pull::Busy { left: self.busy_until - now };
        }
        if self.rounds > 0 {
            self.rounds -= 1;
            self.busy_until = now + g.cycle;
            return if misfire && g.flintlock { Pull::Misfire } else { Pull::Fire };
        }
        self.load(g, now, ammo)
    }

    /// It is loaded at `now` (topped up, if it has shots left) with what there is of its ammunition.
    pub fn load(&mut self, g: &Gun, now: i64, ammo: u32) -> Pull {
        let taken = (g.magazine.saturating_sub(self.rounds)).min(ammo);
        if taken == 0 {
            return if self.rounds >= g.magazine { Pull::Busy { left: 0 } } else { Pull::NoAmmo };
        }
        self.rounds += taken;
        self.busy_until = now + g.reload;
        Pull::Load { until: self.busy_until, taken }
    }

    /// Whether it has a shot ready at `now`.
    pub fn ready(&self, g: &Gun, now: i64) -> bool {
        self.rounds > 0 && !self.busy(g, now)
    }
}

/// Whether an NPC this good at fighting hits what it shoots at `dist` away (`roll` 0..1): a
/// beginner one time in three close up, a master nearly always; half as often at the end of the
/// gun's range.
pub fn npc_hits(fighting: u8, dist: f64, g: &Gun, roll: f32) -> bool {
    if dist > g.range {
        return false;
    }
    let skill = (0.35 + 0.03 * f32::from(fighting.min(20))).min(0.95);
    roll < skill * (1.0 - 0.5 * (dist / g.range) as f32)
}

/// How many shots an NPC fires at a pull: a burst from a gun that fires by itself, else one.
pub fn npc_burst(g: &Gun) -> u32 {
    if g.auto { 3 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guns_and_their_reach() {
        let m = gun("projectfelix:musket").unwrap();
        assert!(gun("minecraft:bow").is_none());
        assert_eq!(damage_at(m, 10.0), 16.0);
        assert_eq!(damage_at(m, 28.0), 16.0);
        assert!((damage_at(m, 56.0) - 9.6).abs() < 0.01);
        assert_eq!(damage_at(m, 57.0), 0.0);
        let b = gun("projectfelix:blunderbuss").unwrap();
        assert!(b.pellets as f32 * b.damage > m.damage, "close up the blunderbuss is worst to meet");
        assert!(b.range < gun("projectfelix:flintlock_pistol").unwrap().range);
        assert!(misfires(true, 0.2) && !misfires(false, 0.2) && misfires(false, 0.01));
        // Every gun is sound, and takes something there is a word for.
        for g in GUNS {
            assert!(g.magazine >= 1 && g.cycle >= 1 && g.reload > g.cycle && g.damage > 0.0 && g.pellets >= 1, "{}", g.id);
            assert_ne!(ammo_name(g.ammo), "ammunition", "{}", g.id);
            assert_eq!(g.flintlock, g.ammo == CARTRIDGE);
            assert_eq!(GUNS.iter().filter(|o| o.id == g.id).count(), 1);
        }
    }

    #[test]
    fn loading_and_firing() {
        let g = gun("projectfelix:flintlock_pistol").unwrap();
        let mut a = Arm::default();
        assert_eq!(a.pull(g, 100, 0, false), Pull::NoAmmo);
        assert_eq!(a.pull(g, 100, 5, false), Pull::Load { until: 150, taken: 1 });
        assert_eq!(a.pull(g, 120, 5, false), Pull::Busy { left: 30 });
        assert!(!a.ready(g, 149) && a.ready(g, 150));
        assert_eq!(a.pull(g, 150, 5, false), Pull::Fire);
        assert_eq!(a.pull(g, 152, 5, false), Pull::Busy { left: 6 });
        assert_eq!(a.pull(g, 158, 5, false), Pull::Load { until: 208, taken: 1 });
        assert_eq!(a.pull(g, 300, 5, true), Pull::Misfire);
        assert_eq!(a.rounds, 0);
        // A time far in the past (another world's clock) does not leave it busy for ever.
        let mut old = Arm { rounds: 0, busy_until: 1_000_000 };
        assert_eq!(old.pull(g, 500, 1, false), Pull::Load { until: 550, taken: 1 });
    }

    #[test]
    fn magazines() {
        let p = gun("projectfelix:pistol").unwrap();
        let mut a = Arm::default();
        // Loaded with what there is.
        assert_eq!(a.pull(p, 0, 7, false), Pull::Load { until: 30, taken: 7 });
        let mut fired = 0;
        let mut now = 30;
        while a.pull(p, now, 0, true) == Pull::Fire {
            fired += 1;
            now += p.cycle;
        }
        assert_eq!(fired, 7, "a modern gun never misfires");
        assert_eq!(a.pull(p, now, 0, false), Pull::NoAmmo);
        // Topped up before it is empty.
        let mut b = Arm { rounds: 10, busy_until: 0 };
        assert_eq!(b.load(p, 100, 64), Pull::Load { until: 130, taken: 2 });
        assert_eq!(b.rounds, 12);
        assert_eq!(b.load(p, 200, 64), Pull::Busy { left: 0 }, "full already");
        // Between two shots of a rifle there is no time for a third.
        let r = gun("projectfelix:assault_rifle").unwrap();
        let mut c = Arm { rounds: 30, busy_until: 0 };
        assert_eq!(c.pull(r, 10, 0, false), Pull::Fire);
        assert_eq!(c.pull(r, 11, 0, false), Pull::Busy { left: 2 });
        assert_eq!(c.pull(r, 13, 0, false), Pull::Fire);
        assert_eq!(npc_burst(r), 3);
        assert_eq!(npc_burst(p), 1);
    }

    #[test]
    fn steadiness() {
        let r = gun("projectfelix:assault_rifle").unwrap();
        assert!(spread_now(r, 0, true) < spread_now(r, 0, false));
        assert!(spread_now(r, 5, false) > spread_now(r, 0, false) * 1.9);
        assert_eq!(spread_now(r, 10, false), spread_now(r, 50, false), "it only gets so wild");
        let s = gun("projectfelix:sniper_rifle").unwrap();
        assert_eq!(spread_now(s, 0, false), UNSCOPED);
        assert!(spread_now(s, 0, true) < 0.2);
        let p = gun("projectfelix:pistol").unwrap();
        assert_eq!(spread_now(p, 9, false), p.spread, "one shot a pull does not spray");
        assert!(in_head(1.7, 1.62) && in_head(1.45, 1.62) && !in_head(1.2, 1.62));
    }

    #[test]
    fn aim_and_hits() {
        let south = look(0.0, 0.0);
        assert!(south[2] > 0.999 && south[0].abs() < 1e-6);
        let west = look(90.0, 0.0);
        assert!(west[0] < -0.999);
        let up = look(0.0, -90.0);
        assert!(up[1] > 0.999);
        // A ball strays no more than the gun's spread.
        for i in 0..50 {
            let d = scatter(south, 9.0, f64::from(i) / 50.0, f64::from(i * 7 % 50) / 50.0);
            let cos = d[0] * south[0] + d[1] * south[1] + d[2] * south[2];
            assert!(cos >= 9.0f64.to_radians().cos() - 1e-9, "{d:?}");
        }
        assert_eq!(scatter(up, 9.0, 0.0, 0.3), up);
        // A box three blocks south, head high.
        let hit = ray_box([0.0, 1.6, 0.0], south, [-0.3, 0.0, 2.7], [0.3, 1.8, 3.3], 24.0);
        assert!((hit.unwrap() - 2.7).abs() < 1e-9);
        assert!(ray_box([0.0, 1.6, 0.0], south, [-0.3, 0.0, 2.7], [0.3, 1.8, 3.3], 2.0).is_none(), "out of reach");
        assert!(ray_box([0.0, 1.6, 0.0], south, [1.0, 0.0, 2.7], [1.6, 1.8, 3.3], 24.0).is_none(), "beside it");
        assert!(ray_box([0.0, 1.6, 0.0], [0.0, 0.0, -1.0], [-0.3, 0.0, 2.7], [0.3, 1.8, 3.3], 24.0).is_none(), "behind");
        let m = gun("projectfelix:musket").unwrap();
        assert!(npc_hits(0, 5.0, m, 0.3) && !npc_hits(0, 5.0, m, 0.4));
        assert!(npc_hits(20, 5.0, m, 0.85));
        assert!(!npc_hits(20, 60.0, m, 0.0));
    }
}
