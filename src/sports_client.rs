//! Ball games on the client: the three keys (Options > Controls: Z left, C right, X smash or
//! shoot), the score and prompts over the HUD for the player in a match, and the court and ball
//! the NPC's renderer draws.
use crate::matches::KEY;
use crate::sports::{Key, Match, Phase, Side, Sport};
use crate::{models, runtime};
use mod_api::{
    BlockEntityDrawContext, ClientHost, GuiLayer, GuiLayerDrawEvent, GuiLayerPlacement, KeyRegistration, PayloadBytes,
    PayloadId, RegisterGuiLayersEvent, Registrar,
};
use std::sync::{LazyLock, Mutex};

const MOD: &str = "projectfelix";
const LAYER: &str = "projectfelix:sport";
const KEYS: [(&str, &str, Key); 3] = [("left", "key.keyboard.z", Key::Left), ("right", "key.keyboard.c", Key::Right), ("smash", "key.keyboard.x", Key::Smash)];

pub fn register(r: &mut dyn Registrar) {
    for (id, key, _) in KEYS {
        r.register_key_mapping(KeyRegistration::new(id, key).label(&format!("key.projectfelix.{id}")).category("key.categories.projectfelix"));
    }
}

pub fn register_layers(e: &mut RegisterGuiLayersEvent<'_>) {
    e.layers.push(GuiLayer { id: LAYER.into(), placement: GuiLayerPlacement::AboveAll });
}

/// The match the local player plays now, its NPC's id, and the keys' names as bound.
#[derive(Default)]
struct Mine {
    npc: Option<u32>,
    names: [String; 3],
}

static MINE: LazyLock<Mutex<Mine>> = LazyLock::new(Default::default);

/// The client tick: which match is the player's, and their presses sent to the server.
pub fn tick(host: &mut dyn ClientHost) {
    let me = host.local_player().map(|p| format!("{:032x}", p.uuid()));
    let npc = me.and_then(|me| runtime::courts().into_iter().find(|(_, c)| c.player == me).map(|(id, _)| id));
    let mut names = [String::new(), String::new(), String::new()];
    for (i, (id, _, _)) in KEYS.iter().enumerate() {
        let key = host.mod_key(MOD, id).map(|(k, _)| k).unwrap_or_default();
        names[i] = key.rsplit('.').next().unwrap_or("?").to_uppercase();
    }
    // Presses always drain, so none waits for a later match.
    for (i, (id, _, _)) in KEYS.iter().enumerate() {
        while host.consume_click(MOD, id) {
            if let Some(npc) = npc {
                let mut b = npc.to_le_bytes().to_vec();
                b.push(i as u8);
                let _ = host.send_payload_to_server(&PayloadId(KEY.into()), PayloadBytes(b));
            }
        }
    }
    if let Ok(mut m) = MINE.lock() {
        m.npc = npc;
        m.names = names;
    }
}

fn centred(draw: &mut dyn mod_api::DrawContext, text: &str, y: f32, color: u32) {
    let [w, _] = draw.size();
    let tw = draw.text_width(text);
    draw.text(text, [(w - tw) / 2.0, y], color, true);
}

