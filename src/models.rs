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
use std::sync::{Arc, LazyLock, Mutex, RwLock};

/// A bone ready to draw: where it sits in its parent (texels), its rest rotation (radians, z then
/// y then x), and its faces relative to its pivot.
pub struct BonePart {
    pub name: String,
    lower: String,
    offset: [f32; 3],
    rest: [f32; 3],
    part: ModelPart,
    /// The part without its front faces: a helmet's shell that leaves the face seen.
    open_front: ModelPart,
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

/// The player left the world: the NPCs' own models (`npc:<id>`) were that world's.
pub fn forget_npcs() {
    if let Ok(mut s) = SAVED.write() {
        s.retain(|k, _| !k.starts_with("npc:"));
    }
}

/// A model under the id `id` (an NPC's own: `npc:<id>`).
pub fn insert_named(id: &str, m: LoadedModel) {
    if let Ok(mut s) = SAVED.write() {
        s.insert(id.to_string(), Arc::new(m));
    }
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
                open_front: ModelPart::new(polygons(b).into_iter().filter(|q| q.normal[2] > -0.5).collect(), Vec::new(), PartPose::ZERO),
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
    bone_extras(draw, m, i);
    for &c in &b.children {
        draw_bone(draw, m, c, pose, look, texture, tint, ok);
    }
    draw.pop();
}

/// What an NPC wears and holds, drawn on its bones: armour as a coloured shell (bone, colour),
/// items at its hands.
#[derive(Clone, Default)]
pub struct Extras {
    pub armour: Vec<(&'static str, u32)>,
    pub hand: Option<mod_api::ItemStack>,
    pub off_hand: Option<mod_api::ItemStack>,
    pub seed: i32,
}

static EXTRAS: Mutex<Option<Extras>> = Mutex::new(None);

/// Draws with `extras` on the bones, for what `f` draws.
pub fn with_extras<R>(extras: Option<Extras>, f: impl FnOnce() -> R) -> R {
    if let Ok(mut e) = EXTRAS.lock() {
        *e = extras;
    }
    let r = f();
    if let Ok(mut e) = EXTRAS.lock() {
        *e = None;
    }
    r
}

/// A hand's place on bone `i`, in the bone's frame (texels): a locator, else the arm's end.
fn hand_on(m: &LoadedModel, i: usize, right: bool) -> Option<[f32; 3]> {
    let b = m.geo.bones.get(i)?;
    let locator = if right { "hand" } else { "left_hand" };
    if let Some((bone, at)) = m.geo.locator(locator)
        && bone == i
    {
        return Some([at[0] - b.pivot[0], at[1] - b.pivot[1], at[2] - b.pivot[2]]);
    }
    let arm = m.geo.bone(if right { "rightArm" } else { "leftArm" })?;
    let has_locator = m.geo.locator(locator).is_some();
    (arm == i && !has_locator).then_some([0.0, -10.0, 0.0])
}

fn bone_extras(draw: &mut dyn BlockEntityDrawContext, m: &LoadedModel, i: usize) {
    let Ok(guard) = EXTRAS.lock() else { return };
    let Some(x) = guard.as_ref() else { return };
    let b = &m.bones[i];
    for (bone, colour) in &x.armour {
        if b.lower == *bone && !b.part.polygons.is_empty() {
            draw.push();
            draw.scale([1.14, 1.06, 1.14]);
            draw.model_part(if b.lower == "head" { &b.open_front } else { &b.part }, WHITE, *colour);
            draw.pop();
        }
    }
    for (stack, right) in [(&x.hand, true), (&x.off_hand, false)] {
        let (Some(stack), Some(at)) = (stack, hand_on(m, i, right)) else { continue };
        draw.push();
        draw.translate([at[0] / 16.0, at[1] / 16.0, at[2] / 16.0]);
        draw.rotate_about([1.0, 0.0, 0.0], 90.0, [0.0; 3]);
        draw.rotate_about([0.0, 0.0, 1.0], 180.0, [0.0; 3]);
        let ctx = if right { mod_api::ItemDisplayContext::ThirdPersonRightHand } else { mod_api::ItemDisplayContext::ThirdPersonLeftHand };
        draw.item(stack, ctx, None, x.seed);
        draw.pop();
    }
}

/// The white sprite texel quads are drawn with, tinted.
pub const WHITE: &str = "projectfelix:entity/white";

/// Draws `model` posed by `pose` as `draw` does, its faces as the coloured texel quads of a
/// skin (`texels`, a list a bone in bone order) instead of a texture.
pub fn draw_texels(draw: &mut dyn BlockEntityDrawContext, model: &LoadedModel, pose: &HashMap<String, Full>, look: Option<[f32; 2]>, texels: &[Vec<crate::skin::Texel>], light: u32) -> bool {
    let mut ok = true;
    for &r in &model.roots {
        texel_bone(draw, model, r, pose, look, texels, light, &mut ok);
    }
    ok
}

#[allow(clippy::too_many_arguments)]
fn texel_bone(draw: &mut dyn BlockEntityDrawContext, m: &LoadedModel, i: usize, pose: &HashMap<String, Full>, look: Option<[f32; 2]>, texels: &[Vec<crate::skin::Texel>], tint: u32, ok: &mut bool) {
    let b = &m.bones[i];
    let f = pose.get(&b.lower).copied().unwrap_or_default();
    draw.push();
    draw.translate([(b.offset[0] + f.position[0]) / 16.0, (b.offset[1] + f.position[1]) / 16.0, (b.offset[2] + f.position[2]) / 16.0]);
    let mut r = [b.rest[0] + f.rotation[0], b.rest[1] + f.rotation[1], b.rest[2] + f.rotation[2]];
    if let Some([yaw, pitch]) = look.filter(|_| b.lower == "head") {
        r[1] += yaw.to_radians();
        r[0] -= pitch.to_radians();
    }
    for (axis, angle) in [([0.0, 0.0, 1.0], r[2]), ([0.0, 1.0, 0.0], r[1]), ([1.0, 0.0, 0.0], r[0])] {
        if angle != 0.0 {
            draw.rotate_about(axis, angle.to_degrees(), [0.0; 3]);
        }
    }
    if f.scale != [1.0; 3] {
        draw.scale(f.scale);
    }
    // A hurt NPC is drawn redder: the tint multiplies each texel's colour.
    let mul = |c: u32| {
        let ch = |shift: u32| (((c >> shift) & 0xFF) * ((tint >> shift) & 0xFF) / 255) << shift;
        ch(16) | ch(8) | ch(0)
    };
    for t in texels.get(i).map(|v| v.as_slice()).unwrap_or(&[]) {
        let c = t.corners.map(|p| [p[0] / 16.0, p[1] / 16.0, p[2] / 16.0]);
        if !draw.quad(WHITE, c, [0.0, 0.0, 1.0, 1.0], mul(t.rgb)) {
            *ok = false;
            break;
        }
    }
    bone_extras(draw, m, i);
    for &c in &b.children {
        texel_bone(draw, m, c, pose, look, texels, tint, ok);
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
