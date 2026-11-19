//! The NPC maker, in the world (`/felix create`, or `/felix npc edit <id>` for one already made):
//! a name, a model, a skin of the player's own, a greeting and up to four topics the player can
//! ask about. Skins are PNG files in `horizon-mod-data/projectfelix/skins/` on the player's side;
//! models of the player's own (`user:<name>`) go into the NPC's file, so everyone sees them.
//!
//! Its rules (the Rules page): when it sees a monster, hears a word, is hurt..., if it is night, it
//! guards..., then it says something, attacks, plays an animation..., else something else. Up to
//! [`MAX_RULES`] here; rules of its file the page can't show are kept as they are.
//!
//! Its quests (the Quests page): bring an item, kill some of a mob, go to a place, or walk it some
//! blocks away; what it says when it asks, and an item as the reward. Up to [`MAX_QUESTS`].
//!
//! Its shop (the Shop page): what it sells and buys, how many at a time, for how many emeralds.
//!
//! Click a field to type in it; Tab and Enter go to the next one, Ctrl+V pastes, Escape closes.
use crate::companion::DEFAULT_MODEL;
use crate::custom::SAVE;
use crate::models;
use crate::profile::{self, Profile};
use crate::rules::{self, CONDS, DOS, Row, WHENS};
use crate::quest::{self, KINDS};
use crate::shop;
use crate::serde_json::{Value, json};
use mod_api::{
    ClientHost, ConfigPath, DrawContext, Flow, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId,
    ScreenCharTypedPreEvent, ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenMouseScrolledPreEvent,
    ScreenRenderPostEvent, SoundPlay,
};
use std::sync::{Arc, LazyLock, Mutex};

const SCREEN: &str = "projectfelix";
const SKINS: &str = "skins";
const TOPICS: usize = 4;
const FIELDS: usize = 2 + TOPICS * 2;
/// A save's file must fit one packet to the server.
const MAX_FILE: usize = 32_000;
/// The model choice that keeps an NPC's own model (one it was imported with).
const OWN_MODEL: &str = "(its own model)";
/// The skin choice that keeps the skin the NPC has.
const KEPT_SKIN: &str = "(the skin it has)";
const NO_SKIN: &str = "(the model's texture)";
/// Rule texts are fields from here on: three a rule (the event's word, then's, else's).
const RULE_TEXT: usize = 100;
/// Quest `n`'s fields: its word, count, offer, reward.
const QUEST_TEXT: usize = 200;
pub const MAX_QUESTS: usize = 3;
/// Offer `n`'s fields: item, count, price.
const SHOP_TEXT: usize = 300;
pub const MAX_OFFERS: usize = 6;
pub const MAX_RULES: usize = 4;
/// The characters the maker offers (a file may bring another).
const CHARACTERS: &[&str] = &["projectfelix:kind", "projectfelix:touchy", "projectfelix:stoic"];
/// The days the maker offers (none: it follows its owner).
const ROUTINES: &[&str] = &["", "projectfelix:villager", "projectfelix:guard", "projectfelix:night_owl"];
const W: f32 = 330.0;
const ROW: f32 = 14.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Do {
    Field(usize),
    Model,
    Skin,
    Character,
    Day,
    Save(bool),
    Close,
    /// 1: rules, 2: quests (again: back to the topics).
    Page(u8),
    Kind(usize),
    DelQuest(usize),
    AddQuest,
    SellBuy(usize),
    DelOffer(usize),
    AddOffer,
    When(usize),
    If(usize),
    Then(usize),
    Else(usize),
    DelRule(usize),
    AddRule,
}

struct Skin {
    name: String,
    png: Vec<u8>,
    image: Arc<(u32, u32, Vec<u8>)>,
}

struct Creator {
    /// The id of the NPC being changed; a new one's id comes from its name.
    editing: Option<String>,
    fields: Vec<String>,
    focus: Option<usize>,
    models: Vec<String>,
    model: usize,
    own_model: Option<Vec<u8>>,
    skins: Vec<Skin>,
    /// 0: none (or the kept one, when `kept` is set); then `skins`.
    skin: usize,
    kept: Option<Skin>,
    rules: Vec<Row>,
    /// Rules of its file a row can't show, kept as they are.
    kept_rules: Vec<Value>,
    quests: Vec<quest::Row>,
    kept_quests: Vec<Value>,
    offers: Vec<shop::Row>,
    /// A shop file rows can't show, kept as it is.
    kept_shop: Option<Value>,
    /// 0: name and topics, 1: rules, 2: quests.
    page: u8,
    characters: Vec<String>,
    character: usize,
    routines: Vec<String>,
    routine: usize,
    buttons: Vec<([f32; 4], Do)>,
    status: String,
    shown: bool,
    close: bool,
    outbox: Vec<Vec<u8>>,
}

static CREATOR: LazyLock<Mutex<Option<Creator>>> = LazyLock::new(Default::default);

pub fn active() -> bool {
    CREATOR.lock().is_ok_and(|c| c.is_some())
}

fn label(i: usize) -> String {
    match i {
        0 => "Name".into(),
        1 => "Greeting".into(),
        i if (i - 2) % 2 == 0 => format!("Ask {}", (i - 2) / 2 + 1),
        _ => "Answer".into(),
    }
}

fn max_len(i: usize) -> usize {
    if i == 0 { 32 } else { 160 }
}

