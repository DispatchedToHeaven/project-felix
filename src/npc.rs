//! The NPC: entity type `projectfelix:npc` (content file `data/projectfelix/horizon/entity_type/npc.json`,
//! a tame wolf's body) drawn with a Blockbench model and its animations.
use crate::companion::{DEFAULT_MODEL, Look};
use crate::{models, runtime};
use mod_api::{BlockEntityDrawContext, EntityRenderState, EntityRenderer, ModelPart, RegisterEntityRenderersEvent};
use std::collections::HashMap;
use std::sync::Arc;


struct NpcRenderer;

impl EntityRenderer for NpcRenderer {
    fn submit(&self, entity: &EntityRenderState, draw: &mut dyn BlockEntityDrawContext) -> Result<(), String> {
        let look = runtime::look(entity.id).unwrap_or_else(|| Look::of_model(DEFAULT_MODEL));
        // A look whose model isn't loaded falls back to the default one.
        let (look, model) = match models::get(&look.model) {
            Some(m) => (look, m),
            None => {
                let fallback = Look::of_model(DEFAULT_MODEL);
                // Nothing to draw with yet (resources still loading): draw nothing rather than
                // fail, which would switch the renderer off for every NPC.
                let Some(m) = models::get(&fallback.model) else { return Ok(()) };
                (fallback, m)
            }
        };
        let Some(mut living) = entity.living else { return Ok(()) };
        let now = draw.game_time() as f64 + draw.partial() as f64;
        // In a scene the NPC faces the way the scene began, whatever its body does.
        let scene = runtime::scene_of(entity.id, now);
        if let Some((yaw, _, _)) = &scene {
            living.body_rot = *yaw;
        }
        let table = runtime::table_of(entity.id);
        if let Some(t) = &table {
            living.body_rot = t.yaw;
        }
        let pose = runtime::pose(entity.id, &look.model, &living, entity.age, now);
        let mut root = model.root.clone();
        apply(&mut root, &pose);
        // Look where the head looks, on top of what the animations do.
        if let Some(head) = find_mut(&mut root, "head") {
            head.y_rot += living.y_rot.to_radians();
            // The model is drawn y up: looking down (positive pitch) turns the head the other way
            // from a vanilla model's (y down).
            head.x_rot -= living.x_rot.to_radians();
        }
        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], 180.0 - living.body_rot, [0.0; 3]);
        let s = living.scale * living.age_scale;
        draw.scale([s, s, s]);
        if let Some((_, scene, start)) = &scene {
            draw_props(draw, scene, ((now - start) / 20.0) as f32);
        }
        if let Some(t) = &table {
            draw_table(draw, t);
        }
        if living.death_time > 0.0 {
            let tip = ((living.death_time - 1.0) / 20.0 * 1.6).max(0.0).sqrt().min(1.0) * 90.0;
            draw.rotate_about([0.0, 0.0, 1.0], tip, [0.0; 3]);
        }
        let tint = if living.has_red_overlay { 0xFF9999 } else { 0xFFFFFF };
        // A texture the atlas doesn't have: the model's usual one instead.
        if !draw.model_part(&root, &look.texture, tint) {
            draw.model_part(&root, &Look::of_model(&look.model).texture, tint);
        }
        draw.pop();
        Ok(())
    }
}

/// A scene's props, at `t` seconds into it, in the NPC's frame (turned as the NPC, scaled as it).
fn draw_props(draw: &mut dyn BlockEntityDrawContext, scene: &crate::scene::Scene, t: f32) {
    for prop in &scene.props {
        let Some(model) = models::get(&prop.model) else { continue };
        let mut root = model.root.clone();
        if let Some(anim) = &prop.animation {
            apply(&mut root, &runtime::prop_pose(anim, t));
        }
        draw.push();
        draw.translate(crate::scene::prop_place(prop.offset));
        if prop.turn != 0.0 {
            draw.rotate_about([0.0, 1.0, 0.0], prop.turn, [0.0; 3]);
        }
        if !draw.model_part(&root, &prop.texture, 0xFFFFFF) {
            draw.model_part(&root, &Look::of_model(&prop.model).texture, 0xFFFFFF);
        }
        draw.pop();
    }
}

/// Where the table stands: in front of the NPC, between it and the player with white.
const TABLE_AHEAD: f32 = 1.2;

/// A game's table with its position. White sits across from the NPC, so from the NPC's frame
/// (front towards -z) rank 1 is the far side and the a-file on the NPC's right (+x), white's left.
fn draw_table(draw: &mut dyn BlockEntityDrawContext, t: &crate::games::Table) {
    let (Some(table), Some(pieces)) = (models::get("projectfelix:game_table"), models::get("projectfelix:game_pieces")) else { return };
    let texture = "projectfelix:entity/chess_table";
    draw.push();
    draw.translate([0.0, 0.0, -TABLE_AHEAD]);
    draw.model_part(&table.root, texture, 0xFFFFFF);
    for (sq, c) in t.board.chars().enumerate() {
        if c == '.' {
            continue;
        }
        let name = if t.kind == "checkers" {
            match c {
                'w' => "w_man",
                'W' => "w_dking",
                'b' => "b_man",
                _ => "b_dking",
            }
            .to_string()
        } else {
            format!("{}{}", if c.is_ascii_uppercase() { 'w' } else { 'b' }, c.to_ascii_lowercase())
        };
        let Some(part) = pieces.root.get_child(&name) else { continue };
        let (file, rank) = ((sq % 8) as f32, (sq / 8) as f32);
        draw.push();
        // Two texels a square, the board's surface 12.1 texels up; from the NPC's frame the a-file
        // is at +x and rank 1 at -z (towards white).
        draw.translate([(3.5 - file) * 2.0 / 16.0, 12.1 / 16.0, (rank - 3.5) * 2.0 / 16.0]);
        // Black's pieces face the NPC's side (+z) and white's the other way.
        if c.is_ascii_uppercase() {
            draw.rotate_about([0.0, 1.0, 0.0], 180.0, [0.0; 3]);
        }
        draw.model_part(part, texture, 0xFFFFFF);
        draw.pop();
    }
    draw.pop();
}

/// Each bone part moved, turned and scaled by its animated change (bones by lowercase name).
fn apply(part: &mut ModelPart, pose: &HashMap<String, runtime::Full>) {
    for (name, child) in part.children_mut() {
        if let Some(f) = pose.get(&name.to_ascii_lowercase()) {
            child.x += f.position[0];
            child.y += f.position[1];
            child.z += f.position[2];
            child.x_rot += f.rotation[0];
            child.y_rot += f.rotation[1];
            child.z_rot += f.rotation[2];
            child.x_scale *= f.scale[0];
            child.y_scale *= f.scale[1];
            child.z_scale *= f.scale[2];
        }
        apply(child, pose);
    }
}

fn find_mut<'a>(part: &'a mut ModelPart, name: &str) -> Option<&'a mut ModelPart> {
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
