//! Bonds on the server (see `bond`): how close each NPC is to each player, the gifts it takes,
//! the time and fights they share, and romance: asked out, proposed to, parted. Hidden numbers;
//! players see the stage in words, makers in the extended view see the closeness too.
use crate::bond::{self, Romance, Stage, RING};
use crate::companion::{self, Mode};
use mod_api::{
    AttachmentCodec, AttachmentType, Command, Compound, EntityRef, EquipmentSlot, Flow, Holder, LivingRef, MobCategory,
    PlayerRef, RegisterCommandsEvent, RunModCommandEvent, ServerHost, Tag, Text,
};
use std::sync::{LazyLock, Mutex};

/// One NPC's bond with one player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tie {
    pub who: u128,
    pub name: String,
    pub close: f32,
    pub romance: Romance,
    /// Since when the romance is what it is.
    pub since: i64,
    pub last_gift: i64,
    pub last_talk: i64,
    pub seen: i64,
    /// When it was last asked out (it won't be asked again for a day).
    pub asked: i64,
    /// The day, and what fights gave that day (capped).
    pub day: i64,
    pub fought: f32,
    pub treat_day: i64,
    /// Memories: when they met, monsters fought side by side, lands gone through, a gift it loved.
    pub first: i64,
    pub fights: u32,
    pub places: Vec<String>,
    pub loved: String,
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

pub static BONDS: AttachmentType<Vec<Tie>> = AttachmentType {
    namespace: "projectfelix",
    name: "bonds",
    default: Vec::new,
    codec: Some(AttachmentCodec {
        encode: |list| {
            let mut c = Compound::new();
            let ties = list
                .iter()
                .map(|t| {
                    let mut x = Compound::new();
                    x.insert("who".into(), uuid_tag(t.who));
                    x.insert("name".into(), Tag::String(t.name.as_str().into()));
                    x.insert("close".into(), Tag::Double(f64::from(t.close).to_bits()));
                    x.insert("romance".into(), Tag::Byte(t.romance.byte() as i8));
                    x.insert("fights".into(), Tag::Int(t.fights as i32));
                    x.insert("places".into(), Tag::List(t.places.iter().map(|p| Tag::String(p.as_str().into())).collect()));
                    x.insert("loved".into(), Tag::String(t.loved.as_str().into()));
                    for (k, v) in [("since", t.since), ("gift", t.last_gift), ("talk", t.last_talk), ("seen", t.seen), ("asked", t.asked), ("day", t.day), ("treat", t.treat_day), ("first", t.first)] {
                        x.insert(k.into(), Tag::Long(v));
                    }
                    x.insert("fought".into(), Tag::Double(f64::from(t.fought).to_bits()));
                    Tag::Compound(x)
                })
                .collect();
            c.insert("ties".into(), Tag::List(ties));
            Ok(c)
        },
        decode: |c: &Compound| {
            let Some(Tag::List(list)) = c.get(&"ties".into()) else { return Ok(Vec::new()) };
            Ok(list
                .iter()
                .filter_map(|t| {
                    let Tag::Compound(x) = t else { return None };
                    let long = |k: &str| match x.get(&k.into()) {
                        Some(Tag::Long(v)) => *v,
                        _ => 0,
                    };
                    let double = |k: &str| match x.get(&k.into()) {
                        Some(Tag::Double(v)) => f64::from_bits(*v) as f32,
                        _ => 0.0,
                    };
                    Some(Tie {
                        who: tag_uuid(x.get(&"who".into()))?,
                        name: match x.get(&"name".into()) {
                            Some(Tag::String(s)) => s.to_string_lossy(),
                            _ => String::new(),
                        },
                        close: double("close"),
                        romance: match x.get(&"romance".into()) {
                            Some(Tag::Byte(b)) => Romance::from_byte(*b as u8),
                            _ => Romance::None,
                        },
                        since: long("since"),
                        last_gift: long("gift"),
                        last_talk: long("talk"),
                        seen: long("seen"),
                        asked: long("asked"),
                        day: long("day"),
                        fought: double("fought"),
                        treat_day: long("treat"),
                        first: long("first"),
                        fights: match x.get(&"fights".into()) {
                            Some(Tag::Int(v)) => (*v).max(0) as u32,
                            _ => 0,
                        },
                        places: match x.get(&"places".into()) {
                            Some(Tag::List(l)) => l.iter().filter_map(|t| if let Tag::String(s) = t { Some(s.to_string_lossy()) } else { None }).collect(),
                            _ => Vec::new(),
                        },
                        loved: match x.get(&"loved".into()) {
                            Some(Tag::String(s)) => s.to_string_lossy(),
                            _ => String::new(),
                        },
                    })
                })
                .collect())
        },
    }),
    copy_on_death: false,
    sync: false,
};

