//! Minds at work on the server (see `mind`): every NPC's thoughts move its mood; it gets tired and
//! sleeps, gets bored and looks for fun, likes a fine home and minds a dreary one; low too long it
//! breaks (sulks, hides, binges, snaps, throws a tantrum, wanders off, freezes) and is relieved
//! after; high, it may be inspired (works twice as fast, sells cheaper, brings someone a present).
//! It says how it feels when asked ("How are you feeling?"). `/felix mind <npc>`,
//! `/felix mind set <npc> <mood>`, `/felix mind break <npc> <kind>`.
use crate::companion::{self, Mode};
use crate::mind::{self, Break, Inspiration, Mind, Trait};
use mod_api::{
    AttachmentCodec, AttachmentType, Command, CommandArgumentValue, Compound, EntityRef, Flow, Holder, LivingRef, PlayerRef,
    RegisterCommandsEvent, RunModCommandEvent, ServerHost, Tag, Text,
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

pub static MIND: AttachmentType<Option<Mind>> = AttachmentType {
    namespace: "projectfelix",
    name: "mind",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |m| {
            let mut c = Compound::new();
            if let Some(m) = m {
                c.insert("json".into(), Tag::String(m.to_json().to_string().as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            Ok(match c.get(&"json".into()) {
                Some(Tag::String(s)) => crate::serde_json::from_str(&s.to_string_lossy()).ok().map(|v| Mind::from_json(&v)),
                _ => None,
            })
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// Each loaded NPC's mind, as last brought up to date, and what it last did about a break.
#[derive(Default)]
struct State {
    minds: HashMap<u32, Mind>,
    /// When it last went somewhere or snapped in its break.
    acted: HashMap<u32, i64>,
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
        s.minds.remove(&id);
        s.acted.remove(&id);
    }
}

fn roll() -> u64 {
    let Ok(mut s) = STATE.lock() else { return 0 };
    s.seed = s.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut x = s.seed;
    x ^= x >> 31;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^ (x >> 29)
}

fn chance(p: f32) -> bool {
    (roll() % 1_000_000) as f32 / 1_000_000.0 < p
}

pub fn traits_of(host: &mut dyn ServerHost, npc: EntityRef) -> Vec<Trait> {
    npc.uuid(host).map(mind::traits).unwrap_or_default()
}

/// NPC `npc`'s mind (read once, then kept while it is loaded).
fn get(host: &mut dyn ServerHost, npc: EntityRef) -> Mind {
    if let Some(m) = STATE.lock().ok().and_then(|s| s.minds.get(&npc.id()).cloned()) {
        return m;
    }
    MIND.get(host, Holder::Entity(npc.id())).ok().flatten().unwrap_or_default()
}

fn put(host: &mut dyn ServerHost, npc: EntityRef, m: Mind) {
    let _ = MIND.set(host, Holder::Entity(npc.id()), Some(m.clone()));
    if let Ok(mut s) = STATE.lock() {
        s.minds.insert(npc.id(), m);
    }
}

fn now_of(host: &dyn mod_api::Host, npc: EntityRef) -> i64 {
    npc.level(host).and_then(|l| l.game_time(host)).unwrap_or(0)
}

/// What a loaded NPC is going through, if anything.
pub fn breaking(id: u32) -> Option<Break> {
    STATE.lock().ok()?.minds.get(&id)?.breaking()
}

pub fn inspired(id: u32) -> Option<Inspiration> {
    STATE.lock().ok()?.minds.get(&id)?.inspired()
}

pub fn obey_factor(id: u32) -> f32 {
    STATE.lock().ok().and_then(|s| s.minds.get(&id).map(Mind::obey_factor)).unwrap_or(1.0)
}

/// How fast NPC `npc` works: its traits, and twice that inspired.
pub fn work_factor(host: &mut dyn ServerHost, npc: EntityRef) -> f32 {
    let f = mind::work_factor(&traits_of(host, npc));
    if inspired(npc.id()) == Some(Inspiration::Work) { f * 2.0 } else { f }
}

/// NPC `npc` remembers `thought` now.
pub fn remember(host: &mut dyn ServerHost, npc: EntityRef, thought: &str) {
    let now = now_of(&*host, npc);
    let mut m = get(host, npc);
    m.remember(thought, now);
    put(host, npc, m);
}

/// NPC `npc` had fun of `kind` worth `amount`.
pub fn enjoy(host: &mut dyn ServerHost, npc: EntityRef, kind: usize, amount: f32) {
    let traits = traits_of(host, npc);
    let mut m = get(host, npc);
    m.enjoy(kind, amount, &traits);
    put(host, npc, m);
}

/// What a feeling (see `heart`) leaves in its mind; `counts`: the feeling wasn't one said too
/// often.
pub fn felt(host: &mut dyn ServerHost, npc: EntityRef, event: &str, counts: bool) {
    let (thought, fun) = match event {
        "gift" => (Some("gift"), None),
        "praise" => (Some("praised"), Some((mind::TALK, 6.0))),
        "talk" => (Some("talked"), Some((mind::TALK, 12.0))),
        "apology" => (Some("apology"), None),
        "scold" => (Some("scolded"), None),
        "hurt_by_player" => (Some("hurt"), None),
        "friend_hurt" => (Some("friend_hurt"), None),
        "friend_killed" => (Some("friend_died"), None),
        "won_game" => (Some("won_game"), Some((mind::GAMES, 30.0))),
        "lost_game" => (Some("lost_game"), Some((mind::GAMES, 25.0))),
        _ => (None, None),
    };
    let good = thought.and_then(mind::thought).is_some_and(|t| t.mood > 0.0);
    if !counts && good {
        return;
    }
    let now = now_of(&*host, npc);
    let traits = traits_of(host, npc);
    let mut m = get(host, npc);
    if let Some(t) = thought {
        m.remember(t, now);
    }
    if let Some((kind, amount)) = fun {
        m.enjoy(kind, amount, &traits);
    }
    put(host, npc, m);
}

/// NPC `npc` ate `item`: a meal, a treat, or something raw.
pub fn ate(host: &mut dyn ServerHost, npc: EntityRef, item: &str) {
    let i = item.trim_start_matches("minecraft:");
    let thought = if matches!(i, "pumpkin_pie" | "cookie" | "honey_bottle" | "golden_apple" | "cake" | "sweet_berries" | "glow_berries") {
        "ate_treat"
    } else if i.starts_with("cooked_") || matches!(i, "bread" | "baked_potato" | "mushroom_stew" | "beetroot_soup" | "rabbit_stew") {
        "ate_meal"
    } else if matches!(i, "potato" | "beetroot" | "cod" | "salmon" | "dried_kelp") || i.starts_with("raw_") {
        "ate_raw"
    } else {
        return;
    };
    remember(host, npc, thought);
}

/// NPC `victim` died: those who saw it remember, its family grieves.
pub fn on_death(host: &mut dyn ServerHost, victim: EntityRef, family: &[u128]) {
    let Some(at) = victim.position(&*host) else { return };
    let dim = victim.level(&*host).map(|l| l.dimension_id());
    for id in companion::npcs() {
        if id == victim.id() {
            continue;
        }
        let Some(e) = host.entity(id) else { continue };
        if e.level(&*host).map(|l| l.dimension_id()) != dim {
            continue;
        }
        let kin = e.uuid(host).is_some_and(|u| family.contains(&u));
        let near = e.position(&*host).is_some_and(|p| (p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2) + (p[2] - at[2]).powi(2) < 24.0 * 24.0);
        if kin {
            remember(host, e, "family_died");
            crate::heart::say_aloud(host, e, "No... no, not them. Please, no.");
        } else if near {
            remember(host, e, "saw_death");
        }
    }
}

/// A player talks to NPC `npc`: an NPC inspired to give brings them its present, if it likes them.
pub fn on_talk(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) {
    if inspired(npc.id()) != Some(Inspiration::Gift) {
        return;
    }
    let (friend_at, _) = crate::heart::thresholds(npc.id());
    if crate::heart::relation_of(npc.id(), player.uuid()) < friend_at {
        return;
    }
    const PRESENTS: &[(&str, u32)] = &[("cake", 1), ("cookie", 6), ("emerald", 3), ("golden_apple", 1), ("poppy", 5), ("glow_berries", 8), ("book", 1)];
    let (item, n) = PRESENTS[(roll() % PRESENTS.len() as u64) as usize];
    let name = player.name(&*host).unwrap_or_default();
    crate::talk::queue(format!("give {name} minecraft:{item} {n}"));
    crate::heart::say_to(host, npc, player, "I made this for you. Well... found it. It's for you!");
    let mut m = get(host, npc);
    m.insp = None;
    put(host, npc, m);
}

/// Whether NPC `npc` won't talk now (and it says so).
pub fn shuts_out(host: &mut dyn ServerHost, npc: EntityRef) -> bool {
    let Some(b) = breaking(npc.id()).filter(|b| b.shuts_out()) else { return false };
    let line = match b {
        Break::Freeze => "...",
        Break::Snap => "Go away!",
        Break::Hide => "Not now. Please, just leave me alone.",
        _ => "I don't want to talk right now.",
    };
    crate::heart::say_aloud(host, npc, line);
    true
}

/// How fine NPC `npc`'s home is: the blocks about its bed (else its home spot), or none.
fn home_score(host: &mut dyn ServerHost, npc: EntityRef) -> Option<i32> {
    let level = npc.level(&*host)?;
    let at = crate::routines::bed_of(npc.id()).or_else(|| {
        let h = crate::routines::place_named(host, npc, "home")?;
        Some([h[0].floor() as i32, h[1].floor() as i32, h[2].floor() as i32])
    })?;
    let mut score = 0.0f32;
    for x in at[0] - 4..=at[0] + 4 {
        for z in at[2] - 4..=at[2] + 4 {
            for y in at[1] - 1..=at[1] + 3 {
                let id = level.block(&*host, [x, y, z]).map(|k| k.id(&*host)).unwrap_or_default();
                score += mind::block_worth(&id);
            }
        }
    }
    // A home under the open sky is no home.
    if level.can_see_sky(&*host, [at[0], at[1] + 1, at[2]]) == Some(true) {
        score -= 10.0;
    }
    Some(score.round() as i32)
}

/// What NPC `npc` is thinking now (the thoughts of now).
fn current(host: &mut dyn ServerHost, npc: EntityRef, m: &mut Mind, traits: &[Trait], now: i64, awake: bool) -> Vec<&'static str> {
    let id = npc.id();
    let mut out = Vec::new();
    mind::need_thoughts(crate::needs::food_of(id), m.rest, m.fun, &mut out);
    let living = companion::mode_of(id) == Mode::Live && crate::routines::has_routine(id);
    if living {
        // Its home, looked at again twice a day.
        if now - m.home.1 >= mind::DAY / 2 || now < m.home.1 {
            m.home = (home_score(host, npc), now);
        }
        if let Some(t) = mind::home_thought(m.home.0, traits) {
            out.push(t);
        }
    }
    let level = npc.level(&*host);
    let night = level.and_then(|l| l.day_time(&*host)).is_some_and(|t| (13_000..23_000).contains(&t.rem_euclid(24_000)));
    mind::trait_thoughts(traits, night, awake, &mut out);
    if let (Some(l), Some(p)) = (level, npc.position(&*host)) {
        let head = [p[0].floor() as i32, p[1].floor() as i32 + 1, p[2].floor() as i32];
        if l.is_raining_at(&*host, head) == Some(true) {
            out.push("soaked");
        }
    }
    let life = LivingRef::new(npc);
    if let (Some(h), Some(max)) = (life.health(&*host), life.max_health(&*host))
        && max > 0.0
    {
        let f = h / max;
        if f < 0.35 {
            out.push("bad_pain");
        } else if f < 0.7 {
            out.push("pain");
        }
    }
    if crate::families::partner_name(host, npc).is_some() {
        out.push("with_love");
    }
    out
}

/// The tick of each level: needs, thoughts, mood, breaks and inspirations of its NPCs.
pub fn tick(host: &mut dyn ServerHost) {
    if !crate::pace::every("minds", 100) {
        return;
    }
    let npcs = companion::npcs();
    for id in &npcs {
        let Some(npc) = host.entity(*id) else { continue };
        let Some(uuid) = npc.uuid(host) else { continue };
        let traits = mind::traits(uuid);
        let now = now_of(&*host, npc);
        let mut m = get(host, npc);
        let awake = !crate::routines::asleep(*id);
        let in_bed = crate::routines::bed_of(*id).is_some();
        let prev = m.last;
        let current = current(host, npc, &mut m, &traits, now, awake);
        // Sleeping without a bed, or in the rain, it remembers in the morning.
        if !awake {
            if !in_bed && !m.has_memory("slept_rough", now) && crate::routines::has_routine(*id) {
                m.remember("slept_rough", now);
            }
            if current.contains(&"soaked") && !m.has_memory("slept_cold", now) {
                m.remember("slept_cold", now);
            }
        }
        let had = m.brk;
        m.advance(now, awake, in_bed, &traits, &current);
        let dt = if prev == 0 { 0 } else { (now - prev).clamp(0, mind::DAY) };
        // Fun as it goes: a gathering, an event, out with its owner.
        if awake && dt > 0 {
            let hours = dt as f32 / mind::HOUR as f32;
            match crate::routines::doing(*id).as_str() {
                "event" => {
                    m.enjoy(mind::GATHERING, 10.0 * hours, &traits);
                    if !m.has_memory("festival", now) {
                        m.remember("festival", now);
                    }
                }
                "meet" => {
                    m.enjoy(mind::GATHERING, 8.0 * hours, &traits);
                }
                _ if companion::mode_of(*id) == Mode::Follow => {
                    m.enjoy(mind::SIGHTS, 3.0 * hours, &traits);
                }
                _ if m.home.0.is_some_and(|s| s >= 10) => {
                    m.enjoy(mind::SIGHTS, 1.5 * hours, &traits);
                }
                _ => {}
            }
        }
        // A break over: it comes back.
        if had.is_some() && m.brk.is_none() {
            companion::go_to(*id, None);
            crate::heart::say_aloud(host, npc, ["Sorry about all that. I feel better now.", "Phew. I needed that.", "I'm all right now. Really."][(*id as usize) % 3]);
        }
        // Low: it may break. High: it may be inspired.
        if awake && dt > 0 && m.brk.is_none() {
            if let Some(level) = mind::break_level(m.mood, &traits) {
                if chance(mind::break_chance(level, dt)) {
                    let b = mind::pick_break(level, &traits, roll());
                    start_break(host, npc, &mut m, b, now);
                }
            } else if m.insp.is_none() && chance(mind::inspire_chance(m.mood, dt)) {
                let i = mind::pick_inspiration(roll());
                m.insp = Some((i, now + 3 * mind::DAY));
                crate::heart::say_aloud(host, npc, i.line());
            }
        }
        if let Some((b, _)) = m.brk {
            in_break(host, npc, b, now);
        }
        put(host, npc, m);
    }
    if let Ok(mut s) = STATE.lock() {
        s.minds.retain(|id, _| npcs.contains(id));
        s.acted.retain(|id, _| npcs.contains(id));
    }
}

fn start_break(host: &mut dyn ServerHost, npc: EntityRef, m: &mut Mind, b: Break, now: i64) {
    m.brk = Some((b, now + b.lasts()));
    crate::heart::say_aloud(host, npc, b.line());
    if let Ok(mut s) = STATE.lock() {
        s.acted.remove(&npc.id());
    }
    match b {
        Break::Binge => {
            for _ in 0..4 {
                if !crate::needs::eat_from_pantry(host, npc) {
                    break;
                }
            }
        }
        Break::Tantrum => {
            crate::routines::play_animation(host, npc, "animation.felix.angry");
            // Its things, thrown out of its chest.
            if let Some(c) = crate::jobs::chest_of(host, npc).flatten()
                && let Some(at) = npc.position(&*host)
            {
                for item in ["minecraft:wheat", "minecraft:bread", "minecraft:oak_log", "minecraft:emerald", "minecraft:cod", "minecraft:apple"] {
                    let n = crate::jobs::amount(host, c, item).min(8) as u32;
                    if n > 0 && crate::jobs::take(host, c, item, n) {
                        crate::talk::queue(format!("summon item {:.1} {:.1} {:.1} {{Item:{{id:\"{item}\",count:{n}}}}}", at[0], at[1] + 1.0, at[2]));
                    }
                }
            }
        }
        Break::Freeze => crate::routines::play_animation(host, npc, "animation.felix.sad"),
        _ => {}
    }
}

/// What it does through a break: where it goes, whom it snaps at.
fn in_break(host: &mut dyn ServerHost, npc: EntityRef, b: Break, now: i64) {
    let id = npc.id();
    let last = STATE.lock().ok().and_then(|s| s.acted.get(&id).copied()).unwrap_or(i64::MIN / 2);
    let every = match b {
        Break::Snap => 400,
        Break::Leave | Break::Hide | Break::Freeze => 1200,
        _ => 600,
    };
    if now - last < every && now >= last {
        return;
    }
    if let Ok(mut s) = STATE.lock() {
        s.acted.insert(id, now);
    }
    let Some(pos) = npc.position(&*host) else { return };
    let r = roll();
    let near = |d: f64| {
        let a = (r % 360) as f64 * std::f64::consts::PI / 180.0;
        [pos[0] + a.cos() * d, pos[1], pos[2] + a.sin() * d]
    };
    match b {
        Break::Sulk | Break::Daze => companion::go_to(id, Some(near(4.0 + (r % 7) as f64))),
        Break::Hide => {
            let home = crate::routines::bed_of(id).map(|b| [b[0] as f64 + 0.5, b[1] as f64, b[2] as f64 + 0.5]).or_else(|| crate::routines::place_named(host, npc, "home"));
            companion::go_to(id, Some(home.unwrap_or(pos)));
        }
        Break::Leave => {
            // Away from home, the same way each time.
            let home = crate::routines::place_named(host, npc, "home").unwrap_or(pos);
            let a = (id % 8) as f64 * std::f64::consts::FRAC_PI_4;
            companion::go_to(id, Some([home[0] + a.cos() * 48.0, home[1], home[2] + a.sin() * 48.0]));
        }
        Break::Freeze => {
            companion::go_to(id, Some(pos));
            crate::routines::play_animation(host, npc, "animation.felix.sad");
        }
        Break::Snap => {
            // The nearest NPC within 8 blocks hears it.
            let mut best: Option<(f64, EntityRef)> = None;
            for o in companion::npcs() {
                if o == id {
                    continue;
                }
                let Some(e) = host.entity(o) else { continue };
                let Some(p) = e.position(&*host) else { continue };
                let d = (p[0] - pos[0]).powi(2) + (p[1] - pos[1]).powi(2) + (p[2] - pos[2]).powi(2);
                if d < 64.0 && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, e));
                }
            }
            if let Some((_, other)) = best {
                let name = other.name(&*host).unwrap_or_else(|| "you".into());
                let lines = ["You're useless, {n}.", "Nobody likes you, {n}.", "Get out of my way, {n}!", "What's that smell? Oh, it's {n}."];
                crate::heart::say_aloud(host, npc, &lines[(r % 4) as usize].replace("{n}", &name));
                remember(host, other, "insulted");
            }
        }
        _ => {}
    }
}

/// How NPC `npc` feels, as it tells a player.
pub fn tell(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) {
    let now = now_of(&*host, npc);
    let traits = traits_of(host, npc);
    let mut m = get(host, npc);
    let awake = !crate::routines::asleep(npc.id());
    let current = current(host, npc, &mut m, &traits, now, awake);
    let thoughts = m.thoughts(now, &current);
    let mut line = format!("I'm {}.", mind::mood_word(m.mood));
    let top: Vec<String> = thoughts.iter().take(3).map(|(t, _)| t.label.to_lowercase()).collect();
    if !top.is_empty() {
        line.push_str(&format!(" On my mind: {}.", top.join(", ")));
    }
    if let Some(i) = m.inspired() {
        line.push_str(&format!(" Today I'm {}!", i.doing()));
    }
    crate::heart::say_to(host, npc, player, &line);
    crate::heart::say_to(host, npc, player, &format!("(Folk say I'm {}.)", mind::words(&traits)));
    put(host, npc, m);
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix mind <targets:entity>", 2));
    e.commands.push(Command::op("felix mind set <targets:entities> <mood:integer>", 2));
    e.commands.push(Command::op("felix mind break <targets:entities> <kind:word>", 2));
    e.commands.push(Command::op("felix mind inspire <targets:entities> <kind:word>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if lit.first() != Some(&"felix") || lit.get(1) != Some(&"mind") {
        return Flow::Continue;
    }
    let npcs: Vec<EntityRef> = companion::targets(e).into_iter().filter(|t| companion::is_npc(&*host, *t)).collect();
    let Some(first) = npcs.first().copied() else {
        let _ = host.commands().feedback(Text::literal("That is not an NPC."));
        return Flow::Cancel;
    };
    let word = e.command.arguments.iter().find(|a| a.name == "kind").and_then(|a| match &a.value {
        CommandArgumentValue::String(s) => Some(s.clone()),
        _ => None,
    });
    let lines: Vec<String> = match lit.as_slice() {
        ["felix", "mind"] => {
            let npc = first;
            let now = now_of(&*host, npc);
            let traits = traits_of(host, npc);
            let mut m = get(host, npc);
            let awake = !crate::routines::asleep(npc.id());
            let current = current(host, npc, &mut m, &traits, now, awake);
            let name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
            let mut out = vec![format!(
                "{name}: mood {:.0} -> {:.0} ({}), rest {:.0}, fun {:.0}, food {}; {}",
                m.mood,
                m.target(now, &current),
                mind::mood_word(m.mood),
                m.rest,
                m.fun,
                crate::needs::food_of(npc.id()),
                mind::words(&traits)
            )];
            if let Some((b, until)) = m.brk {
                out.push(format!("Breaking: {} ({}s left)", b.doing(), (until - now) / 20));
            }
            if let Some((i, until)) = m.insp {
                out.push(format!("Inspired: {} ({}s left)", i.doing(), (until - now) / 20));
            }
            if let Some(h) = m.home.0 {
                out.push(format!("Home: {h}"));
            }
            for (t, v) in m.thoughts(now, &current) {
                out.push(format!("  {:+.0} {}", v, t.label));
            }
            put(host, npc, m);
            out
        }
        ["felix", "mind", "set"] => {
            let mood = e.command.arguments.iter().find(|a| a.name == "mood").and_then(|a| match a.value {
                CommandArgumentValue::Integer(v) => Some(v.clamp(0, 100) as f32),
                _ => None,
            });
            let Some(mood) = mood else { return Flow::Cancel };
            for n in &npcs {
                let mut m = get(host, *n);
                m.mood = mood;
                put(host, *n, m);
            }
            vec![format!("{} NPCs now have mood {mood:.0}", npcs.len())]
        }
        ["felix", "mind", "break"] => {
            let Some(b) = word.as_deref().and_then(Break::parse) else {
                let _ = host.commands().feedback(Text::literal("Breaks: sulk, hide, binge, snap, tantrum, daze, leave, freeze."));
                return Flow::Cancel;
            };
            for n in &npcs {
                let now = now_of(&*host, *n);
                let mut m = get(host, *n);
                start_break(host, *n, &mut m, b, now);
                put(host, *n, m);
            }
            vec![format!("{} NPCs are {}", npcs.len(), b.doing())]
        }
        ["felix", "mind", "inspire"] => {
            let Some(i) = word.as_deref().and_then(Inspiration::parse) else {
                let _ = host.commands().feedback(Text::literal("Inspirations: work, trade, gift."));
                return Flow::Cancel;
            };
            for n in &npcs {
                let now = now_of(&*host, *n);
                let mut m = get(host, *n);
                m.insp = Some((i, now + 3 * mind::DAY));
                put(host, *n, m);
                crate::heart::say_aloud(host, *n, i.line());
            }
            vec![format!("{} NPCs are {}", npcs.len(), i.doing())]
        }
        _ => return Flow::Continue,
    };
    for l in lines {
        let _ = host.commands().feedback(Text::literal(l));
    }
    Flow::Cancel
}
