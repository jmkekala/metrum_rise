// SPDX-License-Identifier: GPL-2.0-only

//! Exact-input local topology/profile planning shared by preview and authoritative insertion.

use super::road_preview::RoadPreviewRequest;
use super::{RoadRejection, SimCore};
use crate::simulation::agriculture::{FieldClearanceIndex, PolygonFootprint};
use crate::simulation::buildings::allocator::{
    BuildingAllocator, ExplicitServicePlacementRejection,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::road_edit::{
    PreparedRoadInput, RoadPreviewValidation, RoadTopologyPlan, plan_road_edit,
};
use crate::simulation::network::roadbed::{PieceMesh, RoadbedSystem};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::{
    ZoningSystem,
    cells::{CellBounds, CellKey, CellMaps, CellStore},
    footprint_newly_covers, road_piece_newly_covers,
};
use glam::DVec2;
use godot::prelude::{Vector2, Vector3};

/// Immutable preparation and solved local graph delta against pinned road/source revisions.
///
/// Retains resolved splits/connections, terminal extensions, junction-adjusted profiles and clips
/// when the edit's mutation frontier is complete. Readiness pins every dependency; commit
/// verifies the plan under the simulation lock before adopting it.
/// Exact request/revision matching is O(raw input points), identity checks O(local records).
#[derive(Debug)]
pub(crate) struct RoadEditPlan {
    request: RoadPreviewRequest,
    terrain_source_generation: u64,
    prepared: PreparedRoadInput,
    validation: RoadPreviewValidation,
    topology: Option<RoadTopologyPlan>,
}

/// A plotted reservation under a road edit's new pieces (`ZONE-09`, `ZONE-10`): it refuses the
/// stroke or the in-place edit, where zoned land is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlottedOverlap {
    /// A committed farm field (`field_overlap`).
    Field,
    /// A mine's extraction area (`extraction_area_overlap`).
    ExtractionArea,
    /// The lot of an explicitly placed building, by index (`plotted_building_overlap`).
    Building(usize),
}

impl PlottedOverlap {
    /// The reason the commit reports for this overlap.
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Field => "field_overlap",
            Self::ExtractionArea => "extraction_area_overlap",
            Self::Building(_) => "plotted_building_overlap",
        }
    }

    /// The refusal of an explicit building whose lot holds this overlap (`ZONE-11`).
    pub(crate) fn placement_rejection(self) -> ExplicitServicePlacementRejection {
        match self {
            Self::Field => ExplicitServicePlacementRejection::FieldOverlap,
            Self::ExtractionArea => ExplicitServicePlacementRejection::ExtractionAreaOverlap,
            Self::Building(_) => ExplicitServicePlacementRejection::SiteOverlap,
        }
    }

    /// The refusal of a field (`field`) or an extraction area whose polygon holds this overlap
    /// (`ZONE-11`).
    pub(crate) fn work_area_refusal(self, field: bool) -> String {
        let (label, other) = if field {
            ("field", "a mine's extraction area")
        } else {
            ("extraction area", "another mine's extraction area")
        };
        match self {
            Self::Field if field => "field overlaps another field".to_owned(),
            Self::Field => format!("{label} overlaps a farm field"),
            Self::ExtractionArea => format!("{label} overlaps {other}"),
            Self::Building(_) => format!("{label} overlaps a placed building's lot"),
        }
    }

    /// The commit's report of this overlap; a building is named by its asset.
    pub(crate) fn rejection(self, core: &SimCore) -> RoadRejection {
        self.rejection_on(&CoreLand::of(core))
    }

    /// The report of this overlap against `land`, which names a building by its asset when it
    /// can ([`RoadLandSource::building_asset`]).
    pub(crate) fn rejection_on<S: RoadLandSource>(self, land: &S) -> RoadRejection {
        let detail = match self {
            Self::Field => "the road or its junctions overlap a farm field".to_owned(),
            Self::ExtractionArea => {
                "the road or its junctions overlap a mine's extraction area".to_owned()
            }
            Self::Building(index) => format!(
                "the road or its junctions overlap the lot of the placed building {}",
                land.building_asset(index).unwrap_or("?")
            ),
        };
        RoadRejection::new(self.reason(), detail)
    }
}