/// `gui_layer_draw`: the score and what to press, for the player in a match.
pub fn draw_layer(_host: &dyn ClientHost, e: &mut GuiLayerDrawEvent<'_>) {
    if e.layer != LAYER {
        return;
    }
    let (npc, names) = match MINE.lock() {
        Ok(m) => (m.npc, m.names.clone()),
        Err(_) => return,
    };
    let Some(npc) = npc else { return };
    let Some(court) = runtime::courts().into_iter().find(|(id, _)| *id == npc).map(|(_, c)| c) else { return };
    let Some(m) = crate::serde_json::from_str(&court.json).ok().and_then(|v| Match::from_json(&v)) else { return };
    let now = runtime::clock();
    let draw = &mut *e.draw;
    let [w, h] = draw.size();
    let title = match m.sport {
        Sport::TableTennis => "Table tennis",
        Sport::Tennis => "Tennis",
        Sport::Basketball => "Basketball",
    };
    draw.fill([w / 2.0 - 70.0, 4.0, 140.0, 26.0], 0x9010_1018);
    centred(draw, title, 7.0, 0xFFFF_D86A);
    centred(draw, &format!("You  {}  :  {}  NPC", m.score[0], m.score[1]), 18.0, 0xFFFF_FFFF);
    let key_name = |k: Key| match k {
        Key::Left => &names[0],
        Key::Right => &names[1],
        Key::Smash => &names[2],
    };
    let y = h * 0.62;
    match m.phase {
        Phase::Flight { from: Side::Npc, .. } => {
            if let Some((k, progress)) = m.prompt(now as i64) {
                let (text, color) = match k {
                    Key::Left => (format!("<<  LEFT  [{}]", key_name(k)), 0xFF60_C0FF),
                    Key::Right => (format!("[{}]  RIGHT  >>", key_name(k)), 0xFFFF_B060),
                    Key::Smash => (format!("^^  SMASH  [{}]  ^^", key_name(k)), 0xFFFF_6060),
                };
                centred(draw, &text, y, color);
                // How close the ball is: a bar filling up to the window, the window in green.
                let bw = 120.0;
                let x0 = w / 2.0 - bw / 2.0;
                draw.fill([x0, y + 12.0, bw, 4.0], 0x80FF_FFFF);
                let win = m.sport.window() as f32 / 20.0;
                let span = match m.phase {
                    Phase::Flight { launched, arrives, .. } => ((arrives - launched) as f32 / 20.0).max(0.1),
                    _ => 1.0,
                };
                // The bar spans the flight and half again; the window is around its arrival (1.0).
                let at = |p: f32| x0 + bw * (p / 1.5).clamp(0.0, 1.0);
                let (p0, p1) = ((1.0 - win / span).max(0.0), (1.0 + win / span).min(1.5));
                draw.fill([at(p0), y + 12.0, at(p1) - at(p0), 4.0], 0xC040_E040);
                draw.fill([at(progress) - 1.0, y + 10.0, 2.0, 8.0], 0xFFFF_FFFF);
            }
        }
        Phase::Serve { by: Side::Player, .. } => centred(draw, &format!("Your serve: [{}] or [{}]", names[0], names[1]), y, 0xFFFF_FFFF),
        Phase::Aim { at } if now as i64 >= at => {
            centred(draw, &format!("Shoot: [{}] at the top of the meter", names[2]), y, 0xFFFF_FFFF);
            let (bx, bh) = (w / 2.0 + 80.0, 60.0);
            let by = y - 40.0;
            draw.fill([bx, by, 8.0, bh], 0x80FF_FFFF);
            draw.fill([bx, by, 8.0, bh * 0.15], 0xC040_E040);
            let v = m.meter(now as i64);
            draw.fill([bx - 2.0, by + bh * (1.0 - v) - 1.0, 12.0, 2.0], 0xFFFF_FFFF);
        }
        Phase::Point { winner, .. } => {
            let (text, color) = match (m.sport, winner) {
                (Sport::Basketball, Some(Side::Player)) => ("Basket!", 0xFF60_FF60),
                (Sport::Basketball, Some(Side::Npc)) => ("The NPC scores.", 0xFFFF_8080),
                (Sport::Basketball, None) => ("Missed.", 0xFFC0_C0C0),
                (_, Some(Side::Player)) => ("Your point!", 0xFF60_FF60),
                _ => ("The NPC's point.", 0xFFFF_8080),
            };
            centred(draw, text, y, color);
        }
        Phase::Over { winner, .. } => {
            let text = if winner == Some(Side::Player) { "You win!" } else { "The NPC wins." };
            centred(draw, text, y, 0xFFFF_D86A);
        }
        _ => {}
    }
}

/// The court, its props and the ball, in the NPC's frame (front towards -z): `z` forwards in a
/// match's places becomes -z here.
pub fn draw_court(draw: &mut dyn BlockEntityDrawContext, m: &Match, now: f64) {
    let texture = "projectfelix:entity/sports";
    let prop = |draw: &mut dyn BlockEntityDrawContext, model: &str, at: [f32; 3]| {
        if let Some(model) = models::get(model) {
            draw.push();
            draw.translate([at[0], at[1], -at[2]]);
            draw.model_part(&model.root, texture, 0xFFFFFF);
            draw.pop();
        }
    };
    match m.sport {
        Sport::TableTennis => prop(draw, "projectfelix:table_tennis_table", [0.0, 0.0, 1.65]),
        Sport::Tennis => prop(draw, "projectfelix:tennis_court", [0.0, 0.0, 4.5]),
        Sport::Basketball => prop(draw, "projectfelix:hoop", [3.2, 0.0, 2.5]),
    }
    let Some(ball) = m.ball(now) else { return };
    let Some(balls) = models::get("projectfelix:balls") else { return };
    let name = match m.sport {
        Sport::TableTennis => "table_tennis",
        Sport::Tennis => "tennis",
        Sport::Basketball => "basketball",
    };
    if let Some(part) = balls.root.get_child(name) {
        draw.push();
        draw.translate([ball[0], ball[1], -ball[2]]);
        draw.model_part(part, texture, 0xFFFFFF);
        draw.pop();
    }
}
