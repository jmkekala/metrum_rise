// SPDX-License-Identifier: GPL-2.0-only

//! Engine-independent authoring history. Each immutable document revision is stored once.
//! Commands cost O(document size) to compare; undo/redo and transaction setup are O(1).
//! Snapshot copies belong at the bridge boundary, never in pointer-motion or simulation loops.

use std::collections::VecDeque;
use std::sync::Arc;

pub(crate) const HISTORY_LIMIT: usize = 100;

struct Command<T> {
    before: Arc<T>,
    after: Arc<T>,
    label: String,
}

pub(crate) struct Document<T> {
    data: Arc<T>,
    saved: Arc<T>,
    history: VecDeque<Command<T>>,
    cursor: usize,
    transaction: Option<(Arc<T>, String)>,
}

impl<T: Default + PartialEq> Default for Document<T> {
    fn default() -> Self {
        let data = Arc::new(T::default());
        Self {
            saved: data.clone(),
            data,
            history: VecDeque::new(),
            cursor: 0,
            transaction: None,
        }
    }
}

impl<T: Default + PartialEq> Document<T> {
    pub(crate) fn data(&self) -> &T {
        &self.data
    }
    pub(crate) fn is_dirty(&self) -> bool {
        self.data != self.saved
    }
    pub(crate) fn is_editing(&self) -> bool {
        self.transaction.is_some()
    }
    pub(crate) fn mark_saved(&mut self) {
        self.saved = self.data.clone();
    }
    pub(crate) fn can_undo(&self) -> bool {
        self.transaction.is_none() && self.cursor > 0
    }
    pub(crate) fn can_redo(&self) -> bool {
        self.transaction.is_none() && self.cursor < self.history.len()
    }
    pub(crate) fn undo_label(&self) -> &str {
        if self.can_undo() {
            &self.history[self.cursor - 1].label
        } else {
            ""
        }
    }
    pub(crate) fn redo_label(&self) -> &str {
        if self.can_redo() {
            &self.history[self.cursor].label
        } else {
            ""
        }
    }

    pub(crate) fn reset(&mut self, data: T, saved: bool) {
        self.data = Arc::new(data);
        self.saved = if saved {
            self.data.clone()
        } else {
            Arc::new(T::default())
        };
        self.history.clear();
        self.cursor = 0;
        self.transaction = None;
    }

    pub(crate) fn apply(&mut self, data: T, label: String) -> bool {
        if *self.data == data {
            return false;
        }
        let before = std::mem::replace(&mut self.data, Arc::new(data));
        if self.transaction.is_none() {
            self.record(before, label);
        }
        true
    }

    pub(crate) fn begin(&mut self, label: String) {
        if self.transaction.is_none() {
            self.transaction = Some((self.data.clone(), label));
        }
    }

    pub(crate) fn commit(&mut self) -> bool {
        let Some((before, label)) = self.transaction.take() else {
            return false;
        };
        if before != self.data {
            self.record(before, label);
        }
        true
    }

    pub(crate) fn cancel(&mut self) -> bool {
        let Some((before, _)) = self.transaction.take() else {
            return false;
        };
        self.data = before;
        true
    }

    pub(crate) fn undo(&mut self) -> bool {
        if !self.can_undo() {
            return false;
        }
        self.cursor -= 1;
        self.data = self.history[self.cursor].before.clone();
        true
    }

    pub(crate) fn redo(&mut self) -> bool {
        if !self.can_redo() {
            return false;
        }
        self.data = self.history[self.cursor].after.clone();
        self.cursor += 1;
        true
    }

    fn record(&mut self, before: Arc<T>, label: String) {
        self.history.truncate(self.cursor);
        self.history.push_back(Command {
            before,
            after: self.data.clone(),
            label,
        });
        if self.history.len() > HISTORY_LIMIT {
            self.history.pop_front();
        }
        self.cursor = self.history.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transactions_savepoints_branches_and_bounded_history() {
        let mut doc = Document::<Vec<i32>>::default();
        doc.reset(vec![1], true);
        doc.begin("Drag".into());
        doc.apply(vec![2], "intermediate".into());
        doc.apply(vec![3], "intermediate".into());
        assert!(!doc.can_undo());
        doc.commit();
        assert_eq!(doc.undo_label(), "Drag");
        assert!(doc.is_dirty());
        doc.undo();
        assert_eq!(doc.data(), &[1]);
        assert!(!doc.is_dirty());
        doc.redo();
        doc.mark_saved();
        doc.undo();
        assert!(doc.is_dirty());
        doc.redo();
        assert!(!doc.is_dirty());
        doc.begin("Cancel".into());
        doc.apply(vec![4], String::new());
        doc.cancel();
        assert!(!doc.is_dirty());
        doc.undo();
        doc.apply(vec![5], "Branch".into());
        assert!(!doc.can_redo());
        for value in 0..110 {
            doc.apply(vec![value], value.to_string());
        }
        let mut count = 0;
        while doc.undo() {
            count += 1;
        }
        assert_eq!(count, HISTORY_LIMIT);
        doc.reset(vec![], true);
        assert!(!doc.can_undo() && !doc.can_redo() && !doc.is_dirty());
    }

    #[test]
    fn history_shares_revisions_and_no_op_transactions_preserve_redo() {
        let mut doc = Document::<Vec<i32>>::default();
        doc.apply(vec![1], "one".into());
        doc.apply(vec![2], "two".into());
        assert!(Arc::ptr_eq(&doc.history[0].after, &doc.history[1].before));
        doc.undo();
        doc.begin("No change".into());
        doc.commit();
        assert!(doc.can_redo());
        assert!(!doc.apply(vec![1], "Same value".into()));
        doc.redo();
        assert_eq!(doc.data(), &[2]);
    }
}
