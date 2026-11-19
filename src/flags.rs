//! What players said and did in talks, kept with them (see `dialog`: `remember`, `forget`,
//! `flag`, `answer`), what the world around a player is like for conditions (biome, time,
//! weather), and the extended view: `/felix view extended` shows a maker the numbers and the
//! workings players don't see (exact relation changes, what answers do, which node it is).
use mod_api::{
    AttachmentCodec, AttachmentType, Command, CommandSource, Compound, EntityRef, Flow, Holder, PlayerRef,
    RegisterCommandsEvent, RunModCommandEvent, ServerHost, Tag, Text,
};
use std::collections::HashMap;

/// The tag a player in the extended view has.
pub const VIEW_TAG: &str = "felix_view";

pub static FLAGS: AttachmentType<Vec<(String, String)>> = AttachmentType {
    namespace: "projectfelix",
    name: "flags",
    default: Vec::new,
    codec: Some(AttachmentCodec {
        encode: |list| {
            let mut c = Compound::new();
            for (k, v) in list {
                c.insert(k.as_str().into(), Tag::String(v.as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            Ok(c.iter()
                .filter_map(|(k, v)| match v {
                    Tag::String(s) => Some((k.to_string_lossy(), s.to_string_lossy())),
                    _ => None,
                })
                .collect())
        },
    }),
    copy_on_death: true,
    sync: false,
};

/// Everything player `p` is remembered by.
pub fn all(host: &mut dyn ServerHost, p: PlayerRef) -> HashMap<String, String> {
    FLAGS.get(host, Holder::Player(p.uuid())).unwrap_or_default().into_iter().collect()
}

pub fn set(host: &mut dyn ServerHost, p: PlayerRef, key: &str, value: &str) {
    let mut list = FLAGS.get(host, Holder::Player(p.uuid())).unwrap_or_default();
    list.retain(|(k, _)| k != key);
    list.push((key.to_string(), value.to_string()));
    // A long memory forgets the oldest.
    if list.len() > 512 {
        list.remove(0);
    }
    let _ = FLAGS.set(host, Holder::Player(p.uuid()), list);
}

pub fn forget(host: &mut dyn ServerHost, p: PlayerRef, key: &str) {
    let mut list = FLAGS.get(host, Holder::Player(p.uuid())).unwrap_or_default();
    list.retain(|(k, _)| k != key && !k.starts_with(&format!("{key}:")));
    let _ = FLAGS.set(host, Holder::Player(p.uuid()), list);
}

/// Whether player `p` sees the workings (the extended view).
pub fn extended(host: &dyn mod_api::Host, p: PlayerRef) -> bool {
    p.entity().tags(host).unwrap_or_default().iter().any(|t| t == VIEW_TAG)
}

/// The world about player `p`: its biome, the time of day (`day`, `dusk`, `night`, `dawn`) and
/// the weather (`clear`, `rain`, `thunder`).
pub fn surroundings(host: &dyn mod_api::Host, p: PlayerRef) -> (String, &'static str, &'static str) {
    let me = p.entity();
    let Some(level) = me.level(host) else { return (String::new(), "day", "clear") };
    let pos = me.position(host).unwrap_or([0.0; 3]);
    let biome = level.biome_at(host, [pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32]).unwrap_or_default();
    let t = level.day_time(host).unwrap_or(0).rem_euclid(24000);
    let time = match t {
        0..12000 => "day",
        12000..13800 => "dusk",
        13800..22200 => "night",
        _ => "dawn",
    };
    let weather = if level.is_thundering(host) == Some(true) {
        "thunder"
    } else if level.is_raining(host) == Some(true) {
        "rain"
    } else {
        "clear"
    };
    (biome, time, weather)
}

/// How player `p` stands with NPC `npc`, in words (no numbers): `a friend`, `friendly`…
pub fn standing_word(npc: u32, p: u128) -> &'static str {
    let (friend_at, hate_at) = crate::heart::thresholds(npc);
    match crate::shop::Standing::of(crate::heart::relation_of(npc, p), friend_at, hate_at) {
        crate::shop::Standing::Friend => "a friend",
        crate::shop::Standing::Likes => "friendly",
        crate::shop::Standing::Neutral => "",
        crate::shop::Standing::Dislikes => "cold",
        crate::shop::Standing::Hates => "hostile",
    }
}

/// Tells player `p` how what it did went down with NPC `npc`: in the extended view the numbers,
/// else only which way, in words.
pub fn tell_change(host: &mut dyn ServerHost, npc: EntityRef, p: PlayerRef, before: i32, what: &str) {
    let after = crate::heart::relation_of(npc.id(), p.uuid());
    let name = npc.name(&*host).unwrap_or_else(|| "NPC".into());
    if extended(&*host, p) {
        let word = standing_word(npc.id(), p.uuid());
        let mood = crate::heart::mood_of(npc.id());
        let line = format!("[{name}] {what}: relation {before} > {after}{}, mood {mood}", if word.is_empty() { String::new() } else { format!(" ({word})") });
        let _ = p.tell(host, Text::literal(line));
    } else if after != before {
        let line = if after > before { format!("{name} seems pleased.") } else { format!("{name} didn't like that.") };
        let _ = p.action_bar(host, Text::literal(line));
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix view <mode:word>", 2));
    e.commands.push(Command::op("felix flags", 2));
    e.commands.push(Command::op("felix flags clear", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if !matches!(lit.as_slice(), ["felix", "view"] | ["felix", "flags"] | ["felix", "flags", "clear"]) {
        return Flow::Continue;
    }
    let CommandSource::Player { player, .. } = e.source else {
        let _ = host.commands().feedback(Text::literal("For a player."));
        return Flow::Cancel;
    };
    let p = *player;
    let name = p.name(&*host).unwrap_or_default();
    let line = match lit.as_slice() {
        ["felix", "view"] => match crate::companion::text_argument(e, "mode").as_deref() {
            Some("extended") => {
                crate::talk::queue(format!("tag {name} add {VIEW_TAG}"));
                "Extended view: the numbers and workings of talks are shown.".to_string()
            }
            Some("normal") => {
                crate::talk::queue(format!("tag {name} remove {VIEW_TAG}"));
                "Normal view: talks as players see them.".to_string()
            }
            _ => "Views: extended (numbers, answers' effects, nodes), normal".to_string(),
        },
        ["felix", "flags", "clear"] => {
            let _ = FLAGS.set(host, Holder::Player(p.uuid()), Vec::new());
            "Your talk memory is cleared.".to_string()
        }
        _ => {
            let mut list: Vec<String> = all(host, p).into_iter().map(|(k, v)| format!("{k}={v}")).collect();
            list.sort();
            if list.is_empty() { "Nothing remembered.".to_string() } else { format!("{} remembered: {}", list.len(), list.join(", ")) }
        }
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
