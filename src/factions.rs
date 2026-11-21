//! Villages among others on the server (see `faction`): each Felix village keeps what it thinks of
//! every player, by what they did for it or to it, and how it gets on with each other village.
//! An enemy is not spoken to and its guards go for them; the unwelcome pay more, friends and
//! allies less, and may ask the head for a share of the village's food. Heads send players with
//! their good word to other villages; caravans come from villages on good terms, friends send
//! food to the hungry and fighters to those under attack, neighbours quarrel (see `storyteller`). Kept with the world in
//! `factions.json`. `/felix standing`, `/felix ties`, and for ops `/felix standing set <value>`,
//! `/felix ties set <value>` (the nearest village, the two nearest villages).
use crate::faction::{self, Bond, Deed, Errand, Rank, Standing, Tie};
use crate::serde_json::{Value, json};
use mod_api::{Command, CommandArgumentValue, CommandSource, ConfigPath, EntityRef, Flow, Host, PlayerRef, RegisterCommandsEvent, RunModCommandEvent, ServerHost, Text};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

const OVERWORLD: &str = "minecraft:overworld";
const FILE: &str = "factions.json";
const DAY: i64 = faction::DAY;
/// How far from its post a guard sees an enemy of the village.
const SIGHT: f64 = 16.0;

/// What is known of a village (as it was when its people were last about).
#[derive(Clone, Debug, Default)]
struct Known {
    people: usize,
    food: u64,
    day: i64,
    /// Food it promised another village, to come out of its chests.
    owes: u32,
}