fn read_skins(host: &dyn ClientHost) -> Vec<Skin> {
    let mut out = Vec::new();
    crate::editor::read_folder(host, SKINS, |name, png| {
        if !name.to_ascii_lowercase().ends_with(".png") {
            return;
        }
        match crate::image::decode_png(&png) {
            Ok(img) if img.0 <= 256 && img.1 <= 256 => out.push(Skin { name: name.to_string(), png, image: Arc::new(img) }),
            Ok(_) => eprintln!("[projectfelix] skin {name} is over 256 x 256"),
            Err(e) => eprintln!("[projectfelix] skin {name}: {e}"),
        }
    });
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `npc_open` from the server: open the maker, blank or on an NPC's file.
pub struct OpenHandler;

impl IPayloadHandler for OpenHandler {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(host) = context.client_host() else { return Ok(()) };
        crate::editor::load_saved(&*host);
        let editing = if payload.0.is_empty() { None } else { Profile::unpack(&payload.0, &crate::image::inflate).ok() };
        let mut models: Vec<String> = models::list().into_iter().filter(|m| !m.starts_with("npc:")).collect();
        let mut fields = vec![String::new(); FIELDS];
        fields[1] = "Hello, @player!".into();
        let mut kept = None;
        let mut own_model = None;
        let mut model = models.iter().position(|m| m == DEFAULT_MODEL).unwrap_or(0);
        let (mut rows, mut kept_rules) = (Vec::new(), Vec::new());
        let mut characters: Vec<String> = CHARACTERS.iter().map(|s| s.to_string()).collect();
        for k in crate::char_editor::known() {
            if !characters.contains(&k) {
                characters.push(k);
            }
        }
        let mut character = 0;
        let mut routines: Vec<String> = ROUTINES.iter().map(|s| s.to_string()).collect();
        let mut routine = 0;
        if let Some(r) = editing.as_ref().map(|p| p.routine.clone()).filter(|r| !r.is_empty()) {
            routine = routines.iter().position(|x| *x == r).unwrap_or_else(|| {
                routines.push(r);
                routines.len() - 1
            });
        }
        if let Some(ch) = editing.as_ref().map(|p| p.character.clone()).filter(|c| !c.is_empty()) {
            character = characters.iter().position(|c| *c == ch).unwrap_or_else(|| {
                characters.push(ch);
                characters.len() - 1
            });
        }
        if let Some(v) = editing.as_ref().and_then(|p| p.rules.as_ref()).and_then(|r| crate::serde_json::from_str::<Value>(r).ok()) {
            for r in v.get("rules").unwrap_or(&v).as_array().cloned().unwrap_or_default() {
                match Row::from_json(&r).filter(|_| rows.len() < MAX_RULES) {
                    Some(row) => rows.push(row),
                    None => kept_rules.push(r),
                }
            }
        }
        let (mut offers, mut kept_shop) = (Vec::new(), None);
        if let Some(v) = editing.as_ref().and_then(|p| p.shop.as_ref()).and_then(|r| crate::serde_json::from_str::<Value>(r).ok()) {
            match shop::rows_from_json(&v).filter(|r| r.len() <= MAX_OFFERS) {
                Some(r) => offers = r,
                None => kept_shop = Some(v),
            }
        }
        let (mut qrows, mut kept_quests) = (Vec::new(), Vec::new());
        if let Some(v) = editing.as_ref().and_then(|p| p.quests.as_ref()).and_then(|r| crate::serde_json::from_str::<Value>(r).ok()) {
            for q in v.get("quests").unwrap_or(&v).as_array().cloned().unwrap_or_default() {
                match quest::Row::from_json(&q).filter(|_| qrows.len() < MAX_QUESTS) {
                    Some(row) => qrows.push(row),
                    None => kept_quests.push(q),
                }
            }
        }
        if let Some(p) = &editing {
            fields[0] = p.name.clone();
            fields[1] = p.greeting.clone();
            for (k, (ask, answer)) in p.topics.iter().take(TOPICS).enumerate() {
                fields[2 + k * 2] = ask.clone();
                fields[3 + k * 2] = answer.clone();
            }
            if let Ok(img) = crate::image::decode_png(&p.skin) {
                kept = Some(Skin { name: KEPT_SKIN.into(), png: p.skin.clone(), image: Arc::new(img) });
            }
            if let Some(m) = &p.model_file {
                own_model = Some(m.clone());
                models.insert(0, OWN_MODEL.into());
                model = 0;
            } else if let Some(i) = models.iter().position(|m| *m == p.model) {
                model = i;
            }
        }
        crate::menu::yield_screen();
        let shown = host.mod_screen() == Some(SCREEN) || host.mod_screen_open(SCREEN);
        let skins = read_skins(&*host);
        let status = if skins.is_empty() && kept.is_none() {
            "Put PNG skins in horizon-mod-data/projectfelix/skins/".to_string()
        } else {
            format!("{} skins in horizon-mod-data/projectfelix/skins/", skins.len())
        };
        let c = Creator {
            editing: editing.as_ref().map(|p| p.id.clone()),
            fields,
            focus: Some(0),
            models,
            model,
            own_model,
            skin: if kept.is_some() || skins.is_empty() { 0 } else { 1 },
            skins,
            kept,
            rules: rows,
            kept_rules,
            quests: qrows,
            offers,
            kept_shop,
            kept_quests,
            page: 0,
            characters,
            character,
            routines,
            routine,
            buttons: Vec::new(),
            status,
            shown,
            close: false,
            outbox: Vec::new(),
        };
        if let Ok(mut lock) = CREATOR.lock() {
            *lock = Some(c);
        }
        Ok(())
    }
}

