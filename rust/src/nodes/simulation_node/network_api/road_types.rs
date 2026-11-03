// SPDX-License-Identifier: GPL-2.0-only

//! Road-type Godot API methods (`ROAD-101`): the road types the Roads menu, Road Properties and
//! the road builder list, the layout check the builder asks on every edit, and the player's
//! library edits. The library is `user://road_types.json`, read on first use (the tests' isolated
//! profile moves `user://` with it). The tool draws a type through the existing calls: GDScript
//! passes the listed type's lanes per call and its median to `set_road_tool_median`, and Road
//! Properties applies it through `preview_edge_cross_section` / `set_edge_cross_section`.
//!
//! A part crosses the bridge as a dictionary: `{"kind": "sidewalk"}`, `{"kind": "car_lane",
//! "forward": bool}`, `{"kind": "median", "style": 1 painted | 2 kerbed grass | 3 kerbed paving,
//! "width_m": float}` or `{"kind": "footpath"}` (a walkway's single part).

use super::super::*;
use crate::simulation::network::graph::{MedianStyle, RoadMedian};
use crate::simulation::network::road_types::library::RoadTypeLibrary;
use crate::simulation::network::road_types::{
    RoadCrossSection, RoadPart, RoadType, RoadTypeRefusal, check_layout, layout_widths, type_of,
};
use godot::classes::ProjectSettings;

const LIBRARY_PATH: &str = "user://road_types.json";

impl SimulationNode {
    // Runs `f` on the library, creating its handle (no file access) on first use.
    fn with_road_types<R>(&self, f: impl FnOnce(&mut RoadTypeLibrary) -> R) -> R {
        let mut library = self
            .road_types
            .lock()
            .expect("road type library lock poisoned");
        let library = library.get_or_insert_with(|| {
            let path = ProjectSettings::singleton().globalize_path(LIBRARY_PATH);
            RoadTypeLibrary::new(path.to_string())
        });
        f(library)
    }

    /// The id of the first usable road type, built-ins first, whose layout builds `section`, or
    /// the empty string for a layout no type has (Road Properties' "Custom layout").
    pub(crate) fn road_type_of(&self, section: RoadCrossSection) -> GString {
        self.with_road_types(|library| {
            type_of(library.types(), section).map_or_else(GString::new, |ty| ty.id.as_str().into())
        })
    }
}

#[godot_api(secondary)]
impl SimulationNode {
    /// Every road type (`ROAD-101`): `types`, the built-ins in the Roads menu's order then the
    /// player's in library order, and `error` with `error_detail`: empty, `library_unreadable`
    /// (the file does not parse; it is left untouched and edits are refused until it is fixed)
    /// or `library_newer_version` (read-only). Each type is a dictionary of `id`, `name`,
    /// `builtin`, `usable`, `parts` (see the module docs) and the fields of
    /// [`Self::check_road_layout`] but `suggested_name`: an unusable type (kept in the file,
    /// never drawn) carries its `invalid_reason`. `O(types x parts)`; the file is read once.
    #[func]
    pub fn get_road_types(&self) -> VarDictionary {
        self.with_road_types(|library| {
            let mut types = VarArray::new();
            for ty in library.types() {
                types.push(&type_to_variant(ty).to_variant());
            }
            let error = library.error();
            vdict! {
                "types": types,
                "error": error.as_ref().map_or("", |error| error.reason),
                "error_detail": error.as_ref().map_or("", |error| error.detail.as_str()),
            }
        })
    }