/// The land a [`RoadLandTake`] reads (`ZONE-09`): the live core at the commit
/// ([`CoreLand`]), the road tool's published view in the hover preview (round RZ3). The rules
/// are [`RoadLandTake::of`]'s alone; a source only answers lookups.
pub(crate) trait RoadLandSource {
    /// The cell store's maps.
    type Maps: CellMaps;
    /// How a building on a taken lot is named: its index in the live core, its stable handle in
    /// the view, whose indices a swap-remove may have renamed since it was published.
    type Building: Copy + Ord;
    /// Committed field reservations.
    fn fields(&self) -> &FieldClearanceIndex;
    /// Mines' extraction areas.
    fn extraction(&self) -> &FieldClearanceIndex;
    /// The zoning cells, painted and claimed.
    fn cells(&self) -> &CellStore<Self::Maps>;
    /// The lowest-index explicit site other than `ignore` (a building index) whose lot bounds
    /// meet `min`..`max` and whose lot passes `test`, as the allocator's
    /// [`crate::simulation::buildings::allocator::BuildingAllocator::first_explicit_site_lot`].
    fn first_explicit_site_lot(
        &self,
        min: Vector2,
        max: Vector2,
        ignore: Option<usize>,
        test: impl FnMut(&[Vector2; 4]) -> bool,
    ) -> Option<usize>;
    /// Visits the authored parcels (`ZONE-01`, no cell lot) whose bounds may meet `min`..`max`,
    /// each at least once, with their raw id and corners; nothing without authored parcels.
    fn visit_authored_parcels(
        &self,
        min: Vector2,
        max: Vector2,
        visit: impl FnMut(u64, &[Vector2; 4]),
    );
    /// The building standing on the parcel `lot`; `at` is a point inside the parcel.
    fn occupant(&self, lot: u64, at: DVec2) -> Option<Self::Building>;
    /// The asset of the building at `index` (an explicit site's), when the source holds the
    /// buildings: a refusal names it.
    fn building_asset(&self, _index: usize) -> Option<&str> {
        None
    }
}

/// The live core as a [`RoadLandSource`]: the commit's land, read with the site query index
/// prepared (else every explicit site is a candidate). It borrows only the three parts of the
/// core it reads, so a [`super::RoadEditSite`] can hold it beside the core's graph.
#[derive(Clone, Copy)]
pub(crate) struct CoreLand<'a> {
    allocator: &'a BuildingAllocator,
    zoning: &'a ZoningSystem,
    zone_cell_m: f32,
}

impl<'a> CoreLand<'a> {
    /// The land of `core`.
    pub(crate) fn of(core: &'a SimCore) -> Self {
        Self::from_parts(&core.allocator, &core.zoning, core.config.zone_cell_m)
    }

    /// The land of a core borrowed in parts.
    pub(crate) fn from_parts(
        allocator: &'a BuildingAllocator,
        zoning: &'a ZoningSystem,
        zone_cell_m: f32,
    ) -> Self {
        Self {
            allocator,
            zoning,
            zone_cell_m,
        }
    }
}

