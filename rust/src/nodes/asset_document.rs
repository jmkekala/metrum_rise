// SPDX-License-Identifier: GPL-2.0-only

//! Typed Godot boundary for Rust-owned authoring history; no JSON round trips on edits.
//! Deep-copy only when dictionaries enter or leave the immutable document store.

use crate::assets::authoring::document::{Document, HISTORY_LIMIT};
use godot::prelude::*;

mod commands;

/// Asset document and command history, independent of widgets and preview resources.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct AssetAuthoringDocument {
    base: Base<RefCounted>,
    core: Document<VarDictionary>,
    /// Last successfully saved authoring draft path.
    draft_path: GString,
}

#[godot_api]
impl AssetAuthoringDocument {
    /// Last successfully saved authoring draft path; publication never changes it.
    #[func]
    pub fn draft_path(&self) -> GString {
        self.draft_path.clone()
    }

    /// Maximum retained undo commands.
    #[constant]
    pub const HISTORY_LIMIT: i32 = HISTORY_LIMIT as i32;

    /// Emitted after releasing the mutable bridge borrow so UI callbacks may read the document.
    #[signal]
    fn changed();

    /// Return an independent, typed snapshot; callers cannot mutate history through it.
    #[func]
    pub fn snapshot(&self) -> VarDictionary {
        self.core.data().duplicate_deep()
    }

    /// Inspect only selected entries, without copying the document or importing resources.
    #[func]
    pub fn selection_actions(&self, targets: Array<VarDictionary>) -> VarDictionary {
        commands::capabilities(self.core.data(), &targets)
    }

    /// Compare the current publication origin without copying the document for a library menu.
    #[func]
    pub fn has_publication_origin(&self, pack: GString, asset: GString) -> bool {
        self.core
            .data()
            .get("origin")
            .and_then(|v| v.try_to::<VarDictionary>().ok())
            .is_some_and(|origin| {
                origin.get("pack_id") == Some(pack.to_variant())
                    && origin.get("asset_id") == Some(asset.to_variant())
            })
    }

    /// Working and undo-retained file references that library trash must not invalidate.
    #[func]
    pub fn protected_sources(&self) -> PackedStringArray {
        let mut paths = std::collections::BTreeSet::new();
        for state in self.core.revisions() {
            for key in ["supporting_sources", "colour_sources"] {
                if let Some(sources) = state
                    .get(key)
                    .and_then(|v| v.try_to::<VarDictionary>().ok())
                {
                    for (_, path) in sources.iter_shared() {
                        if let Ok(path) = path.try_to::<GString>() {
                            paths.insert(path.to_string());
                        }
                    }
                }
            }
            if let Some(sources) = state
                .get("sources")
                .and_then(|v| v.try_to::<VarArray>().ok())
            {
                for part in sources
                    .iter_shared()
                    .filter_map(|v| v.try_to::<VarArray>().ok())
                {
                    for path in part
                        .iter_shared()
                        .filter_map(|v| v.try_to::<GString>().ok())
                    {
                        paths.insert(path.to_string());
                    }
                }
            }
            if let Some(path) = state
                .get("thumbnail_source")
                .and_then(|v| v.try_to::<GString>().ok())
            {
                paths.insert(path.to_string());
            }
        }
        PackedStringArray::from_iter(paths.iter().map(|p| GString::from(p.as_str())))
    }

    /// Prepare one reversible authored edit; callers preview placement before applying it.
    /// Copies the document once at a command boundary, never on pointer motion.
    #[func]
    pub fn prepare_edit(
        &self,
        action: GString,
        targets: Array<VarDictionary>,
        args: VarDictionary,
    ) -> VarDictionary {
        commands::prepare(self.core.data(), &action.to_string(), &targets, &args)
    }

    /// Replace the document and discard old history, preserving unknown metadata.
    #[func(gd_self)]
    pub fn reset(
        mut this: Gd<Self>,
        data: VarDictionary,
        #[opt(default = true)] saved: bool,
        #[opt(default = "")] path: GString,
    ) {
        {
            let mut doc = this.bind_mut();
            doc.core.reset(data.duplicate_deep(), saved);
            doc.draft_path = path;
        }
        this.emit_signal("changed", &[]);
    }