impl Creator {
    fn routine_name(&self) -> String {
        match self.routines.get(self.routine).map(String::as_str).unwrap_or("") {
            "" => "(none: it follows its owner)".into(),
            id => id.strip_prefix("projectfelix:").unwrap_or(id).replace('_', " "),
        }
    }

    fn character_name(&self) -> String {
        let id = self.characters.get(self.character).map(String::as_str).unwrap_or("projectfelix:kind");
        id.strip_prefix("projectfelix:").unwrap_or(id).to_string()
    }

    /// Field `i`: a main page field, or a rule's text.
    fn text(&mut self, i: usize) -> Option<&mut String> {
        if i < FIELDS {
            return self.fields.get_mut(i);
        }
        if let Some(j) = i.checked_sub(SHOP_TEXT) {
            let o = self.offers.get_mut(j / 3)?;
            return Some(match j % 3 {
                0 => &mut o.item,
                1 => &mut o.count,
                _ => &mut o.price,
            });
        }
        if let Some(j) = i.checked_sub(QUEST_TEXT) {
            let q = self.quests.get_mut(j / 4)?;
            return Some(match j % 4 {
                0 => &mut q.what,
                1 => &mut q.count,
                2 => &mut q.offer,
                _ => &mut q.reward,
            });
        }
        let j = i.checked_sub(RULE_TEXT)?;
        let r = self.rules.get_mut(j / 3)?;
        Some(match j % 3 {
            0 => &mut r.when_arg,
            1 => &mut r.then_arg,
            _ => &mut r.otherwise_arg,
        })
    }

    /// The fields of the page shown, in Tab order.
    fn order(&self) -> Vec<usize> {
        if self.page == 0 {
            return (0..FIELDS).collect();
        }
        if self.page == 3 {
            return (0..self.offers.len()).flat_map(|n| [SHOP_TEXT + n * 3, SHOP_TEXT + n * 3 + 1, SHOP_TEXT + n * 3 + 2]).collect();
        }
        if self.page == 2 {
            let mut o = Vec::new();
            for (n, q) in self.quests.iter().enumerate() {
                let base = QUEST_TEXT + n * 4;
                o.push(base);
                if q.kind < 2 {
                    o.push(base + 1);
                }
                o.extend([base + 2, base + 3]);
            }
            return o;
        }
        let mut o = Vec::new();
        for (n, r) in self.rules.iter().enumerate() {
            let base = RULE_TEXT + n * 3;
            if WHENS[r.when].1.is_some() {
                o.push(base);
            }
            if DOS[r.then].2 {
                o.push(base + 1);
            }
            if r.cond != 0 && DOS[r.otherwise].2 {
                o.push(base + 2);
            }
        }
        o
    }

    fn skin(&self) -> Option<&Skin> {
        if self.skin == 0 { self.kept.as_ref() } else { self.skins.get(self.skin - 1) }
    }

    fn skin_name(&self) -> &str {
        match self.skin() {
            Some(s) => &s.name,
            None => NO_SKIN,
        }
    }

    fn model_name(&self) -> &str {
        self.models.get(self.model).map(String::as_str).unwrap_or(DEFAULT_MODEL)
    }

    fn profile(&self, host: &dyn ClientHost) -> Result<Profile, String> {
        let name = self.fields[0].trim();
        if name.is_empty() {
            return Err("Give it a name".into());
        }
        let mut p = Profile::new(name);
        if let Some(id) = &self.editing {
            p.id = id.clone();
        }
        p.greeting = self.fields[1].trim().to_string();
        p.topics = (0..TOPICS)
            .map(|k| (self.fields[2 + k * 2].trim().to_string(), self.fields[3 + k * 2].trim().to_string()))
            .filter(|(ask, _)| !ask.is_empty())
            .map(|(ask, answer)| (ask, if answer.is_empty() { "...".into() } else { answer }))
            .collect();
        let model = self.model_name();
        if model == OWN_MODEL {
            p.model_file = self.own_model.clone();
        } else if let Some(user) = model.strip_prefix("user:") {
            // The player's own model goes into the file: the server and the others lack it.
            let file = ConfigPath::new(&format!("models/{user}.geo.json")).map_err(|e| format!("{e:?}"))?;
            p.model_file = Some(host.data_files().read(&file).map_err(|e| format!("model {user}: {e:?}"))?);
        } else {
            p.model = model.to_string();
        }
        if let Some(s) = self.skin() {
            p.skin = s.png.clone();
        }
        p.character = self.characters.get(self.character).cloned().unwrap_or_default();
        p.routine = self.routines.get(self.routine).cloned().unwrap_or_default();
        for (n, r) in self.rules.iter().enumerate() {
            if let Some(what) = WHENS[r.when].1.filter(|_| r.when_arg.trim().is_empty()) {
                return Err(format!("Rule {}: \"{}\" needs the {what}", n + 1, WHENS[r.when].0));
            }
        }
        let mut list: Vec<Value> = self.rules.iter().map(Row::to_json).collect();
        list.extend(self.kept_rules.iter().cloned());
        if !list.is_empty() {
            let v = json!({ "rules": list });
            rules::read(&v).map_err(|e| format!("Rules: {e}"))?;
            p.rules = Some(crate::serde_json::to_string_pretty(&v).unwrap_or_default());
        }
        p.shop = match shop::rows_to_json(&self.offers).map_err(|e| format!("Shop: {e}"))? {
            Some(v) => Some(crate::serde_json::to_string_pretty(&v).unwrap_or_default()),
            None => self.kept_shop.as_ref().map(|v| crate::serde_json::to_string_pretty(v).unwrap_or_default()),
        };
        let mut list = Vec::new();
        for (n, q) in self.quests.iter().enumerate() {
            list.push(q.to_json().map_err(|e| format!("Quest {}: {e}", n + 1))?);
        }
        list.extend(self.kept_quests.iter().cloned());
        if !list.is_empty() {
            let v = json!({ "quests": list });
            quest::read_list(&v).map_err(|e| format!("Quests: {e}"))?;
            p.quests = Some(crate::serde_json::to_string_pretty(&v).unwrap_or_default());
        }
        Ok(p)
    }

