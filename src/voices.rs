//! Voices on the server (see `voice`): which one an NPC has (`/felix voice`), and how each line
//! it says sounds, sent with the bubble; the clients near play the notes (`speech`).
use crate::companion;
use crate::voice::{self, PRESETS};
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, Command, CommandArgumentValue, Compound, EntityRef, Flow, Holder,
    RegisterCommandsEvent, RunModCommandEvent, ServerHost, Tag, Text,
};

/// An NPC's own voice (a preset's name), over its character's.
pub static VOICE: AttachmentType<Option<String>> = AttachmentType {
    namespace: "projectfelix",
    name: "voice",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |v| {
            let mut c = Compound::new();
            if let Some(v) = v {
                c.insert("name".into(), Tag::String(v.as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| match c.get(&"name".into()) {
            Some(Tag::String(s)) => Ok(Some(s.to_string_lossy())),
            None => Ok(None),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// How NPC `npc` sounds saying a line now: its sound (empty: silent), pitch, volume, ticks a note.
pub fn tone_for(host: &mut dyn ServerHost, npc: EntityRef) -> (String, f32, f32, u8) {
    let uuid = npc.uuid(host).unwrap_or(0);
    let own = VOICE.get(host, Holder::Entity(npc.id())).ok().flatten();
    let (theirs, mood, mood_voice) = crate::heart::voice_facts(npc.id());
    let preset = own.as_deref().or(theirs.as_deref()).and_then(voice::preset).unwrap_or_else(|| voice::default_for(uuid));
    let t = voice::tone(&mood, mood_voice);
    (preset.sound.to_string(), preset.pitch * voice::own_pitch(uuid) * t.pitch, t.volume, t.gap)
}

/// Gives NPC `npc` voice `name` (empty or `default`: its character's again).
pub fn set_voice(host: &mut dyn ServerHost, npc: EntityRef, name: &str) {
    let v = Some(name.trim()).filter(|n| !n.is_empty() && *n != "default").map(str::to_string);
    let _ = VOICE.set(host, Holder::Entity(npc.id()), v);
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix voice <targets:entities> <voice:word>", 2));
    e.commands.push(Command::op("felix voices", 2));
}

fn names() -> String {
    PRESETS.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let line = match lit.as_slice() {
        ["felix", "voices"] => format!("Voices: {}, default (its character's)", names()),
        ["felix", "voice"] => {
            let name = companion::text_argument(e, "voice").unwrap_or_default();
            if name != "default" && voice::preset(&name).is_none() {
                format!("No voice {name}. Voices: {}, default", names())
            } else {
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
                    set_voice(host, *n, &name);
                }
                format!("{} NPCs now have the voice {name}", npcs.len())
            }
        }
        _ => return Flow::Continue,
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
