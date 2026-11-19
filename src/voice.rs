//! NPC voices: while a line is over its head, an NPC babbles it, a short note a syllable, no
//! words. No mod API types here.
//!
//! A voice is a sound (`beep`, `soft`, `pluck`, `bell`, `deep`, `buzz`, `none`); an NPC has its
//! own (`/felix voice`, the maker), else its character's (`"voice": "deep"`), else one of the
//! light ones by its UUID, with a pitch of its own. Its mood moves the pitch, how loud, and how
//! fast: angry is lower, louder and quicker, sad lower, softer and slower, happy higher. A mood
//! may set its own: `"voice": { "pitch": 0.8, "volume": 1 }`. A question rises at its end.

/// A voice: its name, the sound it plays and the pitch it has at 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    pub name: &'static str,
    pub sound: &'static str,
    pub pitch: f32,
}

pub const PRESETS: &[Preset] = &[
    Preset { name: "beep", sound: "minecraft:block.note_block.bit", pitch: 1.0 },
    Preset { name: "soft", sound: "minecraft:block.note_block.flute", pitch: 0.9 },
    Preset { name: "pluck", sound: "minecraft:block.note_block.harp", pitch: 1.0 },
    Preset { name: "bell", sound: "minecraft:block.note_block.chime", pitch: 0.8 },
    Preset { name: "deep", sound: "minecraft:block.note_block.bass", pitch: 1.5 },
    Preset { name: "buzz", sound: "minecraft:block.note_block.didgeridoo", pitch: 1.4 },
    Preset { name: "none", sound: "", pitch: 1.0 },
];

/// The voices an NPC without one gets, by its UUID.
const LIGHT: &[&str] = &["beep", "soft", "pluck", "bell"];

/// At most so many notes a line.
pub const MAX_NOTES: usize = 16;

pub fn preset(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name.trim())
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

fn fold(uuid: u128) -> u64 {
    (uuid as u64) ^ ((uuid >> 64) as u64)
}

/// The voice of an NPC that has none of its own.
pub fn default_for(uuid: u128) -> &'static Preset {
    let name = LIGHT[(mix(fold(uuid)) % LIGHT.len() as u64) as usize];
    preset(name).unwrap_or(&PRESETS[0])
}

/// Its own pitch: from 0.88 to 1.12, by its UUID.
pub fn own_pitch(uuid: u128) -> f32 {
    let r = (mix(fold(uuid) ^ 0x5eed) % 1000) as f32 / 999.0;
    0.88 + r * 0.24
}

/// How a mood sounds: times the pitch, how loud (0 to 1), ticks between notes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    pub pitch: f32,
    pub volume: f32,
    pub gap: u8,
}

/// How mood `mood` sounds: its own `voice` if it has one, else by its name.
pub fn tone(mood: &str, own: Option<(f32, f32)>) -> Tone {
    let (pitch, volume, gap) = match mood {
        "angry" | "furious" => (0.8, 1.0, 2),
        "upset" | "annoyed" => (0.9, 0.8, 3),
        "sad" | "grieving" => (0.78, 0.45, 4),
        "happy" | "glad" => (1.15, 0.8, 2),
        "scared" | "afraid" => (1.25, 0.6, 2),
        "sleepy" | "tired" => (0.85, 0.4, 4),
        _ => (1.0, 0.7, 3),
    };
    let (pitch, volume) = own.unwrap_or((pitch, volume));
    Tone { pitch: pitch.clamp(0.5, 2.0), volume: volume.clamp(0.0, 1.0), gap }
}

/// One note: ticks after the line comes, its pitch and volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub at: u32,
    pub pitch: f32,
    pub volume: f32,
}

/// The syllables of `word`: its groups of vowels, at least one if it has letters.
fn syllables(word: &str) -> usize {
    let mut n = 0;
    let mut in_vowel = false;
    for c in word.chars().map(|c| c.to_ascii_lowercase()) {
        let v = "aeiouy".contains(c);
        if v && !in_vowel {
            n += 1;
        }
        in_vowel = v;
    }
    if n == 0 && word.chars().any(char::is_alphanumeric) { 1 } else { n }
}

/// The notes of `text` said at `pitch` (the voice's, its own and its mood's), `volume`, a note
/// every `gap` ticks; a pause after a comma, a longer one after a sentence. `seed` makes each
/// line its own tune, the same each time it is played.
pub fn notes(text: &str, pitch: f32, volume: f32, gap: u8, seed: u64) -> Vec<Note> {
    let gap = u32::from(gap.max(1));
    let mut out = Vec::new();
    let mut at = 0;
    let loud = if text.contains('!') { 1.2 } else { 1.0 };
    'words: for word in text.split_whitespace() {
        for _ in 0..syllables(word) {
            if out.len() >= MAX_NOTES {
                break 'words;
            }
            let r = (mix(seed ^ (out.len() as u64).wrapping_mul(0x9e37_79b9)) % 1000) as f32 / 999.0;
            let p = pitch * (0.92 + r * 0.16);
            out.push(Note { at, pitch: p, volume: (volume * loud).min(1.0) });
            at += gap;
        }
        match word.chars().last() {
            Some(',' | ';' | ':') => at += gap,
            Some('.' | '!' | '?') => at += gap * 2,
            _ => {}
        }
    }
    // A question rises at its end.
    if text.trim_end().ends_with('?')
        && let Some(last) = out.last_mut()
    {
        last.pitch *= 1.25;
    }
    for n in &mut out {
        n.pitch = n.pitch.clamp(0.5, 2.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_babble() {
        assert_eq!(syllables("hello"), 2);
        assert_eq!(syllables("Hmm"), 1);
        assert_eq!(syllables("..."), 0);
        let n = notes("Hello there, friend!", 1.0, 0.7, 3, 7);
        assert_eq!(n.len(), 5);
        // A pause after the comma.
        assert_eq!(n.iter().map(|x| x.at).collect::<Vec<_>>(), vec![0, 3, 6, 9, 15]);
        assert!(n.iter().all(|x| x.volume > 0.8 && (0.9..1.1).contains(&x.pitch)));
        assert_eq!(notes("Hello there, friend!", 1.0, 0.7, 3, 7), n);
        let q = notes("Who?", 1.0, 0.5, 2, 1);
        assert!(q[0].pitch > 1.1);
        assert_eq!(notes(&"la ".repeat(40), 1.0, 0.5, 2, 1).len(), MAX_NOTES);
        assert!(tone("angry", None).pitch < tone("calm", None).pitch && tone("angry", None).volume > tone("calm", None).volume);
        assert_eq!(tone("calm", Some((3.0, 0.2))).pitch, 2.0);
        let p = own_pitch(42);
        assert!((0.88..=1.12).contains(&p));
        assert!(LIGHT.contains(&default_for(12345).name));
        assert_eq!(preset("deep").map(|p| p.sound), Some("minecraft:block.note_block.bass"));
    }
}