    /// Checks a layout, an array of part dictionaries left to right along the road's forward
    /// direction (`ROAD-101`): `is_valid`; `invalid_reason` and `detail` (`part <index>`) when
    /// refused (`layout_unknown_part` for a dictionary that is not a part, else the layout
    /// check's keys); the cross-section a valid layout builds as `fwd_lanes`, `bkw_lanes`,
    /// `median_style` (0 none) and `median_width`; `part_widths_m` (one per part, refused
    /// layouts included), `carriageway_m` (the road's width between its sidewalks) and
    /// `total_m` (sidewalks included); and `suggested_name`, a free name for a new type of a
    /// valid layout ("2+2 Grass 3 m", with " (2)" when taken), else empty. `O(parts)` plus the
    /// name lookup's `O(types)`.
    #[func]
    pub fn check_road_layout(&self, parts: VarArray) -> VarDictionary {
        let parts = match parts_from_variant(&parts) {
            Ok(parts) => parts,
            Err(refusal) => {
                let mut result = layout_to_variant(&[], Err(&refusal));
                result.set("is_valid", false);
                result.set("suggested_name", "");
                return result;
            }
        };
        let verdict = check_layout(&parts).map_err(RoadTypeRefusal::from);
        let mut result = layout_to_variant(&parts, verdict.as_ref());
        result.set("is_valid", verdict.is_ok());
        let name = verdict.map_or_else(
            |_| String::new(),
            |section| self.with_road_types(|library| library.suggested_name(section)),
        );
        result.set("suggested_name", name.as_str());
        result
    }

    /// Adds a road type to the player's library (`ROAD-101`): `{id}` when written, else
    /// `{invalid_reason, detail}`: the layout check's keys, `layout_unknown_part`,
    /// `name_empty`, `name_too_long`, `name_invalid_characters`, `name_taken` (any type,
    /// built-ins included, ignoring case), `library_unreadable`, `library_newer_version` or
    /// `library_write_failed`. Library edits are not undoable and never change a built road.
    #[func]
    pub fn create_road_type(&self, name: GString, parts: VarArray) -> VarDictionary {
        let created = parts_from_variant(&parts).and_then(|parts| {
            self.with_road_types(|library| library.create(&name.to_string(), &parts))
        });
        match created {
            Ok(id) => vdict! { "id": id.as_str() },
            Err(refusal) => refusal_to_variant(&refusal),
        }
    }

    /// Reads the road-type library file again (`ROAD-101`, the road builder on opening), so a
    /// file fixed or edited by hand lists without a restart; [`Self::get_road_types`] then gives
    /// the fresh list. Writes always re-read the file first. `O(types x parts)`.
    #[func]
    pub fn reload_road_types(&self) {
        self.with_road_types(RoadTypeLibrary::reload);
    }

    /// Checks a type name before a create or an update (`ROAD-101`, the road builder's name
    /// field): the empty string when [`Self::create_road_type`] (`id` empty) or
    /// [`Self::update_road_type`] of custom type `id` would accept it, else `name_empty`,
    /// `name_too_long`, `name_invalid_characters` or `name_taken`. Uses the list as last read;
    /// the write checks again. `O(types x name)`.
    #[func]
    pub fn check_road_type_name(&self, name: GString, id: GString) -> GString {
        self.with_road_types(|library| library.check_name(&name.to_string(), &id.to_string()))
            .err()
            .map_or_else(GString::new, |refusal| refusal.reason.into())
    }

    /// Renames custom road type `id` to `name` and gives it `parts` (pass the current values
    /// for what stays) (`ROAD-101`). Returns the empty string when written, else the refusal
    /// key: those of [`Self::create_road_type`], `road_type_builtin` or `road_type_unknown`.
    /// Roads already built keep their own layout.
    #[func]
    pub fn update_road_type(&self, id: GString, name: GString, parts: VarArray) -> GString {
        let updated = parts_from_variant(&parts).and_then(|parts| {
            self.with_road_types(|library| {
                library.update(&id.to_string(), &name.to_string(), &parts)
            })
        });
        updated
            .err()
            .map_or_else(GString::new, |refusal| refusal.reason.into())
    }

    /// Deletes custom road type `id` (`ROAD-101`). Returns the empty string when written, else
    /// `road_type_builtin`, `road_type_unknown`, `library_unreadable`, `library_newer_version`
    /// or `library_write_failed`. Roads already built keep their own layout.
    #[func]
    pub fn delete_road_type(&self, id: GString) -> GString {
        self.with_road_types(|library| library.delete(&id.to_string()))
            .err()
            .map_or_else(GString::new, |refusal| refusal.reason.into())
    }
}