impl RoadLandSource for CoreLand<'_> {
    type Maps = crate::simulation::zoning::cells::LiveMaps;
    type Building = usize;

    fn fields(&self) -> &FieldClearanceIndex {
        &self.allocator.field_clearance
    }

    fn extraction(&self) -> &FieldClearanceIndex {
        &self.allocator.extraction_clearance
    }

    fn cells(&self) -> &CellStore {
        &self.zoning.cells
    }

    fn first_explicit_site_lot(
        &self,
        min: Vector2,
        max: Vector2,
        ignore: Option<usize>,
        test: impl FnMut(&[Vector2; 4]) -> bool,
    ) -> Option<usize> {
        self.allocator
            .first_explicit_site_lot(min, max, self.zone_cell_m, ignore, test)
    }

    fn visit_authored_parcels(
        &self,
        min: Vector2,
        max: Vector2,
        mut visit: impl FnMut(u64, &[Vector2; 4]),
    ) {
        let parcels = &self.zoning.parcels;
        if parcels.has_authored_parcels() {
            parcels.visit_in_bounds(min, max, |parcel| {
                if parcel.cell_lot().is_none() {
                    visit(parcel.id().raw(), &parcel.corners());
                }
            });
        }
    }

    fn occupant(&self, lot: u64, _at: DVec2) -> Option<usize> {
        self.zoning.parcel_by_raw_id(lot)?.occupied_building()
    }

    fn building_asset(&self, index: usize) -> Option<&str> {
        self.allocator
            .buildings
            .get(index)
            .map(|building| building.asset_id.as_str())
    }
}

/// What a road edit's new pieces take of the land under them (`ZONE-09`; the in-place edits,
/// `ZONE-10`): a plotted reservation refuses the edit; zoned land is taken, its lots removed with
/// the buildings on them and the paint of its covered cells erased. Only land the edit newly
/// covers counts: a field or extraction area when a new piece overlaps one and no replaced piece
/// overlapped any, a cell, lot or parcel when a new piece covers its road-contact interior and no
/// replaced piece did ([`road_piece_newly_covers`]). An edit may take land beyond its pieces by
/// the same rules, as extra footprints (a roundabout's island, decision D-ring:
/// [`footprint_newly_covers`]). A plotted placement (`ZONE-11`) takes the land under its lot or
/// polygon by the same rules, as one footprint and no pieces ([`RoadLandTake::of_footprint`]).
/// `B` names the buildings as the source does ([`RoadLandSource::Building`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadLandTake<B = usize> {
    /// The first plotted reservation, pieces in order and each piece's field, then extraction
    /// area, then explicit site, then the extra footprints in order; the other fields are empty
    /// when it is set.
    pub(crate) plotted: Option<PlottedOverlap>,
    /// The reserved cells the pieces newly cover, sorted by key, with their paint (zero for an
    /// occupied lot's grace claim).
    pub(crate) cells: Vec<(CellKey, u16)>,
    /// The parcels the edit removes, raw ids ascending: the lots claiming those cells and the
    /// authored parcels (`ZONE-01`) the pieces newly cover (decision D1).
    pub(crate) lots: Vec<u64>,
    /// A point inside each of `lots`, in the same order: the road tool's view finds a parcel by
    /// its chunk.
    pub(crate) lot_points: Vec<DVec2>,
    /// The buildings standing on those parcels, descending, the order in which their
    /// swap-removes leave the indices still to remove valid.
    pub(crate) buildings: Vec<B>,
}

impl<B> Default for RoadLandTake<B> {
    fn default() -> Self {
        Self {
            plotted: None,
            cells: Vec::new(),
            lots: Vec::new(),
            lot_points: Vec::new(),
            buildings: Vec::new(),
        }
    }
}

impl<B> RoadLandTake<B> {
    /// Whether the edit takes no zoned land: no cell, lot or parcel.
    pub(crate) fn is_empty(&self) -> bool {
        self.cells.is_empty() && self.lots.is_empty()
    }
}

impl RoadLandTake {
    /// [`Self::of`] against the live core, as the commit takes it, with no extra footprint.
    pub(crate) fn in_core(core: &SimCore, pieces: &[&PieceMesh], before: &[&PieceMesh]) -> Self {
        Self::of(&CoreLand::of(core), pieces, before, &[], None)
    }
}

// What newly covers land in a take: a piece the edit installs, or an extra footprint.
#[derive(Clone, Copy)]
enum Cover<'a> {
    Piece(&'a PieceMesh),
    Footprint(&'a PolygonFootprint<'static>),
}