#[derive(Default)]
struct State {
    read: bool,
    world: Option<std::path::PathBuf>,
    standing: HashMap<String, HashMap<u128, Standing>>,
    ties: HashMap<String, Tie>,
    known: HashMap<String, Known>,
    errands: HashMap<u128, Errand>,
    /// Who is an enemy where (the village's tag, the player): what guards look for.
    foes: Vec<(String, u128)>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

fn hex(u: u128) -> String {
    format!("{u:032x}")
}

fn unhex(s: &str) -> Option<u128> {
    u128::from_str_radix(s, 16).ok()
}

fn tag_of(name: &str) -> String {
    format!("felix_{}", name.to_lowercase())
}

fn refoe(s: &mut State) {
    s.foes = s.standing.iter().flat_map(|(v, m)| m.iter().filter(|(_, st)| st.rank() == Rank::Enemy).map(|(u, _)| (tag_of(v), *u))).collect();
}

fn read(host: &mut dyn ServerHost) {
    let world = host.world_dir();
    if STATE.lock().map(|s| s.read && s.world == world).unwrap_or(true) {
        return;
    }
    let v = ConfigPath::new(FILE).ok().and_then(|p| host.world_data_files().read(&p).ok()).and_then(|b| crate::serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
    let i = |x: &Value, k: &str| x.get(k).and_then(Value::as_i64).unwrap_or(0);
    let mut standing: HashMap<String, HashMap<u128, Standing>> = HashMap::new();
    if let Some(m) = v.get("standing").and_then(Value::as_object) {
        for (village, players) in m {
            let Some(players) = players.as_object() else { continue };
            let list = standing.entry(village.clone()).or_default();
            for (u, x) in players {
                let Some(u) = unhex(u) else { continue };
                list.insert(
                    u,
                    Standing {
                        value: (i(x, "value") as i32).clamp(faction::MIN, faction::MAX),
                        name: x.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                        day: i(x, "day"),
                        favour_day: i(x, "favour_day"),
                        favours: i(x, "favours").clamp(0, 255) as u8,
                        hit_at: i(x, "hit_at"),
                        share_day: i(x, "share_day"),
                    },
                );
            }
        }
    }
    let mut ties = HashMap::new();
    if let Some(m) = v.get("ties").and_then(Value::as_object) {
        for (k, x) in m {
            ties.insert(k.clone(), Tie { value: (i(x, "value") as i32).clamp(faction::MIN, faction::MAX), day: i(x, "day") });
        }
    }
    let mut known = HashMap::new();
    if let Some(m) = v.get("known").and_then(Value::as_object) {
        for (k, x) in m {
            known.insert(k.clone(), Known { people: i(x, "people").max(0) as usize, food: i(x, "food").max(0) as u64, day: i(x, "day"), owes: i(x, "owes").max(0) as u32 });
        }
    }
    let mut errands = HashMap::new();
    if let Some(m) = v.get("errands").and_then(Value::as_object) {
        for (u, x) in m {
            let (Some(u), Some(from), Some(to)) = (unhex(u), x.get("from").and_then(Value::as_str), x.get("to").and_then(Value::as_str)) else { continue };
            errands.insert(u, Errand { from: from.to_string(), to: to.to_string(), day: i(x, "day") });
        }
    }
    if let Ok(mut s) = STATE.lock() {
        *s = State { read: true, world, standing, ties, known, errands, foes: Vec::new() };
        refoe(&mut s);
    }
}

fn write(host: &mut dyn ServerHost) {
    let v = STATE.lock().map(|s| {
        let standing: crate::serde_json::Map<String, Value> = s
            .standing
            .iter()
            .map(|(village, m)| {
                let players: crate::serde_json::Map<String, Value> = m
                    .iter()
                    .map(|(u, x)| (hex(*u), json!({ "value": x.value, "name": x.name, "day": x.day, "favour_day": x.favour_day, "favours": x.favours, "hit_at": x.hit_at, "share_day": x.share_day })))
                    .collect();
                (village.clone(), Value::Object(players))
            })
            .collect();
        let ties: crate::serde_json::Map<String, Value> = s.ties.iter().map(|(k, t)| (k.clone(), json!({ "value": t.value, "day": t.day }))).collect();
        let known: crate::serde_json::Map<String, Value> = s.known.iter().map(|(k, x)| (k.clone(), json!({ "people": x.people, "food": x.food, "day": x.day, "owes": x.owes }))).collect();
        let errands: crate::serde_json::Map<String, Value> = s.errands.iter().map(|(u, e)| (hex(*u), json!({ "from": e.from, "to": e.to, "day": e.day }))).collect();
        json!({ "standing": standing, "ties": ties, "known": known, "errands": errands })
    });
    if let (Ok(v), Ok(p)) = (v, ConfigPath::new(FILE))
        && let Err(e) = host.world_data_files().write(&p, &crate::serde_json::to_vec_pretty(&v).unwrap_or_default())
    {
        eprintln!("[projectfelix] villages' standings not saved: {e:?}");
    }
}

fn now_of(host: &dyn Host) -> i64 {
    host.level(OVERWORLD).and_then(|l| l.game_time(host)).unwrap_or(0)
}

fn tell(host: &mut dyn ServerHost, p: PlayerRef, line: &str) {
    let _ = p.tell(host, Text::literal(line.to_string()));
}

fn tie_of(a: &str, b: &str) -> i32 {
    STATE.lock().ok().and_then(|s| s.ties.get(&faction::key(a, b)).map(|t| t.value)).unwrap_or(0)
}

/// Warms (or cools) the tie between `a` and `b`: how they got on before, and now.
fn warm(a: &str, b: &str, by: i32, day: i64) -> (Bond, Bond) {
    let Ok(mut s) = STATE.lock() else { return (Bond::Neighbours, Bond::Neighbours) };
    let t = s.ties.entry(faction::key(a, b)).or_insert(Tie { value: 0, day });
    let before = t.bond();
    t.change(by);
    (before, t.bond())
}

/// The other villages, and `name`'s tie with each.
fn ties_of(host: &mut dyn ServerHost, name: &str) -> Vec<(String, i32)> {
    crate::settlements::list(host).into_iter().map(|(_, n)| n).filter(|n| n != name).map(|n| {
        let t = tie_of(name, &n);
        (n, t)
    }).collect()
}

/// How many other villages `name` knows of.
pub fn neighbours(host: &mut dyn ServerHost, name: &str) -> usize {
    crate::settlements::list(host).into_iter().filter(|(_, n)| n != name).count()
}

fn standing_of(name: &str, player: u128) -> Standing {
    STATE.lock().ok().and_then(|s| s.standing.get(name).and_then(|m| m.get(&player).cloned())).unwrap_or_default()
}

/// What is said when a village comes to see a player differently.
fn rank_line(village: &str, before: Rank, after: Rank) -> String {
    if after > before {
        match after {
            Rank::Ally => format!("{village} now counts you an ally: one of its own."),
            Rank::Friend => format!("{village} now counts you a friend."),
            Rank::Stranger => format!("{village} has put its grudge against you aside."),
            _ => format!("{village} no longer sees you as an enemy. You are not welcome there yet."),
        }
    } else {
        match after {
            Rank::Enemy => format!("{village} now sees you as an enemy. Its guards will not let you near."),
            Rank::Unwelcome => format!("You are no longer welcome in {village}."),
            Rank::Stranger => format!("{village} no longer counts you a friend."),
            _ => format!("{village} no longer counts you an ally."),
        }
    }
}

/// Player `player` did `d` for (or to) village `name`: its standing there moves, and word of it
/// goes to the villages that are that one's friends or rivals.
pub fn deed(host: &mut dyn ServerHost, name: &str, player: PlayerRef, d: Deed) {
    apply(host, name, player, d, true);
}

fn apply(host: &mut dyn ServerHost, name: &str, player: PlayerRef, d: Deed, spreads: bool) -> i32 {
    read(host);
    let now = now_of(&*host);
    let day = now / DAY;
    let uuid = player.uuid();
    let pname = player.name(&*host).unwrap_or_default();
    let others = if spreads { ties_of(host, name) } else { Vec::new() };
    let mut told = Vec::new();
    let mut numbers = Vec::new();
    let moved = {
        let Ok(mut s) = STATE.lock() else { return 0 };
        let moved = {
            let st = s.standing.entry(name.to_string()).or_default().entry(uuid).or_insert_with(|| Standing { day, ..Standing::default() });
            st.name = pname.clone();
            let before = st.rank();
            let moved = st.deed(d, now);
            if moved != 0 {
                numbers.push(format!("[{name}] standing {moved:+} ({})", st.value));
                if st.rank() != before {
                    told.push(rank_line(name, before, st.rank()));
                }
            }
            moved
        };
        if moved != 0 {
            for (o, tie) in &others {
                let e = faction::echo(*tie, moved);
                if e == 0 {
                    continue;
                }
                let so = s.standing.entry(o.clone()).or_default().entry(uuid).or_insert_with(|| Standing { day, ..Standing::default() });
                so.name = pname.clone();
                let before = so.rank();
                if so.change(e) != 0 {
                    numbers.push(format!("[{o}] standing {e:+} ({}): word from {name}", so.value));
                    if so.rank() != before {
                        told.push(rank_line(o, before, so.rank()));
                    }
                }
            }
            refoe(&mut s);
        }
        moved
    };
    if moved == 0 {
        return 0;
    }
    write(host);
    for l in told {
        tell(host, player, &l);
    }
    if crate::flags::extended(&*host, player) {
        for l in numbers {
            tell(host, player, &l);
        }
    }
    moved
}

/// Player `player` did `d` for (or to) NPC `npc`: it counts with its village, if it has one.
pub fn deed_by(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef, d: Deed) {
    if let Some(name) = crate::chiefs::village_of(host, npc) {
        deed(host, &name, player, d);
    }
}

/// What `player` is to `npc`'s village (none: it has no village).
pub fn rank_of(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) -> Option<(String, Rank)> {
    let name = crate::chiefs::village_of(host, npc)?;
    read(host);
    let r = standing_of(&name, player.uuid()).rank();
    Some((name, r))
}

/// Talk starts: true (and the NPC says so) if its village won't have `player` spoken to.
pub fn shuns(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) -> bool {
    // Fast when nobody is anybody's enemy.
    if STATE.lock().is_ok_and(|s| s.read && s.foes.is_empty()) {
        return false;
    }
    let Some((name, Rank::Enemy)) = rank_of(host, npc, player) else { return false };
    let pname = player.name(&*host).unwrap_or_default();
    crate::heart::say_to(host, npc, player, &Rank::Enemy.says(&name, &pname));
    true
}

/// What `npc` asks of `player` beside its usual prices, by what they are to its village, and a
/// note for its stall.
pub fn price_factor(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) -> (f32, &'static str) {
    match rank_of(host, npc, player).map(|r| r.1) {
        Some(r @ Rank::Unwelcome) => (r.price().unwrap_or(1.0), "(a worse deal: the village doesn't care for you)"),
        Some(r @ Rank::Friend) => (r.price().unwrap_or(1.0), "(a better deal for a friend of the village)"),
        Some(r @ Rank::Ally) => (r.price().unwrap_or(1.0), "(a better deal for an ally of the village)"),
        Some(Rank::Enemy) => (1.5, ""),
        _ => (1.0, ""),
    }
}

/// An enemy of guard `me`'s village in sight of its post, if any (not one who can't be hurt).
pub fn foe_near(host: &dyn Host, me: EntityRef, post: [f64; 3]) -> Option<EntityRef> {
    let foes = {
        let s = STATE.lock().ok()?;
        if s.foes.is_empty() {
            return None;
        }
        s.foes.clone()
    };
    let tags = me.tags(host).unwrap_or_default();
    let players = me.level(host)?.online_players(host)?;
    players
        .into_iter()
        .filter(|p| foes.iter().any(|(t, u)| *u == p.uuid() && tags.iter().any(|x| x == t)))
        .filter(|p| p.is_creative(host) != Some(true) && p.is_spectator(host) != Some(true))
        .filter_map(|p| {
            let at = p.entity().position(host)?;
            let d = (at[0] - post[0]).powi(2) + (at[1] - post[1]).powi(2) + (at[2] - post[2]).powi(2);
            (d < SIGHT * SIGHT).then_some((d, p.entity()))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|x| x.1)
}

/// A player hit `npc`: its village thinks less of them.
pub fn hit(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) {
    deed_by(host, npc, player, Deed::Hit);
}

/// Something died by a player's hand, or by that of an NPC of theirs: a raider at a village's
/// gates is to their credit there, one of a village's own is not forgiven for a long time.
pub fn on_death(host: &mut dyn ServerHost, victim: u32, killer: i32) {
    if killer <= 0 {
        return;
    }
    let Some(e) = host.entity(victim) else { return };
    let Some(k) = host.entity(killer as u32) else { return };
    let owner = mod_api::LivingRef::new(k).owner(&*host);
    let players = e.level(&*host).and_then(|l| l.online_players(&*host)).unwrap_or_default();
    let Some(p) = players.iter().copied().find(|p| p.entity().id() == k.id()).or_else(|| players.iter().copied().find(|p| Some(p.uuid()) == owner)) else { return };
    let tags = e.tags(&*host).unwrap_or_default();
    if tags.iter().any(|t| t == "felix_raider") {
        let village = crate::settlements::list(host).into_iter().map(|(_, n)| n).find(|n| tags.contains(&format!("felix_raider_{}", n.to_lowercase())));
        if let Some(name) = village {
            deed(host, &name, p, Deed::RaiderSlain);
        }
    } else if crate::companion::is_npc(&*host, e) {
        deed_by(host, e, p, Deed::Killed);
    }
}

/// Which village a caravan to `name` comes from, if there is one it gets on with: the visit
/// warms their tie.
pub fn caravan_from(host: &mut dyn ServerHost, name: &str, roll: u64, now: i64) -> Option<String> {
    read(host);
    let ties = ties_of(host, name);
    let from = faction::caravan_from(&ties, roll)?.to_string();
    warm(name, &from, faction::CARAVAN, now / DAY);
    write(host);
    Some(from)
}

/// The friendly village that could send `name` (of `people`, with `food`) some food now, and how much.
pub fn aid_for(host: &mut dyn ServerHost, name: &str, people: usize, food: u64) -> Option<(String, u32)> {
    read(host);
    let mut ties = ties_of(host, name);
    ties.sort_by_key(|t| -t.1);
    let s = STATE.lock().ok()?;
    ties.into_iter().find_map(|(o, tie)| {
        let k = s.known.get(&o)?;
        let n = faction::aid(tie, people, food, k.people, k.food.saturating_sub(u64::from(k.owes)))?;
        Some((o, n))
    })
}

/// Village `from` sends `n` food to `to`: it comes out of its chests when its people are next
/// about, and the two are closer for it.
pub fn send_aid(host: &mut dyn ServerHost, from: &str, to: &str, n: u32, now: i64) {
    read(host);
    if let Ok(mut s) = STATE.lock()
        && let Some(k) = s.known.get_mut(from)
    {
        k.owes += n;
    }
    warm(from, to, faction::AID, now / DAY);
    write(host);
}

/// The friendly village that sends its own to stand with `name` against raiders, and how many.
pub fn helpers_for(host: &mut dyn ServerHost, name: &str) -> Option<(String, u32)> {
    read(host);
    let mut ties = ties_of(host, name);
    ties.sort_by_key(|t| -t.1);
    let s = STATE.lock().ok()?;
    ties.into_iter().find_map(|(o, tie)| {
        let n = crate::defence::helpers(tie, s.known.get(&o)?.people);
        (n > 0).then_some((o, n))
    })
}

/// Villages `a` and `b` stood together: they are closer for it.
pub fn thank(host: &mut dyn ServerHost, a: &str, b: &str, by: i32, now: i64) {
    read(host);
    warm(a, b, by, now / DAY);
    write(host);
}

/// Tells `line` to the friends and allies of village `name` among the players, wherever in the
/// overworld they are (those within 64 of its middle hear what happens there anyway).
pub fn call_friends(host: &mut dyn ServerHost, name: &str, center: [i32; 2], line: &str) {
    read(host);
    let Some(level) = host.level(OVERWORLD) else { return };
    for p in level.online_players(&*host).unwrap_or_default() {
        let near = p.entity().position(&*host).is_some_and(|q| (q[0] - f64::from(center[0])).powi(2) + (q[2] - f64::from(center[1])).powi(2) < 64.0 * 64.0);
        if !near && standing_of(name, p.uuid()).rank() >= Rank::Friend {
            tell(host, p, line);
        }
    }
}

/// `name` falls out with a village near it: which, over what, and how they get on now.
pub fn fall_out(host: &mut dyn ServerHost, name: &str, roll: u64, now: i64) -> Option<(String, &'static str, Bond)> {
    read(host);
    let ties = ties_of(host, name);
    if ties.is_empty() {
        return None;
    }
    let other = ties[((roll >> 8) % ties.len() as u64) as usize].0.clone();
    let (_, after) = warm(name, &other, faction::QUARREL, now / DAY);
    write(host);
    Some((other, faction::quarrel(roll), after))
}

/// Takes up to `n` food from the chests of `people`'s village: what was taken.
fn take_food(host: &mut dyn ServerHost, people: &[EntityRef], n: u32) -> Vec<(String, u32)> {
    let mut left = n;
    let mut out: Vec<(String, u32)> = Vec::new();
    for c in crate::chiefs::chests(host, people) {
        for f in crate::need::FOODS.iter().filter(|f| f.1 >= 3) {
            if left == 0 {
                return out;
            }
            let k = crate::jobs::amount(host, c, f.0).min(u64::from(left)) as u32;
            if k > 0 && crate::jobs::take(host, c, f.0, k) {
                left -= k;
                match out.iter_mut().find(|o| o.0 == f.0) {
                    Some(o) => o.1 += k,
                    None => out.push((f.0.to_string(), k)),
                }
            }
        }
    }
    out
}

/// The overworld's tick: grudges fade, feuds cool, errands grow stale; each village about is
/// looked at once a day, and what it promised others leaves its chests.
pub fn tick(host: &mut dyn ServerHost) {
    let now = now_of(&*host);
    if !crate::pace::since("factions", now, 600) {
        return;
    }
    read(host);
    let day = now / DAY;
    let mut changed = false;
    if let Ok(mut s) = STATE.lock() {
        for st in s.standing.values_mut().flat_map(|m| m.values_mut()) {
            let (v, d) = (st.value, st.day);
            st.drift(day);
            changed |= v != st.value || d != st.day;
        }
        for t in s.ties.values_mut() {
            let (v, d) = (t.value, t.day);
            t.drift(day);
            changed |= v != t.value || d != t.day;
        }
        let n = s.errands.len();
        s.errands.retain(|_, e| !e.stale(day));
        changed |= n != s.errands.len();
        if changed {
            refoe(&mut s);
        }
    }
    for (center, name) in crate::settlements::list(host) {
        let k = STATE.lock().ok().and_then(|s| s.known.get(&name).cloned());
        if k.as_ref().is_some_and(|k| k.day == day && k.owes == 0) {
            continue;
        }
        let people = crate::chiefs::members(host, &name);
        if people.is_empty() {
            continue;
        }
        let mut owes = k.map_or(0, |k| k.owes);
        if owes > 0 {
            let sent: u32 = take_food(host, &people, owes).iter().map(|x| x.1).sum();
            // What it hasn't got it can't send: the rest is let go.
            owes = if sent == 0 { 0 } else { owes - sent };
        }
        let r = crate::chiefs::report(host, &name, center, &people, now);
        if let Ok(mut s) = STATE.lock() {
            s.known.insert(name.clone(), Known { people: r.people, food: r.food, day, owes });
        }
        changed = true;
    }
    if changed {
        write(host);
    }
}

/// What villagers say of their village among others (the `faction` dialog action): what folk
/// think of the player (`standing`), how the village gets on with others (`ties`); and, from the
/// head, its good word to carry to another village (`errand`), word brought from one (`word`), a
/// share of its food for a friend (`help`).
pub fn talk(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef, what: &str) {
    read(host);
    let Some(name) = crate::chiefs::village_of(host, npc) else {
        crate::heart::say_to(host, npc, player, "I belong to no village. I go where I please.");
        return;
    };
    let pname = player.name(&*host).unwrap_or_default();
    let uuid = player.uuid();
    let now = now_of(&*host);
    let day = now / DAY;
    let extended = crate::flags::extended(&*host, player);
    let mine = standing_of(&name, uuid);
    match what {
        "standing" => {
            let mut line = mine.rank().says(&name, &pname);
            if extended {
                line.push_str(&format!(" (standing {})", mine.value));
            }
            crate::heart::say_to(host, npc, player, &line);
            return;
        }
        "ties" => {
            let ties = ties_of(host, &name);
            let line = if ties.is_empty() {
                "I know of no other village near here.".to_string()
            } else {
                let list: Vec<String> = ties.iter().map(|(o, t)| if extended { format!("{} ({t})", Bond::of(*t).phrase(o)) } else { Bond::of(*t).phrase(o) }).collect();
                format!("We're {}.", list.join("; "))
            };
            crate::heart::say_to(host, npc, player, &line);
            return;
        }
        _ => {}
    }
    // The rest is the head's to say.
    let people = crate::chiefs::members(host, &name);
    let head = crate::chiefs::head_of(host, &name, &people);
    if head.map(|h| h.id()) != Some(npc.id()) {
        let line = match crate::chiefs::head_name(host, &name) {
            Some(h) => format!("You'd better ask {h}, our head."),
            None => "We've no head to speak for us just now.".to_string(),
        };
        crate::heart::say_to(host, npc, player, &line);
        return;
    }
    match what {
        "errand" => {
            let ties = ties_of(host, &name);
            let carried = STATE.lock().ok().and_then(|s| s.errands.get(&uuid).cloned()).filter(|e| !e.stale(day));
            let line = if mine.rank() < Rank::Stranger {
                "We've no word to send with the likes of you.".to_string()
            } else if let Some(e) = carried {
                format!("You already carry {}'s word to {}. See it there first.", e.from, e.to)
            } else if let Some(to) = faction::errand_to(&ties).map(str::to_string) {
                let there = crate::settlements::list(host).into_iter().find(|(_, n)| *n == to).map(|(c, _)| c);
                let way = npc.position(&*host).zip(there).map(|(at, c)| {
                    let (dx, dz) = (f64::from(c[0]) - at[0], f64::from(c[1]) - at[2]);
                    format!(" It lies about {:.0} blocks to the {}.", dx.hypot(dz), crate::book::direction(dx, dz))
                });
                if let Ok(mut s) = STATE.lock() {
                    s.errands.insert(uuid, Errand { from: name.clone(), to: to.clone(), day });
                }
                write(host);
                format!("Would you carry our good word to {to}, {pname}?{} Speak to their head, and say {name} sent you.", way.unwrap_or_default())
            } else {
                format!("There's no other village I know of, {pname}.")
            };
            crate::heart::say_to(host, npc, player, &line);
        }
        "word" => {
            let carried = STATE.lock().ok().and_then(|s| s.errands.get(&uuid).cloned()).filter(|e| !e.stale(day));
            let Some(e) = carried.filter(|e| e.to == name) else {
                let line = match STATE.lock().ok().and_then(|s| s.errands.get(&uuid).cloned()).filter(|e| !e.stale(day)) {
                    Some(e) => format!("That word is for {}, not for us.", e.to),
                    None => format!("You carry no word for us, {pname}."),
                };
                crate::heart::say_to(host, npc, player, &line);
                return;
            };
            if let Ok(mut s) = STATE.lock() {
                s.errands.remove(&uuid);
            }
            let (before, after) = warm(&e.from, &name, faction::WORD, day);
            apply(host, &e.from, player, Deed::Envoy, false);
            apply(host, &name, player, Deed::Envoy, false);
            for m in &people {
                crate::minds::remember(host, *m, "good_word");
            }
            let news = format!("{pname} brought {}'s good word to {name}.", e.from);
            crate::chiefs::record(host, &name, now, &news);
            crate::chiefs::record(host, &e.from, now, &news);
            let pay = 2.min(crate::markets::purse_of(host, npc));
            if pay > 0 {
                crate::markets::spend(host, npc, pay);
                crate::talk::queue(format!("give {pname} minecraft:emerald {pay}"));
            }
            write(host);
            crate::heart::say_to(host, npc, player, &format!("From {}? Then tell them {name} thinks kindly of them too. Thank you, {pname}.", e.from));
            if after != before {
                tell(host, player, &format!("{name} is now {}.", after.phrase(&e.from)));
            }
        }
        "help" => {
            let n = mine.rank().share();
            let center = crate::settlements::list(host).into_iter().find(|(_, x)| *x == name).map(|(c, _)| c).unwrap_or([0, 0]);
            let line = if n == 0 {
                format!("We look after our own first, {pname}. Maybe when folk here know you better.")
            } else if mine.share_day == day + 1 {
                format!("You've had your share today, {pname}. Come back tomorrow.")
            } else if crate::chiefs::report(host, &name, center, &people, now).food < 3 * people.len() as u64 + u64::from(n) {
                format!("I wish I could, {pname}. We're short ourselves.")
            } else {
                let got = take_food(host, &people, n);
                if got.is_empty() {
                    format!("I wish I could, {pname}. We're short ourselves.")
                } else {
                    for (item, k) in &got {
                        crate::talk::queue(format!("give {pname} {item} {k}"));
                    }
                    if let Ok(mut s) = STATE.lock()
                        && let Some(st) = s.standing.get_mut(&name).and_then(|m| m.get_mut(&uuid))
                    {
                        st.share_day = day + 1;
                    }
                    write(host);
                    let list: Vec<String> = got.iter().map(|(i, k)| format!("{k} {}", i.trim_start_matches("minecraft:").replace('_', " "))).collect();
                    format!("Here, {pname}: {}. {name} looks after its friends.", list.join(" and "))
                }
            };
            crate::heart::say_to(host, npc, player, &line);
        }
        _ => {}
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::new("felix standing"));
    e.commands.push(Command::op("felix standing set <value:integer>", 2));
    e.commands.push(Command::new("felix ties"));
    e.commands.push(Command::op("felix ties set <value:integer>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if lit.first() != Some(&"felix") || !matches!(lit.get(1), Some(&"standing") | Some(&"ties")) {
        return Flow::Continue;
    }
    read(host);
    let player = match e.source {
        CommandSource::Player { player, .. } => Some(*player),
        _ => None,
    };
    let at = e.command.position;
    let value = e.command.arguments.iter().find(|a| a.name == "value").and_then(|a| match a.value {
        CommandArgumentValue::Integer(v) => Some((v as i32).clamp(faction::MIN, faction::MAX)),
        _ => None,
    });
    let mut villages = crate::settlements::list(host);
    villages.sort_by(|a, b| {
        let d = |c: [i32; 2]| (f64::from(c[0]) - at[0]).powi(2) + (f64::from(c[1]) - at[2]).powi(2);
        d(a.0).total_cmp(&d(b.0))
    });
    let numbers = player.is_none_or(|p| crate::flags::extended(&*host, p) || p.permission_level(&*host).unwrap_or(0) >= 2);
    let day = now_of(&*host) / DAY;
    let line = match (lit.as_slice(), value) {
        (["felix", "standing"], _) => {
            let Some(p) = player else { return Flow::Cancel };
            let list: Vec<String> = villages
                .iter()
                .map(|(_, n)| {
                    let s = standing_of(n, p.uuid());
                    if numbers { format!("{n}: {} ({})", s.rank().word(), s.value) } else { format!("{n}: {}", s.rank().word()) }
                })
                .collect();
            if list.is_empty() { "No villages yet.".to_string() } else { format!("What you are to the villages: {}.", list.join("; ")) }
        }
        (["felix", "standing", "set"], Some(v)) => {
            let (Some(p), Some((_, name))) = (player, villages.first().cloned()) else { return Flow::Cancel };
            let pname = p.name(&*host).unwrap_or_default();
            if let Ok(mut s) = STATE.lock() {
                let st = s.standing.entry(name.clone()).or_default().entry(p.uuid()).or_insert_with(|| Standing { day, ..Standing::default() });
                st.value = v;
                st.name = pname;
                refoe(&mut s);
            }
            write(host);
            format!("{name} now sees you as {} ({v}).", Rank::of(v).word())
        }
        (["felix", "ties"], _) => {
            let mut list = Vec::new();
            for (i, (_, a)) in villages.iter().enumerate() {
                for (_, b) in villages.iter().skip(i + 1) {
                    let t = tie_of(a, b);
                    list.push(if numbers { format!("{a} is {} ({t})", Bond::of(t).phrase(b)) } else { format!("{a} is {}", Bond::of(t).phrase(b)) });
                }
            }
            if list.is_empty() { "There are no two villages yet.".to_string() } else { list.join("; ") + "." }
        }
        (["felix", "ties", "set"], Some(v)) => {
            let (Some((_, a)), Some((_, b))) = (villages.first().cloned(), villages.get(1).cloned()) else {
                let _ = host.commands().feedback(Text::literal("There are no two villages yet."));
                return Flow::Cancel;
            };
            if let Ok(mut s) = STATE.lock() {
                s.ties.insert(faction::key(&a, &b), Tie { value: v, day });
            }
            write(host);
            format!("{a} is now {} ({v}).", Bond::of(v).phrase(&b))
        }
        _ => return Flow::Cancel,
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