    fn save(&mut self, host: &dyn ClientHost, spawn: bool) {
        match self.profile(host) {
            Ok(p) => {
                let file = p.pack(&crate::image::crc32);
                if file.len() > MAX_FILE {
                    self.status = format!("Too big: {} KB of {} KB (a smaller skin or model)", file.len() / 1024, MAX_FILE / 1024);
                    return;
                }
                let mut bytes = vec![u8::from(spawn)];
                bytes.extend(file);
                self.outbox.push(bytes);
                self.status = format!("Sent {} ({})", p.name, p.id);
                self.editing = Some(p.id);
                if spawn {
                    self.close = true;
                }
            }
            Err(e) => self.status = e,
        }
    }
}

/// The client tick: sends what was saved, and forgets the maker once its screen is closed.
pub fn tick(host: &mut dyn ClientHost) {
    let (out, close) = {
        let Ok(mut lock) = CREATOR.lock() else { return };
        let Some(c) = lock.as_mut() else { return };
        let out = std::mem::take(&mut c.outbox);
        let close = c.close;
        if close || (c.shown && host.mod_screen() != Some(SCREEN)) {
            *lock = None;
        } else if !c.shown && host.mod_screen() == Some(SCREEN) {
            c.shown = true;
        }
        (out, close)
    };
    for bytes in out {
        let _ = host.send_payload_to_server(&PayloadId(SAVE.into()), PayloadBytes(bytes));
    }
    if close {
        host.mod_screen_close(SCREEN);
    }
}

pub(crate) fn inside(r: [f32; 4], at: [f32; 2]) -> bool {
    at[0] >= r[0] && at[0] < r[0] + r[2] && at[1] >= r[1] && at[1] < r[1] + r[3]
}

/// The end of `text` that fits `width` (the end is where the typing is).
pub(crate) fn tail(draw: &dyn DrawContext, text: &str, width: f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    for start in 0..=chars.len() {
        let s: String = chars[start..].iter().collect();
        if draw.text_width(&s) <= width {
            return s;
        }
    }
    String::new()
}

/// The start of `text` that fits `width`, with an ellipsis when cut.
pub(crate) fn head(draw: &dyn DrawContext, text: &str, width: f32) -> String {
    if draw.text_width(text) <= width {
        return text.to_string();
    }
    let mut s: String = text.chars().collect();
    while !s.is_empty() && draw.text_width(&format!("{s}…")) > width {
        s.pop();
    }
    format!("{s}…")
}

pub(crate) fn button<T>(draw: &mut dyn DrawContext, buttons: &mut Vec<([f32; 4], T)>, r: [f32; 4], text: &str, what: T, mouse: [f32; 2]) {
    draw.fill(r, if inside(r, mouse) { 0xE050_5868 } else { 0xD028_2C38 });
    draw.fill([r[0], r[1] + r[3] - 1.0, r[2], 1.0], 0xFF10_1018);
    let tw = draw.text_width(text);
    draw.text(text, [r[0] + (r[2] - tw) / 2.0, r[1] + (r[3] - 8.0) / 2.0], 0xFFF0_F0F0, false);
    buttons.push((r, what));
}

/// A field: its label to the left, its text, and the cursor when it has the focus.
#[allow(clippy::too_many_arguments)]
pub(crate) fn field<T>(draw: &mut dyn DrawContext, buttons: &mut Vec<([f32; 4], T)>, r: [f32; 4], text: &str, focus: bool, what: T, mouse: [f32; 2], blink: bool) {
    draw.fill(r, if focus { 0xF018_1820 } else if inside(r, mouse) { 0xE038_3C4C } else { 0xC020_2430 });
    draw.fill([r[0], r[1] + r[3] - 1.0, r[2], 1.0], if focus { 0xFFFF_D86A } else { 0xFF50_5060 });
    // The end where the typing is; the start otherwise.
    let shown = if focus { tail(draw, text, r[2] - 8.0) } else { head(draw, text, r[2] - 8.0) };
    draw.text(&shown, [r[0] + 3.0, r[1] + 3.0], 0xFFFF_FFFF, false);
    if focus && blink {
        let x = r[0] + 3.0 + draw.text_width(&shown);
        draw.fill([x, r[1] + 2.0, 1.0, 9.0], 0xFFFF_FFFF);
    }
    buttons.push((r, what));
}

/// A picker: a click takes the next choice, the wheel moves either way.
#[allow(clippy::too_many_arguments)]
pub(crate) fn picker<T>(draw: &mut dyn DrawContext, buttons: &mut Vec<([f32; 4], T)>, r: [f32; 4], value: &str, what: T, mouse: [f32; 2]) {
    draw.fill(r, if inside(r, mouse) { 0xE040_4458 } else { 0xC030_3040 });
    let v = tail(draw, value, r[2] - 18.0);
    draw.text(&v, [r[0] + 3.0, r[1] + 3.0], 0xFFFF_FFFF, false);
    draw.text(">", [r[0] + r[2] - 9.0, r[1] + 3.0], 0xFFA0_A0A0, false);
    buttons.push((r, what));
}