const DAY: i64 = 24000;

fn now_of(host: &dyn mod_api::Host, e: EntityRef) -> i64 {
    e.level(host).and_then(|l| l.game_time(host)).unwrap_or(0)
}

/// NPC `npc`'s tie with player `p` (made if new), changed by `f`; what `f` gave back.
fn with_tie<T>(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef, f: impl FnOnce(&mut Tie) -> T) -> T {
    let mut list = BONDS.get(host, Holder::Entity(npc.id())).unwrap_or_default();
    let i = match list.iter().position(|t| t.who == p.uuid()) {
        Some(i) => i,
        None => {
            let first = npc.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
            list.push(Tie { who: p.uuid(), name: p.name(&*host).unwrap_or_default(), first, ..Tie::default() });
            list.len() - 1
        }
    };
    let out = f(&mut list[i]);
    let _ = BONDS.set(host, Holder::Entity(npc.id()), list);
    out
}

fn tie(host: &mut dyn ServerHost, npc: EntityRef, who: u128) -> Option<Tie> {
    BONDS.get(host, Holder::Entity(npc.id())).ok()?.into_iter().find(|t| t.who == who)
}

/// Whether NPC `npc`'s character allows romance (`"romance": false` in it says no).
fn allows(host: &mut dyn ServerHost, npc: EntityRef) -> bool {
    !crate::heart::no_romance(host, npc)
}

/// Where NPC `npc` and player `p` stand.
pub fn stage(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef) -> Stage {
    let t = tie(host, npc, p.uuid()).unwrap_or_default();
    let relation = crate::heart::relation_of(npc.id(), p.uuid());
    let (met, _) = crate::heart::memory_of(npc.id(), p.uuid());
    let (friend_at, _) = crate::heart::thresholds(npc.id());
    Stage::of(relation, t.close, t.romance, met || t.last_talk != 0, friend_at)
}

/// How close they are (for the extended view).
pub fn closeness(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef) -> f32 {
    tie(host, npc, p.uuid()).map_or(0.0, |t| t.close)
}

/// The player's partner or sweetheart among the NPCs loaded, if one (its name).
fn player_taken(host: &mut dyn ServerHost, p: PlayerRef, but: u32) -> Option<String> {
    for id in companion::npcs() {
        if id == but {
            continue;
        }
        let Some(e) = host.entity(id) else { continue };
        if tie(host, e, p.uuid()).is_some_and(|t| t.romance != Romance::None) {
            return Some(e.name(&*host).unwrap_or_else(|| "someone".into()));
        }
    }
    None
}

/// Closeness up (or down) by `by`; the extended view says so.
pub fn add(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef, by: f32, why: &str) {
    let (before, after) = with_tie(host, npc, p, |t| {
        let b = t.close;
        t.close = (t.close + by).clamp(0.0, 100.0);
        (b, t.close)
    });
    if crate::flags::extended(&*host, p) && (after - before).abs() >= 0.5 {
        let name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
        let _ = p.tell(host, Text::literal(format!("[{name}] close {before:.0} > {after:.0} ({why})")));
    }
}

/// A talk starts: the first talk of a day brings them a little closer.
pub fn on_talk(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef) {
    let now = now_of(&*host, npc);
    let first = with_tie(host, npc, p, |t| {
        let first = t.last_talk == 0 || now / DAY != t.last_talk / DAY;
        t.last_talk = now;
        t.seen = now;
        first
    });
    if first {
        add(host, npc, p, 1.0, "talk");
    }
    if tie(host, npc, p.uuid()).is_some_and(|t| t.romance != Romance::None) {
        hearts(host, npc);
    }
}

fn hearts(host: &mut dyn ServerHost, npc: EntityRef) {
    if let (Some(level), Some(at)) = (npc.level(&*host), npc.position(&*host)) {
        crate::talk::queue(format!("execute in {} run particle minecraft:heart {:.2} {:.2} {:.2} 0.3 0.2 0.3 0 4", level.dimension_id(), at[0], at[1] + 2.1, at[2]));
    }
}

