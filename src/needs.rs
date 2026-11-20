//! Needs on the server (see `need`): every NPC's food runs down; it eats from its pantry, says
//! when it is hungry, and players can feed it. `/felix needs <npc>`, `/felix needs set`.
use crate::companion;
use crate::need::{self, EATS_AT, FOODS, FULL, HUNGRY};
use mod_api::{
    AttachmentCodec, AttachmentType, Command, CommandArgumentValue, Compound, EntityRef, Flow, Hand, Holder, LevelRef,
    LivingRef, PlayerRef, RegisterCommandsEvent, RunModCommandEvent, ServerHost, SoundSource, Tag, Text,
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// How fed an NPC is, and when that was counted (game time; 0: not yet).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fed {
    pub food: u8,
    pub last: i64,
}

impl Default for Fed {
    fn default() -> Fed {
        Fed { food: FULL, last: 0 }
    }
}

pub static NEED: AttachmentType<Fed> = AttachmentType {
    namespace: "projectfelix",
    name: "need",
    default: Fed::default,
    codec: Some(AttachmentCodec {
        encode: |f| {
            let mut c = Compound::new();
            c.insert("food".into(), Tag::Byte(f.food as i8));
            c.insert("last".into(), Tag::Long(f.last));
            Ok(c)
        },
        decode: |c: &Compound| {
            let food = match c.get(&"food".into()) {
                Some(Tag::Byte(b)) => (*b as u8).min(FULL),
                _ => FULL,
            };
            let last = match c.get(&"last".into()) {
                Some(Tag::Long(t)) => *t,
                _ => 0,
            };
            Ok(Fed { food, last })
        },
    }),
    copy_on_death: false,
    sync: false,
};

#[derive(Default)]
struct State {
    /// Each NPC's food as last counted, and when it last said it was hungry.
    food: HashMap<u32, u8>,
    said: HashMap<u32, i64>,
    /// Each NPC's chest by its home as last looked for (none: there is none), and when.
    pantry: HashMap<u32, (Option<[i32; 3]>, i64)>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// The server stopped: nothing of its world is kept.
pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
}

/// NPC `id`'s food as last counted.
pub fn food_of(id: u32) -> u8 {
    STATE.lock().ok().and_then(|s| s.food.get(&id).copied()).unwrap_or(FULL)
}

/// How much NPC `id`'s food lets it obey.
pub fn obey_factor(id: u32) -> f32 {
    need::obey_factor(food_of(id))
}

/// Where NPC `npc` keeps its food: its job's chest, else a chest by its home.
fn pantry(host: &mut dyn ServerHost, npc: EntityRef) -> Option<(LevelRef, [i32; 3])> {
    if let Some(c) = crate::jobs::chest_of(host, npc).flatten() {
        return Some(c);
    }
    let level = npc.level(&*host)?;
    // Looked for once a minute, found or not.
    let now = level.game_time(&*host).unwrap_or(0);
    let known = STATE.lock().ok().and_then(|s| s.pantry.get(&npc.id()).copied()).filter(|(_, at)| now >= *at && now - at < 1200);
    let chest = match known {
        Some((c, _)) => c,
        None => {
            let c = crate::routines::place_named(host, npc, "home").and_then(|home| crate::jobs::chest_near(&*host, level, home, 4));
            if let Ok(mut s) = STATE.lock() {
                s.pantry.insert(npc.id(), (c, now));
            }
            c
        }
    };
    chest.map(|c| (level, c))
}

/// NPC `npc` eats something worth `points`.
pub fn eat(host: &mut dyn ServerHost, npc: EntityRef, points: u8) {
    let now = npc.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
    let fed = NEED.get(host, Holder::Entity(npc.id())).unwrap_or_default();
    let (food, last) = need::after(fed.food, if fed.last == 0 { now } else { fed.last }, now);
    let food = (food + points).min(FULL);
    let _ = NEED.set(host, Holder::Entity(npc.id()), Fed { food, last });
    if let Ok(mut s) = STATE.lock() {
        s.food.insert(npc.id(), food);
    }
    let _ = LivingRef::new(npc).swing(host, Hand::Main, false);
    if let (Some(level), Some(at)) = (npc.level(&*host), npc.position(&*host)) {
        let _ = level.play_sound(host, "minecraft:entity.generic.eat", [at[0], at[1] + 1.4, at[2]], SoundSource::Neutral, 0.7, 1.0);
    }
}

/// NPC `npc` eats the best thing in its pantry; false if there is nothing.
pub fn eat_from_pantry(host: &mut dyn ServerHost, npc: EntityRef) -> bool {
    let Some(p) = pantry(host, npc) else { return false };
    let Some((item, points)) = FOODS.iter().find(|f| crate::jobs::amount(host, p, f.0) > 0).copied() else { return false };
    if !crate::jobs::take(host, p, item, 1) {
        return false;
    }
    eat(host, npc, points);
    crate::minds::ate(host, npc, item);
    true
}

