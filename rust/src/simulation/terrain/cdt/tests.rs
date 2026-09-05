// SPDX-License-Identifier: GPL-2.0-only

//! CDT regression tests and shared terrain/road fixtures.

use std::collections::BTreeMap;

use super::*;

mod builder;
mod canonicalize;
mod dem;
mod fixtures;
mod loop_clip;
mod ownership;
mod seam_quality;

use fixtures::*;
