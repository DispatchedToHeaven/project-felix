//! Project Felix: NPCs with Blockbench models and animations.
//!
//! - `geo`, `anim` and `molang` read Bedrock models, animations and Molang; they use no mod
//!   API types, so their tests run on their own.
//! - `models` and `animations` are the client's hold on them: every resource pack's files,
//!   reloaded with the assets.
//! - `sync` is what an entity plays, set on the server (`/felix animate`) and synced to clients.
//! - `runtime` plays the animations on the client: idle and walk, synced actions, blends, effects.
//! - `npc` is the NPC entity type (`projectfelix:npc`) and its renderer.
use mod_api::{
    AddClientReloadListenersEvent, ClientHost, ClientTickPostEvent, Flow, Hooks, RegisterCommandsEvent,
    RegisterEntityRenderersEvent, Registrar, RegistrationContext, RunModCommandEvent, ServerHost,
};

pub use mod_api::serde_json;

pub mod anim;
pub mod animations;
pub mod geo;
pub mod models;
pub mod molang;
pub mod npc;
pub mod runtime;
pub mod sync;

#[derive(Default)]
struct Felix;

#[mod_api::horizon_mod]
impl Hooks for Felix {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        runtime::register(r);
    }

    /// Models from `assets/<namespace>/geo/*.geo.json` and animations from
    /// `assets/<namespace>/animations/*.json`, read on every resource load.
    fn add_client_reload_listeners(&self, _host: &dyn ClientHost, e: &mut AddClientReloadListenersEvent<'_>) {
        models::add_listener(e);
        animations::add_listener(e);
    }

    fn register_entity_renderers(&self, _cx: &RegistrationContext, e: &mut RegisterEntityRenderersEvent<'_>) {
        npc::register_renderer(e);
    }

    fn register_commands(&self, _host: &mut dyn ServerHost, e: &mut RegisterCommandsEvent<'_>) {
        sync::register_commands(e);
    }

    fn run_mod_command(&self, host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
        sync::run_command(host, e)
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _e: &mut ClientTickPostEvent) {
        runtime::tick(host);
    }
}
