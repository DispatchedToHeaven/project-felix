//! Skills at work on the server (see `skill`): an NPC gets better at what it does (its job, its
//! trade, talking), works quicker and brings more the better it is, is glad doing what it loves,
//! and tells what it is good at and the life it had ("What are you good at?").
//! `/felix skills <npc>`, `/felix skills set <npc> <skill> <level>`.
use crate::companion;
use crate::skill::{self, Skill, Skills};
use mod_api::{
    AttachmentCodec, AttachmentType, Command, CommandArgumentValue, Compound, EntityRef, Flow, Holder, PlayerRef, RegisterCommandsEvent,
    RunModCommandEvent, ServerHost, Tag, Text,
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

pub static SKILLS: AttachmentType<Option<Skills>> = AttachmentType {
    namespace: "projectfelix",
    name: "skills",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |s| {
            let mut c = Compound::new();
            if let Some(s) = s {
                c.insert("json".into(), Tag::String(s.to_json().to_string().as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            Ok(match c.get(&"json".into()) {
                Some(Tag::String(s)) => crate::serde_json::from_str(&s.to_string_lossy()).ok().and_then(|v| Skills::from_json(&v)),
                _ => None,
            })
        },
    }),
    copy_on_death: false,
    sync: false,
};

static CACHE: LazyLock<Mutex<HashMap<u32, Skills>>> = LazyLock::new(Default::default);

pub fn clear() {
    if let Ok(mut c) = CACHE.lock() {
        c.clear();
    }
}

pub fn forget(id: u32) {
    if let Ok(mut c) = CACHE.lock() {
        c.remove(&id);
    }
}

/// What it does (its job, or `child`), for its past.
fn job_name(host: &mut dyn ServerHost, npc: EntityRef) -> Option<String> {
    if crate::families::is_child(host, npc) {
        return Some("child".into());
    }
    crate::jobs::job_of(host, npc).map(|j| j.name().to_string())
}

/// NPC `npc`'s skills (made from its past the first time).
pub fn of(host: &mut dyn ServerHost, npc: EntityRef) -> Skills {
    if let Some(s) = CACHE.lock().ok().and_then(|c| c.get(&npc.id()).cloned()) {
        return s;
    }
    let s = match SKILLS.get(host, Holder::Entity(npc.id())).ok().flatten() {
        Some(s) => s,
        None => {
            let uuid = npc.uuid(host).unwrap_or(0);
            let job = job_name(host, npc);
            let s = Skills::new(uuid, job.as_deref());
            let _ = SKILLS.set(host, Holder::Entity(npc.id()), Some(s.clone()));
            s
        }
    };
    if let Ok(mut c) = CACHE.lock() {
        c.insert(npc.id(), s.clone());
    }
    s
}

fn put(host: &mut dyn ServerHost, npc: EntityRef, s: Skills) {
    let _ = SKILLS.set(host, Holder::Entity(npc.id()), Some(s.clone()));
    if let Ok(mut c) = CACHE.lock() {
        c.insert(npc.id(), s);
    }
}

pub fn level(host: &mut dyn ServerHost, npc: EntityRef, s: Skill) -> u8 {
    of(host, npc).level(s)
}

/// Whether it loves (2), likes (1) or neither (0) skill `s`.
pub fn passion(host: &mut dyn ServerHost, npc: EntityRef, s: Skill) -> u8 {
    of(host, npc).passion[s.index()]
}

/// NPC `npc` practised `s`, worth `amount`: it gets better, and says so when it goes up a level.
pub fn practice(host: &mut dyn ServerHost, npc: EntityRef, s: Skill, amount: f32) {
    let now = npc.level(&*host).and_then(|l| l.game_time(&*host)).unwrap_or(0);
    let mut k = of(host, npc);
    let up = k.practice(s, amount, now);
    put(host, npc, k);
    if let Some(l) = up.filter(|l| *l >= 2) {
        let line = match l {
            20 => format!("I don't think anyone alive is better at {} than me now.", s.name()),
            16.. => format!("I've mastered {}. Years of it, and it shows.", s.name()),
            10.. => format!("I'm really getting good at {}!", s.name()),
            _ => format!("I'm getting better at {}.", s.name()),
        };
        crate::heart::say_aloud(host, npc, &line);
    }
}

/// How fast NPC `npc` does `job`: its skill at it.
pub fn work_speed(host: &mut dyn ServerHost, npc: EntityRef, job: &str) -> f32 {
    Skill::of_job(job).map_or(1.0, |s| skill::speed(level(host, npc, s)))
}

/// The chance a piece of `job` brings one more.
pub fn bonus(host: &mut dyn ServerHost, npc: EntityRef, job: &str) -> f32 {
    Skill::of_job(job).map_or(0.0, |s| skill::bonus(level(host, npc, s)))
}

/// The thought of doing its job now, if it likes or loves it.
pub fn work_thought(host: &mut dyn ServerHost, npc: EntityRef) -> Option<&'static str> {
    let job = crate::jobs::working_job(npc.id())?;
    let s = Skill::of_job(job.name())?;
    match passion(host, npc, s) {
        0 => None,
        1 => Some("work_like"),
        _ => Some("work_love"),
    }
}

/// What it says of itself: its past and what it is good at.
pub fn tell(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) {
    let uuid = npc.uuid(host).unwrap_or(0);
    let job = job_name(host, npc);
    let (child, adult) = skill::past_of(uuid, job.as_deref());
    let past = match adult {
        Some(a) => format!("{}, {}.", child.says, a.says),
        None => format!("{}.", child.says),
    };
    crate::heart::say_to(host, npc, player, &past);
    let k = of(host, npc);
    // What it is fair at or better, else what it likes.
    let best: Vec<String> = k.best(3).into_iter().filter(|(_, l, p)| *l >= 4 || *p > 0).map(|(s, l, p)| skill::describe(s, l, p)).collect();
    let line = if best.is_empty() { "I'm not much good at anything yet. But I'm learning!".to_string() } else { format!("I'm {}.", best.join("; ")) };
    crate::heart::say_to(host, npc, player, &line);
}

/// The tick of each level: skills past 10 fade a little each day without use.
pub fn tick(host: &mut dyn ServerHost) {
    if !crate::pace::every("skills", 1200) {
        return;
    }
    let npcs = companion::npcs();
    for id in &npcs {
        let Some(npc) = host.entity(*id) else { continue };
        let Some(now) = npc.level(&*host).and_then(|l| l.game_time(&*host)) else { continue };
        let mut k = of(host, npc);
        let before = k.clone();
        k.fade_day(now, crate::mind::DAY);
        if k != before {
            put(host, npc, k);
        }
    }
    if let Ok(mut c) = CACHE.lock() {
        c.retain(|id, _| npcs.contains(id));
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix skills <targets:entity>", 2));
    e.commands.push(Command::op("felix skills set <targets:entities> <skill:word> <level:integer>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if lit.first() != Some(&"felix") || lit.get(1) != Some(&"skills") {
        return Flow::Continue;
    }
    let npcs: Vec<EntityRef> = companion::targets(e).into_iter().filter(|t| companion::is_npc(&*host, *t)).collect();
    let Some(first) = npcs.first().copied() else {
        let _ = host.commands().feedback(Text::literal("That is not an NPC."));
        return Flow::Cancel;
    };
    let line = match lit.as_slice() {
        ["felix", "skills"] => {
            let k = of(host, first);
            let uuid = first.uuid(host).unwrap_or(0);
            let job = job_name(host, first);
            let (child, adult) = skill::past_of(uuid, job.as_deref());
            let name = first.name(&*host).unwrap_or_else(|| "NPC".into());
            let list: Vec<String> = skill::SKILLS
                .iter()
                .map(|s| {
                    let l = k.level(*s);
                    let (from, to) = (skill::xp_for(l), skill::xp_for((l + 1).min(20)));
                    let pct = if to > from { ((k.xp[s.index()] - from) / (to - from) * 100.0) as i32 } else { 100 };
                    format!("{} {l}{} ({pct}%)", s.name(), ["", "+", "++"][k.passion[s.index()] as usize])
                })
                .collect();
            format!("{name} ({}{}): {}", child.id, adult.map(|a| format!(", {}", a.id)).unwrap_or_default(), list.join(", "))
        }
        ["felix", "skills", "set"] => {
            let s = e.command.arguments.iter().find(|a| a.name == "skill").and_then(|a| match &a.value {
                CommandArgumentValue::String(s) => Skill::parse(s),
                _ => None,
            });
            let l = e.command.arguments.iter().find(|a| a.name == "level").and_then(|a| match a.value {
                CommandArgumentValue::Integer(v) => Some(v.clamp(0, 20) as u8),
                _ => None,
            });
            let (Some(s), Some(l)) = (s, l) else {
                let _ = host.commands().feedback(Text::literal("Skills: farming, woodcutting, fishing, cooking, building, trading, talking."));
                return Flow::Cancel;
            };
            for n in &npcs {
                let mut k = of(host, *n);
                k.xp[s.index()] = skill::xp_for(l);
                put(host, *n, k);
            }
            format!("{} NPCs are now level {l} at {}", npcs.len(), s.name())
        }
        _ => return Flow::Continue,
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
