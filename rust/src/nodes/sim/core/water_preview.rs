//! Transient authored-water preview state and render diagnostics.

/// Validation state for one transient world-editor lake-fill preview.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorldLakeFillPreviewStatus {
    /// The preview covers a closed basin and can be committed.
    Ready,
    /// The chosen surface is at or below the seed terrain height.
    SurfaceBelowSeedTerrain,
    /// The chosen surface spills out of the basin and reaches the world edge.
    EscapesWorldEdge,
    /// The chosen open-water surface does not connect to the world edge.
    DoesNotReachWorldEdge,
}

/// Preview feature kind for world-editor surface fills.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorldWaterFillKind {
    /// Closed inland basin fill.
    Lake,
    /// Edge-connected open-water fill.
    OpenWater,
}

/// Debug summary for one authored water fill that contributes to a render patch.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AuthoredWaterPatchFillDebug {
    /// Authored fill kind that produced the patch contribution.
    pub(crate) kind: WorldWaterFillKind,
    /// Committed fill index in its authored list, or `-1` for a transient preview.
    pub(crate) fill_index: i32,
    /// Whether this contribution came from the active transient preview.
    pub(crate) preview: bool,
    /// Snapped seed X coordinate in world metres.
    pub(crate) world_x: f32,
    /// Snapped seed Z coordinate in world metres.
    pub(crate) world_z: f32,
    /// Authored flat water surface elevation in metres.
    pub(crate) surface_elevation_m: f32,
    /// Number of cells in the complete fill body.
    pub(crate) filled_cells: usize,
    /// Whether the complete fill body touches the world edge.
    pub(crate) touches_world_edge: bool,
    /// Number of non-zero water samples contributed inside the requested patch.
    pub(crate) patch_nonzero_samples: usize,
    /// Maximum contributed water depth inside the requested patch.
    pub(crate) patch_max_depth_m: f32,
    /// Sum of contributed water depths inside the requested patch.
    pub(crate) patch_sum_depth_m: f32,
}

/// Transient lake-fill preview state owned by the world editor runtime.
///
/// This state is never serialized into `WorldDefinition`. It exists only so the
/// editor can show live water feedback while the author adjusts the target
/// surface elevation before confirming the lake fill.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLakeFillPreview {
    /// Preview feature kind.
    pub(crate) kind: WorldWaterFillKind,
    /// Snapped seed X coordinate in world metres.
    pub(crate) seed_world_x: f32,
    /// Snapped seed Z coordinate in world metres.
    pub(crate) seed_world_z: f32,
    /// Seed terrain height in rendered world metres.
    pub(crate) seed_height_m: f32,
    /// Preview surface elevation in rendered world metres.
    pub(crate) surface_elevation_m: f32,
    /// Preview validation outcome.
    pub(crate) status: WorldLakeFillPreviewStatus,
    /// Number of filled terrain cells in the preview flood.
    pub(crate) filled_cells: usize,
}

impl WorldLakeFillPreview {
    /// Returns `true` when the preview is valid and may be committed.
    pub(crate) fn is_valid(self) -> bool {
        self.status == WorldLakeFillPreviewStatus::Ready
    }
}