/// The skin as a picture, in `r`, runs of one colour drawn as one rectangle.
fn draw_skin(draw: &mut dyn DrawContext, r: [f32; 4], img: &(u32, u32, Vec<u8>)) {
    let (w, h, px) = (img.0, img.1, &img.2);
    if w == 0 || h == 0 {
        return;
    }
    let s = (r[2] / w as f32).min(r[3] / h as f32);
    // A checker behind it, for the transparent parts.
    draw.fill([r[0], r[1], w as f32 * s, h as f32 * s], 0xFF30_3038);
    for y in 0..h {
        let mut x = 0;
        while x < w {
            let i = ((y * w + x) * 4) as usize;
            let c = [px[i], px[i + 1], px[i + 2], px[i + 3]];
            let mut end = x + 1;
            while end < w && px[((y * w + end) * 4) as usize..][..4] == c {
                end += 1;
            }
            if c[3] >= 128 {
                let argb = 0xFF00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
                draw.fill([r[0] + x as f32 * s, r[1] + y as f32 * s, (end - x) as f32 * s, s], argb);
            }
            x = end;
        }
    }
}

/// `screen_render_post`: the maker's panel.
pub fn render(host: &dyn ClientHost, e: &mut ScreenRenderPostEvent<'_>) {
    if host.mod_screen() != Some(SCREEN) {
        return;
    }
    let Ok(mut lock) = CREATOR.lock() else { return };
    let Some(c) = lock.as_mut() else { return };
    let draw = &mut *e.draw;
    let mouse = e.screen.mouse;
    let [sw, sh] = draw.size();
    let height = ROW * (7.0 + TOPICS as f32) + 44.0;
    let (x0, y0) = (((sw - W) / 2.0).max(0.0), ((sh - height) / 2.0).max(0.0));
    let blink = (crate::runtime::clock() as i64 / 6) % 2 == 0;
    let mut buttons = Vec::new();
    draw.fill([x0 - 6.0, y0 - 6.0, W + 12.0, height + 12.0], 0xE010_1018);
    let title = match &c.editing {
        Some(id) => format!("NPC {id}"),
        None => format!("New NPC: {}", profile::make_id(c.fields[0].trim())),
    };
    draw.text(&title, [x0, y0], 0xFFFF_D86A, false);
    if c.page == 3 {
        draw_shop(draw, &mut buttons, c, [x0, y0 + ROW], mouse, blink);
        bottom(draw, &mut buttons, c, [x0, y0 + height - 26.0], mouse);
        c.buttons = buttons;
        return;
    }
    if c.page == 2 {
        draw_quests(draw, &mut buttons, c, [x0, y0 + ROW], mouse, blink);
        bottom(draw, &mut buttons, c, [x0, y0 + height - 26.0], mouse);
        c.buttons = buttons;
        return;
    }
    if c.page == 1 {
        draw_rules(draw, &mut buttons, c, [x0, y0 + ROW], mouse, blink);
        bottom(draw, &mut buttons, c, [x0, y0 + height - 26.0], mouse);
        c.buttons = buttons;
        return;
    }
    let lab = 58.0;
    let preview = 96.0;
    let fw = W - lab - preview - 8.0;
    let mut y = y0 + ROW;
    for (i, what, value) in [(0, Do::Field(0), None), (usize::MAX, Do::Model, Some(c.model_name().to_string())), (usize::MAX, Do::Skin, Some(c.skin_name().to_string())), (usize::MAX, Do::Character, Some(c.character_name())), (usize::MAX, Do::Day, Some(c.routine_name())), (1, Do::Field(1), None)] {
        let text = match what {
            Do::Model => "Model".to_string(),
            Do::Skin => "Skin".to_string(),
            Do::Character => "Character".to_string(),
            Do::Day => "Day".to_string(),
            _ => label(i),
        };
        draw.text(&text, [x0, y + 3.0], 0xFFA0_A0A0, false);
        let r = [x0 + lab, y, fw, ROW - 2.0];
        match value {
            Some(v) => picker(draw, &mut buttons, r, &v, what, mouse),
            None => field(draw, &mut buttons, r, &c.fields[i], c.focus == Some(i), what, mouse, blink),
        }
        y += ROW;
    }
    // Topics: what the player asks, and the answer, side by side.
    y += 4.0;
    draw.text("Topics: what the player asks, and the answer", [x0, y], 0xFFA0_A0A0, false);
    y += 10.0;
    let half = (W - preview - 8.0 - 4.0) / 2.0;
    for k in 0..TOPICS {
        for (j, i) in [2 + k * 2, 3 + k * 2].into_iter().enumerate() {
            let r = [x0 + j as f32 * (half + 4.0), y, half, ROW - 2.0];
            field(draw, &mut buttons, r, &c.fields[i], c.focus == Some(i), Do::Field(i), mouse, blink);
            if c.fields[i].is_empty() && c.focus != Some(i) {
                draw.text(&label(i), [r[0] + 3.0, r[1] + 3.0], 0xFF60_6070, false);
            }
        }
        y += ROW;
    }
    // The skin, beside the fields.
    let pr = [x0 + W - preview, y0 + ROW, preview, preview];
    draw.fill(pr, 0xC020_2430);
    match c.skin() {
        Some(s) => {
            let img = s.image.clone();
            draw_skin(draw, pr, &img);
            draw.text(&format!("{} x {}", img.0, img.1), [pr[0], pr[1] + preview + 2.0], 0xFF80_8090, false);
        }
        None => draw.text("no skin", [pr[0] + 4.0, pr[1] + 4.0], 0xFF80_8090, false),
    }
    let _ = y;
    bottom(draw, &mut buttons, c, [x0, y0 + height - 26.0], mouse);
    c.buttons = buttons;
}

