//! Firearms: flintlocks, as a village smith could make them. No mod API types here.
//!
//! A pistol (quick, short), a musket (slow, far, hard) and a blunderbuss (a handful of shot, close
//! up). Each holds one charge: a paper cartridge is rammed home (which takes a while), then the
//! next pull of the trigger fires. The ball flies straight, a little off the aim (more for the
//! worse guns), hits the first thing in its way, and hits less hard far off. The priming powder
//! sometimes fails to catch, and often does in the rain. A shot is heard a long way.

/// What every gun is loaded with.
pub const CARTRIDGE: &str = "projectfelix:cartridge";
/// Ticks after a shot before anything else can be done with the gun.
pub const AFTER_SHOT: i64 = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gun {
    pub id: &'static str,
    pub name: &'static str,
    /// What one ball (or pellet) does, close up.
    pub damage: f32,
    pub pellets: u32,
    /// How far it carries (blocks).
    pub range: f64,
    /// How far off the aim a ball may go (degrees).
    pub spread: f64,
    /// Ticks to load it.
    pub reload: i64,
    /// How far the shot is heard (blocks).
    pub heard: f64,
}

pub const GUNS: [Gun; 3] = [
    Gun { id: "projectfelix:flintlock_pistol", name: "flintlock pistol", damage: 9.0, pellets: 1, range: 24.0, spread: 2.5, reload: 50, heard: 48.0 },
    Gun { id: "projectfelix:musket", name: "musket", damage: 16.0, pellets: 1, range: 56.0, spread: 1.0, reload: 80, heard: 64.0 },
    Gun { id: "projectfelix:blunderbuss", name: "blunderbuss", damage: 4.0, pellets: 7, range: 14.0, spread: 9.0, reload: 90, heard: 64.0 },
];

/// The gun item `item` is, if it is one.
pub fn gun(item: &str) -> Option<&'static Gun> {
    GUNS.iter().find(|g| g.id == item)
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

/// A gun in someone's hands: charged or not, and busy (being loaded, or just fired) until a time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Arm {
    pub loaded: bool,
    pub busy_until: i64,
}

/// What a pull of the trigger did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pull {
    Fire,
    /// The powder didn't catch: the charge is wasted.
    Misfire,
    /// A cartridge goes in: loaded at `until`.
    Load { until: i64 },
    NoAmmo,
    /// Still loading (or just fired): `left` ticks to go.
    Busy { left: i64 },
}

impl Arm {
    /// The trigger is pulled at `now`: a loaded gun fires (or `misfire`s), an empty one is loaded
    /// if there is a cartridge (`has_ammo`).
    pub fn pull(&mut self, g: &Gun, now: i64, has_ammo: bool, misfire: bool) -> Pull {
        if now < self.busy_until && now >= self.busy_until - g.reload.max(AFTER_SHOT) {
            return Pull::Busy { left: self.busy_until - now };
        }
        if self.loaded {
            self.loaded = false;
            self.busy_until = now + AFTER_SHOT;
            return if misfire { Pull::Misfire } else { Pull::Fire };
        }
        if !has_ammo {
            return Pull::NoAmmo;
        }
        self.loaded = true;
        self.busy_until = now + g.reload;
        Pull::Load { until: self.busy_until }
    }

    /// Whether it is charged and ready at `now`.
    pub fn ready(&self, now: i64) -> bool {
        self.loaded && now >= self.busy_until
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
    }

    #[test]
    fn loading_and_firing() {
        let g = gun("projectfelix:flintlock_pistol").unwrap();
        let mut a = Arm::default();
        assert_eq!(a.pull(g, 100, false, false), Pull::NoAmmo);
        assert_eq!(a.pull(g, 100, true, false), Pull::Load { until: 150 });
        assert_eq!(a.pull(g, 120, true, false), Pull::Busy { left: 30 });
        assert!(!a.ready(149) && a.ready(150));
        assert_eq!(a.pull(g, 150, true, false), Pull::Fire);
        assert_eq!(a.pull(g, 152, true, false), Pull::Busy { left: 6 });
        assert_eq!(a.pull(g, 158, true, false), Pull::Load { until: 208 });
        assert_eq!(a.pull(g, 300, true, true), Pull::Misfire);
        assert!(!a.loaded);
        // A time far in the past (another world's clock) does not leave it busy for ever.
        let mut old = Arm { loaded: false, busy_until: 1_000_000 };
        assert_eq!(old.pull(g, 500, true, false), Pull::Load { until: 550 });
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