/// The player gives it what they hold.
pub fn give_held(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef) {
    let name = p.name(&*host).unwrap_or_default();
    let Some(stack) = LivingRef::new(p.entity()).equipment(&*host, EquipmentSlot::MainHand).filter(|s| s.count() > 0) else {
        crate::heart::say_to(host, npc, p, &format!("Hold what you want to give me, {name}."));
        return;
    };
    let item = stack.item().id(&*host);
    let now = now_of(&*host, npc);
    if tie(host, npc, p.uuid()).is_some_and(|t| t.last_gift != 0 && now / DAY == t.last_gift / DAY) {
        crate::heart::say_to(host, npc, p, &format!("You've given me plenty today, {name}."));
        return;
    }
    let uuid = npc.uuid(host).unwrap_or(0);
    let (rel, close, word) = bond::gift(&item, uuid);
    crate::talk::queue(format!("clear {name} {item} 1"));
    with_tie(host, npc, p, |t| {
        t.last_gift = now;
        if word == "love" {
            t.loved = item.clone();
        }
    });
    let what = bond::short(&item);
    let a = if what.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
    let cap = if a == "an" { "An" } else { "A" };
    let line = match word {
        "love" => format!("{cap} {what}! Oh, {name}, I love these!"),
        "like" => format!("{cap} {what}? Thank you, {name}."),
        "hate" => format!("Ugh, {a} {what}? Really, {name}?"),
        _ => format!("Oh. {cap} {what}. Thanks, I suppose."),
    };
    crate::heart::say_to(host, npc, p, &line);
    let before = crate::heart::relation_of(npc.id(), p.uuid());
    crate::heart::befriend(host, npc, p, rel);
    crate::flags::tell_change(host, npc, p, before, &format!("gift {word}"));
    add(host, npc, p, close, "gift");
    if word == "love" {
        hearts(host, npc);
        crate::heart::feel(host, npc, Some(p), "gift");
    }
}

/// A land they went through together, to remember.
pub fn note_place(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef, land: &str) {
    with_tie(host, npc, p, |t| {
        if !t.places.iter().any(|x| x == land) {
            t.places.push(land.to_string());
            if t.places.len() > 12 {
                t.places.remove(0);
            }
        }
    });
}

/// It brings up something they went through together.
pub fn reminisce(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef) {
    let now = now_of(&*host, npc);
    let t = tie(host, npc, p.uuid()).unwrap_or_default();
    let days = if t.first == 0 { 0 } else { (now - t.first) / DAY };
    let line = crate::remark::memory(days, t.fights, &t.places, &t.loved, (now as u64 / 20) ^ u64::from(npc.id()));
    let name = p.name(&*host).unwrap_or_default();
    crate::heart::say_to(host, npc, p, &line.replace("@player", &name));
}

/// It tells what it loves and can't stand, once close enough.
pub fn tell_tastes(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef) {
    let close = closeness(host, npc, p);
    let uuid = npc.uuid(host).unwrap_or(0);
    let (loves, hates) = bond::tastes(uuid);
    let line = if close >= 10.0 {
        format!("I'd do anything for {}, or {}. And I can't stand {}.", bond::short(loves[0]), bond::short(loves[1]), bond::short(hates))
    } else {
        "Maybe once we know each other a little better.".to_string()
    };
    crate::heart::say_to(host, npc, p, &line);
}