/// The buttons below either page, and the status line.
fn bottom(draw: &mut dyn DrawContext, buttons: &mut Vec<([f32; 4], Do)>, c: &Creator, at: [f32; 2], mouse: [f32; 2]) {
    let [x0, y] = at;
    button(draw, buttons, [x0, y, 44.0, 16.0], "Save", Do::Save(false), mouse);
    button(draw, buttons, [x0 + 48.0, y, 72.0, 16.0], "Save & spawn", Do::Save(true), mouse);
    let rules = if c.page == 1 { "Topics".to_string() } else { format!("Rules ({})", c.rules.len() + c.kept_rules.len()) };
    button(draw, buttons, [x0 + 124.0, y, 54.0, 16.0], &rules, Do::Page(1), mouse);
    let quests = if c.page == 2 { "Topics".to_string() } else { format!("Quests ({})", c.quests.len() + c.kept_quests.len()) };
    button(draw, buttons, [x0 + 182.0, y, 58.0, 16.0], &quests, Do::Page(2), mouse);
    let shop = if c.page == 3 { "Topics".to_string() } else if c.offers.is_empty() && c.kept_shop.is_none() { "Shop".to_string() } else { "Shop *".to_string() };
    button(draw, buttons, [x0 + 244.0, y, 42.0, 16.0], &shop, Do::Page(3), mouse);
    button(draw, buttons, [x0 + W - 40.0, y, 40.0, 16.0], "Close", Do::Close, mouse);
    let status = tail(draw, &c.status, W);
    draw.text(&status, [x0, y + 20.0], 0xFFC0_C0C8, false);
}

/// A field with a grey hint while it is empty.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hint_field<T>(draw: &mut dyn DrawContext, buttons: &mut Vec<([f32; 4], T)>, r: [f32; 4], text: &str, hint: &str, focus: bool, what: T, mouse: [f32; 2], blink: bool) {
    field(draw, buttons, r, text, focus, what, mouse, blink);
    if text.is_empty() && !focus {
        let h = head(draw, hint, r[2] - 6.0);
        draw.text(&h, [r[0] + 3.0, r[1] + 3.0], 0xFF60_6070, false);
    }
}

/// The rules page: two lines a rule. When [event] [its word] [condition] x;
/// then [action] [text], else [action] [text].
fn draw_rules(draw: &mut dyn DrawContext, buttons: &mut Vec<([f32; 4], Do)>, c: &Creator, at: [f32; 2], mouse: [f32; 2], blink: bool) {
    let [x0, mut y] = at;
    let grey = 0xFFA0_A0A0;
    let note = if c.kept_rules.is_empty() {
        "When it..., if..., then it... else it...".to_string()
    } else {
        format!("When it..., if..., then... ({} more from its file kept)", c.kept_rules.len())
    };
    draw.text(&note, [x0, y], grey, false);
    y += 12.0;
    let h = ROW - 2.0;
    for (n, r) in c.rules.iter().enumerate() {
        let base = RULE_TEXT + n * 3;
        draw.fill([x0 - 2.0, y - 2.0, W + 4.0, 2.0 * ROW + 2.0], 0x4030_3048);
        picker(draw, buttons, [x0, y, 112.0, h], WHENS[r.when].0, Do::When(n), mouse);
        let mut x = x0 + 115.0;
        if let Some(hint) = WHENS[r.when].1 {
            hint_field(draw, buttons, [x, y, 70.0, h], &r.when_arg, hint, c.focus == Some(base), Do::Field(base), mouse, blink);
            x += 73.0;
        }
        picker(draw, buttons, [x, y, x0 + W - 15.0 - x, h], CONDS[r.cond], Do::If(n), mouse);
        button(draw, buttons, [x0 + W - 12.0, y, 12.0, h], "x", Do::DelRule(n), mouse);
        y += ROW;
        draw.text("then", [x0, y + 3.0], grey, false);
        picker(draw, buttons, [x0 + 24.0, y, 82.0, h], DOS[r.then].0, Do::Then(n), mouse);
        if DOS[r.then].2 {
            hint_field(draw, buttons, [x0 + 108.0, y, 56.0, h], &r.then_arg, "text", c.focus == Some(base + 1), Do::Field(base + 1), mouse, blink);
        }
        if r.cond != 0 {
            draw.text("else", [x0 + 168.0, y + 3.0], grey, false);
            picker(draw, buttons, [x0 + 192.0, y, 82.0, h], DOS[r.otherwise].0, Do::Else(n), mouse);
            if DOS[r.otherwise].2 {
                hint_field(draw, buttons, [x0 + 276.0, y, 54.0, h], &r.otherwise_arg, "text", c.focus == Some(base + 2), Do::Field(base + 2), mouse, blink);
            }
        }
        y += ROW + 6.0;
    }
    if c.rules.len() < MAX_RULES {
        button(draw, buttons, [x0, y, 64.0, 14.0], "+ Rule", Do::AddRule, mouse);
    }
}

