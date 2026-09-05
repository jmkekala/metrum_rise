// SPDX-License-Identifier: GPL-2.0-only

//! Built-in zoning-profile registry loading and deterministic runtime compilation.

mod authored;
mod compile;
mod registry;
mod runtime;

pub use registry::{ZoningProfileRegistry, load_builtin_profile_registry};
pub use runtime::{ZoneDensity, ZoneProfileRuntime};