pub fn ask_out(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef) {
    let now = now_of(&*host, npc);
    let name = p.name(&*host).unwrap_or_default();
    let t = tie(host, npc, p.uuid()).unwrap_or_default();
    if t.asked != 0 && now - t.asked < DAY {
        crate::heart::say_to(host, npc, p, "Give me some time to think, please.");
        return;
    }
    let relation = crate::heart::relation_of(npc.id(), p.uuid());
    let taken = crate::families::partner_name(host, npc);
    let other = player_taken(host, p, npc.id());
    let ok = allows(host, npc);
    if let Err(line) = bond::may_ask(relation, t.close, t.romance, ok, taken.as_deref(), other.as_deref()) {
        crate::heart::say_to(host, npc, p, &line.replace("@player", &name));
        return;
    }
    let mood = crate::heart::mood_of(npc.id());
    let roll = ((now as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 40) % 100;
    with_tie(host, npc, p, |t| t.asked = now);
    if roll < u64::from(bond::yes_chance(&mood)) {
        with_tie(host, npc, p, |t| {
            t.romance = Romance::Sweetheart;
            t.since = now;
        });
        crate::heart::say_to(host, npc, p, &format!("I was hoping you'd ask, {name}. Yes!"));
        hearts(host, npc);
        crate::heart::feel(host, npc, Some(p), "praise");
        tell_stage(host, npc, p);
    } else {
        crate::heart::say_to(host, npc, p, &format!("Not now, {name}. Ask me another day."));
    }
}

pub fn propose(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef) {
    let now = now_of(&*host, npc);
    let name = p.name(&*host).unwrap_or_default();
    let t = tie(host, npc, p.uuid()).unwrap_or_default();
    let relation = crate::heart::relation_of(npc.id(), p.uuid());
    let ring = LivingRef::new(p.entity()).equipment(&*host, EquipmentSlot::MainHand).is_some_and(|s| s.item().id(&*host) == RING)
        || p.inventory(&*host).unwrap_or_default().iter().any(|s| s.item().id(&*host) == RING);
    if let Err(line) = bond::may_propose(relation, t.close, t.romance, t.since, now, ring) {
        crate::heart::say_to(host, npc, p, &line.replace("@player", &name));
        return;
    }
    crate::talk::queue(format!("clear {name} {RING} 1"));
    with_tie(host, npc, p, |t| {
        t.romance = Romance::Partner;
        t.since = now;
    });
    crate::heart::say_to(host, npc, p, &format!("Yes! Yes, {name}, a thousand times yes!"));
    hearts(host, npc);
    crate::heart::feel(host, npc, Some(p), "praise");
    tell_stage(host, npc, p);
}

/// The romance ends: the player says so, or it does (`by_it`).
pub fn part(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef, by_it: bool) {
    let had = tie(host, npc, p.uuid()).is_some_and(|t| t.romance != Romance::None);
    if !had {
        return;
    }
    let now = now_of(&*host, npc);
    with_tie(host, npc, p, |t| {
        t.romance = Romance::None;
        t.since = now;
        t.close = (t.close - 20.0).max(0.0);
    });
    let name = p.name(&*host).unwrap_or_default();
    let line = if by_it { format!("I can't do this anymore, {name}. It's over.") } else { format!("I... see. Goodbye, then, {name}.") };
    crate::heart::say_to(host, npc, p, &line);
    crate::heart::feel(host, npc, Some(p), "scold");
    tell_stage(host, npc, p);
}

fn tell_stage(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef) {
    let s = stage(host, npc, p);
    let name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
    let line = match s {
        Stage::Sweetheart => format!("{name} is your sweetheart now."),
        Stage::Partner => format!("{name} is your partner now."),
        _ => format!("{name} and you are apart now."),
    };
    let _ = p.action_bar(host, Text::literal(line));
}

/// The stage in words for a dialog's title (none: nothing past friends).
pub fn stage_word(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef) -> Option<&'static str> {
    Some(stage(host, npc, p).word()).filter(|w| !w.is_empty())
}

