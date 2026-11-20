//! Felix settlements the world makes by itself (see `settlements`): where one may stand, and what
//! it is — cabins about a campfire, doors to the middle, a wheat field, a woodcutter's spot, and
//! the people who live there (a farmer and a baker who share a home, a woodcutter, an elder, a
//! guard), each with a name, a character, a job and a shop. No mod API types here.
//!
//! The world is cut into regions of `REGION` chunks; in about `CHANCE` in a hundred of them one
//! chunk is a settlement's, picked by the world's seed. A settlement comes when that chunk is first
//! made and the land about it is there, on fairly flat, dry ground in a mild biome.

pub const REGION: i32 = 16;
pub const CHANCE: u64 = 45;
/// How far its cabins' doors are from the middle.
pub const RING: i32 = 8;
/// The farm's side, and how far its corner is from the middle.
pub const FARM: i32 = 9;
pub const FARM_AT: i32 = 7;
/// How far the woodcutter's spot is.
pub const WOOD_AT: i32 = 20;

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn hash(seed: i64, a: i32, b: i32, salt: u64) -> u64 {
    mix(mix(seed as u64 ^ salt).wrapping_add((a as u32 as u64) << 32 | (b as u32 as u64)))
}

/// The settlement chunk of region `r`, if it has one.
pub fn site_chunk(seed: i64, r: [i32; 2]) -> Option<[i32; 2]> {
    let h = hash(seed, r[0], r[1], 0xFE11_C0DE);
    if h % 100 >= CHANCE {
        return None;
    }
    let span = (REGION - 4) as u64;
    Some([r[0] * REGION + 2 + ((h >> 8) % span) as i32, r[1] * REGION + 2 + ((h >> 24) % span) as i32])
}

/// Whether chunk `c` is a settlement's.
pub fn is_site(seed: i64, c: [i32; 2]) -> bool {
    site_chunk(seed, [c[0].div_euclid(REGION), c[1].div_euclid(REGION)]) == Some(c)
}

/// Whether a settlement may stand in biome `b`.
pub fn biome_ok(b: &str) -> bool {
    const NOT: &[&str] = &["ocean", "river", "beach", "swamp", "jungle", "badlands", "peaks", "slopes", "dark_forest", "mushroom", "shore", "windswept", "deep_dark", "cave", "ice_spikes"];
    !b.is_empty() && !NOT.iter().any(|n| b.contains(n))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Farmer,
    Baker,
    Woodcutter,
    Elder,
    Guard,
}

impl Role {
    pub fn job(self) -> Option<&'static str> {
        match self {
            Role::Farmer => Some("farm"),
            Role::Baker => Some("bake"),
            Role::Woodcutter => Some("wood"),
            _ => None,
        }
    }

    pub fn shop(self) -> Option<&'static str> {
        match self {
            Role::Farmer => Some("projectfelix:farmer"),
            Role::Baker => Some("projectfelix:baker"),
            Role::Woodcutter => Some("projectfelix:woodcutter"),
            _ => None,
        }
    }

    pub fn routine(self) -> &'static str {
        match self {
            Role::Guard => "projectfelix:guard",
            Role::Elder => "projectfelix:night_owl",
            _ => "projectfelix:villager",
        }
    }
}

/// A cabin: its corner (from the middle, east and south), how it is turned, and the spot before
/// its door.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct House {
    pub corner: [i32; 2],
    pub rot: u8,
    pub front: [i32; 2],
}

/// A cabin's cell `at` (east, up, south from its corner, unturned), from the middle.
pub fn place(h: &House, at: [i32; 3]) -> [i32; 3] {
    let t = crate::plan::turn(at, h.rot);
    [h.corner[0] + t[0], t[1], h.corner[1] + t[2]]
}

/// What goes in a cabin besides its plan (unturned): two beds side by side (a couple shares a
/// cabin), a chest.
pub fn furniture() -> Vec<([i32; 3], &'static str)> {
    vec![
        ([3, 1, 1], "minecraft:red_bed[facing=north,part=head]"),
        ([3, 1, 2], "minecraft:red_bed[facing=north,part=foot]"),
        ([2, 1, 1], "minecraft:red_bed[facing=north,part=head]"),
        ([2, 1, 2], "minecraft:red_bed[facing=north,part=foot]"),
        ([1, 1, 3], "minecraft:chest[facing=east]"),
    ]
}

/// Inside a cabin, where its people sleep and keep home.
pub const INSIDE: [i32; 3] = [1, 1, 2];

#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub name: String,
    pub role: Role,
    pub home: usize,
    pub character: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    pub name: String,
    pub houses: Vec<House>,
    /// The field's corner, and which way (from the middle) it lies.
    pub farm: [i32; 2],
    pub wood: [i32; 2],
    pub people: Vec<Person>,
    /// The farmer and the baker (by index), who share a home; and whether they have a child.
    pub couple: (usize, usize),
    pub child: bool,
}

