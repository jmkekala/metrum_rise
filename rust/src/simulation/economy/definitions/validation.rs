// SPDX-License-Identifier: GPL-2.0-only

//! Validation entry points for authored economy runtime tuning.

mod common;
mod runtime_tuning;

pub(super) use common::validate_range;
pub(super) use runtime_tuning::validate_runtime_tuning;