/// A player feeds NPC `npc` from their own food: the best they have.
pub fn feed(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) {
    let name = player.name(&*host).unwrap_or_default();
    let mut have: HashMap<String, u32> = HashMap::new();
    for stack in player.inventory(&*host).unwrap_or_default() {
        *have.entry(stack.item().id(&*host)).or_insert(0) += stack.count();
    }
    let Some((item, points)) = FOODS.iter().find(|f| have.get(f.0).is_some_and(|n| *n > 0)) else {
        crate::heart::say_to(host, npc, player, &format!("You have nothing to eat either, {name}."));
        return;
    };
    crate::talk::queue(format!("clear {name} {item} 1"));
    eat(host, npc, *points);
    crate::minds::ate(host, npc, item);
    crate::heart::say_to(host, npc, player, "Mmm. That hit the spot.");
    crate::heart::feel(host, npc, Some(player), "gift");
}

/// The server tick: food runs down, NPCs eat from their pantries and say when they are hungry.
pub fn tick(host: &mut dyn ServerHost) {
    // Each level at its own pace (its NPCs are the ones found while it ticks).
    if !crate::pace::every("needs", 100) {
        return;
    }
    let npcs = companion::npcs();
    for id in &npcs {
        let Some(npc) = host.entity(*id) else { continue };
        let Some(now) = npc.level(&*host).and_then(|l| l.game_time(&*host)) else { continue };
        let fed = NEED.get(host, Holder::Entity(*id)).unwrap_or_default();
        let (mut food, last) = need::after(fed.food, if fed.last == 0 { now } else { fed.last }, now);
        // Hungry enough to eat: from its pantry, until it has had enough (after days away, all it
        // missed).
        if food <= EATS_AT
            && let Some(p) = pantry(host, npc)
        {
            for _ in 0..8 {
                let Some((item, points)) = FOODS.iter().find(|f| crate::jobs::amount(host, p, f.0) > 0).copied() else { break };
                if !crate::jobs::take(host, p, item, 1) {
                    break;
                }
                food = (food + points).min(FULL);
                eat_effects(host, npc);
                crate::minds::ate(host, npc, item);
                if food > EATS_AT {
                    break;
                }
            }
        }
        if food != fed.food || last != fed.last {
            let _ = NEED.set(host, Holder::Entity(*id), Fed { food, last });
        }
        let said = STATE.lock().ok().and_then(|s| s.said.get(id).copied()).unwrap_or(i64::MIN / 2);
        if food <= HUNGRY && (now - said >= 3000 || now < said) {
            let lines = if food == 0 { need::STARVING_LINES } else { need::HUNGRY_LINES };
            crate::heart::say_aloud(host, npc, lines[(now as usize / 100 + *id as usize) % lines.len()]);
            // Hungry since the last look, or starving: it takes it badly.
            if fed.food > HUNGRY || food == 0 {
                crate::heart::feel(host, npc, None, "hungry");
            }
            if let Ok(mut s) = STATE.lock() {
                s.said.insert(*id, now);
            }
        }
        if let Ok(mut s) = STATE.lock() {
            s.food.insert(*id, food);
        }
    }
    if let Ok(mut s) = STATE.lock() {
        s.food.retain(|id, _| npcs.contains(id));
        s.said.retain(|id, _| npcs.contains(id));
        s.pantry.retain(|id, _| npcs.contains(id));
    }
}

fn eat_effects(host: &mut dyn ServerHost, npc: EntityRef) {
    let _ = LivingRef::new(npc).swing(host, Hand::Main, false);
    if let (Some(level), Some(at)) = (npc.level(&*host), npc.position(&*host)) {
        let _ = level.play_sound(host, "minecraft:entity.generic.eat", [at[0], at[1] + 1.4, at[2]], SoundSource::Neutral, 0.7, 1.0);
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix needs <targets:entity>", 2));
    e.commands.push(Command::op("felix needs set <targets:entities> <food:integer>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let line = match lit.as_slice() {
        ["felix", "needs"] => {
            let Some(npc) = companion::targets(e).into_iter().find(|t| companion::is_npc(&*host, *t)) else {
                let _ = host.commands().feedback(Text::literal("That is not an NPC."));
                return Flow::Cancel;
            };
            let food = food_of(npc.id());
            let name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
            let word = need::word(food);
            format!("{name}: food {food}/{FULL}{}", if word.is_empty() { String::new() } else { format!(" ({word})") })
        }
        ["felix", "needs", "set"] => {
            let food = e.command.arguments.iter().find(|a| a.name == "food").and_then(|a| match a.value {
                CommandArgumentValue::Integer(v) => Some(v.clamp(0, i32::from(FULL)) as u8),
                _ => None,
            });
            let Some(food) = food else { return Flow::Cancel };
            let npcs: Vec<EntityRef> = companion::targets(e).into_iter().filter(|t| companion::is_npc(&*host, *t)).collect();
            for n in &npcs {
                let now = n.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
                let _ = NEED.set(host, Holder::Entity(n.id()), Fed { food, last: now });
                if let Ok(mut s) = STATE.lock() {
                    s.food.insert(n.id(), food);
                    s.said.remove(&n.id());
                }
            }
            format!("{} NPCs now have food {food}/{FULL}", npcs.len())
        }
        _ => return Flow::Continue,
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
