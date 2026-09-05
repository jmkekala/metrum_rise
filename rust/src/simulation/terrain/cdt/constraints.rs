// SPDX-License-Identifier: GPL-2.0-only

//! Constraint construction, source-preserving noding, and patch-boundary rails.

mod loops;
mod source;
mod topology;

pub(super) use loops::*;
pub(super) use source::*;
pub(super) use topology::*;
