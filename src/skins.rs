//! Skins of NPCs made in the game, on the client: asked of the server once each, decoded, and
//! turned into texel quads for each model they are drawn on (see `skin`).
use crate::custom::ASK;
use crate::models::{self, LoadedModel};
use crate::profile::Profile;
use crate::skin::{self, Texel};
use mod_api::{ClientHost, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// How long to wait (ticks) before asking for a file again.
const ASK_AGAIN: f64 = 200.0;

enum Skin {
    Asked(f64),
    Ready(Arc<(u32, u32, Vec<u8>)>),
}

#[derive(Default)]
struct Cache {
    skins: HashMap<String, Skin>,
    texels: HashMap<(String, String), Arc<Vec<Vec<Texel>>>>,
    ask: Vec<String>,
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);

/// The texel quads of NPC skin `id` on `model` (`model_id`), once the skin is here; asks for it
/// otherwise.
pub fn texels(model_id: &str, model: &LoadedModel, id: &str, clock: f64) -> Option<Arc<Vec<Vec<Texel>>>> {
    let mut c = CACHE.lock().ok()?;
    let key = (model_id.to_string(), id.to_string());
    if let Some(t) = c.texels.get(&key) {
        return Some(t.clone());
    }
    match c.skins.get(id) {
        Some(Skin::Ready(img)) => {
            let img = img.clone();
            let t = Arc::new(skin::texels(&model.geo, &skin::Image { width: img.0, height: img.1, rgba: &img.2 }));
            c.texels.insert(key, t.clone());
            Some(t)
        }
        Some(Skin::Asked(at)) if clock - at < ASK_AGAIN => None,
        _ => {
            c.skins.insert(id.to_string(), Skin::Asked(clock));
            c.ask.push(id.to_string());
            None
        }
    }
}

/// The client tick: asks the server for the files it lacks.
pub fn tick(host: &mut dyn ClientHost) {
    let ask = CACHE.lock().map(|mut c| std::mem::take(&mut c.ask)).unwrap_or_default();
    for id in ask {
        let _ = host.send_payload_to_server(&PayloadId(ASK.into()), PayloadBytes(id.into_bytes()));
    }
}

/// `npc_file` from the server: an NPC's file; its skin and model kept for drawing.
pub struct FileHandler;

impl IPayloadHandler for FileHandler {
    fn handle(&self, payload: PayloadBytes, _context: &mut dyn IPayloadContext) -> Result<(), String> {
        let p = match Profile::unpack(&payload.0, &crate::image::inflate) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[projectfelix] NPC file not read: {e}");
                return Ok(());
            }
        };
        if let Some(m) = p.model_file.as_ref().and_then(|m| models::load(m).ok()) {
            models::insert_named(&format!("npc:{}", p.id), m);
        }
        let Ok(img) = crate::image::decode_png(&p.skin) else { return Ok(()) };
        if let Ok(mut c) = CACHE.lock() {
            c.texels.retain(|(_, id), _| *id != p.id);
            c.skins.insert(p.id.clone(), Skin::Ready(Arc::new(img)));
        }
        Ok(())
    }
}
