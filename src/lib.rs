//! Project Felix: NPCs with Blockbench models and animations.
//!
//! - `geo`, `anim` and `molang` read Bedrock models, animations and Molang; they use no mod
//!   API types, so their tests run on their own.
//! - `models` and `animations` are the client's hold on them: every resource pack's files,
//!   reloaded with the assets.
//! - `sync` is what an entity plays, set on the server (`/felix animate`) and synced to clients.
//! - `runtime` plays the animations on the client: idle and walk, synced actions, blends, effects.
//! - `companion` is the NPC on the server: owner, orders (follow, wait, guard), look, goals, commands.
//! - `dialog` reads dialog trees (no mod API types); `talk` runs them on the server.
//! - `menu` is the talk screen on the client.
//! - `scene` reads scenes (no mod API types); `scenes` starts and ends them on the server.
//! - `chess` and `checkers` are the games' rules and engines (no mod API types); `games` runs
//!   them at an NPC's table on the server, `game_screen` is the board on the client.
//! - `sports` are the ball games' rules (no mod API types); `matches` runs them on the server,
//!   `sports_client` takes the keys and draws the HUD, the court and the ball.
//! - `anim_edit` is an animation as the editor holds it (no mod API types); `editor` is the
//!   animation editor in the world.
//! - `zipfile`, `skin` and `profile` are NPCs made in the game: their files, and skins as
//!   texel quads (no mod API types); `custom` keeps them on the server, `skins` draws their skins
//!   on the client, `creator` is the maker in the world.
//! - `npc` is the NPC entity type (`projectfelix:npc`) and its renderer.
use mod_api::{
    ScreenKeyPressedPreEvent, ScreenMouseButtonReleasedPreEvent, ScreenMouseDraggedPreEvent, ScreenMouseScrolledPreEvent,
    ScreenCharTypedPreEvent, AddClientReloadListenersEvent, AddServerReloadListenersEvent, GuiLayerDrawEvent, RegisterGuiLayersEvent, ClientHost, ClientTickPostEvent, EntityInteractEvent, EntitySizeEvent, Flow, Hooks,
    Host, HostSide, InteractionKeyMappingTriggeredEvent, InteractionResult, LevelTickPostEvent, RegisterCommandsEvent,
    RegisterEntityRenderersEvent, Registrar, RegistrationContext, RunModCommandEvent, ScreenMouseButtonPressedPreEvent,
    ScreenRenderPostEvent, ServerHost,
};

pub use mod_api::serde_json;

pub mod anim;
pub mod anim_edit;
pub mod checkers;
pub mod chess;
pub mod animations;
pub mod companion;
pub mod creator;
pub mod custom;
pub mod dialog;
pub mod editor;
pub mod game_screen;
pub mod games;
pub mod geo;
pub mod image;
pub mod matches;
pub mod menu;
pub mod models;
pub mod profile;
pub mod molang;
pub mod npc;
pub mod runtime;
pub mod scene;
pub mod scenes;
pub mod skin;
pub mod skins;
pub mod sports;
pub mod sports_client;
pub mod sync;
pub mod talk;
pub mod zipfile;

#[derive(Default)]
struct Felix;

