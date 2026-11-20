// SPDX-License-Identifier: GPL-3.0-or-later

//! The road tool's road type pick (`ROAD-112` round R2-6, F12): which road type the next stroke
//! draws, the car type R draws again, and a listed type's lanes, median, parking and attributes
//! as the session's [`RoadTypeSpec`]. Rust owns the library, so the lookup is here; the bridge
//! sets the spec on the session in one option. A pick is atomic: an unknown or unusable type
//! changes neither the pick nor the spec.

use super::measure::RoadTypeSpec;
use crate::simulation::network::graph::SidewalkSides;
use crate::simulation::network::road_types::{
    DEFAULT_SPEED_KMH, RoadAttributes, RoadType, RoadTypePockets,
};

/// The built-in type R starts with and falls back to when the picked type is gone.
pub const DEFAULT_ROAD_TYPE: &str = "builtin:road_2l";
/// The built-in Walkway: picking it leaves the type R draws as it was.
pub const WALKWAY_ROAD_TYPE: &str = "builtin:walkway";

/// The road type picks of the road tool.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadTypePick {
    /// The type the next stroke draws; `None` before the first pick.
    pub current: Option<String>,
    /// The last road type picked other than the Walkway: R draws it again.
    pub last: String,
}

impl Default for RoadTypePick {
    fn default() -> Self {
        Self {
            current: None,
            last: DEFAULT_ROAD_TYPE.to_string(),
        }
    }
}

impl RoadTypePick {
    /// Picks the type `id` of `types` (the library's list): its spec ([`type_spec`]), or the
    /// refusal key (`road_type_unknown` when no type has `id`, the type's own refusal when it is
    /// unusable), leaving the picks as they were. `O(types)` on a UI action.
    pub fn pick(&mut self, types: &[RoadType], id: &str) -> Result<RoadTypeSpec, &'static str> {
        let ty = types
            .iter()
            .find(|ty| ty.id == id)
            .ok_or("road_type_unknown")?;
        let spec = type_spec(ty)?;
        if id != WALKWAY_ROAD_TYPE {
            self.last = id.to_string();
        }
        self.current = Some(id.to_string());
        Ok(spec)
    }

    /// Picks the current type again after the library changed: an edited type's new layout
    /// draws from the next stroke on (built roads keep theirs). A last type that is gone or no
    /// longer usable falls back to the 2-Lane Road; a current type that is gone or no longer
    /// usable falls back to the last type. `None` before the first pick, else the pick's
    /// result. `O(types)`.
    pub fn reload(&mut self, types: &[RoadType]) -> Option<Result<RoadTypeSpec, &'static str>> {
        if !types
            .iter()
            .any(|ty| ty.id == self.last && ty.usable().is_some())
        {
            self.last = DEFAULT_ROAD_TYPE.to_string();
        }
        let current = self.current.clone()?;
        Some(self.pick(types, &current).or_else(|_| {
            let last = self.last.clone();
            self.pick(types, &last)
        }))
    }
}

