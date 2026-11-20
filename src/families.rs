//! Families on the server (see `family`): friends who become couples and share a home, children
//! born to them by it who grow up, and what each remembers of its family. `/felix family <npc>`,
//! `/felix family marry <npc> <npc>`, `/felix family child <npc>`.
use crate::companion::{self, LOOK, Look, NPC};
use crate::family::{self, CHILD_EVERY, COUPLE_AT, COUPLE_CHANCE, GROWS_IN, MAX_CHILDREN};
use mod_api::{
    AttachmentCodec, AttachmentType, Command, CommandArgumentValue, Compound, EntityRef, Flow, Holder, LivingRef,
    PlayerRef, RegisterCommandsEvent, RunModCommandEvent, ServerHost, Tag, Text,
};
use std::sync::{LazyLock, Mutex};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Family {
    pub partner: Option<(u128, String)>,
    pub children: Vec<(u128, String)>,
    pub parents: Vec<(u128, String)>,
    pub last_child: i64,
    /// Born a child (grown: false until it grows up).
    pub child: bool,
    pub grown: bool,
}

fn uuid_tag(u: u128) -> Tag {
    Tag::IntArray(vec![(u >> 96) as i32, (u >> 64) as i32, (u >> 32) as i32, u as i32])
}

fn tag_uuid(t: Option<&Tag>) -> Option<u128> {
    match t {
        Some(Tag::IntArray(v)) if v.len() == 4 => Some(v.iter().fold(0u128, |acc, x| (acc << 32) | (*x as u32 as u128))),
        _ => None,
    }
}

fn people(list: &[(u128, String)]) -> Tag {
    Tag::List(
        list.iter()
            .map(|(u, n)| {
                let mut c = Compound::new();
                c.insert("who".into(), uuid_tag(*u));
                c.insert("name".into(), Tag::String(n.as_str().into()));
                Tag::Compound(c)
            })
            .collect(),
    )
}

fn read_people(t: Option<&Tag>) -> Vec<(u128, String)> {
    let Some(Tag::List(list)) = t else { return Vec::new() };
    list.iter()
        .filter_map(|t| {
            let Tag::Compound(c) = t else { return None };
            let name = match c.get(&"name".into()) {
                Some(Tag::String(s)) => s.to_string_lossy(),
                _ => String::new(),
            };
            Some((tag_uuid(c.get(&"who".into()))?, name))
        })
        .collect()
}

