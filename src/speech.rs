//! What NPCs say, shown over their heads: a bubble for a few seconds, seen by whoever is near,
//! turned to each one's camera. The chat gets a copy as `/felix chat` says: `player` (the
//! default) for a line that names a player near (it is said to them), `all` for every line,
//! `none` for no copies. Each line comes with how its NPC sounds saying it; the clients near
//! babble it (see `voice`).
use mod_api::{
    AttachmentCodec, AttachmentHolder, AttachmentQueries, AttachmentType, BlockEntityDrawContext, ClientHost, Command,
    Compound, EntityRef, Flow, Holder, RegisterCommandsEvent, RunModCommandEvent, ServerHost, SoundPlay, SoundSource, Tag,
    Text,
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// A line over an NPC's head, from and until (game time).
#[derive(Clone, Debug, PartialEq)]
pub struct Bubble {
    pub text: String,
    pub start: i64,
    pub until: i64,
    /// Its voice's sound (empty: silent), pitch, volume and ticks a note (see `voice`).
    pub sound: String,
    pub pitch: f32,
    pub volume: f32,
    pub gap: u8,
}

pub static SPEECH: AttachmentType<Option<Bubble>> = AttachmentType {
    namespace: "projectfelix",
    name: "speech",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |b| {
            let mut c = Compound::new();
            if let Some(b) = b {
                c.insert("text".into(), Tag::String(b.text.as_str().into()));
                c.insert("start".into(), Tag::Long(b.start));
                c.insert("until".into(), Tag::Long(b.until));
                c.insert("sound".into(), Tag::String(b.sound.as_str().into()));
                c.insert("pitch".into(), Tag::Double(f64::from(b.pitch).to_bits()));
                c.insert("volume".into(), Tag::Double(f64::from(b.volume).to_bits()));
                c.insert("gap".into(), Tag::Byte(b.gap as i8));
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            let (Some(Tag::String(t)), Some(Tag::Long(s)), Some(Tag::Long(u))) = (c.get(&"text".into()), c.get(&"start".into()), c.get(&"until".into())) else {
                return Ok(None);
            };
            let f = |k: &str, d: f32| match c.get(&k.into()) {
                Some(Tag::Double(v)) => f64::from_bits(*v) as f32,
                _ => d,
            };
            let sound = match c.get(&"sound".into()) {
                Some(Tag::String(x)) => x.to_string_lossy(),
                _ => String::new(),
            };
            let gap = match c.get(&"gap".into()) {
                Some(Tag::Byte(g)) => *g as u8,
                _ => 3,
            };
            Ok(Some(Bubble { text: t.to_string_lossy(), start: *s, until: *u, sound, pitch: f("pitch", 1.0), volume: f("volume", 0.7), gap }))
        },
    }),
    copy_on_death: false,
    sync: true,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ChatCopy {
    All,
    Player,
    None,
}

static MODE: Mutex<ChatCopy> = Mutex::new(ChatCopy::Player);
/// NPCs with a bubble up, and when it goes.
static SHOWN: LazyLock<Mutex<HashMap<u32, i64>>> = LazyLock::new(Default::default);

pub fn chat_copy() -> ChatCopy {
    MODE.lock().map(|m| *m).unwrap_or(ChatCopy::Player)
}

/// Ticks a line stays up: longer lines longer.
pub fn lasts(text: &str) -> i64 {
    (50 + text.chars().count() as i64 * 2).clamp(60, 220)
}

/// Puts `line` over NPC `npc`'s head.
pub fn show(host: &mut dyn ServerHost, npc: EntityRef, line: &str) {
    let Some(now) = npc.level(&*host).and_then(|l| l.game_time(&*host)) else { return };
    let until = now + lasts(line);
    let (sound, pitch, volume, gap) = crate::voices::tone_for(host, npc);
    let b = Bubble { text: line.chars().take(160).collect(), start: now, until, sound, pitch, volume, gap };
    if SPEECH.set(host, Holder::Entity(npc.id()), Some(b)).is_ok()
        && let Ok(mut s) = SHOWN.lock()
    {
        s.insert(npc.id(), until);
    }
}

/// The server tick: bubbles whose time is up come down.
pub fn tick(host: &mut dyn ServerHost) {
    let due: Vec<u32> = {
        let Ok(s) = SHOWN.lock() else { return };
        s.iter().filter_map(|(id, until)| {
            let now = host.entity(*id).and_then(|e| e.level(&*host)).and_then(|l| l.game_time(&*host))?;
            (now >= *until).then_some(*id)
        }).collect()
    };
    for id in due {
        if host.entity(id).is_some() {
            let _ = SPEECH.set(host, Holder::Entity(id), None);
        }
        if let Ok(mut s) = SHOWN.lock() {
            s.remove(&id);
        }
    }
}

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix chat <mode:word>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    if lit.as_slice() != ["felix", "chat"] {
        return Flow::Continue;
    }
    let mode = match crate::companion::text_argument(e, "mode").as_deref() {
        Some("all") => ChatCopy::All,
        Some("player") => ChatCopy::Player,
        Some("none") => ChatCopy::None,
        _ => {
            let _ = host.commands().feedback(Text::literal("Modes: all, player (lines that name a player go to them), none"));
            return Flow::Cancel;
        }
    };
    if let Ok(mut m) = MODE.lock() {
        *m = mode;
    }
    let _ = host.commands().feedback(Text::literal(format!("NPC lines in chat: {mode:?}")));
    Flow::Cancel
}