/// What a stroke of `ty` draws: its lanes, median, kerbside parking, speed, sidewalk sides,
/// zoning, street lights and turn pockets. A walkway, which has neither a speed nor sidewalks of
/// its own, takes the built-ins' speed and both sides and no pockets, as every stroke did before
/// road types (its stroke builds the same footpath either way). An unusable type refuses with
/// its own key. `O(1)`.
pub fn type_spec(ty: &RoadType) -> Result<RoadTypeSpec, &'static str> {
    let section = ty.verdict.as_ref().map_err(|refusal| refusal.reason)?;
    let (speed_kmh, sidewalks, pockets) = if section.fwd_lanes + section.bkw_lanes == 0 {
        (DEFAULT_SPEED_KMH, SidewalkSides::BOTH, RoadTypePockets::default())
    } else {
        (ty.speed_kmh, section.sidewalks, ty.pockets)
    };
    Ok(RoadTypeSpec {
        fwd_lanes: section.fwd_lanes,
        bkw_lanes: section.bkw_lanes,
        median: section.median,
        parking: section.parking,
        attributes: RoadAttributes::from_tool(
            speed_kmh.into(),
            sidewalks.bits().into(),
            ty.zoning,
            ty.street_lights,
        )?,
        pockets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::sim::road_tool::session::{RoadToolOption, RoadToolSession};
    use crate::simulation::network::graph::{MedianStyle, RoadMedian, RoadParking};
    use crate::simulation::network::road_types::{
        BUILTIN_ROAD_TYPES, RoadCrossSection, RoadTypeRefusal,
    };

    fn builtins() -> Vec<RoadType> {
        BUILTIN_ROAD_TYPES.iter().map(RoadType::from_builtin).collect()
    }

    // A divided custom type at 80 km/h with parking on the right, one sidewalk, no zoning and no
    // lights, and an unusable one.
    fn library() -> Vec<RoadType> {
        let median = RoadMedian {
            style: MedianStyle::KerbedGrass,
            width_m: 3.5,
        };
        let section = RoadCrossSection {
            fwd_lanes: 2,
            bkw_lanes: 1,
            median: Some(median),
            parking: RoadParking::from_code(2).expect("right kerb"),
            sidewalks: SidewalkSides::from_tool(2).expect("right side"),
        };
        let mut types = builtins();
        types.push(RoadType {
            id: "custom:1".to_string(),
            name: "Avenue".to_string(),
            builtin: false,
            parts: Vec::new(),
            speed_kmh: 80,
            zoning: false,
            street_lights: false,
            verdict: Ok(section),
            pockets: RoadTypePockets::default(),
        });
        types.push(RoadType {
            id: "custom:2".to_string(),
            name: "Broken".to_string(),
            builtin: false,
            parts: Vec::new(),
            speed_kmh: 50,
            zoning: true,
            street_lights: true,
            verdict: Err(RoadTypeRefusal::new("too_many_lanes", "")),
            pockets: RoadTypePockets::default(),
        });
        types
    }

    // Picks `id` on `session` as the bridge does: the spec in one option, only on success.
    fn pick_on(
        session: &mut RoadToolSession,
        pick: &mut RoadTypePick,
        types: &[RoadType],
        id: &str,
    ) -> Result<(), &'static str> {
        let spec = pick.pick(types, id)?;
        session.set_option(RoadToolOption::RoadType(spec));
        Ok(())
    }

    #[test]
    fn a_valid_pick_sets_every_setting_of_the_type() {
        let types = library();
        let mut session = RoadToolSession::new();
        let mut pick = RoadTypePick::default();
        pick_on(&mut session, &mut pick, &types, "custom:1").expect("usable type");
        let spec = session.spec();
        assert_eq!((spec.fwd_lanes, spec.bkw_lanes), (2, 1));
        assert_eq!(
            spec.median,
            Some(RoadMedian {
                style: MedianStyle::KerbedGrass,
                width_m: 3.5
            })
        );
        assert_eq!(spec.parking.code(), 2);
        assert_eq!(
            spec.attributes,
            RoadAttributes::from_tool(80, 2, false, false).expect("valid")
        );
        assert_eq!(pick.current.as_deref(), Some("custom:1"));
        assert_eq!(pick.last, "custom:1");
        // The Walkway: no lanes, the built-ins' attributes, and R keeps the avenue.
        pick_on(&mut session, &mut pick, &types, WALKWAY_ROAD_TYPE).expect("walkway");
        let spec = session.spec();
        assert_eq!((spec.fwd_lanes, spec.bkw_lanes, spec.median), (0, 0, None));
        assert_eq!(spec.parking.code(), 0);
        assert_eq!(spec.attributes, RoadAttributes::DEFAULT);
        assert_eq!(pick.current.as_deref(), Some(WALKWAY_ROAD_TYPE));
        assert_eq!(pick.last, "custom:1");
        // A built-in is bitwise the default road's spec.
        pick_on(&mut session, &mut pick, &types, DEFAULT_ROAD_TYPE).expect("built-in");
        assert_eq!(session.spec(), RoadTypeSpec::default());
    }

    #[test]
    fn an_unusable_or_unknown_pick_leaves_the_intent_whole() {
        let types = library();
        let mut session = RoadToolSession::new();
        let mut pick = RoadTypePick::default();
        pick_on(&mut session, &mut pick, &types, "custom:1").expect("usable type");
        let (spec, picked) = (session.spec(), pick.clone());
        assert_eq!(
            pick_on(&mut session, &mut pick, &types, "custom:2"),
            Err("too_many_lanes")
        );
        assert_eq!(
            pick_on(&mut session, &mut pick, &types, "custom:9"),
            Err("road_type_unknown")
        );
        assert_eq!(session.spec(), spec, "a refused pick changes no setting");
        assert_eq!(pick, picked, "a refused pick changes no pick");
    }

    #[test]
    fn a_reload_repicks_the_type_or_falls_back() {
        let mut types = library();
        let mut pick = RoadTypePick::default();
        assert_eq!(pick.reload(&types), None, "nothing picked yet");
        pick.pick(&types, "custom:1").expect("usable type");
        // The avenue is edited to one lane each way: the reload draws the new layout.
        if let Ok(section) = types[5].verdict.as_mut() {
            section.fwd_lanes = 1;
        }
        let spec = pick.reload(&types).expect("picked").expect("still usable");
        assert_eq!((spec.fwd_lanes, spec.bkw_lanes), (1, 1));
        // The avenue is deleted: the current and last types fall back to the 2-Lane Road.
        types.remove(5);
        let spec = pick.reload(&types).expect("picked").expect("default");
        assert_eq!(spec, RoadTypeSpec::default());
        assert_eq!(pick.current.as_deref(), Some(DEFAULT_ROAD_TYPE));
        assert_eq!(pick.last, DEFAULT_ROAD_TYPE);
    }
}
