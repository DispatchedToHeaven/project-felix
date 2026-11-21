//! Health at work on the server (see `health`): an NPC hurt keeps its wounds; they bleed, heal and
//! may turn bad; it may fall sick; too ill it keeps to its bed; the best healer about walks over and
//! tends it, with medicine from the chests (honey, berries, a golden apple) or none, and gets better
//! at it. Players ask "Are you hurt?" and patch NPCs up themselves. `/felix health <npc>`,
//! `/felix health wound <npc> <hp>`, `/felix health sick <npc>`.
use crate::companion::{self, Mode};
use crate::health::{self, Health};
use crate::skill::Skill;
use mod_api::{
    AttachmentCodec, AttachmentType, Command, CommandArgumentValue, Compound, EntityRef, Flow, Holder, LivingRef, PlayerRef, RegisterCommandsEvent,
    RunModCommandEvent, ServerHost, Tag, Text,
};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

pub static HEALTH: AttachmentType<Option<Health>> = AttachmentType {
    namespace: "projectfelix",
    name: "health",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |h| {
            let mut c = Compound::new();
            if let Some(h) = h {
                c.insert("json".into(), Tag::String(h.to_json().to_string().as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            Ok(match c.get(&"json".into()) {
                Some(Tag::String(s)) => crate::serde_json::from_str(&s.to_string_lossy()).ok().map(|v| Health::from_json(&v)),
                _ => None,
            })
        },
    }),
    copy_on_death: false,
    sync: false,
};

#[derive(Default)]
struct State {
    cache: HashMap<u32, Health>,
    /// Hits taken since the last look: NPC, points, whether something hit it.
    pending: Vec<(u32, f32, bool, u32)>,
    /// Hurts this module made itself (bleeding, death), not new wounds.
    own: HashSet<u32>,
    /// NPCs kept to their beds.
    kept: HashSet<u32>,
    /// Healers on their way: to whom, since when.
    healing: HashMap<u32, (u32, i64)>,
    seed: u64,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

pub fn forget(id: u32) {
    if let Ok(mut s) = STATE.lock() {
        s.cache.remove(&id);
        s.kept.remove(&id);
        s.healing.remove(&id);
        s.healing.retain(|_, (p, _)| *p != id);
    }
}

fn roll() -> f32 {
    let Ok(mut s) = STATE.lock() else { return 0.5 };
    s.seed = s.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut x = s.seed;
    x ^= x >> 31;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 29;
    (x % 1_000_000) as f32 / 1_000_000.0
}

/// `living_damage_pre`: an NPC takes `amount` (after its armour), from something or not.
pub fn on_damage(entity: i32, amount: f32, attacked: bool) {
    if entity <= 0 || amount < 1.0 {
        return;
    }
    if let Ok(mut s) = STATE.lock() {
        if s.own.remove(&(entity as u32)) {
            return;
        }
        if s.pending.len() < 256 {
            s.pending.push((entity as u32, amount, attacked, 0));
        }
    }
}

/// Whether a loaded NPC keeps to its bed.
pub fn bedridden(id: u32) -> bool {
    STATE.lock().ok().is_some_and(|s| s.kept.contains(&id))
}

/// NPC `npc` falls sick with `what`: false if it was sick already.
pub fn make_sick(host: &mut dyn ServerHost, npc: EntityRef, what: &str) -> bool {
    let mut h = get(host, npc);
    if h.sick.is_some() {
        return false;
    }
    h.fall_sick(what);
    put(host, npc, h);
    true
}

/// Whether a loaded NPC is sick.
pub fn sick(id: u32) -> bool {
    STATE.lock().ok().is_some_and(|s| s.cache.get(&id).is_some_and(|h| h.sick.is_some()))
}

fn get(host: &mut dyn ServerHost, npc: EntityRef) -> Health {
    if let Some(h) = STATE.lock().ok().and_then(|s| s.cache.get(&npc.id()).cloned()) {
        return h;
    }
    HEALTH.get(host, Holder::Entity(npc.id())).ok().flatten().unwrap_or_default()
}

fn put(host: &mut dyn ServerHost, npc: EntityRef, h: Health) {
    let _ = HEALTH.set(host, Holder::Entity(npc.id()), Some(h.clone()));
    if let Ok(mut s) = STATE.lock() {
        s.cache.insert(npc.id(), h);
    }
}

/// Hurts NPC `npc` by `n` itself (no new wound).
fn hurt(host: &mut dyn ServerHost, npc: EntityRef, n: f32) {
    let Some(u) = npc.uuid(host) else { return };
    if let Ok(mut s) = STATE.lock() {
        s.own.insert(npc.id());
    }
    // (One gone meanwhile is no failure.)
    crate::talk::queue_try(format!("damage {} {n:.1} minecraft:generic", crate::talk::uuid_text(u)));
}

fn tell_near(host: &mut dyn ServerHost, npc: EntityRef, line: &str) {
    let (Some(level), Some(at)) = (npc.level(&*host), npc.position(&*host)) else { return };
    for p in level.online_players(&*host).unwrap_or_default() {
        if p.entity().position(&*host).is_some_and(|q| (q[0] - at[0]).powi(2) + (q[2] - at[2]).powi(2) < 64.0 * 64.0) {
            let _ = p.tell(host, Text::literal(line.to_string()));
        }
    }
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// The best medicine in the chests of `who`: taken, and its strength (none: 0.3).
fn medicine(host: &mut dyn ServerHost, who: &[EntityRef]) -> f32 {
    for e in who {
        let Some(c) = crate::jobs::chest_of(host, *e).flatten() else { continue };
        for m in health::MEDICINE {
            if crate::jobs::amount(host, c, m) > 0 && crate::jobs::take(host, c, m, 1) {
                return health::potency(Some(m));
            }
        }
    }
    health::potency(None)
}

/// The tick of each level: hits become wounds, health goes on, healers see to the hurt and sick.
pub fn tick(host: &mut dyn ServerHost) {
    if !crate::pace::every("healers", 100) {
        return;
    }
    // Hits of this level's NPCs (others wait a while for their level).
    let pending = STATE.lock().map(|mut s| std::mem::take(&mut s.pending)).unwrap_or_default();
    let mut later = Vec::new();
    for (id, n, attacked, waited) in pending {
        let Some(npc) = host.entity(id).filter(|_| crate::pace::here(&*host, id)) else {
            if waited < 60 {
                later.push((id, n, attacked, waited + 1));
            }
            continue;
        };
        let now = npc.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
        let mut h = get(host, npc);
        h.add_wound(health::wound(n, attacked, roll(), now));
        put(host, npc, h);
    }
    if let Ok(mut s) = STATE.lock() {
        s.pending.extend(later);
    }
    let npcs = companion::npcs();
    let mut patients = Vec::new();
    for id in &npcs {
        let Some(npc) = host.entity(*id) else { continue };
        let Some(now) = npc.level(&*host).and_then(|l| l.game_time(&*host)) else { continue };
        let mut h = get(host, npc);
        let prev = h.last;
        // Asleep in its bed, or kept to it.
        let in_bed = crate::routines::bed_of(*id).is_some() || bedridden(*id);
        let rested = crate::minds::rest(*id).unwrap_or(60.0);
        // Sleeping rough it may catch something.
        if prev > 0 && h.sick.is_none() && roll() < health::catch_chance(now - prev, crate::minds::slept_rough(*id, now), false) {
            const SICK: [&str; 3] = ["the flu", "a fever", "a bad cough"];
            h.fall_sick(SICK[(roll() * 3.0) as usize % 3]);
            crate::heart::say_aloud(host, npc, "I don't feel so good...");
        }
        let c = h.advance(now, in_bed, rested, &mut || roll());
        let life = LivingRef::new(npc);
        if c.heal > 0.0 && life.health(&*host).zip(life.max_health(&*host)).is_some_and(|(x, m)| x < m) {
            let _ = life.heal(host, c.heal);
        }
        if c.bleed > 0.0 {
            // Bleeding weakens; it doesn't kill (wounds left to fester do).
            let left = life.health(&*host).unwrap_or(0.0) - 1.0;
            if left > 0.5 {
                hurt(host, npc, c.bleed.min(left));
            }
        }
        if c.infected {
            crate::heart::say_aloud(host, npc, "This wound... it's hot. It doesn't look right.");
        }
        if let Some(what) = &c.recovered {
            crate::heart::say_aloud(host, npc, &format!("The {} is gone. I feel so much better!", what.trim_start_matches("the ").trim_start_matches("a ")));
        }
        if let Some(of) = &c.died_of {
            let name = npc.name(&*host).unwrap_or_else(|| "An NPC".into());
            crate::heart::say_aloud(host, npc, "I'm... so tired...");
            tell_near(host, npc, &format!("{name} died of {of}."));
            put(host, npc, Health::default());
            hurt(host, npc, 1000.0);
            continue;
        }
        // Too ill or hurt: to bed, until it is better.
        let down = h.bedridden();
        let kept = bedridden(*id);
        if down && !kept {
            let home = crate::routines::place_named(host, npc, "home");
            if let Some(home) = home {
                companion::go_to(*id, Some(home));
            }
            crate::heart::say_aloud(host, npc, "I need to lie down.");
            if let Ok(mut s) = STATE.lock() {
                s.kept.insert(*id);
            }
        } else if !down && kept {
            companion::go_to(*id, None);
            if let Ok(mut s) = STATE.lock() {
                s.kept.remove(id);
            }
        }
        // (Nobody of the village tends those who came to rob it.)
        if h.needs_tending(now) && !npc.tags(&*host).unwrap_or_default().iter().any(|t| t == "felix_raider") {
            patients.push(*id);
        }
        put(host, npc, h);
    }
    healers(host, &npcs, &patients);
    if let Ok(mut s) = STATE.lock() {
        s.cache.retain(|id, _| npcs.contains(id));
        s.kept.retain(|id| npcs.contains(id));
    }
}

/// The best healer about each patient goes to it and tends it.
fn healers(host: &mut dyn ServerHost, npcs: &[u32], patients: &[u32]) {
    let busy: HashMap<u32, (u32, i64)> = STATE.lock().map(|s| s.healing.clone()).unwrap_or_default();
    // Healers on their way: there, they tend; too long, they give up.
    for (healer, (patient, since)) in &busy {
        let (Some(he), Some(pe)) = (host.entity(*healer), host.entity(*patient)) else {
            companion::go_to(*healer, None);
            if let Ok(mut s) = STATE.lock() {
                s.healing.remove(healer);
            }
            continue;
        };
        let now = he.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
        let (Some(a), Some(b)) = (he.position(&*host), pe.position(&*host)) else { continue };
        let done = !patients.contains(patient);
        if !done && dist2(a, b) <= 2.5 * 2.5 {
            let potency = medicine(host, &[he, pe]);
            let q = health::quality(potency, crate::skills::level(host, he, Skill::Healing));
            let mut h = get(host, pe);
            h.tend(q, now);
            put(host, pe, h);
            crate::skills::practice(host, he, Skill::Healing, 200.0);
            crate::heart::say_aloud(host, he, ["Hold still. This will sting a little.", "There. Keep it clean and rest.", "Let me see... yes, this will mend."][(now as usize / 7) % 3]);
            crate::minds::remember(host, pe, "cared_for");
        }
        if done || dist2(a, b) <= 2.5 * 2.5 || now - since > 1200 || now < *since {
            companion::go_to(*healer, None);
            if let Ok(mut s) = STATE.lock() {
                s.healing.remove(healer);
            }
        } else {
            companion::go_to(*healer, Some(b));
        }
    }
    // Patients nobody is coming to: the best healer within 48 blocks who is free goes.
    for p in patients {
        if STATE.lock().ok().is_some_and(|s| s.healing.values().any(|(x, _)| x == p)) {
            continue;
        }
        let Some(pe) = host.entity(*p) else { continue };
        let (Some(pos), Some(lv)) = (pe.position(&*host), pe.level(&*host)) else { continue };
        let mut best: Option<(u8, u32)> = None;
        for o in npcs {
            if o == p || STATE.lock().ok().is_some_and(|s| s.healing.contains_key(o) || s.kept.contains(o)) {
                continue;
            }
            let Some(oe) = host.entity(*o) else { continue };
            let free = companion::mode_of(*o) == Mode::Live && !crate::routines::asleep(*o) && crate::minds::breaking(*o).is_none();
            let near = oe.level(&*host) == Some(lv) && oe.position(&*host).is_some_and(|q| dist2(q, pos) <= 48.0 * 48.0);
            if !free || !near || crate::families::is_child(host, oe) {
                continue;
            }
            let l = crate::skills::level(host, oe, Skill::Healing);
            if best.is_none_or(|(b, _)| l > b) {
                best = Some((l, *o));
            }
        }
        if let Some((_, h)) = best {
            let now = lv.game_time(&*host).unwrap_or(0);
            companion::go_to(h, Some(pos));
            if let Ok(mut s) = STATE.lock() {
                s.healing.insert(h, (*p, now));
            }
        }
    }
}

/// The `health` dialog action: `ask` (it tells how it is), `tend` (the player patches it up with the
/// best medicine they carry, or none).
pub fn talk(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef, what: &str) {
    let mut h = get(host, npc);
    if what != "tend" {
        let line = h.describe();
        crate::heart::say_to(host, npc, player, &line);
        return;
    }
    let now = npc.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
    if !h.needs_tending(now) {
        crate::heart::say_to(host, npc, player, "Thank you, but there's nothing to patch up.");
        return;
    }
    let name = player.name(&*host).unwrap_or_default();
    let mut have: HashSet<String> = HashSet::new();
    for s in player.inventory(&*host).unwrap_or_default() {
        have.insert(s.item().id(&*host));
    }
    let used = health::MEDICINE.iter().find(|m| have.contains(**m)).copied();
    if let Some(m) = used {
        crate::talk::queue(format!("clear {name} {m} 1"));
    }
    h.tend(health::quality(health::potency(used), 6), now);
    put(host, npc, h);
    crate::minds::remember(host, npc, "cared_for");
    crate::heart::befriend(host, npc, player, 3);
    crate::factions::deed_by(host, npc, player, crate::faction::Deed::Tended);
    let line = if used.is_some() { format!("Thank you, {name}. That feels better already.") } else { format!("Thank you, {name}. A clean bandage will do.") };
    crate::heart::say_to(host, npc, player, &line);
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix health <targets:entity>", 2));
    e.commands.push(Command::op("felix health wound <targets:entities> <hp:integer>", 2));
    e.commands.push(Command::op("felix health sick <targets:entities>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if lit.first() != Some(&"felix") || lit.get(1) != Some(&"health") {
        return Flow::Continue;
    }
    let npcs: Vec<EntityRef> = companion::targets(e).into_iter().filter(|t| companion::is_npc(&*host, *t)).collect();
    let Some(first) = npcs.first().copied() else {
        let _ = host.commands().feedback(Text::literal("That is not an NPC."));
        return Flow::Cancel;
    };
    let line = match lit.as_slice() {
        ["felix", "health"] => {
            let h = get(host, first);
            let name = first.name(&*host).unwrap_or_else(|| "NPC".into());
            let wounds: Vec<String> = h
                .wounds
                .iter()
                .map(|w| format!("{:.1}{}{}{}", w.hp, if w.bleeding { " bleeding" } else { "" }, if w.tended > 0.0 { format!(" tended {:.1}", w.tended) } else { String::new() }, w.infection.map(|(s, i)| format!(" infected {s:.2}/{i:.2}")).unwrap_or_default()))
                .collect();
            let sick = h.sick.as_ref().map(|s| format!(", {} {:.2}/{:.2}", s.name, s.severity, s.immunity)).unwrap_or_default();
            format!("{name}: wounds [{}]{sick}, scars {}, pain {:.2}{}", wounds.join(", "), h.scars, h.pain(), if bedridden(first.id()) { ", in bed" } else { "" })
        }
        ["felix", "health", "wound"] => {
            let hp = e.command.arguments.iter().find(|a| a.name == "hp").and_then(|a| match a.value {
                CommandArgumentValue::Integer(v) => Some(v.clamp(1, 20) as f32),
                _ => None,
            });
            let Some(hp) = hp else { return Flow::Cancel };
            for n in &npcs {
                let now = n.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
                let mut h = get(host, *n);
                h.add_wound(health::wound(hp, true, 0.0, now));
                put(host, *n, h);
            }
            format!("{} NPCs are wounded ({hp})", npcs.len())
        }
        ["felix", "health", "sick"] => {
            for n in &npcs {
                let mut h = get(host, *n);
                h.fall_sick("the flu");
                put(host, *n, h);
            }
            format!("{} NPCs have the flu", npcs.len())
        }
        _ => return Flow::Continue,
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
