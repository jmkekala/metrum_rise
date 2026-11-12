// SPDX-License-Identifier: GPL-2.0-only

//! `StreamStatus` (`RENDER-32-E`, `docs/terrain.md` "Streaming core"): what one layer has
//! installed, what waits and what the last frame spent, read by the benchmark, the perf channel
//! and tests from one place.

use super::schedule::FrameBudget;

/// One layer's streaming state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LayerStatus {
    /// Chunks with an installed version.
    pub(crate) installed: usize,
    /// Installed chunks drawn.
    pub(crate) visible: usize,
    /// Prepared products waiting for their install.
    pub(crate) pending: usize,
    /// Installs the slot pool refused this frame (every slot of its limit live).
    pub(crate) blocked: usize,
    /// Slots live and spare, and the pool's churn since setup.
    pub(crate) slots_live: usize,
    /// Spare slots.
    pub(crate) slots_spare: usize,
    /// Slots created since setup.
    pub(crate) slots_created: u64,
    /// Slots freed since setup.
    pub(crate) slots_freed: u64,
    /// Installs completed since setup.
    pub(crate) installs_total: u64,
    /// This frame's installs.
    pub(crate) frame: FrameBudget,
}