#[mod_api::horizon_mod]
impl Hooks for Felix {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        runtime::register(r);
        companion::register(r);
        talk::register(r, std::sync::Arc::new(menu::ViewHandler), std::sync::Arc::new(menu::CloseHandler));
        games::register(r, std::sync::Arc::new(game_screen::ViewHandler));
        matches::register(r);
        sports_client::register(r);
        custom::register(r, std::sync::Arc::new(skins::FileHandler), std::sync::Arc::new(creator::OpenHandler));
    }

    /// Models from `assets/<namespace>/geo/*.geo.json` and animations from
    /// `assets/<namespace>/animations/*.json`, read on every resource load.
    fn add_client_reload_listeners(&self, _host: &dyn ClientHost, e: &mut AddClientReloadListenersEvent<'_>) {
        models::add_listener(e);
        animations::add_listener(e);
    }

    /// Dialogs and scenes from `data/<namespace>/felix_dialogs/` and `felix_scenes/`, read on every data reload.
    fn add_server_reload_listeners(&self, _host: &dyn Host, e: &mut AddServerReloadListenersEvent<'_>) {
        talk::add_listener(e);
        scenes::add_listener(e);
    }

    fn register_gui_layers(&self, _cx: &RegistrationContext, e: &mut RegisterGuiLayersEvent<'_>) {
        sports_client::register_layers(e);
    }

    fn gui_layer_draw(&self, host: &dyn ClientHost, e: &mut GuiLayerDrawEvent<'_>) {
        sports_client::draw_layer(host, e);
    }

    fn register_entity_renderers(&self, _cx: &RegistrationContext, e: &mut RegisterEntityRenderersEvent<'_>) {
        npc::register_renderer(e);
    }

    fn register_commands(&self, _host: &mut dyn ServerHost, e: &mut RegisterCommandsEvent<'_>) {
        sync::register_commands(e);
        companion::register_commands(e);
        custom::register_commands(e);
    }

    fn run_mod_command(&self, host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
        if let Flow::Cancel = custom::run_command(host, e) {
            return Flow::Cancel;
        }
        match companion::run_command(host, e) {
            Flow::Cancel => Flow::Cancel,
            _ => sync::run_command(host, e),
        }
    }

    fn level_tick_post(&self, host: HostSide, _e: &mut LevelTickPostEvent) {
        if let HostSide::Server(host) = host {
            companion::level_tick(host);
            talk::level_tick(host);
            scenes::level_tick(host);
            games::level_tick(host);
            matches::level_tick(host);
            custom::level_tick(host);
        }
    }

    fn entity_size(&self, host: &dyn Host, e: &mut EntitySizeEvent<'_>) {
        companion::entity_size(host, e);
    }

    fn entity_interact(&self, host: HostSide, e: &mut EntityInteractEvent<'_>) -> Option<InteractionResult> {
        menu::interact(host, e.entity, e.kind, e.hand)
    }

    fn interaction_key_mapping_triggered(&self, host: &mut dyn ClientHost, e: &mut InteractionKeyMappingTriggeredEvent) -> Flow {
        menu::use_key(host, e)
    }

    fn screen_render_post(&self, host: &dyn ClientHost, e: &mut ScreenRenderPostEvent<'_>) {
        if creator::active() {
            creator::render(host, e);
            return;
        }
        if editor::active() {
            editor::render(host, e);
            return;
        }
        game_screen::render(host, e);
        menu::render(host, e);
    }

    fn screen_mouse_button_pressed_pre(&self, host: &dyn ClientHost, e: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
        if creator::active() {
            return creator::mouse_pressed(host, e);
        }
        if editor::active() {
            return editor::mouse_pressed(host, e);
        }
        match game_screen::mouse_pressed(host, e) {
            Flow::Cancel => Flow::Cancel,
            _ => menu::mouse_pressed(host, e),
        }
    }

    fn screen_mouse_dragged_pre(&self, host: &dyn ClientHost, e: &mut ScreenMouseDraggedPreEvent<'_>) -> Flow {
        if creator::active() {
            Flow::Cancel
        } else if editor::active() {
            editor::mouse_dragged(host, e)
        } else {
            Flow::Continue
        }
    }

    fn screen_mouse_button_released_pre(&self, host: &dyn ClientHost, e: &mut ScreenMouseButtonReleasedPreEvent<'_>) -> Flow {
        if creator::active() {
            Flow::Cancel
        } else if editor::active() {
            editor::mouse_released(host, e)
        } else {
            Flow::Continue
        }
    }

    fn screen_mouse_scrolled_pre(&self, host: &dyn ClientHost, e: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow {
        if creator::active() {
            creator::mouse_scrolled(host, e)
        } else if editor::active() {
            editor::mouse_scrolled(host, e)
        } else {
            Flow::Continue
        }
    }

    fn screen_key_pressed_pre(&self, host: &dyn ClientHost, e: &mut ScreenKeyPressedPreEvent<'_>) -> Flow {
        if creator::active() {
            creator::key_pressed(host, e)
        } else if editor::active() {
            editor::key_pressed(host, e)
        } else {
            Flow::Continue
        }
    }

    fn screen_char_typed_pre(&self, host: &dyn ClientHost, e: &mut ScreenCharTypedPreEvent<'_>) -> Flow {
        if creator::active() { creator::char_typed(host, e) } else { Flow::Continue }
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _e: &mut ClientTickPostEvent) {
        runtime::tick(host);
        menu::tick(host);
        game_screen::tick(host);
        editor::tick(host);
        creator::tick(host);
        skins::tick(host);
        sports_client::tick(host);
    }
}

/// The model NPCs have by default (a function, so the pure modules can name it without the API).
pub fn companion_default_model() -> &'static str {
    "projectfelix:felix"
}