    /// Whether the current revision differs from the last saved or exported revision.
    #[func]
    pub fn is_dirty(&self) -> bool {
        self.core.is_dirty()
    }

    /// Whether a grouped authored edit is active; preview-only gestures never start one.
    #[func]
    pub fn is_editing(&self) -> bool {
        self.core.is_editing()
    }

    /// Record a persisted revision without adding an undo command.
    /// After export, pass the existing draft path to leave its association unchanged.
    #[func(gd_self)]
    pub fn mark_saved(mut this: Gd<Self>, path: GString) {
        {
            let mut doc = this.bind_mut();
            doc.draft_path = path;
            doc.core.mark_saved();
        }
        this.emit_signal("changed", &[]);
    }

    /// Whether undo is available outside an active transaction.
    #[func]
    pub fn can_undo(&self) -> bool {
        self.core.can_undo()
    }
    /// Whether redo is available outside an active transaction.
    #[func]
    pub fn can_redo(&self) -> bool {
        self.core.can_redo()
    }
    /// Label of the next undo command, or empty.
    #[func]
    pub fn undo_label(&self) -> GString {
        self.core.undo_label().into()
    }
    /// Label of the next redo command, or empty.
    #[func]
    pub fn redo_label(&self) -> GString {
        self.core.redo_label().into()
    }

    /// Apply a complete command; copy external mutable dictionaries exactly once.
    #[func(gd_self)]
    pub fn apply(mut this: Gd<Self>, data: VarDictionary, label: GString) {
        let changed = {
            let mut doc = this.bind_mut();
            if doc.core.data() == &data {
                false
            } else {
                doc.core.apply(data.duplicate_deep(), label.to_string())
            }
        };
        if changed {
            this.emit_signal("changed", &[]);
        }
    }

    /// Change one metadata property without touching any other authored values.
    #[func(gd_self)]
    pub fn set_parameter(
        mut this: Gd<Self>,
        key: GString,
        value: Variant,
        #[opt(default = "Edit property")] label: GString,
    ) {
        let changed = {
            let mut doc = this.bind_mut();
            let mut next = doc.core.data().duplicate_shallow();
            let mut params = next
                .get("params")
                .and_then(|v| v.try_to::<VarDictionary>().ok())
                .unwrap_or_default()
                .duplicate_shallow();
            params.set(key, value);
            next.set("params", params);
            // Incoming arrays/dictionaries can be mutable: detach the new property as well.
            doc.core.apply(next.duplicate_deep(), label.to_string())
        };
        if changed {
            this.emit_signal("changed", &[]);
        }
    }

    /// Group text editing or one geometry gesture into a single reversible command.
    #[func]
    pub fn begin_transaction(&mut self, label: GString) {
        self.core.begin(label.to_string());
    }

    /// Finish a grouped command, including its side effects.
    #[func(gd_self)]
    pub fn commit_transaction(this: Gd<Self>) {
        Self::edit_history(this, Document::commit);
    }
    /// Restore the pre-gesture revision without adding history.
    #[func(gd_self)]
    pub fn cancel_transaction(this: Gd<Self>) {
        Self::edit_history(this, Document::cancel);
    }
    /// Restore the previous revision.
    #[func(gd_self)]
    pub fn undo(this: Gd<Self>) {
        Self::edit_history(this, Document::undo);
    }
    /// Restore a revision from the current redo branch.
    #[func(gd_self)]
    pub fn redo(this: Gd<Self>) {
        Self::edit_history(this, Document::redo);
    }
}

impl AssetAuthoringDocument {
    fn edit_history(mut this: Gd<Self>, edit: impl FnOnce(&mut Document<VarDictionary>) -> bool) {
        let changed = edit(&mut this.bind_mut().core);
        if changed {
            this.emit_signal("changed", &[]);
        }
    }
}
