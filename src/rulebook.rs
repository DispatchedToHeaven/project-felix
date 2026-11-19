//! Rules at work on the server (see `rules`): each NPC with rules looks around twice a second,
//! notices day and night, rain and its health, hears chat near it, and feels hits; each rule whose
//! event comes runs its `then` or its `else`.
//!
//! An NPC follows the rules `/felix rules <targets> <id>` gives it (`data/<ns>/felix_rules/`), or
//! those of its file when it was made in the game (`felixnpc:<id>`).
use crate::companion::{self, Mode};
use crate::rules::{self, Do, Rule, When};
use mod_api::{
    AddServerReloadListenersEvent, AttachmentCodec, AttachmentError, AttachmentType, Command, Compound, EntityRef,
    FileToIdConverter, Flow, Holder, Identifier, LivingRef, MobCategory, MobRef, PlayerRef, RegisterCommandsEvent,
    ReloadApply, ReloadResourceError, RunModCommandEvent, ServerHost, ServerReloadContext, ServerReloadListener, Tag,
    Text,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

/// Ticks between looks around.
const SCAN: i64 = 10;
/// How far `say` carries.
const VOICE: f64 = 24.0;

/// The rules an NPC follows; none: those of its file, if it was made in the game.
pub static RULES: AttachmentType<Option<String>> = AttachmentType {
    namespace: "projectfelix",
    name: "rules",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |d| {
            let mut c = Compound::new();
            if let Some(d) = d {
                c.insert("id".into(), Tag::String(d.as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| match c.get(&"id".into()) {
            Some(Tag::String(s)) => Ok(Some(s.to_string_lossy())),
            None => Ok(None),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

static LOADED: LazyLock<Mutex<HashMap<String, Arc<Vec<Rule>>>>> = LazyLock::new(Default::default);
/// Rules of NPCs made in the game, kept across data reloads.
static EXTRA: LazyLock<Mutex<HashMap<String, Arc<Vec<Rule>>>>> = LazyLock::new(Default::default);

pub fn set_extra(name: &str, r: Vec<Rule>) {
    if let Ok(mut e) = EXTRA.lock() {
        e.insert(name.to_string(), Arc::new(r));
    }
}

fn rules_named(id: &str) -> Option<Arc<Vec<Rule>>> {
    EXTRA.lock().ok().and_then(|e| e.get(id).cloned()).or_else(|| LOADED.lock().ok().and_then(|l| l.get(id).cloned()))
}

/// Something that happened, kept for the next look around.
enum Heard {
    Hurt { victim: u32, by: u32 },
    Killed { killer: u32, kind: String, category: &'static str, name: String },
    Chat { player: u32, text: String },
    /// What its character felt (`friend_hurt`, `friend_killed`, `mood:<name>`), about whom.
    Felt { npc: u32, what: String, other: Option<u32> },
}

/// What an NPC remembers between looks.
#[derive(Default)]
struct Mind {
    rules_id: Option<String>,
    fired: HashMap<usize, i64>,
    seen: HashMap<usize, HashSet<u32>>,
    day: Option<bool>,
    rain: Option<bool>,
    low: Option<bool>,
}

#[derive(Default)]
struct State {
    last: i64,
    minds: HashMap<u32, Mind>,
    heard: Vec<Heard>,
    seed: u64,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

fn hear(h: Heard) {
    if let Ok(mut s) = STATE.lock()
        && s.heard.len() < 256
    {
        s.heard.push(h);
    }
}

/// `living_damage_post`: who was hurt, and by whom.
pub fn on_hurt(victim: i32, by: i32) {
    if victim > 0 {
        hear(Heard::Hurt { victim: victim as u32, by: by.max(0) as u32 });
    }
}

/// `living_death`: what an NPC killed (the victim is gone by the next look).
pub fn on_death(host: &dyn mod_api::Host, victim: u32, killer: i32) {
    if killer <= 0 || !STATE.lock().is_ok_and(|s| s.minds.contains_key(&(killer as u32))) {
        return;
    }
    let Some(e) = host.entity(victim) else { return };
    let (kind, category) = kind_of(host, e);
    let name = e.name(host).unwrap_or_else(|| kind.clone());
    hear(Heard::Killed { killer: killer as u32, kind, category, name });
}

/// What NPC `npc`'s character felt (see `heart`), for the rules that wait for it.
pub fn notice(npc: u32, what: &str, other: Option<u32>) {
    hear(Heard::Felt { npc, what: what.to_string(), other });
}

/// `server_chat`: what a player said.
pub fn on_chat(player_entity: i32, text: &str) {
    if player_entity > 0 {
        hear(Heard::Chat { player: player_entity as u32, text: text.to_lowercase() });
    }
}

pub(crate) fn kind_of(host: &dyn mod_api::Host, e: EntityRef) -> (String, &'static str) {
    let kind = e.kind(host).map(|k| k.id(host)).unwrap_or_default();
    let category = if kind == "minecraft:player" {
        "player"
    } else {
        match e.classification(host) {
            Some(MobCategory::Monster) => "monster",
            Some(MobCategory::Creature) => "animal",
            Some(MobCategory::WaterCreature | MobCategory::WaterAmbient | MobCategory::UndergroundWaterCreature | MobCategory::Axolotls) => "water",
            Some(MobCategory::Ambient) => "ambient",
            _ => "other",
        }
    };
    (kind, category)
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

struct Listener;

impl ServerReloadListener for Listener {
    fn prepare(&self, cx: &ServerReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        for id in cx.list() {
            let Some(bytes) = cx.read(id) else { continue };
            match rules::parse(bytes) {
                Ok(r) => {
                    next.insert(id.to_string(), Arc::new(r));
                }
                Err(e) => eprintln!("[projectfelix] rules {id} not loaded: {e}"),
            }
        }
        Ok(<dyn ReloadApply>::new(move || {
            if let Ok(mut live) = LOADED.lock() {
                *live = next;
            }
        }))
    }
}

pub fn add_listener(e: &mut AddServerReloadListenersEvent<'_>) {
    let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:rules"), FileToIdConverter::json("felix_rules")) else {
        return;
    };
    if let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener)) {
        eprintln!("[projectfelix] rules listener not added: {err:?}");
    }
}

// ---- the event at hand ----------------------------------------------------------------------------

/// One event for one NPC: what it is about, and the facts its rules ask.
struct Event {
    target: Option<EntityRef>,
    target_kind: Option<(String, &'static str)>,
    target_name: String,
    player: Option<PlayerRef>,
}

struct Facts<'a> {
    day: bool,
    rain: bool,
    mode: &'static str,
    health: f32,
    owner_distance: Option<f64>,
    owner: Option<u128>,
    random: f32,
    event: &'a Event,
    npc_tags: Vec<String>,
    player_tags: Vec<String>,
    player_items: HashMap<String, u32>,
    mood: String,
    relation: Option<i32>,
    doing: String,
}

impl rules::Facts for Facts<'_> {
    fn mood(&self) -> &str {
        &self.mood
    }
    fn relation(&self) -> Option<i32> {
        self.relation
    }
    fn doing(&self) -> &str {
        &self.doing
    }
    fn is_day(&self) -> bool {
        self.day
    }
    fn raining(&self) -> bool {
        self.rain
    }
    fn mode(&self) -> &str {
        self.mode
    }
    fn health(&self) -> f32 {
        self.health
    }
    fn owner_distance(&self) -> Option<f64> {
        self.owner_distance
    }
    fn player_is_owner(&self) -> bool {
        self.event.player.is_some_and(|p| Some(p.uuid()) == self.owner)
    }
    fn random(&self) -> f32 {
        self.random
    }
    fn target(&self) -> Option<(&str, &str)> {
        self.event.target_kind.as_ref().map(|(k, c)| (k.as_str(), *c))
    }
    fn npc_tag(&self, tag: &str) -> bool {
        self.npc_tags.iter().any(|t| t == tag)
    }
    fn player_tag(&self, tag: &str) -> bool {
        self.player_tags.iter().any(|t| t == tag)
    }
    fn player_items(&self, item: &str) -> u32 {
        self.player_items.get(item).copied().unwrap_or(0)
    }
}

/// The world as an NPC sees it at a look.
struct Here {
    me: EntityRef,
    at: [f64; 3],
    day: bool,
    rain: bool,
    mode: &'static str,
    health: f32,
    owner: Option<u128>,
    owner_at: Option<[f64; 3]>,
    players: Vec<PlayerRef>,
}

fn here(host: &dyn mod_api::Host, me: EntityRef) -> Option<Here> {
    let at = me.position(host)?;
    let level = me.level(host)?;
    let time = level.day_time(host).unwrap_or(0).rem_euclid(24000);
    let living = LivingRef::new(me);
    let health = living.health(host).unwrap_or(1.0) / living.max_health(host).unwrap_or(1.0).max(1.0);
    let owner = living.owner(host);
    let players = level.online_players(host).unwrap_or_default();
    let owner_at = owner.and_then(|o| players.iter().find(|p| p.uuid() == o)).and_then(|p| p.entity().position(host));
    let mode = match companion::mode_of(me.id()) {
        Mode::Follow => "follow",
        Mode::Wait => "wait",
        Mode::Guard => "guard",
        Mode::Live => "live",
    };
    Some(Here { me, at, day: !(12500..23500).contains(&time), rain: level.is_raining(host).unwrap_or(false), mode, health, owner, owner_at, players })
}

fn player_of(h: &Here, e: EntityRef) -> Option<PlayerRef> {
    h.players.iter().copied().find(|p| p.entity().id() == e.id())
}

fn event_about(host: &dyn mod_api::Host, h: &Here, target: Option<EntityRef>) -> Event {
    let target_kind = target.map(|t| kind_of(host, t));
    let target_name = target.and_then(|t| t.name(host)).unwrap_or_default();
    let player = target.and_then(|t| player_of(h, t));
    Event { target, target_kind, target_name, player }
}

/// The rule `i` of `rules` fires, if it has rested long enough.
fn fire(host: &mut dyn ServerHost, h: &Here, rule: &Rule, i: usize, ev: &Event, now: i64, mind: &mut Mind, random: f32) {
    let rest = (rule.cooldown * 20.0) as i64;
    if mind.fired.get(&i).is_some_and(|t| now - t < rest) {
        return;
    }
    mind.fired.insert(i, now);
    let mut player_tags = Vec::new();
    let mut player_items = HashMap::new();
    if let Some(p) = ev.player {
        player_tags = p.entity().tags(&*host).unwrap_or_default();
        for stack in p.inventory(&*host).unwrap_or_default() {
            *player_items.entry(stack.item().id(&*host)).or_insert(0) += stack.count();
        }
    }
    let facts = Facts {
        day: h.day,
        rain: h.rain,
        mode: h.mode,
        health: h.health,
        owner_distance: h.owner_at.map(|o| dist(o, h.at)),
        owner: h.owner,
        random,
        event: ev,
        npc_tags: h.me.tags(&*host).unwrap_or_default(),
        player_tags,
        player_items,
        mood: crate::heart::mood_of(h.me.id()),
        relation: ev.player.map(|p| crate::heart::relation_of(h.me.id(), p.uuid())),
        doing: crate::routines::doing(h.me.id()),
    };
    let dos = rule.decide(&facts).to_vec();
    run(host, h, ev, &dos);
}

fn run(host: &mut dyn ServerHost, h: &Here, ev: &Event, dos: &[Do]) {
    let npc_name = h.me.name(&*host).unwrap_or_else(|| "NPC".into());
    // @player: the event's player; else its owner, if on; else the nearest player.
    let owner = h.owner.and_then(|o| h.players.iter().copied().find(|p| p.uuid() == o));
    let nearest = || {
        h.players
            .iter()
            .filter_map(|p| Some((dist(p.entity().position(&*host)?, h.at), *p)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, p)| p)
    };
    let player_name = ev.player.or(owner).or_else(nearest).and_then(|p| p.name(&*host)).unwrap_or_default();
    let target = if ev.target_name.is_empty() { "something".to_string() } else { ev.target_name.clone() };
    let line = |t: &str| Text::literal(format!("<{npc_name}> {}", rules::fill(t, &player_name, &npc_name, &target)));
    for d in dos {
        match d {
            Do::Say(t) => {
                for p in &h.players {
                    if p.entity().position(&*host).is_some_and(|at| dist(at, h.at) <= VOICE) {
                        let _ = p.tell(host, line(t));
                    }
                }
            }
            Do::Tell(t) => {
                if let Some(p) = ev.player {
                    let _ = p.tell(host, line(t));
                }
            }
            Do::Attack => {
                // Never its owner.
                if let Some(t) = ev.target.filter(|t| ev.player.is_none_or(|p| Some(p.uuid()) != h.owner) && t.id() != h.me.id()) {
                    MobRef::new(h.me).set_target(host, Some(LivingRef::new(t)));
                }
            }
            Do::Look => {
                if let Some(at) = ev.target.and_then(|t| t.position(&*host)) {
                    let _ = MobRef::new(h.me).look_at(host, [at[0], at[1] + 1.4, at[2]]);
                }
            }
            Do::Talk => {
                if let Some(p) = ev.player {
                    crate::talk::start(host, p, h.me.id());
                }
            }
            Do::Act(a) => {
                use crate::dialog::Action;
                // What needs a player waits for one.
                let needs_player = matches!(a, Action::Gear(_) | Action::Shop(_) | Action::Trade(_) | Action::News | Action::Quests(_) | Action::Quest(_) | Action::HandIn | Action::GiveUp | Action::Give(..) | Action::Take(..) | Action::Tag(_) | Action::Untag(_) | Action::Game(..) | Action::Sport(_) | Action::JoinGame | Action::OpenGame | Action::EditAnimation(_));
                if needs_player && ev.player.is_none() {
                    continue;
                }
                let a = match a {
                    Action::Command(c) => Action::Command(c.replace("@target", &target)),
                    other => other.clone(),
                };
                crate::talk::run_for(host, ev.player, h.me, &[a]);
            }
        }
    }
}

// ---- the tick ---------------------------------------------------------------------------------------

/// The server tick: every half second, each NPC with rules looks around and takes what happened.
pub fn tick(host: &mut dyn ServerHost) {
    let npcs = companion::npcs();
    let Some(now) = npcs.iter().find_map(|id| host.entity(*id).and_then(|e| e.level(&*host)).and_then(|l| l.game_time(&*host))) else { return };
    let (heard, seed) = {
        let Ok(mut s) = STATE.lock() else { return };
        if now - s.last < SCAN && now >= s.last {
            return;
        }
        s.last = now;
        s.seed = s.seed.wrapping_mul(6364136223846793005).wrapping_add(now as u64 | 1);
        (std::mem::take(&mut s.heard), s.seed)
    };
    let mut rng = seed;
    let mut random = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 40) as f32 / (1u64 << 24) as f32
    };
    let mut minds = STATE.lock().map(|mut s| std::mem::take(&mut s.minds)).unwrap_or_default();
    minds.retain(|id, _| npcs.contains(id));
    for id in npcs {
        let Some(me) = host.entity(id) else {
            minds.remove(&id);
            continue;
        };
        let mind = minds.entry(id).or_default();
        // Which rules: read again every few seconds (a command may change them).
        if mind.rules_id.is_none() || now % 100 < SCAN {
            let set = RULES.get(host, Holder::Entity(id)).ok().flatten();
            let own = crate::talk::DIALOG.get(host, Holder::Entity(id)).ok().flatten().filter(|d| d.starts_with("felixnpc:"));
            mind.rules_id = Some(set.or(own).unwrap_or_default());
        }
        let Some(rules) = mind.rules_id.as_deref().filter(|r| !r.is_empty()).and_then(rules_named) else { continue };
        let Some(h) = here(&*host, me) else { continue };
        look(host, &h, &rules, mind, now, &heard, &mut random);
    }
    if let Ok(mut s) = STATE.lock() {
        s.minds = minds;
    }
}

fn look(host: &mut dyn ServerHost, h: &Here, rules: &[Rule], mind: &mut Mind, now: i64, heard: &[Heard], random: &mut dyn FnMut() -> f32) {
    let me = h.me.id();
    // Changes since the last look: day and night, rain, health.
    let low = h.health < 1.0 / 3.0;
    let dawn = mind.day == Some(false) && h.day;
    let dusk = mind.day == Some(true) && !h.day;
    let rain = mind.rain == Some(false) && h.rain;
    let hurt_badly = mind.low == Some(false) && low;
    mind.day = Some(h.day);
    mind.rain = Some(h.rain);
    mind.low = Some(low);
    // Who is in sight, for the rules that look.
    let reach = rules.iter().filter(|r| r.when.sight()).map(|r| r.radius).fold(0.0, f64::max);
    let mut around: Vec<(EntityRef, String, &'static str, f64)> = Vec::new();
    if reach > 0.0
        && let Some(level) = h.me.level(&*host)
    {
        let (a, r) = (h.at, reach);
        for e in level.entities_in(&*host, [a[0] - r, a[1] - r / 2.0, a[2] - r], [a[0] + r, a[1] + r / 2.0, a[2] + r]).unwrap_or_default() {
            if e.id() == me {
                continue;
            }
            let Some(at) = e.position(&*host) else { continue };
            let d = dist(at, h.at);
            if d > reach || LivingRef::new(h.me).has_line_of_sight(&*host, e) != Some(true) {
                continue;
            }
            let (kind, category) = kind_of(&*host, e);
            around.push((e, kind, category, d));
        }
        around.sort_by(|x, y| x.3.total_cmp(&y.3));
    }
    for (i, rule) in rules.iter().enumerate() {
        let mut events: Vec<Option<EntityRef>> = Vec::new();
        match &rule.when {
            w if w.sight() => {
                let now_seen: Vec<EntityRef> = around.iter().filter(|(_, k, c, d)| *d <= rule.radius && w.looks_for(k, c)).map(|x| x.0).collect();
                let before = mind.seen.entry(i).or_default();
                // The nearest that came into sight since the last look.
                if let Some(new) = now_seen.iter().find(|e| !before.contains(&e.id())) {
                    events.push(Some(*new));
                }
                *before = now_seen.iter().map(|e| e.id()).collect();
            }
            When::Hurt | When::HurtByPlayer => {
                for hd in heard {
                    if let Heard::Hurt { victim, by } = hd
                        && *victim == me
                    {
                        let by = host.entity(*by);
                        let player = by.is_some_and(|b| player_of(h, b).is_some());
                        if rule.when == When::Hurt || player {
                            events.push(by);
                            break;
                        }
                    }
                }
            }
            When::OwnerHurt => {
                let owner = h.owner.and_then(|o| h.players.iter().find(|p| p.uuid() == o)).map(|p| p.entity().id());
                for hd in heard {
                    if let Heard::Hurt { victim, by } = hd
                        && Some(*victim) == owner
                        && *by != me
                    {
                        events.push(host.entity(*by));
                        break;
                    }
                }
            }
            When::Kills => {
                for hd in heard {
                    if let Heard::Killed { killer, kind, category, name } = hd
                        && *killer == me
                    {
                        let ev = Event { target: None, target_kind: Some((kind.clone(), category)), target_name: name.clone(), player: None };
                        let r = random();
                        fire(host, h, rule, i, &ev, now, mind, r);
                        break;
                    }
                }
            }
            When::Hears(word) => {
                for hd in heard {
                    let Heard::Chat { player, text } = hd else { continue };
                    // A word on its own; words with spaces anywhere in the line.
                    let said = if word.contains(' ') { text.contains(word.as_str()) } else { text.split(|c: char| !c.is_alphanumeric()).any(|w| w == word) };
                    let Some(e) = host.entity(*player).filter(|_| said) else { continue };
                    if e.position(&*host).is_some_and(|at| dist(at, h.at) <= rule.radius) {
                        events.push(Some(e));
                        break;
                    }
                }
            }
            When::Every(s) => {
                let every = ((*s * 20.0) as i64).max(SCAN);
                let last = *mind.fired.entry(i).or_insert(now);
                if now - last >= every {
                    // Every: the cooldown is the period.
                    mind.fired.remove(&i);
                    events.push(None);
                }
            }
            When::FriendHurt | When::FriendKilled | When::Feels(_) | When::HearsRumor | When::QuestDone | When::QuestFailed | When::Starts(_) | When::LevelUp => {
                let key = match &rule.when {
                    When::FriendHurt => "friend_hurt".to_string(),
                    When::FriendKilled => "friend_killed".to_string(),
                    When::Feels(m) => format!("mood:{m}"),
                    When::HearsRumor => "rumor".to_string(),
                    When::QuestDone => "quest_done".to_string(),
                    When::QuestFailed => "quest_failed".to_string(),
                    When::Starts(d) => format!("starts:{d}"),
                    When::LevelUp => "level_up".to_string(),
                    _ => String::new(),
                };
                for hd in heard {
                    if let Heard::Felt { npc, what, other } = hd
                        && *npc == me
                        && *what == key
                    {
                        events.push(other.and_then(|o| host.entity(o)));
                        break;
                    }
                }
            }
            When::Dawn if dawn => events.push(None),
            When::Dusk if dusk => events.push(None),
            When::Rain if rain => events.push(None),
            When::LowHealth if hurt_badly => events.push(None),
            _ => {}
        }
        for target in events {
            let ev = event_about(&*host, h, target);
            let r = random();
            fire(host, h, rule, i, &ev, now, mind, r);
        }
    }
}

// ---- commands ------------------------------------------------------------------------------------------

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix rules <targets:entities> <rules:text>", 2));
    e.commands.push(Command::op("felix rules clear <targets:entities>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let set = match lit.as_slice() {
        ["felix", "rules"] => Some(companion::text_argument(e, "rules").unwrap_or_default().trim().to_string()),
        ["felix", "rules", "clear"] => None,
        _ => return Flow::Continue,
    };
    let mut n = 0;
    let npcs: Vec<EntityRef> = companion::targets(e).into_iter().filter(|t| companion::is_npc(&*host, *t)).collect();
    for t in npcs {
        if RULES.set(host, Holder::Entity(t.id()), set.clone()).is_ok() {
            n += 1;
        }
        if let Ok(mut s) = STATE.lock()
            && let Some(m) = s.minds.get_mut(&t.id())
        {
            m.rules_id = None;
            m.seen.clear();
            m.fired.clear();
        }
    }
    let line = match &set {
        Some(id) if rules_named(id).is_some() => format!("{n} NPCs now follow the rules {id}"),
        Some(id) => format!("{n} NPCs now follow the rules {id} (no such rules are loaded yet)"),
        None => format!("{n} NPCs follow no rules now"),
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
