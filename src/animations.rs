//! The animations of every resource pack, by name (`animation.felix.walk`).
//!
//! Files: `assets/<namespace>/animations/*.json` (Blockbench's `.animation.json` export). A file
//! that won't read is logged and left out; the rest load.
use crate::anim::{self, Animation};
use mod_api::{
    AddClientReloadListenersEvent, ClientReloadContext, ClientReloadListener, FileToIdConverter, Identifier, ReloadApply,
    ReloadResourceError,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

static ANIMATIONS: LazyLock<RwLock<HashMap<String, Arc<Animation>>>> = LazyLock::new(Default::default);

/// The animation `name`, if one loaded.
pub fn get(name: &str) -> Option<Arc<Animation>> {
    ANIMATIONS.read().ok()?.get(name).cloned()
}

struct Listener;

impl ClientReloadListener for Listener {
    fn prepare(&self, cx: &ClientReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        for id in cx.list() {
            let Some(bytes) = cx.read(id) else { continue };
            match anim::parse(bytes) {
                Ok(list) => {
                    for a in list {
                        next.insert(a.name.clone(), Arc::new(a));
                    }
                }
                Err(e) => eprintln!("[projectfelix] animations {id} not loaded: {e}"),
            }
        }
        Ok(<dyn ReloadApply>::new(move || {
            if let Ok(mut live) = ANIMATIONS.write() {
                *live = next;
            }
        }))
    }
}

pub fn add_listener(e: &mut AddClientReloadListenersEvent<'_>) {
    let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:animations"), FileToIdConverter::json("animations"))
    else {
        return;
    };
    if let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener)) {
        eprintln!("[projectfelix] animation listener not added: {err:?}");
    }
}