fn refusal_to_variant(refusal: &RoadTypeRefusal) -> VarDictionary {
    vdict! {
        "invalid_reason": refusal.reason,
        "detail": refusal.detail.as_str(),
    }
}

fn type_to_variant(ty: &RoadType) -> VarDictionary {
    let mut result = layout_to_variant(&ty.parts, ty.verdict.as_ref());
    result.set("id", ty.id.as_str());
    result.set("name", ty.name.as_str());
    result.set("builtin", ty.builtin);
    result.set("usable", ty.verdict.is_ok());
    let mut parts = VarArray::new();
    for part in &ty.parts {
        parts.push(&part_to_variant(*part).to_variant());
    }
    result.set("parts", parts);
    result
}

// The verdict and widths of a layout, shared by the list and the check.
fn layout_to_variant(
    parts: &[RoadPart],
    verdict: Result<&RoadCrossSection, &RoadTypeRefusal>,
) -> VarDictionary {
    let (widths, carriageway_m, total_m) = layout_widths(parts);
    let section = verdict.ok().copied();
    let (reason, detail) = verdict.err().map_or(("", ""), |refusal| {
        (refusal.reason, refusal.detail.as_str())
    });
    let median = section.and_then(|section| section.median);
    vdict! {
        "invalid_reason": reason,
        "detail": detail,
        "fwd_lanes": section.map_or(0, |section| i64::from(section.fwd_lanes)),
        "bkw_lanes": section.map_or(0, |section| i64::from(section.bkw_lanes)),
        "median_style": i64::from(RoadMedian::style_code(median)),
        "median_width": median.map_or(0.0, |median| median.width_m),
        "part_widths_m": PackedFloat32Array::from(widths.as_slice()),
        "carriageway_m": carriageway_m,
        "total_m": total_m,
    }
}

fn part_to_variant(part: RoadPart) -> VarDictionary {
    match part {
        RoadPart::Sidewalk => vdict! { "kind": "sidewalk" },
        RoadPart::CarLane { forward } => vdict! { "kind": "car_lane", "forward": forward },
        RoadPart::Median(median) => vdict! {
            "kind": "median",
            "style": median.style as i64,
            "width_m": median.width_m,
        },
        RoadPart::Footpath => vdict! { "kind": "footpath" },
    }
}

fn parts_from_variant(parts: &VarArray) -> Result<Vec<RoadPart>, RoadTypeRefusal> {
    parts
        .iter_shared()
        .enumerate()
        .map(|(index, part)| {
            part_from_variant(&part)
                .map_err(|reason| RoadTypeRefusal::new(reason, format!("part {index}")))
        })
        .collect()
}

// One part dictionary: `layout_unknown_part` for an unknown kind or a missing or mistyped field,
// `unknown_median_style` for a style code outside 1..=3. The width is checked by the layout.
fn part_from_variant(part: &Variant) -> Result<RoadPart, &'static str> {
    const UNKNOWN: &str = "layout_unknown_part";
    let part: VarDictionary = part.try_to().map_err(|_| UNKNOWN)?;
    let field = |key: &str| part.get(key).ok_or(UNKNOWN);
    let kind: GString = field("kind")?.try_to().map_err(|_| UNKNOWN)?;
    match kind.to_string().as_str() {
        "sidewalk" => Ok(RoadPart::Sidewalk),
        "footpath" => Ok(RoadPart::Footpath),
        "car_lane" => Ok(RoadPart::CarLane {
            forward: field("forward")?.try_to().map_err(|_| UNKNOWN)?,
        }),
        "median" => {
            let style: i64 = field("style")?.try_to().map_err(|_| UNKNOWN)?;
            let style = i32::try_from(style)
                .ok()
                .and_then(MedianStyle::from_code)
                .ok_or("unknown_median_style")?;
            // GDScript may send a whole width as an int.
            let width = field("width_m")?;
            let width_m = width
                .try_to::<f64>()
                .or_else(|_| width.try_to::<i64>().map(|width| width as f64))
                .map_err(|_| UNKNOWN)? as f32;
            Ok(RoadPart::Median(RoadMedian { style, width_m }))
        }
        _ => Err(UNKNOWN),
    }
}
