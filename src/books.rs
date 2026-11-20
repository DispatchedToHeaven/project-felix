//! The village book on the server (see `book`, `book_screen`): `/felix book` or its key (K) sends
//! the player a page for each NPC within reach, nearest first, its own companions on top, with the
//! next event; the book's Find makes one glow and points the way, Call has it walk up and talk
//! (if it likes the player well enough and isn't asleep).
use crate::book::{self, Facts};
use crate::companion::{self, Mode};
use crate::serde_json::{Value, json};
use mod_api::{
    Command, CommandSource, EntityRef, Flow, Holder, IPayloadContext, IPayloadHandler, LivingRef, PayloadBytes,
    PayloadId, PlayerRef, RegisterCommandsEvent, RunModCommandEvent, ServerHost, Text,
};

pub const OPEN: &str = "projectfelix:book_open";
pub const ASK: &str = "projectfelix:book_ask";
/// How far the book reaches, in blocks.
const REACH: f64 = 128.0;
const MOST: usize = 30;
/// How far a call carries.
const CALL_REACH: f64 = 64.0;

fn short(id: &str) -> String {
    id.rsplit(':').next().unwrap_or(id).replace('_', " ")
}

fn block(p: [f64; 3]) -> [i32; 3] {
    [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32]
}

fn player_named(host: &dyn mod_api::Host, who: u128) -> Option<String> {
    ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"]
        .iter()
        .filter_map(|d| host.level(d))
        .flat_map(|l| l.online_players(host).unwrap_or_default())
        .find(|p| p.uuid() == who)
        .and_then(|p| p.name(host))
}

/// What player `p` knows of NPC `npc`.
fn facts(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef) -> Facts {
    let id = npc.id();
    let name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
    let fam = crate::families::FAMILY.get(host, Holder::Entity(id)).unwrap_or_default();
    let names = |l: &[(u128, String)]| l.iter().map(|x| x.1.clone()).collect::<Vec<_>>();
    let owner = match LivingRef::new(npc).owner(&*host) {
        Some(o) if o == p.uuid() => "you".to_string(),
        Some(o) => player_named(&*host, o).unwrap_or_else(|| "someone".into()),
        None => String::new(),
    };
    let doing = crate::routines::doing(id);
    let doing = if doing.is_empty() && companion::mode_of(id) == Mode::Live { "about".to_string() } else { doing };
    let quests = crate::quests::GIVES.get(host, Holder::Entity(id)).ok().flatten().unwrap_or_default().split_whitespace().count();
    let job = crate::jobs::job_of(host, npc).map(|j| j.name().to_string()).unwrap_or_default();
    let work = crate::routines::place_named(host, npc, "work").map(block);
    let home = crate::routines::place_named(host, npc, "home").map(block);
    let character = crate::heart::character_of(host, npc).map(|c| short(&c)).unwrap_or_default();
    let stage = crate::bonds::stage_word(host, npc, p).unwrap_or("").to_string();
    let closeness = crate::bonds::closeness(host, npc, p);
    let event = crate::events::name_at(host, npc).unwrap_or_default();
    Facts {
        name,
        character,
        mood: crate::heart::mood_of(id),
        relation: crate::heart::relation_of(id, p.uuid()),
        standing: crate::flags::standing_word(id, p.uuid()).to_string(),
        stage,
        closeness,
        job,
        work,
        home,
        doing: if doing == "about" { String::new() } else { doing },
        event,
        owner,
        food: crate::needs::food_of(id),
        partner: fam.partner.map(|x| x.1).unwrap_or_default(),
        children: names(&fam.children),
        parents: names(&fam.parents),
        child: fam.child && !fam.grown,
        quests,
        purse: Some(crate::markets::purse_of(host, npc)),
    }
}