impl Cover<'_> {
    // World XZ bounds `[min_x, min_z, max_x, max_z]`.
    fn bounds_xz(self) -> [f64; 4] {
        match self {
            Self::Piece(mesh) => mesh.bounds_xz(),
            Self::Footprint(footprint) => {
                [footprint.min.x, footprint.min.y, footprint.max.x, footprint.max.y].map(f64::from)
            }
        }
    }

    // Whether it newly covers a four-cornered footprint, `before` being the replaced pieces.
    fn newly_covers(self, before: &[&PieceMesh], corners: &[DVec2; 4]) -> bool {
        match self {
            Self::Piece(mesh) => road_piece_newly_covers(mesh, before, corners),
            Self::Footprint(footprint) => footprint_newly_covers(footprint, before, corners),
        }
    }

    // Whether it overlaps a polygon of `index` (fields or extraction areas) other than the
    // owner's.
    fn overlaps(self, index: &FieldClearanceIndex, owner: Option<usize>) -> bool {
        match self {
            Self::Piece(mesh) => index.overlaps_road_piece(mesh),
            Self::Footprint(footprint) => index.overlaps(footprint, owner),
        }
    }

    // The f32 query box that contains these bounds.
    fn query_box(self) -> (Vector2, Vector2) {
        let [min_x, min_z, max_x, max_z] = self.bounds_xz();
        (
            outward(min_x, min_z, f32::next_down),
            outward(max_x, max_z, f32::next_up),
        )
    }
}

