//! Project Felix: NPCs with Blockbench models and animations.
//!
//! - `geo` reads Bedrock geometry (`.geo.json`) into a bone tree; it uses no mod API types,
//!   so its tests run on their own.
//! - `models` is the client's hold on it: the models of every resource pack, reloaded with
//!   the assets, as `ModelPart` trees the renderer draws.
//! - `npc` is the NPC entity type (`projectfelix:npc`) and its renderer.
use mod_api::{AddClientReloadListenersEvent, ClientHost, Hooks, RegisterEntityRenderersEvent, RegistrationContext};

pub use mod_api::serde_json;

pub mod geo;
pub mod models;
pub mod npc;

#[derive(Default)]
struct Felix;

#[mod_api::horizon_mod]
impl Hooks for Felix {
    /// Models from `assets/<namespace>/geo/*.geo.json`, read on every resource load.
    fn add_client_reload_listeners(&self, _host: &dyn ClientHost, e: &mut AddClientReloadListenersEvent<'_>) {
        models::add_listener(e);
    }

    fn register_entity_renderers(&self, _cx: &RegistrationContext, e: &mut RegisterEntityRenderersEvent<'_>) {
        npc::register_renderer(e);
    }
}