const NAMES: &[&str] = &[
    "Ada", "Bram", "Cora", "Dell", "Edda", "Finn", "Greta", "Hugo", "Ilse", "Jory", "Kit", "Lena", "Milo", "Nell", "Otto", "Pia", "Quin", "Rosa", "Sven", "Tilda", "Ulla", "Vik", "Wren", "Yara", "Abel", "Bess", "Cyril", "Dora", "Emil", "Fern",
];
const FIRST: &[&str] = &["Oak", "Ash", "Elder", "Thorn", "Bright", "Mill", "Stone", "Fern", "Hazel", "Willow", "Brook", "Clover"];
const LAST: &[&str] = &["hollow", "ford", "stead", "field", "brook", "dale", "wick", "hearth", "rest", "glen"];
const CHARACTERS: &[&str] = &["projectfelix:kind", "projectfelix:stoic", "projectfelix:touchy"];

/// The settlement seed `h` makes.
pub fn site(h: u64) -> Site {
    let mut r = h;
    let mut next = |n: u64| {
        r = mix(r);
        r % n.max(1)
    };
    let name = format!("{}{}", FIRST[next(FIRST.len() as u64) as usize], LAST[next(LAST.len() as u64) as usize]);
    // Cabins north, east, south, west of the middle, doors toward it; three or four of them.
    let count = 3 + next(2) as usize;
    let skip = next(4) as usize;
    let mut houses = Vec::new();
    for k in 0..4usize {
        if count == 3 && k == skip {
            continue;
        }
        let rot = k as u8;
        let front = match k {
            0 => [0, -RING],
            1 => [RING, 0],
            2 => [0, RING],
            _ => [-RING, 0],
        };
        let t = crate::plan::turn([2, 0, 5], rot);
        houses.push(House { corner: [front[0] - t[0], front[1] - t[2]], rot, front });
    }
    // The field and the woods on two corners between the cabins.
    let diag = next(4) as usize;
    let signs = [[1, 1], [-1, 1], [-1, -1], [1, -1]];
    let s = signs[diag];
    let farm = [if s[0] > 0 { FARM_AT } else { -FARM_AT - FARM + 1 }, if s[1] > 0 { FARM_AT } else { -FARM_AT - FARM + 1 }];
    let w = signs[(diag + 2) % 4];
    let wood = [w[0] * WOOD_AT, w[1] * WOOD_AT];
    // Its people.
    let mut roles = vec![(Role::Farmer, 0usize), (Role::Baker, 0), (Role::Woodcutter, 1), (Role::Elder, 2)];
    if houses.len() == 4 {
        roles.push((Role::Guard, 3));
    }
    let first = next(NAMES.len() as u64) as usize;
    let people = roles
        .into_iter()
        .enumerate()
        .map(|(i, (role, home))| Person { name: NAMES[(first + i * 7) % NAMES.len()].to_string(), role, home: home.min(houses.len() - 1), character: CHARACTERS[next(CHARACTERS.len() as u64) as usize] })
        .collect();
    let child = next(100) < 50;
    Site { name, houses, farm, wood, people, couple: (0, 1), child }
}

/// The spots whose ground a settlement needs (from the middle): to see that it is all there, dry
/// and flat enough.
pub fn samples(s: &Site) -> Vec<[i32; 2]> {
    let mut out = vec![[0, 0]];
    for h in &s.houses {
        out.push(h.front);
        for (x, z) in [(0, 0), (4, 0), (0, 4), (4, 4), (2, 2)] {
            let p = place(h, [x, 0, z]);
            out.push([p[0], p[2]]);
        }
    }
    for (x, z) in [(0, 0), (FARM - 1, 0), (0, FARM - 1), (FARM - 1, FARM - 1), (FARM / 2, FARM / 2)] {
        out.push([s.farm[0] + x, s.farm[1] + z]);
    }
    out.push(s.wood);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settlements_lay_out() {
        // About CHANCE in a hundred regions have one, always in their middle part.
        let found = (0..200).filter_map(|i| site_chunk(42, [i, -i])).count();
        assert!((60..=120).contains(&found), "{found}");
        let c = (0..50).find_map(|i| site_chunk(7, [i, 3])).unwrap();
        assert!(is_site(7, c));
        assert!(!is_site(7, [c[0] + 1, c[1]]));
        assert!(biome_ok("minecraft:plains") && biome_ok("minecraft:snowy_taiga") && !biome_ok("minecraft:deep_ocean") && !biome_ok("minecraft:river"));
        for h in 0..40u64 {
            let s = site(h);
            assert!(s.houses.len() >= 3);
            // Each door's outside spot is its front, facing the middle.
            for house in &s.houses {
                let out = place(house, [2, 0, 5]);
                assert_eq!([out[0], out[2]], house.front);
                let door = place(house, [2, 1, 4]);
                assert!(door[0].abs() + door[2].abs() < RING + 2);
            }
            // The field is clear of the cabins.
            for house in &s.houses {
                for x in 0..5 {
                    for z in 0..5 {
                        let p = place(house, [x, 0, z]);
                        assert!(!(p[0] >= s.farm[0] && p[0] < s.farm[0] + FARM && p[2] >= s.farm[1] && p[2] < s.farm[1] + FARM), "seed {h}");
                    }
                }
            }
            assert!(s.people.iter().all(|p| p.home < s.houses.len()));
            let names: std::collections::HashSet<_> = s.people.iter().map(|p| &p.name).collect();
            assert_eq!(names.len(), s.people.len());
        }
        assert_eq!(crate::plan::turn_state("minecraft:red_bed[facing=north,part=head]", 1), "minecraft:red_bed[facing=east,part=head]");
    }
}