impl<B: Copy + Ord> RoadLandTake<B> {
    /// The take of `pieces`, the pieces the edit installs, and of `extra`, land it takes beyond
    /// them, over `before`, the pieces it replaces, against `land`: the fields, extraction areas
    /// and explicit sites, cells and parcels. `owner` is the building (an index) whose own
    /// field or extraction polygon and own lot the land may overlap: a farm's field or a mine's
    /// area (`ZONE-11`); `None` for road edits. O((pieces + extra) * reservations in their
    /// bounds * top triangles) plus a lookup per covered cell and lot; a piece with nothing
    /// reserved in its bounds costs one bounds query per index.
    pub(crate) fn of<S: RoadLandSource<Building = B>>(
        land: &S,
        pieces: &[&PieceMesh],
        before: &[&PieceMesh],
        extra: &[PolygonFootprint<'static>],
        owner: Option<usize>,
    ) -> Self {
        let covers = || {
            pieces
                .iter()
                .map(|&mesh| Cover::Piece(mesh))
                .chain(extra.iter().map(Cover::Footprint))
        };
        let plotted = Self::first_plotted(land, covers(), before, owner);
        if plotted.is_some() {
            return Self {
                plotted,
                ..Self::default()
            };
        }
        let cells_store = land.cells();
        let mut keys = ZoningSystem::reserved_cells_newly_covered_by_road_pieces(
            cells_store,
            pieces.iter().copied(),
            before,
        );
        if !extra.is_empty() && cells_store.has_reservations() {
            for cover in extra.iter().map(Cover::Footprint) {
                let [min_x, min_z, max_x, max_z] = cover.bounds_xz();
                let bounds = CellBounds {
                    min: DVec2::new(min_x, min_z),
                    max: DVec2::new(max_x, max_z),
                };
                cells_store.visit_reserved_footprints_in_bounds(bounds, |key, corners| {
                    if cover.newly_covers(before, &corners) {
                        keys.push(key);
                    }
                });
            }
            keys.sort_unstable();
            keys.dedup();
        }
        let mut lots = Vec::new();
        let cells = keys
            .into_iter()
            .map(|key| {
                if let Some(lot) = cells_store.lot(key).filter(|&lot| lot != 0)
                    && let Some(frame) = cells_store.frame(key.grid)
                {
                    lots.push((lot, centroid(&frame.corners(key.x, key.y))));
                }
                (key, cells_store.profile(key).unwrap_or(0))
            })
            .collect();
        for cover in covers() {
            let (min, max) = cover.query_box();
            land.visit_authored_parcels(min, max, |id, corners| {
                let corners = corners.map(|p| DVec2::new(f64::from(p.x), f64::from(p.y)));
                if cover.newly_covers(before, &corners) {
                    lots.push((id, centroid(&corners)));
                }
            });
        }
        // One point per lot, the first in cell order, so the products do not depend on the
        // order the pieces visit them in.
        lots.sort_by_key(|&(id, _)| id);
        lots.dedup_by_key(|&mut (id, _)| id);
        let mut buildings: Vec<B> = lots
            .iter()
            .filter_map(|&(id, at)| land.occupant(id, at))
            .collect();
        buildings.sort_unstable_by(|a, b| b.cmp(a));
        buildings.dedup();
        let (lots, lot_points) = lots.into_iter().unzip();
        Self {
            plotted: None,
            cells,
            lots,
            lot_points,
            buildings,
        }
    }

    /// The land a plotted placement takes (`ZONE-11`): [`Self::of`] of its lot or polygon
    /// `footprint` alone, replacing no piece, `owner` the building whose own polygon and lot it
    /// may overlap. O(reservations in the footprint's bounds * its vertices) plus a lookup per
    /// covered cell and lot.
    pub(crate) fn of_footprint<S: RoadLandSource<Building = B>>(
        land: &S,
        footprint: &PolygonFootprint<'static>,
        owner: Option<usize>,
    ) -> Self {
        Self::of(land, &[], &[], std::slice::from_ref(footprint), owner)
    }

    // The first plotted reservation a cover newly covers, covers in order; `owner`'s own
    // polygons and lot are not plotted for it.
    fn first_plotted<'a, S: RoadLandSource>(
        land: &S,
        mut covers: impl Iterator<Item = Cover<'a>>,
        before: &[&PieceMesh],
        owner: Option<usize>,
    ) -> Option<PlottedOverlap> {
        let (fields, extraction) = (land.fields(), land.extraction());
        let fields_before = before.iter().any(|mesh| fields.overlaps_road_piece(mesh));
        let extraction_before = before
            .iter()
            .any(|mesh| extraction.overlaps_road_piece(mesh));
        covers.find_map(|cover| {
            if !fields_before && cover.overlaps(fields, owner) {
                return Some(PlottedOverlap::Field);
            }
            if !extraction_before && cover.overlaps(extraction, owner) {
                return Some(PlottedOverlap::ExtractionArea);
            }
            let (min, max) = cover.query_box();
            land.first_explicit_site_lot(min, max, owner, |lot| {
                let corners = lot.map(|p| DVec2::new(f64::from(p.x), f64::from(p.y)));
                cover.newly_covers(before, &corners)
            })
            .map(PlottedOverlap::Building)
        })
    }
}

/// [`RoadLandTake::of`]'s plotted reservation only: the stroke's refusal, for the one-shot
/// checks (`ROAD-102`, decision D8).
pub(crate) fn first_plotted_overlap<S: RoadLandSource>(
    land: &S,
    pieces: &[&PieceMesh],
    before: &[&PieceMesh],
) -> Option<PlottedOverlap> {
    RoadLandTake::<S::Building>::first_plotted(
        land,
        pieces.iter().map(|&mesh| Cover::Piece(mesh)),
        before,
        None,
    )
}

// The mean of a quad's corners: a point inside a convex lot or cell.
fn centroid(corners: &[DVec2; 4]) -> DVec2 {
    corners.iter().copied().sum::<DVec2>() * 0.25
}

// A world point rounded to f32 away from a query box's interior by `step` (`next_down` for its
// minimum, `next_up` for its maximum), so the f32 box contains the f64 one.
fn outward(x: f64, z: f64, step: fn(f32) -> f32) -> Vector2 {
    Vector2::new(step(x as f32), step(z as f32))
}

