//! What an entity is playing, set on the server and synced to every Horizon client tracking it.
//!
//! An action animation (`/felix animate @e[type=projectfelix:npc] animation.felix.wave`) plays
//! over the entity's own idle and walk animations, from the server's game time it started at,
//! so every player sees it at the same moment, and players who come later see it in progress.
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, Command, CommandArgumentValue, Compound, EntityRef, Flow, Holder,
    RegisterCommandsEvent, RunModCommandEvent, ServerHost, Tag, Text,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub animation: String,
    /// The level's game time when it started.
    pub start: i64,
}

pub static ACTION: AttachmentType<Option<Action>> = AttachmentType {
    namespace: "projectfelix",
    name: "action",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |a| {
            let mut c = Compound::new();
            if let Some(a) = a {
                c.insert("animation".into(), Tag::String(a.animation.as_str().into()));
                c.insert("start".into(), Tag::Long(a.start));
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            let animation = match c.get(&"animation".into()) {
                Some(Tag::String(s)) => s.to_string_lossy(),
                None => return Ok(None),
                _ => return Err(AttachmentError::Decode),
            };
            let start = match c.get(&"start".into()) {
                Some(Tag::Long(t)) => *t,
                _ => return Err(AttachmentError::Decode),
            };
            Ok(Some(Action { animation, start }))
        },
    }),
    copy_on_death: false,
    sync: true,
};

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix animate <targets:entities> <animation:word>", 2));
    e.commands.push(Command::op("felix stop <targets:entities>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit = &e.command.literals;
    if lit.first().map(String::as_str) != Some("felix") {
        return Flow::Continue;
    }
    let targets: Vec<EntityRef> = e
        .command
        .arguments
        .iter()
        .find(|a| a.name == "targets")
        .map(|a| match &a.value {
            CommandArgumentValue::Entities(list) => list.clone(),
            CommandArgumentValue::Entity(one) => vec![*one],
            _ => Vec::new(),
        })
        .unwrap_or_default();
    let animation = e.command.arguments.iter().find(|a| a.name == "animation").and_then(|a| match &a.value {
        CommandArgumentValue::String(s) => Some(s.clone()),
        _ => None,
    });
    let mut done = 0;
    for entity in &targets {
        let value = match (&animation, lit.get(1).map(String::as_str)) {
            (Some(name), Some("animate")) => {
                let Some(level) = entity.level(&*host) else { continue };
                let Some(time) = level.game_time(&*host) else { continue };
                Some(Action { animation: name.clone(), start: time })
            }
            _ => None,
        };
        if ACTION.set(host, Holder::Entity(entity.id()), value).is_ok() {
            done += 1;
        }
    }
    let line = match &animation {
        Some(name) => format!("Playing {name} on {done} entities"),
        None => format!("Stopped the animation of {done} entities"),
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