/// Sends player `p` the book.
pub fn open(host: &mut dyn ServerHost, p: PlayerRef) {
    let Some(at) = p.entity().position(&*host) else { return };
    let level = p.entity().level(&*host);
    let extended = crate::flags::extended(&*host, p);
    let mut near: Vec<(bool, f64, EntityRef)> = companion::npcs()
        .into_iter()
        .filter_map(|id| host.entity(id))
        .filter(|e| e.level(&*host) == level)
        .filter_map(|e| {
            let q = e.position(&*host)?;
            let d = ((q[0] - at[0]).powi(2) + (q[1] - at[1]).powi(2) + (q[2] - at[2]).powi(2)).sqrt();
            let mine = LivingRef::new(e).owner(&*host) == Some(p.uuid());
            (d <= REACH || mine).then_some((mine, d, e))
        })
        .collect();
    near.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.total_cmp(&b.1)));
    near.truncate(MOST);
    let mut list = Vec::new();
    for (mine, d, e) in near {
        let f = facts(host, e, p);
        let q = e.position(&*host).unwrap_or(at);
        let way = book::direction(q[0] - at[0], q[2] - at[2]);
        let whereabouts = if way == "here" { "right here".to_string() } else { format!("{} blocks {way}", d.round() as i64) };
        let brief = if way == "here" { "here".to_string() } else { format!("{} {}", d.round() as i64, book::abbr(way)) };
        list.push(json!({ "id": e.id(), "name": f.name, "mine": mine, "mark": book::mark(&f), "where": whereabouts, "brief": brief, "lines": book::lines(&f, extended) }));
    }
    let next = crate::events::next_up(host);
    let food = crate::markets::food_about(host, p).map(|s| format!("food: {}", s.word())).unwrap_or_default();
    let body = json!({ "npcs": list, "next": next, "food": food, "extended": extended });
    let _ = p.send_payload(host, &PayloadId(OPEN.into()), PayloadBytes(crate::serde_json::to_vec(&body).unwrap_or_default()));
}

/// Makes NPC `id` glow a while and tells `p` the way to it.
fn find(host: &mut dyn ServerHost, p: PlayerRef, id: u32) {
    let Some(npc) = host.entity(id) else { return };
    let (Some(at), Some(q)) = (p.entity().position(&*host), npc.position(&*host)) else { return };
    let name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
    if let Some(uuid) = npc.uuid(host) {
        crate::talk::queue(format!("effect give {} minecraft:glowing 20 0 true", crate::talk::uuid_text(uuid)));
    }
    let d = ((q[0] - at[0]).powi(2) + (q[2] - at[2]).powi(2)).sqrt();
    let way = book::direction(q[0] - at[0], q[2] - at[2]);
    let line = if way == "here" { format!("{name} is right here.") } else { format!("{name}: {} blocks {way}, glowing for a while.", d.round() as i64) };
    let _ = p.action_bar(host, Text::literal(line));
}

/// Calls NPC `id` over: it comes and talks, if it will.
fn call(host: &mut dyn ServerHost, p: PlayerRef, id: u32) {
    let Some(npc) = host.entity(id) else { return };
    let name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
    let (Some(at), Some(q)) = (p.entity().position(&*host), npc.position(&*host)) else { return };
    let d = ((q[0] - at[0]).powi(2) + (q[1] - at[1]).powi(2) + (q[2] - at[2]).powi(2)).sqrt();
    let stage = crate::bonds::stage(host, npc, p);
    let standing = crate::flags::standing_word(id, p.uuid());
    let mine = LivingRef::new(npc).owner(&*host) == Some(p.uuid());
    let line = if d > CALL_REACH {
        format!("{name} is too far to hear you.")
    } else if crate::routines::asleep(id) {
        format!("{name} is asleep.")
    } else if !mine && stage < crate::bond::Stage::Friend && standing != "friendly" && standing != "a friend" {
        crate::heart::say_aloud(host, npc, "Hm? I'm busy.");
        format!("{name} doesn't come for you.")
    } else {
        crate::rulebook::approach(host, npc, p, Some(String::new()));
        format!("{name} is coming.")
    };
    let _ = p.action_bar(host, Text::literal(line));
}

/// The book's asks: `{}` to open it again, `{"find": id}`, `{"call": id}`.
pub struct Ask;

impl IPayloadHandler for Ask {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        let v = crate::serde_json::from_slice::<Value>(&payload.0).unwrap_or(Value::Null);
        let id = |k: &str| v.get(k).and_then(Value::as_u64).map(|x| x as u32);
        if let Some(n) = id("find") {
            find(host, player, n);
        } else if let Some(n) = id("call") {
            call(host, player, n);
        } else {
            open(host, player);
        }
        Ok(())
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::new("felix book"));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if lit.as_slice() != ["felix", "book"] {
        return Flow::Continue;
    }
    match e.source {
        CommandSource::Player { player, .. } => {
            let p = *player;
            open(host, p);
        }
        _ => {
            let _ = host.commands().feedback(Text::literal("For a player."));
        }
    }
    Flow::Cancel
}