impl RoadEditPlan {
    /// Builds the local road solution from borrowed live inputs without cloning the city.
    pub(crate) fn compile(core: &SimCore, request: RoadPreviewRequest) -> Self {
        Self::compile_from(
            &core.heightmap,
            &core.region_graph,
            &core.transit_network.roadbed,
            request,
        )
    }

    /// Builds the local road solution from one generation's terrain, graph and roadbed: the
    /// live core's at commit, the road tool's context in a preview (`ROAD-68-E`), through the
    /// same preparation and validation, so the stroke reads no piece its edit rebuilds (`ROAD-68`
    /// open question 16). O(passes * (stroke points + local excerpt records)), one or two passes.
    pub(crate) fn compile_from(
        terrain: &TerrainSystem,
        graph: &RegionGraph,
        roadbed: &RoadbedSystem,
        request: RoadPreviewRequest,
    ) -> Self {
        let (prepared, validation, topology) = plan_road_edit(
            &request.points,
            request.fwd_lanes.clamp(0, 255) as u8,
            request.bkw_lanes.clamp(0, 255) as u8,
            (request.median, request.parking, request.attributes),
            terrain,
            graph,
            roadbed,
            request.snap_to_existing_roads,
            request.tunnel,
        );
        Self::new(
            request,
            terrain.source_generation(),
            prepared,
            validation,
            topology,
        )
    }

    /// The stroke this plan was compiled for.
    pub(crate) fn request(&self) -> &RoadPreviewRequest {
        &self.request
    }

    /// The prepared stroke.
    pub(crate) fn prepared(&self) -> &PreparedRoadInput {
        &self.prepared
    }

