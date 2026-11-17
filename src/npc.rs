//! The NPC: entity type `projectfelix:npc` (content file `data/projectfelix/horizon/entity_type/npc.json`,
//! a wandering trader's body for now) drawn with a Blockbench model.
use crate::models;
use mod_api::{BlockEntityDrawContext, EntityRenderState, EntityRenderer, RegisterEntityRenderersEvent};
use std::sync::Arc;

/// The model and texture every NPC has until NPCs get their own (0.3.0).
const MODEL: &str = "projectfelix:felix";
const TEXTURE: &str = "projectfelix:entity/felix";

struct NpcRenderer;

impl EntityRenderer for NpcRenderer {
    fn submit(&self, entity: &EntityRenderState, draw: &mut dyn BlockEntityDrawContext) -> Result<(), String> {
        let Some(model) = models::get(MODEL) else {
            return Err(format!("model {MODEL} is not loaded"));
        };
        let Some(living) = entity.living else { return Ok(()) };
        let mut root = model.root.clone();
        // Look where the head looks: Blockbench's usual bone name, any case.
        if let Some(head) = find_mut(&mut root, "head") {
            head.y_rot += living.y_rot.to_radians();
            head.x_rot += living.x_rot.to_radians();
        }
        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], 180.0 - living.body_rot, [0.0; 3]);
        let s = living.scale * living.age_scale;
        draw.scale([s, s, s]);
        if living.death_time > 0.0 {
            let tip = ((living.death_time - 1.0) / 20.0 * 1.6).max(0.0).sqrt().min(1.0) * 90.0;
            draw.rotate_about([0.0, 0.0, 1.0], tip, [0.0; 3]);
        }
        let tint = if living.has_red_overlay { 0xFF9999 } else { 0xFFFFFF };
        let drawn = draw.model_part(&root, TEXTURE, tint);
        draw.pop();
        if drawn { Ok(()) } else { Err(format!("texture {TEXTURE} is missing")) }
    }
}

fn find_mut<'a>(part: &'a mut mod_api::ModelPart, name: &str) -> Option<&'a mut mod_api::ModelPart> {
    let key = part.children().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(n, _)| n.to_string());
    if let Some(key) = key {
        return part.get_child_mut(&key);
    }
    for (_, child) in part.children_mut() {
        if let Some(found) = find_mut(child, name) {
            return Some(found);
        }
    }
    None
}

pub fn register_renderer(e: &mut RegisterEntityRenderersEvent<'_>) {
    e.renderers.register("projectfelix:npc".into(), Arc::new(NpcRenderer));
}
