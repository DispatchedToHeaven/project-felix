//! A village's head, and what it decides for the village. No mod API types here.
//!
//! The grown-ups of a village choose their head every seven days: the one the others like best,
//! who talks well and is in good spirits. Each morning the head looks at how the village stands
//! and decides: a festival when spirits are low, a new cabin when someone slept without a bed, a
//! job for a grown-up who has none (what the village lacks most), a newcomer when there is room
//! and folk are happy, and what to ask travellers for when the village runs short.

/// Days between elections, festivals, newcomers.
pub const TERM: i64 = 7;
pub const FESTIVAL_EVERY: i64 = 4;
pub const NEWCOMER_EVERY: i64 = 3;
/// At most this many live in a village.
pub const MOST: usize = 12;

/// One who may be chosen: how much the others like them (summed), their talking, their mood.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub uuid: u128,
    pub name: String,
    pub support: f32,
    pub talking: u8,
    pub mood: f32,
}

/// Who wins: the most liked, the best spoken and the happiest, in that order of weight.
pub fn elect(c: &[Candidate]) -> Option<&Candidate> {
    c.iter().max_by(|a, b| score(a).total_cmp(&score(b)).then(b.uuid.cmp(&a.uuid)))
}

pub fn score(c: &Candidate) -> f32 {
    c.support + 2.0 * c.talking as f32 + c.mood / 10.0
}

/// How the village stands this morning.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub people: usize,
    pub beds: usize,
    /// Grown-ups with no job (not the head, not a guard), by uuid.
    pub jobless: Vec<u128>,
    /// How many slept without a bed last night.
    pub rough: usize,
    /// Food (worth 3 or more) and logs in the village's chests.
    pub food: u64,
    pub logs: u64,
    pub mood: f32,
    pub days_since_festival: i64,
    pub days_since_newcomer: i64,
    pub building: bool,
    /// Jobs someone in the village does now (a newcomer or the jobless can only take those).
    pub jobs: Vec<String>,
    /// Its guards, and whether raiders came lately (see `defence`).
    pub guards: usize,
    pub wary: bool,
    /// Who could be made a guard: uuid, fighting, has no job.
    pub recruits: Vec<(u128, u8, bool)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Festival,
    BuildCabin,
    GiveJob(u128, String),
    /// Makes this one a guard.
    Appoint(u128),
    Newcomer(String),
    Ask(String, u32),
}

/// What the village lacks most, as a job: food first, then wood.
pub fn needed_job(r: &Report) -> String {
    let has = |j: &str| r.jobs.iter().any(|x| x == j);
    let short_food = r.food < (r.people as u64) * 8;
    let short_wood = r.logs < 32;
    let pick = if short_food && has("farm") {
        "farm"
    } else if short_food && has("fish") {
        "fish"
    } else if short_wood && has("wood") {
        "wood"
    } else if has("farm") {
        "farm"
    } else {
        r.jobs.first().map(String::as_str).unwrap_or("farm")
    };
    pick.to_string()
}

/// The head's decisions this morning.
pub fn decide(r: &Report) -> Vec<Decision> {
    let mut out = Vec::new();
    if r.mood < 45.0 && r.days_since_festival >= FESTIVAL_EVERY {
        out.push(Decision::Festival);
    }
    if (r.rough > 0 || r.people + 1 > r.beds) && !r.building {
        out.push(Decision::BuildCabin);
    }
    // A guard short: the best fighter is made one (one a morning).
    let guard = if r.guards < crate::defence::guards_wanted(r.people, r.wary) { crate::defence::recruit(&r.recruits) } else { None };
    if let Some(u) = guard {
        out.push(Decision::Appoint(u));
    }
    for u in r.jobless.iter().filter(|u| Some(**u) != guard) {
        out.push(Decision::GiveJob(*u, needed_job(r)));
    }
    if r.people < MOST && r.people < r.beds && r.mood >= 50.0 && r.days_since_newcomer >= NEWCOMER_EVERY {
        out.push(Decision::Newcomer(needed_job(r)));
    }
    if r.food < (r.people as u64) * 4 {
        out.push(Decision::Ask("minecraft:bread".into(), (r.people as u32 * 4).max(8)));
    }
    if r.building && r.logs < 16 {
        out.push(Decision::Ask("minecraft:oak_log".into(), 24));
    }
    out
}

