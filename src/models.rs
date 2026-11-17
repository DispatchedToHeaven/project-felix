//! The models of every resource pack, as `ModelPart` trees.
//!
//! Files: `assets/<namespace>/geo/<name>.geo.json` (Blockbench's Bedrock export). The model's
//! id is `<namespace>:<name>`. A file that won't read is logged and left out; the rest load.
use crate::geo;
use mod_api::{
    AddClientReloadListenersEvent, ClientReloadContext, ClientReloadListener, FileToIdConverter, Identifier, ModelPart,
    ModelPolygon, ModelVertex, PartPose, ReloadApply, ReloadResourceError,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

/// A loaded model: its bones as a part tree (the root draws nothing itself) and what it came from.
pub struct LoadedModel {
    pub root: ModelPart,
    /// The bones as read, for locators.
    pub geo: geo::Model,
    pub texture_width: u32,
    pub texture_height: u32,
    pub quads: usize,
}

static MODELS: LazyLock<RwLock<HashMap<String, Arc<LoadedModel>>>> = LazyLock::new(Default::default);

/// The model `id` (`projectfelix:felix`), if it loaded.
pub fn get(id: &str) -> Option<Arc<LoadedModel>> {
    MODELS.read().ok()?.get(id).cloned()
}

struct Listener;

impl ClientReloadListener for Listener {
    fn prepare(&self, cx: &ClientReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        for id in cx.list() {
            let text = id.to_string();
            // `geo/felix.geo.json` arrives as `felix.geo`; a plain `felix.json` is read too.
            let Some(name) = text.strip_suffix(".geo").or(Some(text.as_str())) else { continue };
            let Some(bytes) = cx.read(id) else { continue };
            match geo::parse(bytes) {
                Ok(models) => {
                    let m = models.into_iter().next().unwrap();
                    let loaded = LoadedModel {
                        root: to_parts(&m),
                        texture_width: m.texture_width,
                        texture_height: m.texture_height,
                        quads: m.quad_count(),
                        geo: m,
                    };
                    eprintln!("[projectfelix] model {name}: {} bones, {} faces", loaded.geo.bones.len(), loaded.quads);
                    next.insert(name.to_string(), Arc::new(loaded));
                }
                Err(e) => eprintln!("[projectfelix] model {name} not loaded: {e}"),
            }
        }
        Ok(<dyn ReloadApply>::new(move || {
            if let Ok(mut live) = MODELS.write() {
                *live = next;
            }
        }))
    }
}

pub fn add_listener(e: &mut AddClientReloadListenersEvent<'_>) {
    let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:models"), FileToIdConverter::json("geo")) else {
        return;
    };
    if let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener)) {
        eprintln!("[projectfelix] model listener not added: {err:?}");
    }
}

/// The bone tree as parts: each part sits at its bone's pivot in its parent's frame, turned by
/// the bone's rest rotation, and its faces are relative to the pivot, so the parts draw as
/// GeckoLib draws the bones.
pub fn to_parts(m: &geo::Model) -> ModelPart {
    let children = m.roots.iter().map(|&i| (m.bones[i].name.clone(), bone_part(m, i, [0.0; 3]))).collect();
    ModelPart::new(Vec::new(), children, PartPose::ZERO)
}

fn bone_part(m: &geo::Model, i: usize, parent_pivot: [f32; 3]) -> ModelPart {
    let b = &m.bones[i];
    let polygons = b
        .quads
        .iter()
        .map(|q| {
            let v = q.vertices.map(|v| ModelVertex::new(v.pos, v.u, v.v));
            ModelPolygon::new(v, q.normal)
        })
        .collect();
    let children = b.children.iter().map(|&c| (m.bones[c].name.clone(), bone_part(m, c, b.pivot))).collect();
    let pose = PartPose::offset_and_rotation(
        b.pivot[0] - parent_pivot[0],
        b.pivot[1] - parent_pivot[1],
        b.pivot[2] - parent_pivot[2],
        b.rotation[0],
        b.rotation[1],
        b.rotation[2],
    );
    ModelPart::new(polygons, children, pose)
}