// ---- on the client -----------------------------------------------------------------------------------------

#[derive(Default)]
struct Seen {
    bubbles: HashMap<u32, Bubble>,
    yaw: f32,
    /// The line each NPC was last heard saying (its start).
    heard: HashMap<u32, i64>,
}

static SEEN: LazyLock<Mutex<Seen>> = LazyLock::new(Default::default);

/// The client tick: the bubbles up, and which way the camera looks.
pub fn client_tick(host: &dyn ClientHost) {
    let mut bubbles = HashMap::new();
    if let Ok(query) = host.query(&SPEECH) {
        for row in query.iter().flatten() {
            if let (AttachmentHolder::Entity(e), Some(b)) = (row.holder, row.value) {
                bubbles.insert(e.id(), b.clone());
            }
        }
    }
    let yaw = host.camera().map(|c| c.yaw).unwrap_or(0.0);
    let fresh: Vec<(u32, Bubble)> = match SEEN.lock() {
        Ok(mut s) => {
            let fresh = bubbles.iter().filter(|(id, b)| s.heard.get(*id) != Some(&b.start)).map(|(id, b)| (*id, b.clone())).collect();
            s.heard = bubbles.iter().map(|(id, b)| (*id, b.start)).collect();
            s.bubbles = bubbles;
            s.yaw = yaw;
            fresh
        }
        Err(_) => Vec::new(),
    };
    for (id, b) in fresh {
        babble(host, id, &b);
    }
}

/// NPC `id` babbles line `b`, a note a syllable, if it was just said.
fn babble(host: &dyn ClientHost, id: u32, b: &Bubble) {
    if b.sound.is_empty() {
        return;
    }
    let Some(e) = host.entity(id) else { return };
    let (Some(at), Some(now)) = (e.position(host), e.level(host).and_then(|l| l.game_time(host))) else { return };
    // Not a line that was up already when it came near.
    if now - b.start > 10 {
        return;
    }
    for n in crate::voice::notes(&b.text, b.pitch, b.volume, b.gap, b.start as u64 ^ u64::from(id)) {
        let mut play = SoundPlay::at(&b.sound, SoundSource::Neutral, [at[0], at[1] + 1.6, at[2]]);
        (play.volume, play.pitch) = (n.volume, n.pitch);
        let _ = host.play_local_sound(&play, n.at);
    }
}

/// `text` in lines of at most `width` characters, broken between words.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Draws NPC `id`'s bubble over its head (`height` blocks), if it has one up at `now`.
pub fn draw(draw: &mut dyn BlockEntityDrawContext, id: u32, height: f32, now: f64) {
    let Some((b, yaw)) = SEEN.lock().ok().and_then(|s| Some((s.bubbles.get(&id)?.clone(), s.yaw))) else { return };
    if now < b.start as f64 || now > b.until as f64 {
        return;
    }
    let lines = wrap(&b.text, 28);
    let scale = 1.0 / 48.0;
    draw.push();
    draw.translate([0.0, height + 0.25 + lines.len() as f32 * 10.0 * scale, 0.0]);
    // Turned to the camera, as a name tag is.
    draw.rotate_about([0.0, 1.0, 0.0], 180.0 - yaw, [0.0; 3]);
    draw.scale([scale, -scale, -scale]);
    let widths: Vec<f32> = lines.iter().map(|l| draw.text_width(&Text::literal(l.clone())).unwrap_or(0.0)).collect();
    let w = widths.iter().copied().fold(0.0, f32::max);
    let h = lines.len() as f32 * 10.0;
    // A dark card behind, a little further from the camera.
    let corners = [[-w / 2.0 - 3.0, -2.0, 0.6], [w / 2.0 + 3.0, -2.0, 0.6], [w / 2.0 + 3.0, h, 0.6], [-w / 2.0 - 3.0, h, 0.6]];
    draw.quad(crate::models::WHITE, corners, [0.0, 0.0, 1.0, 1.0], 0x1A1A22);
    for (i, (l, lw)) in lines.iter().zip(widths).enumerate() {
        let _ = draw.text(&Text::literal(l.clone()), [-lw / 2.0, i as f32 * 10.0], 0xFFFFFF, true, None);
    }
    draw.pop();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_wrap() {
        assert_eq!(wrap("Hello there, how are you doing today my friend?", 16), vec!["Hello there, how", "are you doing", "today my friend?"]);
        assert_eq!(wrap("", 10), Vec::<String>::new());
        assert!(lasts("hi") >= 60 && lasts(&"x".repeat(500)) == 220);
    }
}
