//! Quests at work on the server (see `quest`): an NPC's quest board in its dialog, each player's
//! quest log (kept with the player), kills counted, places reached, NPCs walked, hand-ins, rewards,
//! and time running out.
//!
//! An NPC gives the quests `/felix quest give` lists for it, those its dialog's `quests` action
//! names, and those of its file when it was made in the game.
use crate::dialog::{Action, Choice, Dialog, Node};
use crate::quest::{self, Active, Goal, Log, Quest};
use mod_api::{
    AddServerReloadListenersEvent, AttachmentCodec, AttachmentError, AttachmentType, Command, Compound, EntityRef,
    FileToIdConverter, Flow, Holder, Identifier, PlayerRef, RegisterCommandsEvent, ReloadApply, ReloadResourceError,
    RunModCommandEvent, ServerHost, ServerReloadContext, ServerReloadListener, Tag, Text,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// Ticks between looks at the players' quests.
const SCAN: i64 = 20;
const DIMENSIONS: [&str; 3] = ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"];

// ---- saved: a player's log, an NPC's quests ------------------------------------------------------------

fn uuid_tag(u: u128) -> Tag {
    Tag::IntArray(vec![(u >> 96) as i32, (u >> 64) as i32, (u >> 32) as i32, u as i32])
}

fn tag_uuid(t: Option<&Tag>) -> Option<u128> {
    match t {
        Some(Tag::IntArray(v)) if v.len() == 4 => Some(v.iter().fold(0u128, |acc, x| (acc << 32) | (*x as u32 as u128))),
        _ => None,
    }
}

fn text_of(c: &Compound, k: &str) -> String {
    match c.get(&k.into()) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    }
}

pub static LOG: AttachmentType<Log> = AttachmentType {
    namespace: "projectfelix",
    name: "quests",
    default: Log::default,
    codec: Some(AttachmentCodec {
        encode: |log| {
            let mut c = Compound::new();
            let active = log
                .active
                .iter()
                .map(|a| {
                    let mut x = Compound::new();
                    x.insert("quest".into(), Tag::String(a.quest.as_str().into()));
                    x.insert("npc".into(), uuid_tag(a.npc));
                    x.insert("npc_name".into(), Tag::String(a.npc_name.as_str().into()));
                    x.insert("progress".into(), Tag::Int(a.progress as i32));
                    if let Some(d) = a.deadline {
                        x.insert("deadline".into(), Tag::Long(d));
                    }
                    x.insert("start".into(), Tag::List(a.start.iter().map(|v| Tag::Double(v.to_bits())).collect()));
                    Tag::Compound(x)
                })
                .collect();
            c.insert("active".into(), Tag::List(active));
            c.insert("done".into(), Tag::List(log.done.iter().map(|d| Tag::String(d.as_str().into())).collect()));
            Ok(c)
        },
        decode: |c: &Compound| {
            let mut log = Log::default();
            if let Some(Tag::List(list)) = c.get(&"active".into()) {
                for t in list {
                    let Tag::Compound(x) = t else { return Err(AttachmentError::Decode) };
                    let Some(npc) = tag_uuid(x.get(&"npc".into())) else { continue };
                    let mut start = [0.0; 3];
                    if let Some(Tag::List(p)) = x.get(&"start".into()) {
                        for (s, v) in start.iter_mut().zip(p) {
                            if let Tag::Double(b) = v {
                                *s = f64::from_bits(*b);
                            }
                        }
                    }
                    log.active.push(Active {
                        quest: text_of(x, "quest"),
                        npc,
                        npc_name: text_of(x, "npc_name"),
                        progress: match x.get(&"progress".into()) {
                            Some(Tag::Int(v)) => (*v).max(0) as u32,
                            _ => 0,
                        },
                        deadline: match x.get(&"deadline".into()) {
                            Some(Tag::Long(v)) => Some(*v),
                            _ => None,
                        },
                        start,
                    });
                }
            }
            if let Some(Tag::List(list)) = c.get(&"done".into()) {
                log.done = list.iter().filter_map(|t| if let Tag::String(s) = t { Some(s.to_string_lossy()) } else { None }).collect();
            }
            Ok(log)
        },
    }),
    copy_on_death: true,
    sync: false,
};

