// SPDX-License-Identifier: GPL-3.0-or-later

//! Save and world files as the Load and New Game lists show them (`UI-16`), for menus that run
//! before a world exists: whether the game would load a file, decided by the loaders' own header
//! checks.

use crate::simulation::save::check_save;
use crate::simulation::save::refusal::Refused;
use crate::simulation::world_definition::check_world_definition;
use godot::builtin::vdict;
use godot::prelude::*;
use std::path::Path;

/// Read-only verdicts on save and world files for the file lists.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct SaveFiles;

#[godot_api]
impl SaveFiles {
    /// Whether the game loads the file at the native `path`: a save, or a world definition when
    /// `world` is true. Returns an empty dictionary when it loads, else `reason` (the row's short
    /// text) and `message` (the load's own refusal). Reads the file's header only (O(1) pages);
    /// call it when a list is built, not per frame.
    #[func]
    pub fn verdict(path: GString, world: bool) -> VarDictionary {
        let path = path.to_string();
        let path = Path::new(&path);
        let checked = if world {
            check_world_definition(path)
        } else {
            check_save(path)
        };
        match checked {
            Ok(()) => VarDictionary::new(),
            Err(refused) => refusal_dictionary(&refused),
        }
    }
}

fn refusal_dictionary(refused: &Refused) -> VarDictionary {
    vdict! {
        "reason": refused.reason(),
        "message": refused.message.as_str(),
    }
}
