//! Ball games on the server: a match between a player and an NPC, its ticks, the player's keys,
//! and what everyone sees (the `projectfelix:sport` attachment: the match as JSON, whose it is,
//! and which way the NPC faces). The player is put in their place, facing the NPC; walking off
//! gives the match away.
use crate::companion::{self, Mode, ORDERS};
use crate::sports::{Event, Key, Match, Side, Sport};
use crate::sync::{ACTION, Action as Play};
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, Compound, EntityRef, Holder, IPayloadContext, IPayloadHandler,
    PayloadBytes, PayloadId, PlayerRef, Registrar, ServerHost, Tag, Text,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

pub const KEY: &str = "projectfelix:sport_key";
/// Ticks the court stays after a match.
const LINGER: i64 = 60;

/// What clients get of a match.
#[derive(Clone, Debug, PartialEq)]
pub struct Court {
    pub json: String,
    /// The player's UUID, as 32 hex digits.
    pub player: String,
    pub yaw: f32,
}

pub static SPORT: AttachmentType<Option<Court>> = AttachmentType {
    namespace: "projectfelix",
    name: "sport",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |c| {
            let mut t = Compound::new();
            if let Some(c) = c {
                t.insert("json".into(), Tag::String(c.json.as_str().into()));
                t.insert("player".into(), Tag::String(c.player.as_str().into()));
                t.insert("yaw".into(), Tag::Float(c.yaw.to_bits()));
            }
            Ok(t)
        },
        decode: |t: &Compound| {
            let s = |k: &str| match t.get(&k.into()) {
                Some(Tag::String(v)) => Some(v.to_string_lossy()),
                _ => None,
            };
            let (Some(json), Some(player)) = (s("json"), s("player")) else { return Ok(None) };
            let yaw = match t.get(&"yaw".into()) {
                Some(Tag::Float(b)) => f32::from_bits(*b),
                Some(_) => return Err(AttachmentError::Decode),
                None => 0.0,
            };
            Ok(Some(Court { json, player, yaw }))
        },
    }),
    copy_on_death: false,
    sync: true,
};

struct Running {
    m: Match,
    player: PlayerRef,
    restore: Mode,
    yaw: f32,
    sent: String,
    ended: Option<i64>,
}

static MATCHES: LazyLock<Mutex<HashMap<u32, Running>>> = LazyLock::new(Default::default);

struct KeyHandler;

impl IPayloadHandler for KeyHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        let b = &payload.0;
        if b.len() != 5 {
            return Ok(());
        }
        let id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let key = match b[4] {
            0 => Key::Left,
            1 => Key::Right,
            _ => Key::Smash,
        };
        let Some(npc) = host.entity(id) else { return Ok(()) };
        let now = now_of(&*host, npc);
        let events = {
            let Ok(mut all) = MATCHES.lock() else { return Ok(()) };
            let Some(r) = all.get_mut(&id).filter(|r| r.player.uuid() == player.uuid()) else { return Ok(()) };
            r.m.press(key, now)
        };
        react(host, npc, events, now);
        sync(host, npc);
        Ok(())
    }
}

pub fn register(r: &mut dyn Registrar) {
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_server(PayloadId(KEY.into()), Arc::new(KeyHandler));
}

fn now_of(host: &dyn mod_api::Host, npc: EntityRef) -> i64 {
    npc.level(host).and_then(|l| l.game_time(host)).unwrap_or(0)
}

pub fn busy(npc: u32) -> bool {
    MATCHES.lock().is_ok_and(|m| m.contains_key(&npc))
}

/// Starts `sport` between `player` and `npc`: the NPC faces the player, who is put in their
/// place facing it.
pub fn start(host: &mut dyn ServerHost, npc: EntityRef, sport: Sport, player: PlayerRef) -> Result<(), String> {
    if busy(npc.id()) {
        return Err("a match is on here already".into());
    }
    let at = npc.position(&*host).ok_or("no NPC")?;
    let p = player.entity().position(&*host).ok_or("no player")?;
    let yaw = crate::scene::yaw_towards(at, p);
    let now = now_of(&*host, npc);
    let restore = ORDERS.get(host, Holder::Entity(npc.id())).map(|o| o.mode).unwrap_or_default();
    companion::set_orders(host, npc, Mode::Wait);
    // The player's place: `distance` ahead of the NPC as it faces, turned back towards it.
    let r = yaw.to_radians();
    let d = sport.distance() as f64;
    let spot = [at[0] - r.sin() as f64 * d, at[1], at[2] + r.cos() as f64 * d];
    let name = player.name(&*host).unwrap_or_default();
    crate::talk::queue(format!("tp {name} {:.2} {:.2} {:.2} {:.1} 8", spot[0], spot[1], spot[2], yaw + 180.0));
    let seed = now as u64 ^ (npc.id() as u64) << 20 ^ 0x9E37_79B9_7F4A_7C15;
    let m = Match::new(sport, now, seed);
    if let Ok(mut all) = MATCHES.lock() {
        all.insert(npc.id(), Running { m, player, restore, yaw, sent: String::new(), ended: None });
    }
    let _ = player.action_bar(host, Text::literal("Keys: Z left, C right, X smash or shoot (Options > Controls)"));
    sync(host, npc);
    Ok(())
}