/// The quests an NPC gives, by `/felix quest give` (ids, space between).
pub static GIVES: AttachmentType<Option<String>> = AttachmentType {
    namespace: "projectfelix",
    name: "gives",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |g| {
            let mut c = Compound::new();
            if let Some(g) = g {
                c.insert("quests".into(), Tag::String(g.as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| match c.get(&"quests".into()) {
            Some(Tag::String(s)) => Ok(Some(s.to_string_lossy())),
            None => Ok(None),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

// ---- quests from data packs and NPC files --------------------------------------------------------------

static LOADED: LazyLock<Mutex<HashMap<String, Arc<Quest>>>> = LazyLock::new(Default::default);
/// Quests of NPCs made in the game (`felixnpc:<id>/<n>`), and each one's list by its dialog id.
static EXTRA: LazyLock<Mutex<(HashMap<String, Arc<Quest>>, HashMap<String, Vec<String>>)>> = LazyLock::new(Default::default);

pub fn set_extra(dialog: &str, quests: Vec<Quest>) {
    if let Ok(mut e) = EXTRA.lock() {
        let mut ids = Vec::new();
        for (n, q) in quests.into_iter().enumerate() {
            let id = format!("{dialog}/{n}");
            e.0.insert(id.clone(), Arc::new(q));
            ids.push(id);
        }
        e.1.insert(dialog.to_string(), ids);
    }
}

pub fn quest_named(id: &str) -> Option<Arc<Quest>> {
    EXTRA.lock().ok().and_then(|e| e.0.get(id).cloned()).or_else(|| LOADED.lock().ok().and_then(|l| l.get(id).cloned()))
}

struct Listener;

impl ServerReloadListener for Listener {
    fn prepare(&self, cx: &ServerReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        for id in cx.list() {
            let Some(bytes) = cx.read(id) else { continue };
            match quest::parse(bytes) {
                Ok(q) => {
                    next.insert(id.to_string(), Arc::new(q));
                }
                Err(e) => eprintln!("[projectfelix] quest {id} not loaded: {e}"),
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
    let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:quests"), FileToIdConverter::json("felix_quests")) else {
        return;
    };
    if let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener)) {
        eprintln!("[projectfelix] quests listener not added: {err:?}");
    }
}

// ---- helpers --------------------------------------------------------------------------------------------

fn log_of(host: &mut dyn ServerHost, p: PlayerRef) -> Log {
    LOG.get(host, Holder::Player(p.uuid())).unwrap_or_default()
}

fn set_log(host: &mut dyn ServerHost, p: PlayerRef, log: Log) {
    if let Err(e) = LOG.set(host, Holder::Player(p.uuid()), log) {
        eprintln!("[projectfelix] quest log not saved: {e:?}");
    }
}

fn players(host: &dyn mod_api::Host) -> Vec<PlayerRef> {
    DIMENSIONS.iter().filter_map(|d| host.level(d)).flat_map(|l| l.online_players(host).unwrap_or_default()).collect()
}

fn now_of(host: &dyn mod_api::Host, e: EntityRef) -> i64 {
    e.level(host).and_then(|l| l.game_time(host)).unwrap_or(0)
}

fn fill(text: &str, player: &str, npc: &str) -> String {
    crate::dialog::fill(text, player, npc)
}

/// The quests NPC `npc` gives: by command, by its dialog (`listed`), by its file.
fn offered(host: &mut dyn ServerHost, npc: EntityRef, listed: &[String]) -> Vec<String> {
    let mut ids: Vec<String> = GIVES.get(host, Holder::Entity(npc.id())).ok().flatten().unwrap_or_default().split_whitespace().map(str::to_string).collect();
    ids.extend(listed.iter().cloned());
    if let Some(d) = crate::talk::DIALOG.get(host, Holder::Entity(npc.id())).ok().flatten()
        && let Some(own) = EXTRA.lock().ok().and_then(|e| e.1.get(&d).cloned())
    {
        ids.extend(own);
    }
    let mut seen = std::collections::HashSet::new();
    ids.retain(|i| seen.insert(i.clone()));
    ids
}

/// Where a quest stands, in words: `2/3`, `4 min left`.
fn status(a: &Active, q: &Quest, now: i64, have: Option<u32>) -> String {
    let mut parts = Vec::new();
    match &q.goal {
        Goal::Bring { count, .. } => parts.push(format!("{}/{count}", have.unwrap_or(0).min(*count))),
        Goal::Kill { count, .. } => parts.push(format!("{}/{count}", a.progress.min(*count))),
        _ if a.progress > 0 => parts.push("done".into()),
        _ => {}
    }
    if let Some(d) = a.deadline {
        parts.push(format!("{} min left", ((d - now).max(0) + 1199) / 1200));
    }
    parts.join(", ")
}

fn items_of(host: &mut dyn ServerHost, p: PlayerRef, item: &str) -> u32 {
    p.inventory(&*host).unwrap_or_default().iter().filter(|s| s.item().id(&*host) == item).map(|s| s.count()).sum()
}

// ---- the board ---------------------------------------------------------------------------------------------

fn choice(text: &str, actions: Vec<Action>, goto: Option<&str>) -> Choice {
    Choice { text: text.into(), condition: None, actions, goto: goto.map(str::to_string) }
}

fn node(text: Vec<String>, options: Vec<Choice>) -> Node {
    Node { text, animation: None, actions: Vec::new(), options }
}

/// The `quests` dialog action: what this NPC wants done, or how the player's quest for it goes.
pub fn board(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef, listed: &[String]) {
    let Some(uuid) = npc.uuid(host) else { return };
    let log = log_of(host, player);
    let now = now_of(&*host, npc);
    let mut nodes = HashMap::new();
    if let Some(a) = log.with(uuid) {
        let Some(q) = quest_named(&a.quest) else { return };
        let have = match &q.goal {
            Goal::Bring { item, .. } => Some(items_of(host, player, item)),
            _ => None,
        };
        let st = status(a, &q, now, have);
        let hand = match q.goal {
            Goal::Bring { .. } => "Here you go.",
            _ => "It's done.",
        };
        let mut text = vec![q.progress.clone()];
        if !st.is_empty() {
            text.push(format!("({}: {st})", q.goal.describe()));
        }
        nodes.insert(
            "start".to_string(),
            node(text, vec![choice(hand, vec![Action::HandIn], None), choice("I give up.", vec![Action::GiveUp], None), choice("I'm on it.", Vec::new(), None)]),
        );
    } else {
        let relation = crate::heart::relation_of(npc.id(), player.uuid());
        let open: Vec<(String, Arc<Quest>)> = offered(host, npc, listed)
            .into_iter()
            .filter_map(|id| Some((id.clone(), quest_named(&id)?)))
            .filter(|(id, q)| log.may_take(id, q, uuid) && relation >= q.min_relation)
            .collect();
        if open.is_empty() {
            let why = if log.active.len() >= quest::MAX_ACTIVE { "You have enough on your hands, @player." } else { "Nothing right now, @player. Thanks for asking." };
            nodes.insert("start".to_string(), node(vec![why.into()], vec![choice("All right.", Vec::new(), None)]));
        } else {
            let mut options = Vec::new();
            for (i, (id, q)) in open.iter().enumerate() {
                let name = format!("q{i}");
                options.push(choice(&q.title, Vec::new(), Some(&name)));
                let mut text = vec![q.offer.clone()];
                if let Some(t) = q.time {
                    text.push(format!("(in {t:.0} minutes)"));
                }
                nodes.insert(name, node(text, vec![choice("I'll do it.", vec![Action::Quest(id.clone())], None), choice("Not now.", Vec::new(), Some("start"))]));
            }
            options.push(choice("Not now.", Vec::new(), None));
            nodes.insert("start".to_string(), node(vec!["Well, there is something, @player...".into()], options));
        }
    }
    crate::talk::open(host, player, npc, Arc::new(Dialog { start: "start".into(), nodes }), "start");
}

/// The `quest` action: `player` takes quest `id` from `npc`.
pub fn take(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef, id: &str) {
    let (Some(q), Some(uuid)) = (quest_named(id), npc.uuid(host)) else { return };
    let mut log = log_of(host, player);
    if !log.may_take(id, &q, uuid) || crate::heart::relation_of(npc.id(), player.uuid()) < q.min_relation {
        return;
    }
    let now = now_of(&*host, npc);
    let npc_name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
    let start = npc.position(&*host).unwrap_or([0.0; 3]);
    log.active.push(Active { quest: id.to_string(), npc: uuid, npc_name: npc_name.clone(), progress: 0, deadline: q.time.map(|t| now + (t * 1200.0) as i64), start });
    set_log(host, player, log);
    if matches!(q.goal, Goal::Walk { .. }) {
        // It comes along.
        crate::companion::set_orders(host, npc, crate::companion::Mode::Follow);
    }
    let player_name = player.name(&*host).unwrap_or_default();
    let line = format!("New quest: {} ({})", fill(&q.title, &player_name, &npc_name), fill(&q.goal.describe(), &player_name, &npc_name));
    let _ = player.tell(host, Text::literal(line));
}

/// Ends `a` for `player`: done (reward, thanks) or failed.
fn end(host: &mut dyn ServerHost, player: PlayerRef, npc: Option<EntityRef>, a: &Active, q: &Quest, done: bool) {
    let mut log = log_of(host, player);
    log.finish(a.npc, done, q.repeat);
    set_log(host, player, log);
    let player_name = player.name(&*host).unwrap_or_default();
    let line = fill(if done { &q.done } else { &q.fail }, &player_name, &a.npc_name);
    match npc {
        Some(n) => crate::heart::say_aloud(host, n, &line),
        None => {
            let _ = player.tell(host, Text::literal(format!("<{}> {line}", a.npc_name)));
        }
    }
    let title = fill(&q.title, &player_name, &a.npc_name);
    let _ = player.action_bar(host, Text::literal(if done { format!("Quest done: {title}") } else { format!("Quest failed: {title}") }));
    let event = if done { "quest_done" } else { "quest_failed" };
    if let Some(n) = npc {
        if done {
            crate::talk::run_for(host, Some(player), n, &q.reward);
        }
        crate::heart::feel(host, n, Some(player), event);
        crate::rulebook::notice(n.id(), event, Some(player.entity().id()));
    }
}

/// The `hand_in` action: the player says the quest for `npc` is done.
pub fn hand_in(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef) {
    let Some(uuid) = npc.uuid(host) else { return };
    let log = log_of(host, player);
    let Some(a) = log.with(uuid).cloned() else { return };
    let Some(q) = quest_named(&a.quest) else { return };
    let ready = match &q.goal {
        Goal::Bring { item, count } => {
            let have = items_of(host, player, item);
            if have >= *count {
                crate::talk::queue(format!("clear {} {item} {count}", player.name(&*host).unwrap_or_default()));
                true
            } else {
                let name = player.name(&*host).unwrap_or_default();
                crate::heart::say_aloud(host, npc, &format!("That's {have} of {count}, {name}."));
                false
            }
        }
        g => a.progress >= g.needed(),
    };
    if ready {
        end(host, player, Some(npc), &a, &q, true);
    } else if !matches!(q.goal, Goal::Bring { .. }) {
        let player_name = player.name(&*host).unwrap_or_default();
        crate::heart::say_aloud(host, npc, &format!("Not yet, {player_name}: {}.", fill(&q.goal.describe(), &player_name, &a.npc_name)));
    }
}

/// The `give_up` action.
pub fn give_up(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef) {
    let Some(uuid) = npc.uuid(host) else { return };
    let Some(a) = log_of(host, player).with(uuid).cloned() else { return };
    if let Some(q) = quest_named(&a.quest) {
        end(host, player, Some(npc), &a, &q, false);
    }
}

// ---- the tick and the hooks ----------------------------------------------------------------------------------

static LAST: Mutex<i64> = Mutex::new(0);

/// Each second: places reached, NPCs walked far enough, time run out.
pub fn tick(host: &mut dyn ServerHost) {
    let all = players(&*host);
    let Some(now) = all.first().and_then(|p| p.entity().level(&*host)).and_then(|l| l.game_time(&*host)) else { return };
    match LAST.lock() {
        Ok(mut last) if now - *last >= SCAN || now < *last => *last = now,
        _ => return,
    }
    for p in all {
        let log = log_of(host, p);
        for a in &log.active {
            let Some(q) = quest_named(&a.quest) else { continue };
            let npc = crate::heart::npc_by_uuid(a.npc).and_then(|id| host.entity(id));
            if a.deadline.is_some_and(|d| now > d) {
                end(host, p, npc, a, &q, false);
                continue;
            }
            match &q.goal {
                Goal::GoTo { .. } if a.progress == 0 => {
                    if p.entity().position(&*host).is_some_and(|at| q.goal.reached(at, a.start)) {
                        let mut log = log_of(host, p);
                        if let Some(x) = log.active.iter_mut().find(|x| x.npc == a.npc) {
                            x.progress = 1;
                        }
                        set_log(host, p, log);
                        let _ = p.action_bar(host, Text::literal(format!("You're there. Go back to {}.", a.npc_name)));
                    }
                }
                Goal::Walk { .. } => {
                    if let Some(n) = npc
                        && n.position(&*host).is_some_and(|at| q.goal.reached(at, a.start))
                    {
                        end(host, p, Some(n), a, &q, true);
                    }
                }
                _ => {}
            }
        }
    }
}

/// `living_death`: kills that count, and NPCs gone with their quests.
pub fn on_death(host: &mut dyn ServerHost, victim: u32, killer: i32) {
    let Some(v) = host.entity(victim) else { return };
    // An NPC that gave quests is gone: they end.
    if crate::companion::is_npc(&*host, v)
        && let Some(uuid) = v.uuid(host)
    {
        for p in players(&*host) {
            let Some(a) = log_of(host, p).with(uuid).cloned() else { continue };
            let mut log = log_of(host, p);
            log.finish(uuid, false, true);
            set_log(host, p, log);
            let _ = p.tell(host, Text::literal(format!("{} is gone: the quest is over.", a.npc_name)));
        }
    }
    if killer <= 0 {
        return;
    }
    let Some(p) = host.entity(killer as u32).and_then(|k| players(&*host).into_iter().find(|p| p.entity().id() == k.id())) else { return };
    let (kind, category) = crate::rulebook::kind_of(&*host, v);
    let mut log = log_of(host, p);
    let mut lines = Vec::new();
    for a in log.active.iter_mut() {
        let Some(q) = quest_named(&a.quest) else { continue };
        if q.goal.counts_kill(&kind, category) && a.progress < q.goal.needed() {
            a.progress += 1;
            let n = q.goal.needed();
            lines.push(if a.progress >= n { format!("{}/{n}: go back to {}.", a.progress, a.npc_name) } else { format!("{}/{n}", a.progress) });
        }
    }
    if !lines.is_empty() {
        set_log(host, p, log);
        let _ = p.action_bar(host, Text::literal(lines.join("  ")));
    }
}

// ---- commands -------------------------------------------------------------------------------------------------

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::new("felix quests"));
    e.commands.push(Command::op("felix quest give <targets:entities> <quests:text>", 2));
    e.commands.push(Command::op("felix quest clear <targets:entities>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let feedback = |host: &mut dyn ServerHost, line: String| {
        let _ = host.commands().feedback(Text::literal(line));
    };
    match lit.as_slice() {
        ["felix", "quests"] => {
            let mod_api::CommandSource::Player { player, .. } = e.source else {
                feedback(host, "Only a player has quests.".into());
                return Flow::Cancel;
            };
            let p = *player;
            let log = log_of(host, p);
            let now = p.entity().level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
            let player_name = p.name(&*host).unwrap_or_default();
            if log.active.is_empty() {
                feedback(host, "No quests. Ask an NPC if it needs help.".into());
            }
            for a in &log.active {
                let Some(q) = quest_named(&a.quest) else { continue };
                let have = match &q.goal {
                    Goal::Bring { item, .. } => Some(items_of(host, p, item)),
                    _ => None,
                };
                let st = status(a, &q, now, have);
                let line = format!(
                    "{} ({}): {}{}",
                    fill(&q.title, &player_name, &a.npc_name),
                    a.npc_name,
                    fill(&q.goal.describe(), &player_name, &a.npc_name),
                    if st.is_empty() { String::new() } else { format!(", {st}") }
                );
                feedback(host, line);
            }
        }
        ["felix", "quest", "give"] => {
            let ids = crate::companion::text_argument(e, "quests").unwrap_or_default();
            let unknown: Vec<&str> = ids.split_whitespace().filter(|i| quest_named(i).is_none()).collect();
            let note = if unknown.is_empty() { String::new() } else { format!(" (not loaded: {})", unknown.join(", ")) };
            let npcs: Vec<EntityRef> = crate::companion::targets(e).into_iter().filter(|t| crate::companion::is_npc(&*host, *t)).collect();
            let n = npcs.iter().filter(|t| GIVES.set(host, Holder::Entity(t.id()), Some(ids.trim().to_string())).is_ok()).count();
            feedback(host, format!("{n} NPCs now give {}{note}", ids.trim()));
        }
        ["felix", "quest", "clear"] => {
            let npcs: Vec<EntityRef> = crate::companion::targets(e).into_iter().filter(|t| crate::companion::is_npc(&*host, *t)).collect();
            let n = npcs.iter().filter(|t| GIVES.set(host, Holder::Entity(t.id()), None).is_ok()).count();
            feedback(host, format!("{n} NPCs give no quests by command now"));
        }
        _ => return Flow::Continue,
    }
    Flow::Cancel
}
