// SPDX-License-Identifier: GPL-2.0-only

//! The player's road types (`ROAD-101`): one JSON file per user, shared by every city, never
//! part of a save. Path-parameterised and engine-free; the Godot API places it in the user
//! folder.
//!
//! The file is read on first use and again before every write, so another game instance's
//! edits survive; writes are atomic (`write_atomic`). A player's road is never lost silently:
//! a file that does not parse is left byte-identical, reported, and refuses writes until fixed;
//! a newer format version is read-only; an entry that fails a rule stays in the file as it was,
//! listed unusable with its reason. Ids come from a counter stored in the file: deterministic,
//! no clock, no randomness. Library edits are tool configuration, outside the undo stack.
//!
//! Format (version 1), pretty-printed:
//! `{"format": "metrum-road-types", "version": 1, "next_id": 3, "types": [{"id": "custom:1",
//! "name": "Boulevard", "parts": [{"kind": "sidewalk"}, {"kind": "car_lane", "forward": false},
//! {"kind": "median", "style": "grass", "width_m": 3.0}, ...]}]}`. Part kinds `sidewalk`,
//! `car_lane`, `median`, `footpath`; median styles `painted`, `grass`, `paving`.

use super::{
    BUILTIN_ROAD_TYPES, RoadCrossSection, RoadPart, RoadType, RoadTypeRefusal, check_layout,
    clean_name, same_name, suggested_name,
};
use crate::assets::authoring::files::write_atomic;
use crate::simulation::network::graph::{MedianStyle, RoadMedian};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[cfg(test)]
mod tests;

const FORMAT: &str = "metrum-road-types";
/// The library format this game reads and writes.
pub const LIBRARY_VERSION: u32 = 1;
const CUSTOM_ID_PREFIX: &str = "custom:";

// The file's envelope as read: entries stay raw JSON so an unusable one is written back as it was.
#[derive(Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    types: Vec<Value>,
}

// The envelope as written, in this field order.
#[derive(Serialize)]
struct EnvelopeOut<'a> {
    format: &'a str,
    version: u32,
    next_id: u64,
    types: &'a [Value],
}

#[derive(Serialize, Deserialize)]
struct StoredType {
    id: String,
    name: String,
    parts: Vec<StoredPart>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StoredPart {
    Sidewalk,
    CarLane { forward: bool },
    Median { style: String, width_m: f32 },
    Footpath,
}

const STYLE_KEYS: [(MedianStyle, &str); 3] = [
    (MedianStyle::Painted, "painted"),
    (MedianStyle::KerbedGrass, "grass"),
    (MedianStyle::KerbedPaving, "paving"),
];

impl StoredPart {
    fn from_part(part: RoadPart) -> Self {
        match part {
            RoadPart::Sidewalk => Self::Sidewalk,
            RoadPart::CarLane { forward } => Self::CarLane { forward },
            RoadPart::Median(median) => Self::Median {
                style: STYLE_KEYS
                    .iter()
                    .find(|(style, _)| *style == median.style)
                    .map(|(_, key)| key.to_string())
                    .unwrap_or_default(),
                width_m: median.width_m,
            },
            RoadPart::Footpath => Self::Footpath,
        }
    }

    fn to_part(&self) -> Result<RoadPart, &'static str> {
        Ok(match self {
            Self::Sidewalk => RoadPart::Sidewalk,
            Self::CarLane { forward } => RoadPart::CarLane { forward: *forward },
            Self::Median { style, width_m } => RoadPart::Median(RoadMedian {
                style: STYLE_KEYS
                    .iter()
                    .find(|(_, key)| key == style)
                    .map(|(style, _)| *style)
                    .ok_or("unknown_median_style")?,
                width_m: *width_m,
            }),
            Self::Footpath => RoadPart::Footpath,
        })
    }
}

// Whether the file accepts writes.
#[derive(Clone, Debug, PartialEq)]
enum FileState {
    Writable,
    Unreadable(String),
    Newer(u32),
}

/// The player's road-type library at one path, with the built-ins listed first.
pub struct RoadTypeLibrary {
    path: PathBuf,
    loaded: bool,
    state: FileState,
    next_id: u64,
    // The file's entries as JSON, in file order; `types` lists them after the built-ins.
    entries: Vec<Value>,
    types: Vec<RoadType>,
}

