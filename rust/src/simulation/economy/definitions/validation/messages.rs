// SPDX-License-Identifier: GPL-2.0-only

//! Validation message schema and constructors.

use serde::Serialize;

#[derive(Serialize)]
pub(in crate::simulation::economy::definitions) struct ValidationMessage {
    pub(super) severity: &'static str,
    pub(super) code: &'static str,
    pub(super) scope: String,
    pub(super) message: String,
}

impl ValidationMessage {
    pub(in crate::simulation::economy::definitions) fn is_error(&self) -> bool {
        self.severity == "error"
    }
}

pub(super) fn error(
    code: &'static str,
    scope: impl Into<String>,
    message: impl Into<String>,
) -> ValidationMessage {
    ValidationMessage {
        severity: "error",
        code,
        scope: scope.into(),
        message: message.into(),
    }
}

pub(super) fn warning(
    code: &'static str,
    scope: impl Into<String>,
    message: impl Into<String>,
) -> ValidationMessage {
    ValidationMessage {
        severity: "warning",
        code,
        scope: scope.into(),
        message: message.into(),
    }
}