/// The logs a cabin's blocks take: planks four to a log, slabs eight, doors and logs one each;
/// what isn't wooden the village gets by trade.
pub fn logs_for(item: &str, n: u32) -> u32 {
    let i = item.trim_start_matches("minecraft:");
    if i.ends_with("_planks") {
        n.div_ceil(4)
    } else if i.ends_with("_slab") {
        n.div_ceil(8)
    } else if i.ends_with("_log") || i.ends_with("_door") || i.ends_with("_stairs") || i.ends_with("_fence") || i.ends_with("_trapdoor") {
        n
    } else {
        0
    }
}

/// Where the `k`th new cabin goes, from the middle (out past the first ring, round about).
pub fn cabin_spot(k: usize) -> [i32; 2] {
    const SPOTS: [[i32; 2]; 8] = [[0, -18], [18, 0], [0, 18], [-18, 0], [16, -16], [16, 16], [-16, 16], [-16, -16]];
    let s = SPOTS[k % SPOTS.len()];
    let far = (k / SPOTS.len()) as i32;
    [s[0] + s[0].signum() * 8 * far, s[1] + s[1].signum() * 8 * far]
}

/// The turn (0..4) that sets a door at `door` (from the plan's middle, unturned) facing the
/// village's middle, `toward` from the spot.
pub fn facing_turn(door: [i32; 2], toward: [i32; 2], turn: impl Fn([i32; 3], u8) -> [i32; 3]) -> u8 {
    (0..4u8)
        .max_by_key(|r| {
            let d = turn([door[0], 0, door[1]], *r);
            d[0] * toward[0] + d[2] * toward[1]
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, support: f32, talking: u8, mood: f32) -> Candidate {
        Candidate { uuid: name.len() as u128, name: name.into(), support, talking, mood }
    }

    #[test]
    fn the_liked_one_leads() {
        let list = vec![c("Ada", 40.0, 2, 50.0), c("Hugo", 10.0, 9, 60.0), c("Quin", -20.0, 20, 90.0)];
        assert_eq!(elect(&list).unwrap().name, "Ada");
        assert!(elect(&[]).is_none());
    }

    #[test]
    fn mornings() {
        let r = Report {
            people: 5,
            beds: 8,
            jobless: vec![7],
            rough: 1,
            food: 10,
            logs: 50,
            mood: 40.0,
            days_since_festival: 9,
            days_since_newcomer: 9,
            building: false,
            jobs: vec!["farm".into(), "wood".into()],
            ..Report::default()
        };
        let d = decide(&r);
        assert!(d.contains(&Decision::Festival) && d.contains(&Decision::BuildCabin));
        assert!(d.contains(&Decision::GiveJob(7, "farm".into())));
        assert!(!d.iter().any(|x| matches!(x, Decision::Newcomer(_))), "{d:?}");
        assert!(d.iter().any(|x| matches!(x, Decision::Ask(i, _) if i == "minecraft:bread")));
        let happy = Report { mood: 70.0, rough: 0, food: 500, jobless: vec![], ..r };
        let d = decide(&happy);
        assert_eq!(d, vec![Decision::Newcomer("farm".into())]);
        let wood = Report { food: 500, logs: 0, ..happy.clone() };
        assert_eq!(needed_job(&wood), "wood");
        // No guard: the best fighter is made one, and is not given other work.
        let open = Report { jobless: vec![7, 8], recruits: vec![(7, 6, true), (8, 2, true), (9, 6, false)], ..happy.clone() };
        let d = decide(&open);
        assert!(d.contains(&Decision::Appoint(7)) && d.contains(&Decision::GiveJob(8, "farm".into())), "{d:?}");
        assert!(!d.contains(&Decision::GiveJob(7, "farm".into())));
        let kept = Report { guards: 1, ..open.clone() };
        assert!(!decide(&kept).iter().any(|x| matches!(x, Decision::Appoint(_))));
        let wary = Report { guards: 1, wary: true, people: 6, ..open };
        assert!(decide(&wary).contains(&Decision::Appoint(7)));
    }

    #[test]
    fn cabins_face_the_middle() {
        assert_eq!(logs_for("minecraft:oak_planks", 55), 14);
        assert_eq!(logs_for("minecraft:glass_pane", 4), 0);
        let turn = |at: [i32; 3], rot: u8| match rot % 4 {
            0 => at,
            1 => [-at[2], at[1], at[0]],
            2 => [-at[0], at[1], -at[2]],
            _ => [at[2], at[1], -at[0]],
        };
        // A door on the south side (+z): a cabin north of the middle keeps it, one south turns it.
        assert_eq!(facing_turn([0, 2], [0, 1], turn), 0);
        assert_eq!(facing_turn([0, 2], [0, -1], turn), 2);
        let s = cabin_spot(9);
        assert!(s[0].abs() + s[1].abs() > 18);
    }
}
