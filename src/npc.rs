//! The NPC: entity type `projectfelix:npc` (content file `data/projectfelix/horizon/entity_type/npc.json`,
//! a tame wolf's body) drawn with a Blockbench model and its animations.
use crate::companion::{DEFAULT_MODEL, Look};
use crate::{models, runtime};
use mod_api::{BlockEntityDrawContext, EntityRenderState, EntityRenderer, RegisterEntityRenderersEvent};
use std::sync::Arc;

struct NpcRenderer;

impl EntityRenderer for NpcRenderer {
    fn submit(&self, entity: &EntityRenderState, draw: &mut dyn BlockEntityDrawContext) -> Result<(), String> {
        let now = draw.game_time() as f64 + draw.partial() as f64;
        let preview = crate::editor::preview(entity.id, now);
        let mut look = runtime::look(entity.id).unwrap_or_else(|| Look::of_model(DEFAULT_MODEL));
        // The editor shows the model and texture it is set to.
        if let Some(p) = &preview {
            look = Look { model: p.model.clone(), texture: p.texture.clone() };
        }
        // A look whose model isn't loaded falls back to the default one.
        let (look, model) = match models::get(&look.model) {
            Some(m) => (look, m),
            None => {
                // An NPC's own model comes in its file: asked of the server, the default drawn meanwhile.
                if let Some(id) = look.model.strip_prefix("npc:").or_else(|| look.texture.strip_prefix("felixnpc:")) {
                    crate::skins::request(id, now);
                }
                let fallback = Look::of_model(DEFAULT_MODEL);
                // Nothing to draw with yet (resources still loading): draw nothing rather than
                // fail, which would switch the renderer off for every NPC.
                let Some(m) = models::get(&fallback.model) else { return Ok(()) };
                (fallback, m)
            }
        };
        let Some(mut living) = entity.living else { return Ok(()) };
        // In a scene the NPC faces the way the scene began, whatever its body does.
        let scene = runtime::scene_of(entity.id, now);
        if let Some((yaw, _, _)) = &scene {
            living.body_rot = *yaw;
        }
        let table = runtime::table_of(entity.id);
        if let Some(t) = &table {
            living.body_rot = t.yaw;
        }
        let court = runtime::courts()
            .into_iter()
            .find(|(id, _)| *id == entity.id)
            .and_then(|(_, c)| Some((c.yaw, crate::sports::Match::from_json(&crate::serde_json::from_str(&c.json).ok()?)?)));
        if let Some((yaw, _)) = &court {
            living.body_rot = *yaw;
        }
        // In the editor the NPC holds still, turned as the view asks, its head straight.
        if let Some(p) = &preview {
            living.body_rot += p.turn;
            living.y_rot = 0.0;
            living.x_rot = 0.0;
        }
        let pose = runtime::pose(entity.id, &look.model, &living, entity.age, now);
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
        if let Some((_, m)) = &court {
            crate::sports_client::draw_court(draw, m, now);
        }
        if living.death_time > 0.0 {
            let tip = ((living.death_time - 1.0) / 20.0 * 1.6).max(0.0).sqrt().min(1.0) * 90.0;
            draw.rotate_about([0.0, 0.0, 1.0], tip, [0.0; 3]);
        }
        let tint = if living.has_red_overlay { 0xFF9999 } else { 0xFFFFFF };
        // Look where the head looks, on top of what the animations do. A texture the atlas
        // doesn't have: the model's usual one, then the default NPC's.
        let head = Some([living.y_rot, living.x_rot]);
        // What it wears and holds, on its bones.
        models::with_extras(crate::gear::extras(entity.id), || {
            // An NPC made in the game: its skin as texel quads, once the server has sent it.
            if let Some(id) = look.texture.strip_prefix("felixnpc:")
                && let Some(texels) = crate::skins::texels(&look.model, &model, id, now)
                && models::draw_texels(draw, &model, &pose, head, &texels, tint)
            {
                return;
            }
            if !models::draw(draw, &model, &pose, head, &look.texture, tint) && !models::draw(draw, &model, &pose, head, &Look::of_model(&look.model).texture, tint) {
                models::draw(draw, &model, &pose, head, &Look::of_model(DEFAULT_MODEL).texture, tint);
            }
        });
        draw.pop();
        // What it says, over its head.
        crate::speech::draw(draw, entity.id, 2.0 * living.scale * living.age_scale, now);
        Ok(())
    }
}

/// A scene's props, at `t` seconds into it, in the NPC's frame (turned as the NPC, scaled as it).
fn draw_props(draw: &mut dyn BlockEntityDrawContext, scene: &crate::scene::Scene, t: f32) {
    for prop in &scene.props {
        let Some(model) = models::get(&prop.model) else { continue };
        let pose = prop.animation.as_ref().map(|a| runtime::prop_pose(a, t)).unwrap_or_default();
        draw.push();
        draw.translate(crate::scene::prop_place(prop.offset));
        if prop.turn != 0.0 {
            draw.rotate_about([0.0, 1.0, 0.0], prop.turn, [0.0; 3]);
        }
        if !models::draw(draw, &model, &pose, None, &prop.texture, 0xFFFFFF) {
            models::draw(draw, &model, &pose, None, &Look::of_model(&prop.model).texture, 0xFFFFFF);
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

pub fn register_renderer(e: &mut RegisterEntityRenderersEvent<'_>) {
    e.renderers.register("projectfelix:npc".into(), Arc::new(NpcRenderer));
}
