// SPDX-License-Identifier: GPL-3.0-or-later

//! Why the game refuses a save or world file, decided from the file's header (`UI-16`).
//!
//! The save loader ([`super::load_from_sqlite`]) and the world loader
//! (`world_definition::load_world_definition_from_sqlite`) read their header through the same
//! function as the Load and New Game lists ([`super::check_save`],
//! `world_definition::check_world_definition`), so a list never offers a file its load refuses
//! for its format or the grid, and never greys out one that loads. A file whose header passes
//! but whose content is damaged further in is listed and refused at load, as before.

use std::fmt::Display;

/// The kind of refusal, for the list's short reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefusalKind {
    /// Stored before the aligned grid (or off the world rules): refused whatever its format.
    BeforeGrid,
    /// A format number above the newest this game reads.
    Newer,
    /// A format number below the oldest this game still converts.
    Older,
    /// Not a readable save or world: not SQLite, a missing table or row, a bad value in it.
    Damaged,
}

/// A refusal: its kind and the loader's own message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Refused {
    /// What the list says.
    pub(crate) kind: RefusalKind,
    /// The message the load reports (the status line after a pick), shown in the row's tooltip.
    pub(crate) message: String,
}

impl Refused {
    /// A refusal of `kind` with `message`.
    pub(crate) fn new(kind: RefusalKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// An unreadable header: an SQLite error or an invalid value.
    pub(crate) fn damaged(error: impl Display) -> Self {
        Self::new(RefusalKind::Damaged, error.to_string())
    }

    /// The short reason a list row shows, in the game's words.
    pub(crate) fn reason(&self) -> &'static str {
        match self.kind {
            RefusalKind::BeforeGrid => "Made before the grid change, cannot load",
            RefusalKind::Newer => "Made by a newer version, cannot load",
            RefusalKind::Older => "Made by an older version, cannot load",
            RefusalKind::Damaged => "Damaged file, cannot load",
        }
    }
}

impl From<rusqlite::Error> for Refused {
    fn from(error: rusqlite::Error) -> Self {
        Self::damaged(error)
    }
}
