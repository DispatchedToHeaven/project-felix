//! The models of every resource pack, ready to draw, and the entity textures packs have.
//!
//! Files: `assets/<namespace>/geo/<name>.geo.json` (Blockbench's Bedrock export), the model
//! `<namespace>:<name>`; and the player's own in `horizon-mod-data/projectfelix/models/<name>.geo.json`,
//! the model `user:<name>`. A file that won't read is logged and left out; the rest load.
//!
//! A model keeps each bone's faces once, and [`draw`] poses the bones with the draw's pose stack,
//! so a frame copies nothing however many faces a model has.
use crate::geo;
use crate::runtime::Full;
use mod_api::{
    AddClientReloadListenersEvent, BlockEntityDrawContext, ClientReloadContext, ClientReloadListener, FileToIdConverter,
    Identifier, ModelPart, ModelPolygon, ModelVertex, PartPose, ReloadApply, ReloadResourceError,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

/// A bone ready to draw: where it sits in its parent (texels), its rest rotation (radians, z then
/// y then x), and its faces relative to its pivot.
pub struct BonePart {
    pub name: String,
    lower: String,
    offset: [f32; 3],
    rest: [f32; 3],
    part: ModelPart,
    children: Vec<usize>,
}

pub struct LoadedModel {
    /// The bones as a part tree, for drawing one bone on its own (`get_child`).
    pub root: ModelPart,
    /// The bones as read, for locators and the editor.
    pub geo: geo::Model,
    pub bones: Vec<BonePart>,
    roots: Vec<usize>,
    pub quads: usize,
}

static MODELS: LazyLock<RwLock<HashMap<String, Arc<LoadedModel>>>> = LazyLock::new(Default::default);
static SAVED: LazyLock<RwLock<HashMap<String, Arc<LoadedModel>>>> = LazyLock::new(Default::default);
static TEXTURES: LazyLock<RwLock<Vec<String>>> = LazyLock::new(Default::default);

/// The model `id` (`projectfelix:felix`, `user:mine`), if it loaded.
pub fn get(id: &str) -> Option<Arc<LoadedModel>> {
    if let Some(m) = SAVED.read().ok().and_then(|s| s.get(id).cloned()) {
        return Some(m);
    }
    MODELS.read().ok()?.get(id).cloned()
}

/// Every loaded model's id, sorted.
pub fn list() -> Vec<String> {
    let mut ids: Vec<String> = MODELS.read().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    ids.extend(SAVED.read().map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default());
    ids.sort();
    ids.dedup();
    ids
}

/// Every entity texture the packs have, as sprite names (`projectfelix:entity/felix`), sorted.
pub fn textures() -> Vec<String> {
    TEXTURES.read().map(|t| t.clone()).unwrap_or_default()
}

pub fn load(bytes: &[u8]) -> Result<LoadedModel, String> {
    let m = geo::parse(bytes)?.into_iter().next().ok_or("no geometry")?;
    Ok(build(m))
}

/// A model of the player's own, `user:<name>`.
pub fn insert_saved(name: &str, m: LoadedModel) {
    if let Ok(mut s) = SAVED.write() {
        s.insert(format!("user:{name}"), Arc::new(m));
    }
}

fn polygons(b: &geo::Bone) -> Vec<ModelPolygon> {
    b.quads.iter().map(|q| ModelPolygon::new(q.vertices.map(|v| ModelVertex::new(v.pos, v.u, v.v)), q.normal)).collect()
}

fn build(m: geo::Model) -> LoadedModel {
    let bones = m
        .bones
        .iter()
        .map(|b| {
            let parent = b.parent.map_or([0.0; 3], |p| m.bones[p].pivot);
            BonePart {
                name: b.name.clone(),
                lower: b.name.to_ascii_lowercase(),
                offset: [b.pivot[0] - parent[0], b.pivot[1] - parent[1], b.pivot[2] - parent[2]],
                rest: b.rotation,
                part: ModelPart::new(polygons(b), Vec::new(), PartPose::ZERO),
                children: b.children.clone(),
            }
        })
        .collect();
    LoadedModel { root: to_parts(&m), roots: m.roots.clone(), quads: m.quad_count(), bones, geo: m }
}

/// Draws `model` posed by `pose` (bones by lowercase name), the bone named `head` turned by
/// `look` (yaw and pitch, degrees) on top, textured with `texture`. False if the texture is unknown.
pub fn draw(draw: &mut dyn BlockEntityDrawContext, model: &LoadedModel, pose: &HashMap<String, Full>, look: Option<[f32; 2]>, texture: &str, tint: u32) -> bool {
    let mut ok = true;
    for &r in &model.roots {
        draw_bone(draw, model, r, pose, look, texture, tint, &mut ok);
    }
    ok
}

#[allow(clippy::too_many_arguments)]
fn draw_bone(draw: &mut dyn BlockEntityDrawContext, m: &LoadedModel, i: usize, pose: &HashMap<String, Full>, look: Option<[f32; 2]>, texture: &str, tint: u32, ok: &mut bool) {
    let b = &m.bones[i];
    let f = pose.get(&b.lower).copied().unwrap_or_default();
    draw.push();
    draw.translate([(b.offset[0] + f.position[0]) / 16.0, (b.offset[1] + f.position[1]) / 16.0, (b.offset[2] + f.position[2]) / 16.0]);
    let mut r = [b.rest[0] + f.rotation[0], b.rest[1] + f.rotation[1], b.rest[2] + f.rotation[2]];
    if let Some([yaw, pitch]) = look.filter(|_| b.lower == "head") {
        // The model is drawn y up: looking down (positive pitch) turns the head the other way
        // from a vanilla model's (y down).
        r[1] += yaw.to_radians();
        r[0] -= pitch.to_radians();
    }
    // As `ModelPart` turns: z, then y, then x.
    for (axis, angle) in [([0.0, 0.0, 1.0], r[2]), ([0.0, 1.0, 0.0], r[1]), ([1.0, 0.0, 0.0], r[0])] {
        if angle != 0.0 {
            draw.rotate_about(axis, angle.to_degrees(), [0.0; 3]);
        }
    }
    if f.scale != [1.0; 3] {
        draw.scale(f.scale);
    }
    if !b.part.polygons.is_empty() && !draw.model_part(&b.part, texture, tint) {
        *ok = false;
    }
    for &c in &b.children {
        draw_bone(draw, m, c, pose, look, texture, tint, ok);
    }
    draw.pop();
}

struct Listener;

impl ClientReloadListener for Listener {
    fn prepare(&self, cx: &ClientReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        for id in cx.list() {
            let text = id.to_string();
            // `geo/felix.geo.json` arrives as `felix.geo`; a plain `felix.json` is read too.
            let name = text.strip_suffix(".geo").unwrap_or(&text);
            let Some(bytes) = cx.read(id) else { continue };
            match load(bytes) {
                Ok(m) => {
                    next.insert(name.to_string(), Arc::new(m));
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

/// Lists the entity textures (`textures/entity/**.png`) without reading them.
struct Textures;

impl ClientReloadListener for Textures {
    fn prepare(&self, cx: &ClientReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next: Vec<String> = cx
            .list()
            .map(|id| {
                let t = id.to_string();
                let (ns, path) = t.split_once(':').unwrap_or(("minecraft", &t));
                format!("{ns}:entity/{path}")
            })
            .collect();
        next.sort();
        Ok(<dyn ReloadApply>::new(move || {
            if let Ok(mut live) = TEXTURES.write() {
                *live = next;
            }
        }))
    }
}

pub fn add_listener(e: &mut AddClientReloadListenersEvent<'_>) {
    if let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:models"), FileToIdConverter::json("geo"))
        && let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener))
    {
        eprintln!("[projectfelix] model listener not added: {err:?}");
    }
    if let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:textures"), FileToIdConverter::new("textures/entity", ".png"))
        && let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Textures))
    {
        eprintln!("[projectfelix] texture listener not added: {err:?}");
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
    let children = b.children.iter().map(|&c| (m.bones[c].name.clone(), bone_part(m, c, b.pivot))).collect();
    let pose = PartPose::offset_and_rotation(
        b.pivot[0] - parent_pivot[0],
        b.pivot[1] - parent_pivot[1],
        b.pivot[2] - parent_pivot[2],
        b.rotation[0],
        b.rotation[1],
        b.rotation[2],
    );
    ModelPart::new(polygons(b), children, pose)
}