/// The shop page: a line an offer. [sells/buys] [item] [count] for [price] emeralds x.
fn draw_shop(draw: &mut dyn DrawContext, buttons: &mut Vec<([f32; 4], Do)>, c: &Creator, at: [f32; 2], mouse: [f32; 2], blink: bool) {
    let [x0, mut y] = at;
    let grey = 0xFFA0_A0A0;
    let note = if c.kept_shop.is_some() && c.offers.is_empty() {
        "Its file has a shop the page can't show: kept as it is".to_string()
    } else {
        "What it sells and buys, for emeralds (dearer for those it dislikes)".to_string()
    };
    draw.text(&note, [x0, y], grey, false);
    y += 12.0;
    let h = ROW - 2.0;
    for (n, o) in c.offers.iter().enumerate() {
        let base = SHOP_TEXT + n * 3;
        picker(draw, buttons, [x0, y, 48.0, h], if o.buys { "buys" } else { "sells" }, Do::SellBuy(n), mouse);
        hint_field(draw, buttons, [x0 + 51.0, y, 142.0, h], &o.item, "minecraft:bread", c.focus == Some(base), Do::Field(base), mouse, blink);
        hint_field(draw, buttons, [x0 + 196.0, y, 34.0, h], &o.count, "count", c.focus == Some(base + 1), Do::Field(base + 1), mouse, blink);
        draw.text("for", [x0 + 234.0, y + 3.0], grey, false);
        hint_field(draw, buttons, [x0 + 252.0, y, 34.0, h], &o.price, "price", c.focus == Some(base + 2), Do::Field(base + 2), mouse, blink);
        button(draw, buttons, [x0 + W - 12.0, y, 12.0, h], "x", Do::DelOffer(n), mouse);
        y += ROW;
    }
    if c.offers.len() < MAX_OFFERS {
        button(draw, buttons, [x0, y + 2.0, 64.0, 14.0], "+ Offer", Do::AddOffer, mouse);
    }
}

/// The quests page: two lines a quest. [kind] [its word] [count] x; says [offer] gives [reward].
fn draw_quests(draw: &mut dyn DrawContext, buttons: &mut Vec<([f32; 4], Do)>, c: &Creator, at: [f32; 2], mouse: [f32; 2], blink: bool) {
    let [x0, mut y] = at;
    let grey = 0xFFA0_A0A0;
    let note = if c.kept_quests.is_empty() {
        "What it asks players to do, and what it gives for it".to_string()
    } else {
        format!("Its quests ({} more from its file kept)", c.kept_quests.len())
    };
    draw.text(&note, [x0, y], grey, false);
    y += 12.0;
    let h = ROW - 2.0;
    for (n, q) in c.quests.iter().enumerate() {
        let base = QUEST_TEXT + n * 4;
        let focus = |k: usize| c.focus == Some(base + k);
        draw.fill([x0 - 2.0, y - 2.0, W + 4.0, 2.0 * ROW + 2.0], 0x4030_3048);
        picker(draw, buttons, [x0, y, 64.0, h], KINDS[q.kind].0, Do::Kind(n), mouse);
        hint_field(draw, buttons, [x0 + 67.0, y, 150.0, h], &q.what, KINDS[q.kind].1, focus(0), Do::Field(base), mouse, blink);
        if q.kind < 2 {
            hint_field(draw, buttons, [x0 + 220.0, y, 40.0, h], &q.count, "count", focus(1), Do::Field(base + 1), mouse, blink);
        }
        button(draw, buttons, [x0 + W - 12.0, y, 12.0, h], "x", Do::DelQuest(n), mouse);
        y += ROW;
        draw.text("says", [x0, y + 3.0], grey, false);
        hint_field(draw, buttons, [x0 + 26.0, y, 170.0, h], &q.offer, "what it asks (or its own words)", focus(2), Do::Field(base + 2), mouse, blink);
        draw.text("gives", [x0 + 200.0, y + 3.0], grey, false);
        hint_field(draw, buttons, [x0 + 228.0, y, W - 228.0, h], &q.reward, "item count", focus(3), Do::Field(base + 3), mouse, blink);
        y += ROW + 6.0;
    }
    if c.quests.len() < MAX_QUESTS {
        button(draw, buttons, [x0, y, 64.0, 14.0], "+ Quest", Do::AddQuest, mouse);
    }
}

fn cycle(i: usize, by: i32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (i as i64 + by as i64).rem_euclid(n as i64) as usize
}