/// The level tick: every match goes on; a player who walked off gives it away.
pub fn level_tick(host: &mut dyn ServerHost) {
    let ids: Vec<u32> = MATCHES.lock().map(|m| m.keys().copied().collect()).unwrap_or_default();
    for id in ids {
        let Some(npc) = host.entity(id) else { continue };
        let now = now_of(&*host, npc);
        let (player, sport) = match MATCHES.lock().ok().and_then(|m| m.get(&id).map(|r| (r.player, r.m.sport))) {
            Some(x) => x,
            None => continue,
        };
        // Gone, or far from the court: the match is lost.
        let near = match (player.entity().position(&*host), npc.position(&*host)) {
            (Some(p), Some(n)) => {
                let d2 = (p[0] - n[0]).powi(2) + (p[2] - n[2]).powi(2);
                d2 < ((sport.distance() + 8.0) as f64).powi(2)
            }
            _ => false,
        };
        let mut events = Vec::new();
        let mut finish = None;
        if let Ok(mut all) = MATCHES.lock()
            && let Some(r) = all.get_mut(&id)
        {
            if !near && !r.m.over() {
                r.m.score[1] = r.m.score[1].max(r.m.score[0] + 1);
                r.m.phase = crate::sports::Phase::Over { winner: Some(Side::Npc), at: now };
                events.push(Event::Over(Some(Side::Npc)));
            } else {
                events = r.m.tick(now);
            }
            if r.m.over() && r.ended.is_none() {
                r.ended = Some(now);
            }
            if r.ended.is_some_and(|e| now - e >= LINGER) {
                let r = all.remove(&id).unwrap();
                finish = Some(r.restore);
            }
        }
        react(host, npc, events, now);
        if let Some(restore) = finish {
            let _ = SPORT.set(host, Holder::Entity(id), None);
            companion::set_orders(host, npc, restore);
        } else {
            sync(host, npc);
        }
    }
}

fn sync(host: &mut dyn ServerHost, npc: EntityRef) {
    let court = {
        let Ok(mut all) = MATCHES.lock() else { return };
        let Some(r) = all.get_mut(&npc.id()) else { return };
        let json = r.m.to_json().to_string();
        if json == r.sent {
            return;
        }
        r.sent = json.clone();
        Court { json, player: format!("{:032x}", r.player.uuid()), yaw: r.yaw }
    };
    let _ = SPORT.set(host, Holder::Entity(npc.id()), Some(court));
}

/// Sounds, the NPC's moves and the result, for what happened.
fn react(host: &mut dyn ServerHost, npc: EntityRef, events: Vec<Event>, now: i64) {
    if events.is_empty() {
        return;
    }
    let (sport, player, score) = match MATCHES.lock().ok().and_then(|m| m.get(&npc.id()).map(|r| (r.m.sport, r.player, r.m.score))) {
        Some(x) => x,
        None => return,
    };
    let uuid = npc.uuid(host).map(crate::talk::uuid_text).unwrap_or_default();
    let sound = |s: &str, pitch: f32| crate::talk::queue(format!("execute as {uuid} at @s run playsound {s} neutral @a ~ ~1 ~ 1 {pitch}"));
    let anim = |host: &mut dyn ServerHost, name: &str| {
        let _ = ACTION.set(host, Holder::Entity(npc.id()), Some(Play { animation: name.into(), start: now }));
    };
    for e in events {
        match e {
            Event::Swing(Side::Npc) => {
                anim(host, if sport == Sport::Basketball { "animation.felix.shoot" } else { "animation.felix.swing" });
                sound(if sport == Sport::Basketball { "minecraft:entity.player.attack.nodamage" } else { "minecraft:entity.player.attack.weak" }, 1.4);
            }
            Event::Swing(Side::Player) => sound("minecraft:entity.player.attack.weak", 1.6),
            Event::Point(Some(Side::Player)) => sound("minecraft:entity.experience_orb.pickup", 1.0),
            Event::Point(_) => sound("minecraft:block.note_block.bass", 0.8),
            Event::Basket(_) => sound("minecraft:block.note_block.chime", 1.2),
            Event::Miss(_) => sound("minecraft:block.wood.hit", 0.8),
            Event::Over(winner) => {
                let text = match winner {
                    Some(Side::Player) => format!("You win, {} to {}!", score[0], score[1]),
                    _ => format!("The NPC wins, {} to {}.", score[1], score[0]),
                };
                let _ = player.action_bar(host, Text::literal(text));
                anim(host, if winner == Some(Side::Npc) { "animation.felix.cheer" } else { "animation.felix.wave" });
                sound(if winner == Some(Side::Player) { "minecraft:ui.toast.challenge_complete" } else { "minecraft:entity.villager.celebrate" }, 1.0);
            }
        }
    }
}