#[derive(Default)]
struct State {
    ticks: u32,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// A monster died: a player's companions near who saw it grow closer to them (a few a day).
pub fn on_death(host: &mut dyn ServerHost, victim: u32, killer: i32) {
    let Some(v) = host.entity(victim) else { return };
    if v.classification(&*host) != Some(MobCategory::Monster) || killer <= 0 {
        return;
    }
    let Some(k) = host.entity(killer as u32) else { return };
    let Some(level) = v.level(&*host) else { return };
    let players = level.online_players(&*host).unwrap_or_default();
    // The player it was for: the killer, or the owner of the NPC that killed it.
    let p = players.iter().copied().find(|p| p.entity().id() == k.id()).or_else(|| {
        let owner = LivingRef::new(k).owner(&*host)?;
        players.iter().copied().find(|p| p.uuid() == owner)
    });
    let Some(p) = p else { return };
    let Some(at) = p.entity().position(&*host) else { return };
    let now = level.game_time(&*host).unwrap_or(0);
    for id in companion::npcs() {
        let Some(e) = host.entity(id) else { continue };
        if LivingRef::new(e).owner(&*host) != Some(p.uuid()) || e.position(&*host).is_none_or(|q| (q[0] - at[0]).powi(2) + (q[2] - at[2]).powi(2) > 256.0) {
            continue;
        }
        let gain = with_tie(host, e, p, |t| {
            if t.day != now / DAY {
                t.day = now / DAY;
                t.fought = 0.0;
            }
            let g = (6.0 - t.fought).clamp(0.0, 1.0);
            t.fought += g;
            t.fights += 1;
            g
        });
        if gain > 0.0 {
            add(host, e, p, gain, "fought together");
        }
    }
}

/// The tick (every minute): time together brings closeness, time apart takes it, a romance ends
/// when it has come to dislike the player, and partners bring their player a little something.
pub fn tick(host: &mut dyn ServerHost) {
    let step = STATE.lock().map(|mut s| {
        s.ticks = s.ticks.wrapping_add(1);
        s.ticks % 1200 == 0
    });
    if !matches!(step, Ok(true)) {
        return;
    }
    for id in companion::npcs() {
        let Some(e) = host.entity(id) else { continue };
        let Some(level) = e.level(&*host) else { continue };
        let now = level.game_time(&*host).unwrap_or(0);
        let Some(at) = e.position(&*host) else { continue };
        let players = level.online_players(&*host).unwrap_or_default();
        let ties = BONDS.get(host, Holder::Entity(id)).unwrap_or_default();
        for p in &players {
            let near = p.entity().position(&*host).is_some_and(|q| (q[0] - at[0]).powi(2) + (q[2] - at[2]).powi(2) <= 64.0);
            let following = LivingRef::new(e).owner(&*host) == Some(p.uuid()) && companion::mode_of(id) == Mode::Follow;
            if near && (following || ties.iter().any(|t| t.who == p.uuid() && t.romance != Romance::None)) {
                with_tie(host, e, *p, |t| t.seen = now);
                add(host, e, *p, 0.5, "time together");
                // A partner brings a little something, once a day.
                let romance = tie(host, e, p.uuid()).map(|t| (t.romance, t.treat_day)).unwrap_or_default();
                if romance.0 == Romance::Partner && romance.1 != now / DAY && (now / 1200) % 7 == i64::from(id) % 7 {
                    with_tie(host, e, *p, |t| t.treat_day = now / DAY);
                    let item = ["minecraft:poppy", "minecraft:cookie", "minecraft:apple", "minecraft:sweet_berries"][(now / DAY) as usize % 4];
                    let name = p.name(&*host).unwrap_or_default();
                    crate::talk::queue(format!("give {name} {item} 1"));
                    crate::heart::say_to(host, e, *p, &format!("I picked this for you, {name}."));
                    hearts(host, e);
                }
            }
        }
        for t in ties {
            // Days apart: a point a day after three.
            if t.seen != 0 && now - t.seen > 3 * DAY && (now / 1200) % 20 == 0 {
                if let Some(p) = players.iter().find(|p| p.uuid() == t.who) {
                    add(host, e, *p, -1.0, "time apart");
                }
            }
            // It has come to dislike the player: the romance ends.
            if t.romance != Romance::None
                && crate::heart::relation_of(id, t.who) < bond::BREAKS_BELOW
                && let Some(p) = players.iter().find(|p| p.uuid() == t.who)
            {
                part(host, e, *p, true);
            }
        }
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix bond <targets:entity>", 2));
    e.commands.push(Command::op("felix bond set <targets:entity> <close:integer>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if !matches!(lit.as_slice(), ["felix", "bond"] | ["felix", "bond", "set"]) {
        return Flow::Continue;
    }
    let mod_api::CommandSource::Player { player, .. } = e.source else { return Flow::Cancel };
    let p = *player;
    let Some(npc) = companion::targets(e).into_iter().find(|t| companion::is_npc(&*host, *t)) else { return Flow::Cancel };
    if lit.len() == 3 {
        let v = e.command.arguments.iter().find(|a| a.name == "close").and_then(|a| match a.value {
            mod_api::CommandArgumentValue::Integer(v) => Some(v.clamp(0, 100) as f32),
            _ => None,
        });
        if let Some(v) = v {
            with_tie(host, npc, p, |t| t.close = v);
        }
    }
    let s = stage(host, npc, p);
    let t = tie(host, npc, p.uuid()).unwrap_or_default();
    let uuid = npc.uuid(host).unwrap_or(0);
    let (loves, hates) = bond::tastes(uuid);
    let name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
    let line = format!(
        "{name} and you: {} (relation {}, close {:.0}); loves {} and {}, can't stand {}",
        s.name(),
        crate::heart::relation_of(npc.id(), p.uuid()),
        t.close,
        bond::short(loves[0]),
        bond::short(loves[1]),
        bond::short(hates)
    );
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