fn act(host: &dyn ClientHost, c: &mut Creator, what: Do, by: i32) {
    match what {
        Do::Field(i) => c.focus = Some(i),
        Do::Model => c.model = cycle(c.model, by, c.models.len()),
        Do::Character => c.character = cycle(c.character, by, c.characters.len()),
        Do::Day => c.routine = cycle(c.routine, by, c.routines.len()),
        Do::Skin => {
            // The folder read again each time, so a skin just put there shows.
            if c.skin > 0 {
                let name = c.skins.get(c.skin - 1).map(|s| s.name.clone());
                c.skins = read_skins(host);
                c.skin = name.and_then(|n| c.skins.iter().position(|s| s.name == n)).map_or(0, |i| i + 1);
            } else {
                c.skins = read_skins(host);
            }
            c.skin = cycle(c.skin, by, c.skins.len() + 1);
        }
        Do::Save(spawn) => c.save(host, spawn),
        Do::Close => c.close = true,
        Do::Page(p) => {
            c.page = if c.page == p { 0 } else { p };
            c.focus = c.order().first().copied();
        }
        Do::Kind(n) => {
            if let Some(q) = c.quests.get_mut(n) {
                q.kind = cycle(q.kind, by, KINDS.len());
                q.what.clear();
            }
        }
        Do::DelQuest(n) => {
            if n < c.quests.len() {
                c.quests.remove(n);
                c.focus = None;
            }
        }
        Do::SellBuy(n) => {
            if let Some(o) = c.offers.get_mut(n) {
                o.buys = !o.buys;
            }
        }
        Do::DelOffer(n) => {
            if n < c.offers.len() {
                c.offers.remove(n);
                c.focus = None;
            }
        }
        Do::AddOffer => {
            if c.offers.len() < MAX_OFFERS {
                c.offers.push(shop::Row { buys: false, item: "minecraft:bread".into(), count: "3".into(), price: "1".into() });
            }
        }
        Do::AddQuest => {
            if c.quests.len() < MAX_QUESTS {
                c.quests.push(quest::Row { what: "minecraft:apple".into(), count: "3".into(), reward: "minecraft:emerald 1".into(), ..quest::Row::default() });
            }
        }
        Do::When(n) => {
            if let Some(r) = c.rules.get_mut(n) {
                r.when = cycle(r.when, by, WHENS.len());
            }
        }
        Do::If(n) => {
            if let Some(r) = c.rules.get_mut(n) {
                r.cond = cycle(r.cond, by, CONDS.len());
            }
        }
        Do::Then(n) => {
            if let Some(r) = c.rules.get_mut(n) {
                r.then = cycle(r.then, by, DOS.len());
            }
        }
        Do::Else(n) => {
            if let Some(r) = c.rules.get_mut(n) {
                r.otherwise = cycle(r.otherwise, by, DOS.len());
            }
        }
        Do::DelRule(n) => {
            if n < c.rules.len() {
                c.rules.remove(n);
                c.focus = None;
            }
        }
        Do::AddRule => {
            if c.rules.len() < MAX_RULES {
                // A start: when it sees a monster, it says so.
                c.rules.push(Row { then: 1, then_arg: "A @target!".into(), ..Row::default() });
            }
        }
    }
}

pub fn mouse_pressed(host: &dyn ClientHost, e: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = CREATOR.lock() else { return Flow::Continue };
    let Some(c) = lock.as_mut() else { return Flow::Continue };
    let mouse = e.screen.mouse;
    match c.buttons.iter().rev().find(|(r, _)| inside(*r, mouse)).map(|(_, w)| *w) {
        Some(what) => {
            if !matches!(what, Do::Field(_)) {
                host.play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0);
            }
            act(host, c, what, if e.button == 1 { -1 } else { 1 });
        }
        None => c.focus = None,
    }
    Flow::Cancel
}

pub fn mouse_scrolled(host: &dyn ClientHost, e: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = CREATOR.lock() else { return Flow::Continue };
    let Some(c) = lock.as_mut() else { return Flow::Continue };
    let mouse = e.screen.mouse;
    if let Some(what) = c.buttons.iter().find(|(r, w)| inside(*r, mouse) && matches!(w, Do::Model | Do::Skin | Do::Character | Do::Day | Do::SellBuy(_) | Do::Kind(_) | Do::When(_) | Do::If(_) | Do::Then(_) | Do::Else(_))).map(|(_, w)| *w) {
        act(host, c, what, if e.scroll[1] > 0.0 { -1 } else { 1 });
    }
    Flow::Cancel
}

fn type_text(c: &mut Creator, text: &str) {
    let Some(i) = c.focus else { return };
    let Some(field) = c.text(i) else { return };
    for ch in text.chars().filter(|ch| !ch.is_control()) {
        if field.chars().count() >= max_len(i) {
            break;
        }
        field.push(ch);
    }
}

pub fn char_typed(host: &dyn ClientHost, e: &mut ScreenCharTypedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = CREATOR.lock() else { return Flow::Continue };
    let Some(c) = lock.as_mut() else { return Flow::Continue };
    type_text(c, &e.c.to_string());
    Flow::Cancel
}

pub fn key_pressed(host: &dyn ClientHost, e: &mut ScreenKeyPressedPreEvent<'_>) -> Flow {
    if host.mod_screen() != Some(SCREEN) {
        return Flow::Continue;
    }
    let Ok(mut lock) = CREATOR.lock() else { return Flow::Continue };
    let Some(c) = lock.as_mut() else { return Flow::Continue };
    let ctrl = e.modifiers & 2 != 0;
    let shift = e.modifiers & 1 != 0;
    match e.key {
        // Escape goes to the screen, which closes.
        "key.keyboard.escape" => return Flow::Continue,
        "key.keyboard.backspace" => {
            if let Some(f) = c.focus.and_then(|i| c.text(i)) {
                if ctrl {
                    f.clear();
                } else {
                    f.pop();
                }
            }
        }
        "key.keyboard.tab" | "key.keyboard.enter" | "key.keyboard.keypad.enter" => {
            let order = c.order();
            if !order.is_empty() {
                let at = c.focus.and_then(|f| order.iter().position(|i| *i == f));
                let next = match at {
                    Some(p) if shift => (p + order.len() - 1) % order.len(),
                    Some(p) => (p + 1) % order.len(),
                    None => 0,
                };
                c.focus = Some(order[next]);
            }
        }
        "key.keyboard.v" if ctrl => {
            if let Some(text) = host.clipboard_text() {
                type_text(c, &text);
            }
        }
        "key.keyboard.s" if ctrl => c.save(host, false),
        _ => {}
    }
    // Every other key belongs to the typing (no hotbar keys, no inventory).
    Flow::Cancel
}