pub static FAMILY: AttachmentType<Family> = AttachmentType {
    namespace: "projectfelix",
    name: "family",
    default: Family::default,
    codec: Some(AttachmentCodec {
        encode: |f| {
            let mut c = Compound::new();
            if let Some(p) = &f.partner {
                c.insert("partner".into(), people(std::slice::from_ref(p)));
            }
            c.insert("children".into(), people(&f.children));
            c.insert("parents".into(), people(&f.parents));
            c.insert("last_child".into(), Tag::Long(f.last_child));
            c.insert("child".into(), Tag::Byte(f.child as i8));
            c.insert("grown".into(), Tag::Byte(f.grown as i8));
            Ok(c)
        },
        decode: |c: &Compound| {
            let byte = |k: &str| matches!(c.get(&k.into()), Some(Tag::Byte(1)));
            Ok(Family {
                partner: read_people(c.get(&"partner".into())).into_iter().next(),
                children: read_people(c.get(&"children".into())),
                parents: read_people(c.get(&"parents".into())),
                last_child: match c.get(&"last_child".into()) {
                    Some(Tag::Long(t)) => *t,
                    _ => 0,
                },
                child: byte("child"),
                grown: byte("grown"),
            })
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// A child to set up once it is in the level: its id, the parent it takes after, the other.
struct Birth {
    id: u32,
    after: EntityRef,
    other: EntityRef,
    name: String,
}

#[derive(Default)]
struct State {
    births: Vec<Birth>,
    ticks: u32,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// The server stopped: nothing of its world is kept.
pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

/// NPC `e`'s partner among NPCs, by name.
pub fn partner_name(host: &mut dyn ServerHost, e: EntityRef) -> Option<String> {
    family_of(host, e).partner.map(|p| p.1)
}

/// Whether NPC `e` has a family (a partner, children, parents).
pub fn has_family(host: &mut dyn ServerHost, e: EntityRef) -> bool {
    let f = family_of(host, e);
    f.partner.is_some() || !f.children.is_empty() || !f.parents.is_empty()
}

fn family_of(host: &mut dyn ServerHost, e: EntityRef) -> Family {
    FAMILY.get(host, Holder::Entity(e.id())).unwrap_or_default()
}

fn name_of(host: &dyn mod_api::Host, e: EntityRef) -> String {
    e.name(host).unwrap_or_else(|| "NPC".into())
}

fn grown_up(host: &dyn mod_api::Host, e: EntityRef) -> bool {
    LivingRef::new(e).baby(host) != Some(true)
}

/// NPC `e` takes a new partner: the one it had (if here, and another) has none now.
fn leave_old(host: &mut dyn ServerHost, e: EntityRef, new: u128) {
    let Some((old, _)) = family_of(host, e).partner.filter(|p| p.0 != new) else { return };
    let Some(o) = crate::heart::npc_by_uuid(old).and_then(|id| host.entity(id)) else { return };
    let mut f = family_of(host, o);
    f.partner = None;
    let _ = FAMILY.set(host, Holder::Entity(o.id()), f);
}

/// `living_death`: an NPC died, and its partner (if here) has none now.
pub fn on_death(host: &mut dyn ServerHost, victim: u32) {
    let Some(v) = host.entity(victim).filter(|v| companion::is_npc(&*host, *v)) else { return };
    let Some((partner, _)) = family_of(host, v).partner else { return };
    let Some(o) = crate::heart::npc_by_uuid(partner).and_then(|id| host.entity(id)) else { return };
    let mut f = family_of(host, o);
    f.partner = None;
    let _ = FAMILY.set(host, Holder::Entity(o.id()), f);
}

/// NPCs `a` and `b` become a couple, in `a`'s home (else `b`'s).
pub fn couple(host: &mut dyn ServerHost, a: EntityRef, b: EntityRef) -> bool {
    let (Some(ua), Some(ub)) = (a.uuid(host), b.uuid(host)) else { return false };
    leave_old(host, a, ub);
    leave_old(host, b, ua);
    let (na, nb) = (name_of(&*host, a), name_of(&*host, b));
    let mut fa = family_of(host, a);
    let mut fb = family_of(host, b);
    fa.partner = Some((ub, nb.clone()));
    fb.partner = Some((ua, na.clone()));
    let _ = FAMILY.set(host, Holder::Entity(a.id()), fa);
    let _ = FAMILY.set(host, Holder::Entity(b.id()), fb);
    let home = crate::routines::place_named(host, a, "home").or_else(|| crate::routines::place_named(host, b, "home"));
    if let Some(h) = home {
        crate::routines::set_place(host, a, "home", h);
        crate::routines::set_place(host, b, "home", h);
    }
    crate::heart::say_aloud(host, a, &format!("{nb}, will you share my home?"));
    crate::heart::say_aloud(host, b, &format!("Oh, {na}... yes! Yes, I will!"));
    true
}

/// Makes `a` and `b` a couple without a word (a settlement's, made so).
pub fn bind(host: &mut dyn ServerHost, a: EntityRef, b: EntityRef) {
    let (Some(ua), Some(ub)) = (a.uuid(host), b.uuid(host)) else { return };
    leave_old(host, a, ub);
    leave_old(host, b, ua);
    let (na, nb) = (name_of(&*host, a), name_of(&*host, b));
    let mut fa = family_of(host, a);
    let mut fb = family_of(host, b);
    fa.partner = Some((ub, nb));
    fb.partner = Some((ua, na));
    let _ = FAMILY.set(host, Holder::Entity(a.id()), fa);
    let _ = FAMILY.set(host, Holder::Entity(b.id()), fb);
}

/// Couple `a` and `b` have a child by their home (or by `a`).
pub fn birth(host: &mut dyn ServerHost, a: EntityRef, b: EntityRef) -> Result<String, String> {
    let level = a.level(&*host).ok_or("no level")?;
    let now = level.game_time(&*host).unwrap_or(0);
    let at = crate::routines::place_named(host, a, "home").or_else(|| a.position(&*host)).ok_or("no place")?;
    let fa = family_of(host, a);
    let taken: Vec<String> = fa.children.iter().map(|c| c.1.clone()).collect();
    let seed = (now as u64) ^ u64::from(a.id()).wrapping_mul(0x9e37_79b9) ^ u64::from(b.id()) << 17;
    let name = family::child_name(seed, &taken);
    let (after, other) = if family::takes_after(seed) == 0 { (a, b) } else { (b, a) };
    let mut nbt = Compound::new();
    if let Some(u) = LivingRef::new(a).owner(&*host).or_else(|| LivingRef::new(b).owner(&*host)) {
        nbt.insert("Owner".into(), uuid_tag(u));
    }
    nbt.insert("PersistenceRequired".into(), Tag::Byte(1));
    nbt.insert("CustomName".into(), Tag::String(name.as_str().into()));
    nbt.insert("Age".into(), Tag::Int(-(GROWS_IN as i32)));
    let spot = crate::villages::stand_at(&*host, level, [at[0] + 1.0, at[1], at[2] + 1.0]);
    let id = host.summon(level.dimension_id(), NPC, spot, nbt).ok_or("not summoned")?;
    if let Ok(mut s) = STATE.lock() {
        s.births.push(Birth { id, after, other, name: name.clone() });
    }
    for p in [a, b] {
        let mut f = family_of(host, p);
        f.last_child = now;
        let _ = FAMILY.set(host, Holder::Entity(p.id()), f);
    }
    crate::heart::say_aloud(host, a, &format!("Welcome to the world, little {name}!"));
    Ok(name)
}

/// The children just born get their look, character, voice, day and family.
fn set_up(host: &mut dyn ServerHost) {
    let births = STATE.lock().map(|mut s| std::mem::take(&mut s.births)).unwrap_or_default();
    let mut later = Vec::new();
    for b in births {
        let Some(child) = host.entity(b.id) else {
            later.push(b);
            continue;
        };
        let look = LOOK.get(host, Holder::Entity(b.after.id())).ok().flatten().unwrap_or_else(|| Look::of_model(companion::DEFAULT_MODEL));
        let _ = LOOK.set(host, Holder::Entity(b.id), Some(look));
        if let Some(c) = crate::heart::character_of(host, b.after) {
            crate::heart::set_character(host, child, Some(&c));
        }
        let other_uuid = b.other.uuid(host).unwrap_or(0);
        let voice = crate::voices::VOICE.get(host, Holder::Entity(b.other.id())).ok().flatten().unwrap_or_else(|| crate::voice::default_for(other_uuid).name.to_string());
        crate::voices::set_voice(host, child, &voice);
        let parents: Vec<(u128, String)> = [b.after, b.other].iter().filter_map(|p| Some((p.uuid(host)?, name_of(&*host, *p)))).collect();
        let _ = FAMILY.set(host, Holder::Entity(b.id), Family { parents, child: true, ..Family::default() });
        let child_uuid = child.uuid(host).unwrap_or(0);
        for p in [b.after, b.other] {
            let mut f = family_of(host, p);
            f.children.push((child_uuid, b.name.clone()));
            let _ = FAMILY.set(host, Holder::Entity(p.id()), f);
        }
        for place in ["home", "meet"] {
            if let Some(at) = crate::routines::place_named(host, b.after, place) {
                crate::routines::set_place(host, child, place, at);
            }
        }
        crate::routines::set_routine(host, child, Some("projectfelix:child"));
    }
    if let Ok(mut s) = STATE.lock() {
        s.births.extend(later.into_iter().take(16));
    }
}

/// The server tick: children set up, grown children take a grown day, couples form and have children.
pub fn tick(host: &mut dyn ServerHost) {
    set_up(host);
    let step = STATE.lock().map(|mut s| {
        s.ticks = s.ticks.wrapping_add(1);
        s.ticks % 600 == 0
    });
    if !matches!(step, Ok(true)) {
        return;
    }
    let npcs: Vec<EntityRef> = companion::npcs().into_iter().filter_map(|id| host.entity(id)).collect();
    let mut fams: Vec<(EntityRef, Family)> = Vec::new();
    for e in &npcs {
        let f = family_of(host, *e);
        fams.push((*e, f));
    }
    // Children who grew up.
    for (e, f) in &fams {
        if f.child && !f.grown && grown_up(&*host, *e) {
            let mut g = f.clone();
            g.grown = true;
            let _ = FAMILY.set(host, Holder::Entity(e.id()), g);
            crate::routines::set_routine(host, *e, Some("projectfelix:villager"));
            // A parent's job, if one has one.
            let job = f.parents.iter().find_map(|(u, _)| {
                let id = crate::heart::npc_by_uuid(*u)?;
                let p = host.entity(id)?;
                crate::jobs::job_of(host, p)
            });
            if let Some(j) = job {
                crate::jobs::set_job(host, *e, j.name());
            }
            crate::heart::say_aloud(host, *e, "I'm all grown up now!");
        }
    }
    let Some(now) = npcs.first().and_then(|e| e.level(&*host)).and_then(|l| l.game_time(&*host)) else { return };
    // Couples: two grown friends near each other, neither with a partner, not kin. Not a player's
    // sweetheart or partner, and not a companion out with its player: those are the player's to win.
    let mut taken: Vec<u32> = Vec::new();
    for (e, _) in &fams {
        let with_player = LivingRef::new(*e).owner(&*host).is_some() && companion::mode_of(e.id()) == companion::Mode::Follow;
        if with_player || crate::bonds::has_romance(host, *e) {
            taken.push(e.id());
        }
    }
    for i in 0..fams.len() {
        for j in (i + 1)..fams.len() {
            let ((a, fa), (b, fb)) = (&fams[i], &fams[j]);
            if fa.partner.is_some() || fb.partner.is_some() || taken.contains(&a.id()) || taken.contains(&b.id()) {
                continue;
            }
            let (Some(ua), Some(ub)) = (a.uuid(host), b.uuid(host)) else { continue };
            let kin = fa.parents.iter().chain(&fa.children).any(|k| k.0 == ub) || fb.parents.iter().any(|k| fa.parents.iter().any(|m| m.0 == k.0));
            if kin || !grown_up(&*host, *a) || !grown_up(&*host, *b) {
                continue;
            }
            let near = a.position(&*host).zip(b.position(&*host)).is_some_and(|(p, q)| (p[0] - q[0]).powi(2) + (p[2] - q[2]).powi(2) <= 144.0);
            let liked = crate::heart::relation_of(a.id(), ub) >= COUPLE_AT && crate::heart::relation_of(b.id(), ua) >= COUPLE_AT;
            let roll = ((now as u64 ^ u64::from(a.id()) << 20 ^ u64::from(b.id())).wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) % 100;
            if near && liked && roll < u64::from(COUPLE_CHANCE) && couple(host, *a, *b) {
                taken.extend([a.id(), b.id()]);
            }
        }
    }
    // Children: a couple together by their home, now and then.
    for (a, fa) in &fams {
        let Some((pu, _)) = fa.partner else { continue };
        let Some(b) = crate::heart::npc_by_uuid(pu).and_then(|id| host.entity(id)) else { continue };
        if a.uuid(host).is_none_or(|u| u > pu) || fa.children.len() >= MAX_CHILDREN || now - fa.last_child < CHILD_EVERY || fa.child && !fa.grown {
            continue;
        }
        let together = a.position(&*host).zip(b.position(&*host)).is_some_and(|(p, q)| (p[0] - q[0]).powi(2) + (p[2] - q[2]).powi(2) <= 64.0);
        let home = crate::routines::place_named(host, *a, "home");
        let at_home = home.zip(a.position(&*host)).is_some_and(|(h, p)| (h[0] - p[0]).powi(2) + (h[2] - p[2]).powi(2) <= 100.0);
        if together && at_home && fa.last_child != 0 {
            let _ = birth(host, *a, b);
        } else if fa.last_child == 0 {
            // The first child comes a while after they move in.
            let mut f = fa.clone();
            f.last_child = now;
            let _ = FAMILY.set(host, Holder::Entity(a.id()), f);
        }
    }
}

/// NPC `npc` tells `player` about its family.
pub fn tell(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) {
    let f = family_of(host, npc);
    let children: Vec<String> = f.children.iter().map(|c| c.1.clone()).collect();
    let parents: Vec<String> = f.parents.iter().map(|c| c.1.clone()).collect();
    let line = family::describe(f.partner.as_ref().map(|p| p.1.as_str()), &children, &parents);
    crate::heart::say_to(host, npc, player, &line);
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix family <targets:entity>", 2));
    e.commands.push(Command::op("felix family marry <first:entity> <second:entity>", 2));
    e.commands.push(Command::op("felix family child <targets:entity>", 2));
}

fn one(e: &RunModCommandEvent<'_>, name: &str) -> Option<EntityRef> {
    e.command.arguments.iter().find(|a| a.name == name).and_then(|a| match &a.value {
        CommandArgumentValue::Entity(x) => Some(*x),
        CommandArgumentValue::Entities(l) => l.first().copied(),
        _ => None,
    })
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let line = match lit.as_slice() {
        ["felix", "family"] => match companion::targets(e).into_iter().find(|t| companion::is_npc(&*host, *t)) {
            Some(n) => {
                let f = family_of(host, n);
                let names = |l: &[(u128, String)]| l.iter().map(|x| x.1.clone()).collect::<Vec<_>>().join(", ");
                format!(
                    "{}: partner {}; children {}; parents {}{}",
                    name_of(&*host, n),
                    f.partner.as_ref().map_or("none".to_string(), |p| p.1.clone()),
                    if f.children.is_empty() { "none".into() } else { names(&f.children) },
                    if f.parents.is_empty() { "none".into() } else { names(&f.parents) },
                    if f.child && !f.grown { " (a child)" } else { "" }
                )
            }
            None => "That is not an NPC.".into(),
        },
        ["felix", "family", "marry"] => match (one(e, "first"), one(e, "second")) {
            (Some(a), Some(b)) if companion::is_npc(&*host, a) && companion::is_npc(&*host, b) && a.id() != b.id() => {
                couple(host, a, b);
                format!("{} and {} are a couple now", name_of(&*host, a), name_of(&*host, b))
            }
            _ => "Two NPCs.".into(),
        },
        ["felix", "family", "child"] => {
            let Some(a) = companion::targets(e).into_iter().find(|t| companion::is_npc(&*host, *t)) else { return Flow::Cancel };
            let partner = family_of(host, a).partner.and_then(|p| crate::heart::npc_by_uuid(p.0)).and_then(|id| host.entity(id));
            match partner {
                Some(b) => match birth(host, a, b) {
                    Ok(n) => format!("{n} is born"),
                    Err(err) => format!("No child: {err}"),
                },
                None => "It has no partner here.".into(),
            }
        }
        _ => return Flow::Continue,
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
