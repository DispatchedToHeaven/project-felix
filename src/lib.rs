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
//! - `rules` reads NPC rules (event, if, then, else; no mod API types); `rulebook` runs them on
//!   the server.
//! - `character` reads characters (a graph of moods, relations; no mod API types); `heart` keeps
//!   each NPC's mood and relations on the server and makes it obey worse when upset.
//! - `quest` reads quests (no mod API types); `quests` runs them on the server: boards, logs, kills.
//! - `routine` reads days (what an NPC does from what hour; no mod API types); `routines` has NPCs
//!   live by them on the server: work, meet, home, sleep, guard, follow.
//! - `char_edit` is a character as its editor holds it (no mod API types); `char_editor` is the
//!   character editor in the world: moods as boxes, ways as lines.
//! - `shop` reads shops (no mod API types); `shops` runs NPCs' stalls on the server.
//! - `gear` is what companions wear and hold, and how they grow with what they kill.
//! - `scene_edit` is a scene as its editor holds it (no mod API types); `scene_editor` is the scene
//!   editor in the world: the stage from above, the roles, the timeline.
//! - `speech` shows what NPCs say over their heads (and copies some of it to chat).
//! - `event` is the village calendar (no mod API types); `events` gathers NPCs for its days.
//! - `need` is NPCs' food (no mod API types); `needs` feeds them and has them say when hungry.
//! - `remark` is what companions say on the way (no mod API types); `companions` has them say it
//!   and help their hurt player.
//! - `bond` is how close an NPC and a player are, up to romance (no mod API types); `bonds` keeps it.
//! - `family` is NPC families (no mod API types); `families` makes couples and their children.
//! - `squad` is a player's squad of NPCs: keys to attack, fall back, hold; archers shoot.
//! - `plan` is a building plan (no mod API types); `builds` has builders build them.
//! - `job` is what NPCs do at work (no mod API types); `jobs` has them farm, fell trees and fish.
//! - `voice` is how NPCs babble what they say (no mod API types); `voices` gives them theirs.
//! - `dialog_edit` (no mod API types) and `dialog_editor` are the dialog editor; `dialogs` keeps
//!   dialogs made in the game; `flags` is the players' talk memory and the extended view.
//! - `saves`: what is made in the game is kept with its world.
//! - `settle` (no mod API types) and `settlements` are the Felix settlements the world makes.
//! - `market` (no mod API types) and `markets` are the village market: purses, the baker, trade
//!   between NPCs, prices by supply.
//! - `book` (no mod API types), `books` and `book_screen` are the village book (key K).
//! - `quest_edit` (no mod API types) and `quest_editor` are the quest editor; `quest_store` keeps
//!   quests made in the game.
//! - `rules_edit` (no mod API types) and `rules_editor` are the rules editor; `rulesets` keeps
//!   rules made in the game.
//! - `village` is a village as a file (no mod API types); `villages` exports and imports them.
//! - `npc` is the NPC entity type (`projectfelix:npc`) and its renderer.
use mod_api::{
    ScreenKeyPressedPreEvent, ScreenMouseButtonReleasedPreEvent, ScreenMouseDraggedPreEvent, ScreenMouseScrolledPreEvent,
    ScreenCharTypedPreEvent, ChunkLoadEvent, ServerTickEvent, ServerTickPhase, LivingDamagePostEvent, LivingDamagePreEvent, LivingDeathEvent, ServerChatEvent, LevelRandomHost, AddClientReloadListenersEvent, AddServerReloadListenersEvent, GuiLayerDrawEvent, RegisterGuiLayersEvent, ClientHost, ClientTickPostEvent, EntityInteractEvent, EntitySizeEvent, Flow, Hooks,
    Host, HostSide, InteractionKeyMappingTriggeredEvent, InteractionResult, LevelTickPostEvent, RegisterCommandsEvent,
    RegisterEntityRenderersEvent, Registrar, RegistrationContext, RunModCommandEvent, ScreenMouseButtonPressedPreEvent,
    ScreenRenderPostEvent, ServerHost,
};