impl RoadTypeLibrary {
    /// A library stored at `path`. Nothing is read until first use, and the file is created only
    /// by the first write.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            loaded: false,
            state: FileState::Writable,
            next_id: 1,
            entries: Vec::new(),
            types: Vec::new(),
        }
    }

    /// Every type: the built-ins in the Roads menu's order, then the file's entries in file
    /// order, each usable or with its reason. `O(types x parts)` on first use, cached after.
    pub fn types(&mut self) -> &[RoadType] {
        self.ensure_loaded();
        &self.types
    }

    /// Why the file cannot be written, if so: `library_unreadable` (it does not parse; it is
    /// left untouched) or `library_newer_version` (a newer game's file, read-only).
    pub fn error(&mut self) -> Option<RoadTypeRefusal> {
        self.ensure_loaded();
        match &self.state {
            FileState::Writable => None,
            FileState::Unreadable(detail) => {
                Some(RoadTypeRefusal::new("library_unreadable", detail.clone()))
            }
            FileState::Newer(version) => Some(RoadTypeRefusal::new(
                "library_newer_version",
                format!("version {version}; this game reads {LIBRARY_VERSION}"),
            )),
        }
    }

    /// The name a new type of `section` is offered, unique among all types
    /// ([`suggested_name`]).
    pub fn suggested_name(&mut self, section: RoadCrossSection) -> String {
        let types = self.types();
        suggested_name(section, |name| {
            types.iter().any(|ty| same_name(&ty.name, name))
        })
    }

    /// Adds a type named `name` (trimmed) with `parts`, and returns its new id. Refused when
    /// the file cannot be written, the layout fails [`check_layout`], the name breaks a rule
    /// ([`clean_name`]) or another type has it (`name_taken`), or the write fails
    /// (`library_write_failed`).
    pub fn create(&mut self, name: &str, parts: &[RoadPart]) -> Result<String, RoadTypeRefusal> {
        self.reload_for_write()?;
        check_layout(parts)?;
        let name = self.free_name(name, None)?;
        let id = format!("{CUSTOM_ID_PREFIX}{}", self.next_id);
        self.next_id += 1;
        self.entries.push(stored(&id, &name, parts));
        self.write()?;
        Ok(id)
    }

    /// Gives custom type `id` the name `name` and the layout `parts`; built roads keep their
    /// own. Refused as [`Self::create`], and for a built-in (`road_type_builtin`) or an id the
    /// file does not hold (`road_type_unknown`). An unusable entry may be fixed this way.
    pub fn update(
        &mut self,
        id: &str,
        name: &str,
        parts: &[RoadPart],
    ) -> Result<(), RoadTypeRefusal> {
        self.reload_for_write()?;
        let entry = self.entry_index(id)?;
        check_layout(parts)?;
        let name = self.free_name(name, Some(entry))?;
        self.entries[entry] = stored(id, &name, parts);
        self.write()
    }

    /// Removes custom type `id`; built roads keep their layout. Refused as [`Self::update`]
    /// for a built-in, an unknown id or an unwritable file.
    pub fn delete(&mut self, id: &str) -> Result<(), RoadTypeRefusal> {
        self.reload_for_write()?;
        let entry = self.entry_index(id)?;
        self.entries.remove(entry);
        self.write()
    }

    fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.load();
        }
    }

    // Reads the file; a missing file is an empty library.
    fn load(&mut self) {
        self.loaded = true;
        self.state = FileState::Writable;
        self.next_id = 1;
        self.entries.clear();
        match std::fs::read(&self.path) {
            Ok(bytes) => match serde_json::from_slice::<Envelope>(&bytes) {
                Err(error) => self.state = FileState::Unreadable(error.to_string()),
                Ok(envelope) if envelope.format != FORMAT => {
                    self.state = FileState::Unreadable(format!(
                        "format {:?} is not {FORMAT}",
                        envelope.format
                    ))
                }
                Ok(envelope) if envelope.version == 0 => {
                    self.state = FileState::Unreadable("version 0".to_string())
                }
                Ok(envelope) => {
                    if envelope.version > LIBRARY_VERSION {
                        self.state = FileState::Newer(envelope.version);
                    }
                    self.next_id = envelope.next_id.max(1);
                    self.entries = envelope.types;
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => self.state = FileState::Unreadable(error.to_string()),
        }
        // A hand-edited counter never reissues an id the file holds.
        let highest = self
            .entries
            .iter()
            .filter_map(|entry| {
                entry
                    .get("id")?
                    .as_str()?
                    .strip_prefix(CUSTOM_ID_PREFIX)?
                    .parse::<u64>()
                    .ok()
            })
            .max();
        if let Some(highest) = highest {
            self.next_id = self.next_id.max(highest.saturating_add(1));
        }
        self.list_types();
    }

    // The built-ins, then each entry with its verdict against those listed before it.
    fn list_types(&mut self) {
        self.types.clear();
        self.types
            .extend(BUILTIN_ROAD_TYPES.iter().map(RoadType::from_builtin));
        for entry in &self.entries {
            let ty = custom_type(entry, &self.types);
            self.types.push(ty);
        }
    }

    fn reload_for_write(&mut self) -> Result<(), RoadTypeRefusal> {
        self.load();
        self.error().map_or(Ok(()), Err)
    }

    // The entry index of custom type `id` (the first, if a hand edit duplicated it).
    fn entry_index(&self, id: &str) -> Result<usize, RoadTypeRefusal> {
        if BUILTIN_ROAD_TYPES.iter().any(|builtin| builtin.id == id) {
            return Err(RoadTypeRefusal::new("road_type_builtin", id));
        }
        self.types[BUILTIN_ROAD_TYPES.len()..]
            .iter()
            .position(|ty| ty.id == id)
            .ok_or_else(|| RoadTypeRefusal::new("road_type_unknown", id))
    }

    // `name` trimmed, when it follows the rules and no other type has it.
    fn free_name(
        &self,
        name: &str,
        except_entry: Option<usize>,
    ) -> Result<String, RoadTypeRefusal> {
        let name = clean_name(name).map_err(|reason| RoadTypeRefusal::new(reason, name))?;
        let except = except_entry.map(|entry| entry + BUILTIN_ROAD_TYPES.len());
        let taken = self
            .types
            .iter()
            .enumerate()
            .any(|(index, ty)| Some(index) != except && same_name(&ty.name, name));
        if taken {
            return Err(RoadTypeRefusal::new("name_taken", name));
        }
        Ok(name.to_string())
    }

    // Writes the entries; on failure the file is re-read so memory matches the disk.
    fn write(&mut self) -> Result<(), RoadTypeRefusal> {
        let envelope = EnvelopeOut {
            format: FORMAT,
            version: LIBRARY_VERSION,
            next_id: self.next_id,
            types: &self.entries,
        };
        let result = serde_json::to_vec_pretty(&envelope)
            .map_err(|error| error.to_string())
            .and_then(|mut bytes| {
                bytes.push(b'\n');
                if let Some(parent) = self.path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                write_atomic(&self.path, &bytes).map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => {
                self.list_types();
                Ok(())
            }
            Err(detail) => {
                self.load();
                Err(RoadTypeRefusal::new("library_write_failed", detail))
            }
        }
    }
}

fn stored(id: &str, name: &str, parts: &[RoadPart]) -> Value {
    let stored = StoredType {
        id: id.to_string(),
        name: name.to_string(),
        parts: parts.iter().copied().map(StoredPart::from_part).collect(),
    };
    serde_json::to_value(stored).expect("a road type serialises")
}

// One file entry as a listed type: unusable when it does not read, repeats an earlier id,
// breaks a name rule, repeats an earlier name or fails the layout check (in that order).
fn custom_type(entry: &Value, listed: &[RoadType]) -> RoadType {
    let text = |key: &str| {
        entry
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let unusable = |refusal: RoadTypeRefusal, parts: Vec<RoadPart>| RoadType {
        id: text("id"),
        name: text("name"),
        builtin: false,
        parts,
        verdict: Err(refusal),
    };
    let stored = match StoredType::deserialize(entry) {
        Ok(stored) => stored,
        Err(error) => {
            return unusable(
                RoadTypeRefusal::new("entry_unreadable", error.to_string()),
                Vec::new(),
            );
        }
    };
    let parts: Result<Vec<RoadPart>, RoadTypeRefusal> = stored
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            part.to_part()
                .map_err(|reason| RoadTypeRefusal::new(reason, format!("part {index}")))
        })
        .collect();
    let parts = match parts {
        Ok(parts) => parts,
        Err(refusal) => return unusable(refusal, Vec::new()),
    };
    let verdict = if listed.iter().any(|ty| ty.id == stored.id) {
        Err(RoadTypeRefusal::new(
            "road_type_duplicate_id",
            stored.id.as_str(),
        ))
    } else if let Err(reason) = clean_name(&stored.name) {
        Err(RoadTypeRefusal::new(reason, stored.name.as_str()))
    } else if listed.iter().any(|ty| same_name(&ty.name, &stored.name)) {
        Err(RoadTypeRefusal::new("name_taken", stored.name.as_str()))
    } else {
        check_layout(&parts).map_err(RoadTypeRefusal::from)
    };
    RoadType {
        id: stored.id,
        name: clean_name(&stored.name).unwrap_or(&stored.name).to_string(),
        builtin: false,
        parts,
        verdict,
    }
}