    /// Readiness against the live core. The local water query repeats under the commit lock, so
    /// a change in that independent system cannot authorize an obsolete result. Fields,
    /// extraction areas, plotted sites, parcels and painted cells are judged on the pieces the
    /// commit installs ([`RoadLandTake`], `ZONE-09`), which a plan alone does not have.
    pub(crate) fn status(&self, core: &SimCore) -> &'static str {
        if !self.validation.is_valid {
            return "invalid";
        }
        if self.topology.is_none() {
            return "provisional";
        }
        if self.request.surface_generation != core.road_tool_surface_generation
            || self.terrain_source_generation != core.heightmap.source_generation()
            || self.topology_for(&core.region_graph).is_none()
        {
            return "stale";
        }
        if !core.transit_network.roadbed.is_current() {
            return "pending";
        }
        if self.failed_check(core).is_some() {
            return "invalid";
        }
        "ready"
    }

    // The live-state check that refuses this road: water, the reason it names.
    fn failed_check(&self, core: &SimCore) -> Option<&'static str> {
        let fwd = self.request.fwd_lanes.clamp(0, 255) as u8;
        let bkw = self.request.bkw_lanes.clamp(0, 255) as u8;
        let validation = crate::nodes::sim::road_tool::validate_road_candidate_against_water(
            &self.prepared,
            fwd,
            bkw,
            &core.watermap,
            self.validation.clone(),
        );
        (!validation.is_valid).then_some(validation.invalid_reason)
    }

    /// Explains why this plan cannot be committed: the failed check when one failed, otherwise
    /// the plan's state (`road_plan_stale`, `road_plan_pending`, ...). Built only on refusal.
    pub(crate) fn rejection(&self, core: &SimCore) -> RoadRejection {
        let validation = &self.validation;
        let prepared = &self.prepared.points;
        if !validation.is_valid {
            return RoadRejection::from_validation(validation.invalid_reason, validation, prepared);
        }
        let status = self.status(core);
        if status == "invalid"
            && let Some(reason) = self.failed_check(core)
        {
            return RoadRejection::from_validation(reason, validation, prepared);
        }
        let (reason, detail) = match status {
            "stale" => (
                "road_plan_stale",
                "the road or terrain changed after the stroke was prepared",
            ),
            "pending" => (
                "road_plan_pending",
                "the roadbed was still being rebuilt for an earlier edit",
            ),
            "provisional" => (
                "road_plan_provisional",
                "the stroke's local road plan was incomplete",
            ),
            _ => (
                "road_plan_mismatch",
                "the prepared plan does not match the submitted stroke",
            ),
        };
        RoadRejection::new(reason, detail)
    }

    /// Retains the preparation and solved graph delta.
    pub(super) fn new(
        request: RoadPreviewRequest,
        terrain_source_generation: u64,
        prepared: PreparedRoadInput,
        validation: RoadPreviewValidation,
        topology: Option<RoadTopologyPlan>,
    ) -> Self {
        Self {
            request,
            terrain_source_generation,
            prepared,
            validation,
            topology,
        }
    }

    /// Borrows the solved input only for an exact click on unchanged road/terrain dependencies.
    pub(crate) fn prepared_input_for(
        &self,
        surface_generation: u64,
        terrain_source_generation: u64,
        raw_points: &[Vector3],
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> Option<&PreparedRoadInput> {
        (self.validation.is_valid
            && self.request.surface_generation == surface_generation
            && self.terrain_source_generation == terrain_source_generation
            && self.request.fwd_lanes == fwd_lanes
            && self.request.bkw_lanes == bkw_lanes
            && self.request.snap_to_existing_roads == snap_to_existing_roads
            && self.request.points == raw_points)
            .then_some(&self.prepared)
    }

    /// Returns the plan's road validation after exact input/dependency matching.
    pub(crate) fn validation(&self) -> &RoadPreviewValidation {
        &self.validation
    }

    /// Borrows the solved local delta after exact request/revision matching by the caller.
    pub(crate) fn topology_for(&self, graph: &RegionGraph) -> Option<&RoadTopologyPlan> {
        self.topology
            .as_ref()
            .filter(|topology| topology.source_identities_match(graph))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::road_edit::prepare_road_input_for_tool;
    use crate::simulation::network::roadbed::RoadbedSystem;
    use crate::simulation::terrain::TerrainSystem;

    #[test]
    fn plan_requires_exact_raw_inputs_and_source_revisions() {
        let raw = vec![Vector3::ZERO, Vector3::new(24.0, 0.0, 0.0)];
        let prepared = prepare_road_input_for_tool(
            &raw,
            &TerrainSystem::new(65, 65),
            &RegionGraph::new(),
            &RoadbedSystem::new(16.0),
            true,
            false,
        );
        let plan = RoadEditPlan::new(
            RoadPreviewRequest {
                surface_generation: 11,
                points: raw.clone(),
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
                tunnel: false,
                median: None,
                parking: crate::simulation::network::graph::RoadParking::NONE,
                attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
            },
            3,
            prepared,
            RoadPreviewValidation::valid(0.0),
            None,
        );
        assert!(plan.prepared_input_for(11, 3, &raw, 1, 1, true).is_some());
        assert!(plan.prepared_input_for(12, 3, &raw, 1, 1, true).is_none());
        assert!(plan.prepared_input_for(11, 4, &raw, 1, 1, true).is_none());
        assert!(plan.prepared_input_for(11, 3, &raw, 2, 1, true).is_none());
        assert!(plan.prepared_input_for(11, 3, &raw, 1, 2, true).is_none());
        assert!(plan.prepared_input_for(11, 3, &raw, 1, 1, false).is_none());
        let mut changed = raw.clone();
        changed[0].y += 0.01; // May prepare identically, but is still a different authored input.
        assert!(
            plan.prepared_input_for(11, 3, &changed, 1, 1, true)
                .is_none()
        );
        assert!(
            plan.prepared_input_for(11, 3, &plan.prepared.points, 1, 1, true)
                .is_none()
        );
    }
}