pub use mod_api::serde_json;

pub mod anim;
pub mod anim_edit;
pub mod bond;
pub mod book;
pub mod market;
pub mod markets;
pub mod book_screen;
pub mod books;
pub mod bonds;
pub mod builds;
pub mod checkers;
pub mod chess;
pub mod animations;
pub mod char_edit;
pub mod char_editor;
pub mod character;
pub mod companion;
pub mod companions;
pub mod creator;
pub mod custom;
pub mod dialog;
pub mod dialog_edit;
pub mod dialog_editor;
pub mod dialogs;
pub mod editor;
pub mod event;
pub mod flags;
pub mod events;
pub mod families;
pub mod family;
pub mod game_screen;
pub mod games;
pub mod gear;
pub mod heart;
pub mod need;
pub mod needs;
pub mod job;
pub mod jobs;
pub mod geo;
pub mod image;
pub mod matches;
pub mod menu;
pub mod models;
pub mod profile;
pub mod quest;
pub mod quest_edit;
pub mod quest_editor;
pub mod quest_store;
pub mod remark;
pub mod quests;
pub mod molang;
pub mod plan;
pub mod npc;
pub mod routine;
pub mod routines;
pub mod rulebook;
pub mod rules;
pub mod rules_edit;
pub mod rules_editor;
pub mod rulesets;
pub mod runtime;
pub mod scene;
pub mod scene_edit;
pub mod scene_editor;
pub mod scenes;
pub mod shop;
pub mod shops;
pub mod skin;
pub mod skins;
pub mod speech;
pub mod saves;
pub mod settle;
pub mod settlements;
pub mod squad;
pub mod sports;
pub mod sports_client;
pub mod sync;
pub mod talk;
pub mod village;
pub mod villages;
pub mod voice;
pub mod voices;
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
        char_editor::register(r);
        dialog_editor::register(r);
        rules_editor::register(r);
        quest_editor::register(r);
        book_screen::register(r);
        scene_editor::register(r);
        squad::register(r);
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
        rulebook::add_listener(e);
        heart::add_listener(e);
        quests::add_listener(e);
        routines::add_listener(e);
        shops::add_listener(e);
        events::add_listener(e);
        builds::add_listener(e);
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
        rulebook::register_commands(e);
        heart::register_commands(e);
        quests::register_commands(e);
        routines::register_commands(e);
        shops::register_commands(e);
        gear::register_commands(e);
        speech::register_commands(e);
        villages::register_commands(e);
        voices::register_commands(e);
        jobs::register_commands(e);
        events::register_commands(e);
        flags::register_commands(e);
        dialogs::register_commands(e);
        rulesets::register_commands(e);
        needs::register_commands(e);
        builds::register_commands(e);
        squad::register_commands(e);
        families::register_commands(e);
        bonds::register_commands(e);
        quest_store::register_commands(e);
        books::register_commands(e);
        markets::register_commands(e);
        settlements::register_commands(e);
        scenes::register_commands(e);
    }

    fn run_mod_command(&self, host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
        if let Flow::Cancel = custom::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = settlements::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = markets::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = books::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = quest_store::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = bonds::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = families::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = squad::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = builds::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = needs::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = rulesets::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = dialogs::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = flags::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = events::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = jobs::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = voices::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = villages::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = speech::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = scenes::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = gear::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = shops::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = routines::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = quests::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = heart::run_command(host, e) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = rulebook::run_command(host, e) {
            return Flow::Cancel;
        }
        match companion::run_command(host, e) {
            Flow::Cancel => Flow::Cancel,
            _ => sync::run_command(host, e),
        }
    }

    fn chunk_load(&self, host: &mut dyn ServerHost, e: &mut ChunkLoadEvent<'_>) {
        settlements::chunk_load(host, e.dimension, e.chunk, e.new);
    }

    fn level_tick_post(&self, host: HostSide, _e: &mut LevelTickPostEvent) {
        if let HostSide::Server(host) = host {
            companion::level_tick(host);
            talk::level_tick(host);
            scenes::level_tick(host);
            games::level_tick(host);
            matches::level_tick(host);
            custom::level_tick(host);
            heart::level_tick(host);
            gear::level_tick(host);
            villages::level_tick(host);
            dialogs::level_tick(host);
            rulesets::level_tick(host);
            quest_store::level_tick(host);
            settlements::level_tick(host);
            builds::level_tick(host);
        }
    }

    fn server_tick(&self, host: &mut dyn ServerHost, e: &mut ServerTickEvent<'_>) {
        if let ServerTickPhase::ServerTickEnd { world_running: true } = e.phase {
            heart::tick(host);
            quests::tick(host);
            routines::tick(host);
            jobs::tick(host);
            markets::tick(host);
            events::tick(host);
            needs::tick(host);
            squad::tick(host);
            families::tick(host);
            bonds::tick(host);
            companions::tick(host);
            shops::tick(host);
            speech::tick(host);
            rulebook::tick(host);
        }
    }

    /// Armour an NPC wears takes its share of a hit (the game doesn't count an NPC's armour).
    fn living_damage_pre(&self, _level: &mut dyn LevelRandomHost, e: &mut LivingDamagePreEvent<'_>) {
        gear::on_damage_pre(e);
    }

    fn living_damage_post(&self, _level: &mut dyn LevelRandomHost, e: &mut LivingDamagePostEvent<'_>) {
        rulebook::on_hurt(e.entity, e.source.causing);
        heart::on_hurt(e.entity, e.source.causing);
        routines::on_hurt(e.entity);
    }

    fn living_death(&self, host: &mut dyn ServerHost, e: &mut LivingDeathEvent<'_>) -> Flow {
        rulebook::on_death(&*host, e.entity, e.source.causing);
        heart::on_death(host, e.entity, e.source.causing);
        quests::on_death(host, e.entity, e.source.causing);
        gear::on_death(host, e.entity, e.source.causing);
        bonds::on_death(host, e.entity, e.source.causing);
        Flow::Continue
    }

    fn server_chat(&self, _host: &mut dyn ServerHost, e: &mut ServerChatEvent<'_>) -> Flow {
        rulebook::on_chat(e.player_id, e.raw_text);
        Flow::Continue
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
        if book_screen::active() {
            book_screen::render(host, e);
            return;
        }
        if quest_editor::active() {
            quest_editor::render(host, e);
            return;
        }
        if rules_editor::active() {
            rules_editor::render(host, e);
            return;
        }
        if dialog_editor::active() {
            dialog_editor::render(host, e);
            return;
        }
        if scene_editor::active() {
            scene_editor::render(host, e);
            return;
        }
        if char_editor::active() {
            char_editor::render(host, e);
            return;
        }
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
        if book_screen::active() {
            return book_screen::mouse_pressed(host, e);
        }
        if quest_editor::active() {
            return quest_editor::mouse_pressed(host, e);
        }
        if rules_editor::active() {
            return rules_editor::mouse_pressed(host, e);
        }
        if dialog_editor::active() {
            return dialog_editor::mouse_pressed(host, e);
        }
        if scene_editor::active() {
            return scene_editor::mouse_pressed(host, e);
        }
        if char_editor::active() {
            return char_editor::mouse_pressed(host, e);
        }
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
        if dialog_editor::active() {
            dialog_editor::mouse_dragged(host, e)
        } else if scene_editor::active() {
            scene_editor::mouse_dragged(host, e)
        } else if char_editor::active() {
            char_editor::mouse_dragged(host, e)
        } else if creator::active() {
            Flow::Cancel
        } else if editor::active() {
            editor::mouse_dragged(host, e)
        } else {
            Flow::Continue
        }
    }

    fn screen_mouse_button_released_pre(&self, host: &dyn ClientHost, e: &mut ScreenMouseButtonReleasedPreEvent<'_>) -> Flow {
        if dialog_editor::active() {
            dialog_editor::mouse_released(host)
        } else if scene_editor::active() {
            scene_editor::mouse_released(host)
        } else if char_editor::active() {
            char_editor::mouse_released(host)
        } else if creator::active() {
            Flow::Cancel
        } else if editor::active() {
            editor::mouse_released(host, e)
        } else {
            Flow::Continue
        }
    }

    fn screen_mouse_scrolled_pre(&self, host: &dyn ClientHost, e: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow {
        if book_screen::active() {
            book_screen::mouse_scrolled(host, e.scroll[1] > 0.0)
        } else if quest_editor::active() {
            quest_editor::mouse_scrolled(host, e.screen.mouse, e.scroll[1] > 0.0)
        } else if rules_editor::active() {
            rules_editor::mouse_scrolled(host, e.screen.mouse, e.scroll[1] > 0.0)
        } else if dialog_editor::active() {
            dialog_editor::mouse_scrolled(host, e.screen.mouse, e.scroll[1] > 0.0)
        } else if scene_editor::active() {
            scene_editor::mouse_scrolled(host, e.screen.mouse, e.scroll[1] > 0.0)
        } else if char_editor::active() {
            char_editor::mouse_scrolled(host, e.screen.mouse, e.scroll[1] > 0.0)
        } else if creator::active() {
            creator::mouse_scrolled(host, e)
        } else if editor::active() {
            editor::mouse_scrolled(host, e)
        } else {
            Flow::Continue
        }
    }

    fn screen_key_pressed_pre(&self, host: &dyn ClientHost, e: &mut ScreenKeyPressedPreEvent<'_>) -> Flow {
        if book_screen::active() {
            book_screen::key_pressed(host, e)
        } else if quest_editor::active() {
            quest_editor::key_pressed(host, e)
        } else if rules_editor::active() {
            rules_editor::key_pressed(host, e)
        } else if dialog_editor::active() {
            dialog_editor::key_pressed(host, e)
        } else if scene_editor::active() {
            scene_editor::key_pressed(host, e)
        } else if char_editor::active() {
            char_editor::key_pressed(host, e)
        } else if creator::active() {
            creator::key_pressed(host, e)
        } else if editor::active() {
            editor::key_pressed(host, e)
        } else {
            Flow::Continue
        }
    }

    fn screen_char_typed_pre(&self, host: &dyn ClientHost, e: &mut ScreenCharTypedPreEvent<'_>) -> Flow {
        if quest_editor::active() {
            quest_editor::char_typed(host, e)
        } else if rules_editor::active() {
            rules_editor::char_typed(host, e)
        } else if dialog_editor::active() {
            dialog_editor::char_typed(host, e)
        } else if scene_editor::active() {
            scene_editor::char_typed(host, e)
        } else if char_editor::active() {
            char_editor::char_typed(host, e)
        } else if creator::active() {
            creator::char_typed(host, e)
        } else {
            Flow::Continue
        }
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _e: &mut ClientTickPostEvent) {
        runtime::tick(host);
        gear::client_tick(&*host);
        speech::client_tick(&*host);
        menu::tick(host);
        game_screen::tick(host);
        editor::tick(host);
        creator::tick(host);
        char_editor::tick(host);
        dialog_editor::tick(host);
        rules_editor::tick(host);
        quest_editor::tick(host);
        book_screen::tick(host);
        squad::client_tick(host);
        scene_editor::tick(host);
        skins::tick(host);
        sports_client::tick(host);
    }
}

/// The model NPCs have by default (a function, so the pure modules can name it without the API).
pub fn companion_default_model() -> &'static str {
    "projectfelix:felix"
}
