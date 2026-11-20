//! The village market on the server (see `market`): NPCs' purses, the baker's oven, trade between
//! NPCs' chests, prices by what the village has, and talk of shortages. `/felix market` tells
//! what the village about you has; `/felix purse <npc> <n>` fills a purse.
use crate::companion::{self, LiveDo};
use crate::job::Job;
use crate::market::{self, Supply};
use crate::need::{FOODS, HUNGRY};
use mod_api::{
    AttachmentCodec, AttachmentType, Command, CommandArgumentValue, Compound, EntityRef, Flow, Hand, Holder, LevelRef,
    LivingRef, PlayerRef, RegisterCommandsEvent, RunModCommandEvent, ServerHost, SoundSource, Tag, Text,
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Purse {
    pub coins: u32,
    /// The last day it was paid its wage.
    pub day: i64,
}

impl Default for Purse {
    fn default() -> Self {
        Purse { coins: 5, day: -1 }
    }
}

pub static PURSE: AttachmentType<Purse> = AttachmentType {
    namespace: "projectfelix",
    name: "purse",
    default: Purse::default,
    codec: Some(AttachmentCodec {
        encode: |p| {
            let mut c = Compound::new();
            c.insert("coins".into(), Tag::Int(p.coins as i32));
            c.insert("day".into(), Tag::Long(p.day));
            Ok(c)
        },
        decode: |c: &Compound| {
            Ok(Purse {
                coins: match c.get(&"coins".into()) {
                    Some(Tag::Int(v)) => (*v).max(0) as u32,
                    _ => 5,
                },
                day: match c.get(&"day".into()) {
                    Some(Tag::Long(v)) => *v,
                    _ => -1,
                },
            })
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// How far a village reaches about an NPC or a player.
const RANGE: f64 = 64.0;
/// Ticks between two market rounds.
const ROUND: u32 = 400;

#[derive(Default)]
struct State {
    ticks: u32,
    /// When each baker bakes next.
    oven: HashMap<u32, i64>,
    /// When each NPC last said something of the market, and each player last heard of a shortage.
    said: HashMap<u32, i64>,
    heard: HashMap<u128, i64>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

pub fn purse_of(host: &mut dyn ServerHost, npc: EntityRef) -> u32 {
    PURSE.get(host, Holder::Entity(npc.id())).unwrap_or_default().coins
}

fn set_purse(host: &mut dyn ServerHost, npc: EntityRef, coins: u32, day: Option<i64>) {
    let p = PURSE.get(host, Holder::Entity(npc.id())).unwrap_or_default();
    let _ = PURSE.set(host, Holder::Entity(npc.id()), Purse { coins: coins.min(market::MOST), day: day.unwrap_or(p.day) });
}

/// NPC `npc` earns `n` (a player bought from it).
pub fn earn(host: &mut dyn ServerHost, npc: EntityRef, n: u32) {
    let c = purse_of(host, npc);
    set_purse(host, npc, c.saturating_add(n), None);
}

/// NPC `npc` spends `n` (it bought from a player), as far as it has.
pub fn spend(host: &mut dyn ServerHost, npc: EntityRef, n: u32) {
    let c = purse_of(host, npc);
    set_purse(host, npc, c.saturating_sub(n), None);
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn name_of(host: &dyn mod_api::Host, e: EntityRef) -> String {
    e.name(host).unwrap_or_else(|| "NPC".into())
}

/// An NPC with a job and its chest.
#[derive(Clone, Copy)]
struct Stall {
    npc: EntityRef,
    job: Job,
    chest: (LevelRef, [i32; 3]),
}

/// The village about `at`: its stalls, and how many NPCs live there.
fn village(host: &mut dyn ServerHost, level: LevelRef, at: [f64; 3]) -> (Vec<Stall>, u64) {
    let near: Vec<EntityRef> = companion::npcs()
        .into_iter()
        .filter_map(|id| host.entity(id))
        .filter(|e| e.level(&*host) == Some(level) && e.position(&*host).is_some_and(|p| dist(p, at) <= RANGE))
        .collect();
    let mut stalls = Vec::new();
    for e in &near {
        let Some(job) = crate::jobs::job_of(host, *e) else { continue };
        if let Some(c) = crate::jobs::chest_of(host, *e).flatten() {
            stalls.push(Stall { npc: *e, job, chest: c });
        }
    }
    (stalls, near.len() as u64)
}

fn stock(host: &mut dyn ServerHost, stalls: &[Stall], item: &str) -> u64 {
    stalls.iter().map(|s| crate::jobs::amount(host, s.chest, item)).sum()
}

fn food_stock(host: &mut dyn ServerHost, stalls: &[Stall]) -> u64 {
    FOODS.iter().filter(|f| f.1 >= 3).map(|f| stock(host, stalls, f.0)).sum()
}

/// How the village about NPC `npc` stands for good `item`.
pub fn supply_of(host: &mut dyn ServerHost, npc: EntityRef, item: &str) -> Supply {
    let (Some(level), Some(at)) = (npc.level(&*host), npc.position(&*host)) else { return Supply::Enough };
    let (stalls, people) = village(host, level, at);
    if stalls.is_empty() {
        return Supply::Enough;
    }
    market::supply(stock(host, &stalls, item), people)
}

/// How the village about player `p` stands for food (none: no village).
pub fn food_about(host: &mut dyn ServerHost, p: PlayerRef) -> Option<Supply> {
    let (level, at) = (p.entity().level(&*host)?, p.entity().position(&*host)?);
    let (stalls, people) = village(host, level, at);
    if stalls.is_empty() {
        return None;
    }
    let food = food_stock(host, &stalls);
    Some(market::supply(food, people))
}

fn may_say(id: u32, now: i64, every: i64) -> bool {
    STATE.lock().is_ok_and(|mut s| {
        let last = s.said.get(&id).copied().unwrap_or(i64::MIN / 2);
        if now - last >= every || now < last {
            s.said.insert(id, now);
            true
        } else {
            false
        }
    })
}

fn pick(lines: &[&'static str], seed: i64) -> &'static str {
    lines[(seed.unsigned_abs() as usize) % lines.len()]
}

/// The baker's work, at `at` (its work place): it bakes what its chest allows.
pub fn bake(host: &mut dyn ServerHost, me: EntityRef, at: Option<[f64; 3]>) {
    let Some(level) = me.level(&*host) else { return };
    let (Some(now), Some(pos)) = (level.game_time(&*host), me.position(&*host)) else { return };
    let at = at.unwrap_or(pos);
    let next = STATE.lock().ok().and_then(|s| s.oven.get(&me.id()).copied()).unwrap_or(0);
    if now < next && now + 1000 > next {
        return;
    }
    if let Ok(mut s) = STATE.lock() {
        s.oven.insert(me.id(), now + Job::Bake.rest());
    }
    companion::set_live(host, me, at, LiveDo::Work);
    let Some(chest) = crate::jobs::chest_of(host, me).flatten() else {
        if may_say(me.id(), now, 3000) {
            crate::heart::say_aloud(host, me, "I need a chest for my bread.");
        }
        return;
    };
    let have: Vec<(&str, u64)> = market::RECIPES.iter().map(|r| (r.0, crate::jobs::amount(host, chest, r.0))).collect();
    let Some((from, n, to)) = market::bake(&|i| have.iter().find(|h| h.0 == i).map_or(0, |h| h.1)) else { return };
    if !crate::jobs::take(host, chest, from, n) {
        return;
    }
    if crate::jobs::put_item(host, chest, to, 1) == 0 {
        crate::jobs::put_item(host, chest, from, n);
        if may_say(me.id(), now, 3000) {
            crate::heart::say_aloud(host, me, "My shelves are full!");
        }
        return;
    }
    let _ = LivingRef::new(me).swing(host, Hand::Main, false);
    crate::routines::play_animation(host, me, "animation.felix.swing");
    let _ = level.play_sound(host, "minecraft:block.smoker.smoke", [at[0], at[1] + 1.0, at[2]], SoundSource::Blocks, 0.7, 1.0);
    crate::talk::queue(format!("execute in {} run particle minecraft:campfire_cosy_smoke {:.2} {:.2} {:.2} 0.2 0.3 0.2 0.01 3", level.dimension_id(), pos[0], pos[1] + 1.2, pos[2]));
    if may_say(me.id(), now, 2400) {
        crate::heart::say_aloud(host, me, pick(market::BAKER, now / 20));
    }
}

/// Whether NPC `a` and `b` are family (they share food for nothing).
fn kin(host: &mut dyn ServerHost, a: EntityRef, b: EntityRef) -> bool {
    let Some(ub) = b.uuid(host) else { return false };
    let f = crate::families::FAMILY.get(host, Holder::Entity(a.id())).unwrap_or_default();
    f.partner.is_some_and(|p| p.0 == ub) || f.children.iter().any(|c| c.0 == ub) || f.parents.iter().any(|c| c.0 == ub)
}

/// Moves `n` of `item` from one stall's chest to another: how many moved.
fn carry(host: &mut dyn ServerHost, from: (LevelRef, [i32; 3]), to: (LevelRef, [i32; 3]), item: &str, n: u64) -> u64 {
    if !crate::jobs::take(host, from, item, n as u32) {
        return 0;
    }
    let went = u64::from(crate::jobs::put_item(host, to, item, n as u32));
    if went < n {
        crate::jobs::put_item(host, from, item, (n - went) as u32);
    }
    went
}

/// A baker short of what it bakes from buys some from a farmer or fisher.
fn baker_buys(host: &mut dyn ServerHost, me: Stall, stalls: &[Stall], people: u64, now: i64) {
    let have: Vec<(&str, u64)> = market::RECIPES.iter().map(|r| (r.0, crate::jobs::amount(host, me.chest, r.0))).collect();
    if market::bake(&|i| have.iter().find(|h| h.0 == i).map_or(0, |h| h.1)).is_some() {
        return;
    }
    let my_name = name_of(&*host, me.npc);
    for (item, per, _) in market::RECIPES {
        let sellers: Vec<Stall> = stalls.iter().copied().filter(|s| s.npc.id() != me.npc.id() && matches!(s.job, Job::Farm | Job::Fish)).collect();
        for s in sellers {
            let avail = crate::jobs::amount(host, s.chest, item).saturating_sub(market::KEEP);
            let mut n = avail.min(market::LOT);
            n -= n % u64::from(*per);
            if n == 0 {
                continue;
            }
            let supply = market::supply(stock(host, stalls, item), people);
            let free = kin(host, s.npc, me.npc);
            let cost = if free { 0 } else { market::cost(item, n, supply) };
            if purse_of(host, me.npc) < cost {
                if may_say(me.npc.id(), now, 2400) {
                    crate::heart::say_aloud(host, me.npc, pick(market::BROKE, now / 20));
                }
                return;
            }
            let moved = carry(host, s.chest, me.chest, item, n);
            if moved == 0 {
                continue;
            }
            spend(host, me.npc, cost);
            earn(host, s.npc, cost);
            let seller = name_of(&*host, s.npc);
            crate::heart::say_aloud(host, me.npc, &format!("I'll take {moved} {}, {seller}.", market::short(item)));
            let reply = if free { format!("For you, {my_name}? No charge.") } else { format!("{cost} emerald{}, thank you, {my_name}!", if cost == 1 { "" } else { "s" }) };
            crate::heart::say_aloud(host, s.npc, &reply);
            return;
        }
    }
    if may_say(me.npc.id(), now, 3000) {
        crate::heart::say_aloud(host, me.npc, pick(market::NO_FLOUR, now / 20));
    }
}

/// A hungry NPC with an empty pantry buys a meal from a stall that has food.
fn buy_meal(host: &mut dyn ServerHost, me: EntityRef, stalls: &[Stall], people: u64, now: i64) {
    if let Some(own) = crate::jobs::chest_of(host, me).flatten()
        && FOODS.iter().any(|f| crate::jobs::amount(host, own, f.0) > 0)
    {
        return;
    }
    let my_name = name_of(&*host, me);
    for (item, points) in FOODS.iter().filter(|f| f.1 >= 3) {
        for s in stalls.iter().filter(|s| s.npc.id() != me.id()) {
            let n = crate::jobs::amount(host, s.chest, item).min(2);
            if n == 0 {
                continue;
            }
            let supply = market::supply(stock(host, stalls, item), people);
            let free = kin(host, s.npc, me);
            let cost = if free { 0 } else { market::cost(item, n, supply) };
            if purse_of(host, me) < cost {
                if may_say(me.id(), now, 2400) {
                    crate::heart::say_aloud(host, me, "I'm hungry, and I haven't a coin for food.");
                }
                return;
            }
            if !crate::jobs::take(host, s.chest, item, n as u32) {
                continue;
            }
            spend(host, me, cost);
            earn(host, s.npc, cost);
            crate::needs::eat(host, me, (*points as u64 * n).min(20) as u8);
            let seller = name_of(&*host, s.npc);
            let what = if n == 1 { market::short(item) } else { format!("{n} {}", market::short(item)) };
            crate::heart::say_aloud(host, me, &format!("{what}, please, {seller}."));
            let reply = if free { format!("Here, {my_name}. Eat up.") } else { format!("That's {cost}, {my_name}. Enjoy!") };
            crate::heart::say_aloud(host, s.npc, &reply);
            return;
        }
    }
}

fn players(host: &dyn mod_api::Host) -> Vec<PlayerRef> {
    ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"]
        .iter()
        .filter_map(|d| host.level(d))
        .flat_map(|l| l.online_players(host).unwrap_or_default())
        .collect()
}

/// A player near a village short of food hears of it now and then.
fn talk_of_shortage(host: &mut dyn ServerHost, now: i64) {
    for p in players(&*host) {
        let heard = STATE.lock().ok().and_then(|s| s.heard.get(&p.uuid()).copied()).unwrap_or(i64::MIN / 2);
        if now - heard < 6000 && now >= heard {
            continue;
        }
        let Some(s) = food_about(host, p) else { continue };
        if s > Supply::Short {
            continue;
        }
        let Some(at) = p.entity().position(&*host) else { continue };
        let level = p.entity().level(&*host);
        let speaker = companion::npcs()
            .into_iter()
            .filter_map(|id| host.entity(id))
            .filter(|e| e.level(&*host) == level && !crate::routines::asleep(e.id()))
            .filter_map(|e| Some((dist(e.position(&*host)?, at), e)))
            .filter(|(d, _)| *d <= 16.0)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, e)) = speaker {
            crate::heart::say_aloud(host, e, pick(market::SHORT_FOOD, now / 20 + i64::from(e.id())));
            if let Ok(mut st) = STATE.lock() {
                st.heard.insert(p.uuid(), now);
            }
        }
    }
}

/// The server tick: wages, bakers buying, hungry NPCs buying meals, talk of shortages.
pub fn tick(host: &mut dyn ServerHost) {
    let step = STATE.lock().map(|mut s| {
        s.ticks = s.ticks.wrapping_add(1);
        s.ticks % ROUND == 0
    });
    if !matches!(step, Ok(true)) {
        return;
    }
    let npcs: Vec<EntityRef> = companion::npcs().into_iter().filter_map(|id| host.entity(id)).collect();
    for me in &npcs {
        let (Some(level), Some(at)) = (me.level(&*host), me.position(&*host)) else { continue };
        let (Some(now), Some(t)) = (level.game_time(&*host), level.day_time(&*host)) else { continue };
        let day = t.div_euclid(24000);
        let job = crate::jobs::job_of(host, *me);
        // A day's wage for those who work.
        let purse = PURSE.get(host, Holder::Entity(me.id())).unwrap_or_default();
        if job.is_some() && purse.day != day {
            let pay = if purse.day < 0 { 0 } else { market::WAGE };
            set_purse(host, *me, purse.coins + pay, Some(day));
        }
        let hungry = crate::needs::food_of(me.id()) <= HUNGRY;
        if job != Some(Job::Bake) && !hungry {
            continue;
        }
        let (stalls, people) = village(host, level, at);
        if job == Some(Job::Bake)
            && let Some(mine) = stalls.iter().copied().find(|s| s.npc.id() == me.id())
        {
            baker_buys(host, mine, &stalls, people, now);
        }
        if hungry {
            buy_meal(host, *me, &stalls, people, now);
        }
    }
    if let Some(now) = npcs.first().and_then(|e| e.level(&*host)).and_then(|l| l.game_time(&*host)) {
        talk_of_shortage(host, now);
    }
    let ids: Vec<u32> = npcs.iter().map(|e| e.id()).collect();
    if let Ok(mut s) = STATE.lock() {
        s.oven.retain(|id, _| ids.contains(id));
        s.said.retain(|id, _| ids.contains(id));
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix market", 2));
    e.commands.push(Command::op("felix purse <targets:entities> <coins:integer>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let line = match lit.as_slice() {
        ["felix", "market"] => {
            let mod_api::CommandSource::Player { player, .. } = e.source else { return Flow::Cancel };
            let p = *player;
            let (Some(level), Some(at)) = (p.entity().level(&*host), p.entity().position(&*host)) else { return Flow::Cancel };
            let (stalls, people) = village(host, level, at);
            if stalls.is_empty() {
                "No village about: no NPC with a job and a chest within 64 blocks.".to_string()
            } else {
                let mut goods: Vec<String> = Vec::new();
                for item in ["minecraft:wheat", "minecraft:potato", "minecraft:carrot", "minecraft:bread", "minecraft:baked_potato", "minecraft:cod", "minecraft:cooked_cod", "minecraft:oak_log"] {
                    let n = stock(host, &stalls, item);
                    if n > 0 {
                        goods.push(format!("{} {n} ({})", market::short(item), market::supply(n, people).word()));
                    }
                }
                let food = food_stock(host, &stalls);
                let purses: Vec<String> = stalls.iter().map(|s| (name_of(&*host, s.npc), s.job.name(), s.npc)).collect::<Vec<_>>().into_iter().map(|(n, j, e)| format!("{n} ({j}) {}", purse_of(host, e))).collect();
                format!("Village of {people}: food {} ({food}). Goods: {}. Purses: {}", market::supply(food, people).word(), if goods.is_empty() { "none".into() } else { goods.join(", ") }, purses.join(", "))
            }
        }
        ["felix", "purse"] => {
            let coins = e.command.arguments.iter().find(|a| a.name == "coins").and_then(|a| match a.value {
                CommandArgumentValue::Integer(v) => Some(v.max(0) as u32),
                _ => None,
            });
            let npcs: Vec<EntityRef> = e
                .command
                .arguments
                .iter()
                .find(|a| a.name == "targets")
                .map(|a| match &a.value {
                    CommandArgumentValue::Entities(list) => list.clone(),
                    CommandArgumentValue::Entity(one) => vec![*one],
                    _ => Vec::new(),
                })
                .unwrap_or_default()
                .into_iter()
                .filter(|t| companion::is_npc(&*host, *t))
                .collect();
            for n in &npcs {
                set_purse(host, *n, coins.unwrap_or(0), None);
            }
            format!("{} purses set to {}", npcs.len(), coins.unwrap_or(0))
        }
        _ => return Flow::Continue,
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
