// SPDX-License-Identifier: GPL-3.0-or-later

//! Bay markings generated from the validated places themselves (`PARK-30`): white lines along
//! each car place's sides and head, and L-shaped corner marks at a loading bay's four corners,
//! drawn on every lot the same way, so no asset models its own paint.
//!
//! Geometry is pure: a place's rectangle gives raw segments, and collinear segments that overlap
//! or touch are merged, so the side two neighbouring places share is drawn once and a row's head
//! lines become one line. Each merged line is one instance of a unit quad (one `MultiMesh` per
//! 512 m parking chunk, one draw call), lying on the visible surface: a line on a level pad is one
//! instance, one crossing a graded tie-in is split where the surface bends.
//!
//! The store keeps each building's rows, rebuilt only when that building's places or docks change
//! (site publication, load, a ground change under them), never by custody, so nothing runs per
//! frame; a chunk publishes its buildings' rows, shared with the store, one after another, so an
//! edit costs its own lot's lines and only the bridge joins a chunk's rows. A snapshot rebuilds
//! at most `PLACES_PER_SNAPSHOT` places' worth of marked buildings and leaves the rest to the next
//! ones, except after a load or a ground reset, which rebuild every building at once. Kerb places
//! draw nothing here (their bay paint is `PARK-04`). Render data only: nothing in the simulation
//! reads it, and it is never saved.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Width of every bay line and corner mark, metres: the Finnish lane line's 0.10 m.
pub(crate) const LINE_WIDTH_M: f32 = 0.10;
/// Length of each leg of a loading bay's corner mark, metres, from the corner's outer edge.
pub(crate) const CORNER_LEG_M: f32 = 0.8;
// How far a line floats above the surface: the road markings' decal lift.
const LIFT_M: f32 = crate::config::ROAD_DECAL_RENDER_Z_BIAS_M;
// Collinear lines closer than these (angle in radians, offset and gap in metres) are one line.
const ANGLE_STEP_RAD: f32 = 1.0e-4;
const OFFSET_STEP_M: f32 = 0.01;
const MERGE_GAP_M: f32 = 0.01;
// A line follows the surface to this tolerance, half the lift, so it never sinks into it; it is
// split no shorter than `MIN_PIECE_M`, the finest graded lattice.
const HEIGHT_TOLERANCE_M: f32 = LIFT_M * 0.5;
const MIN_PIECE_M: f32 = 0.5;
// The places and docks of the marked buildings one snapshot rebuilds at most (a building at
// least, so a larger lot still paints whole); the others wait, in id order, for the next
// snapshots. An hour's construction marks the lots within the graded change reach of its sites:
// on the 30k city up to 46 lots, 107 places, about 22 us of one worker a place on graded house
// driveways (2.4 ms in all), which landed on one frame. This share is under a tenth of a
// millisecond spread over the workers, bounded below by its dearest lot (0.2 ms), and that
// burst paints over four snapshots.
const PLACES_PER_SNAPSHOT: usize = 32;
/// Floats per instance: a row-major 3 x 4 transform, as `MultiMesh` takes it.
pub(crate) const STRIDE: usize = 12;

/// One place's rectangle as the paint reads it: the centre, the heading the place rows keep
/// (the forward axis, entry to head, is `(-sin, -cos)`), the width and the length, and whether it
/// is a loading bay (corner marks) rather than a car place (sides and head).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaintPlace {
    pub centre: [f32; 2],
    pub heading: f32,
    pub width: f32,
    pub length: f32,
    pub dock: bool,
}

impl PaintPlace {
    /// A place from its centre, its forward axis (entry to head, any length) and its size.
    pub(crate) fn from_forward(
        centre: [f32; 2],
        forward: [f32; 2],
        width: f32,
        length: f32,
        dock: bool,
    ) -> Self {
        Self {
            centre,
            heading: (-forward[0]).atan2(-forward[1]),
            width,
            length,
            dock,
        }
    }
}

/// One marking line in world `[x, z]`, `LINE_WIDTH_M` wide, centred on the segment.
pub(crate) type Segment = [[f32; 2]; 2];

/// One chunk's published paint: its buildings' instance rows, one after another in building id
/// order, each list shared with the store, and the store revision that last changed them.
#[derive(Clone, Debug, Default)]
pub(crate) struct PaintChunk {
    pub version: u64,
    pub buildings: Arc<[Arc<[f32]>]>,
}

impl PaintChunk {
    /// The floats of the chunk's rows. O(its buildings).
    pub(crate) fn len(&self) -> usize {
        self.buildings.iter().map(|rows| rows.len()).sum()
    }

    /// Whether the chunk holds no paint (it lost its last lot). O(1).
    pub(crate) fn is_empty(&self) -> bool {
        self.buildings.is_empty()
    }

    /// Copies the chunk's rows into `out`, which is [`Self::len`] floats long. O(rows).
    pub(crate) fn copy_rows(&self, out: &mut [f32]) {
        let mut start = 0;
        for rows in self.buildings.iter() {
            out[start..start + rows.len()].copy_from_slice(rows);
            start += rows.len();
        }
    }

    /// The chunk's rows, joined (tests).
    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<f32> {
        let mut out = vec![0.0; self.len()];
        self.copy_rows(&mut out);
        out
    }
}

/// The paint half of a render snapshot: every chunk that ever held paint (an empty one was
/// cleared at `version`) and the newest revision. Cloning copies one `Arc`.
#[derive(Clone, Debug, Default)]
pub(crate) struct PaintPublication {
    pub revision: u64,
    pub chunks: Arc<BTreeMap<(i32, i32), PaintChunk>>,
}

/// The paint store: the docks' chunk lists, the buildings to repaint, each building's rows and
/// chunk, each chunk's buildings, and the published rows.
#[derive(Debug, Default)]
pub(super) struct LotPaint {
    // The dock rows of each chunk (docks are in no car chunk list), for a ground change's
    // rectangle; `Place::chunk_slot` of a dock is its slot here, so removal is O(1).
    pub(super) docks: HashMap<(i32, i32), Vec<Handle<Place>>>,
    dirty: BTreeSet<u64>,
    // Every building was marked at once (a load, a ground reset): the next snapshot rebuilds all.
    full: bool,
    // Each painted building's chunk (that of its first painted place) and rows.
    buildings: HashMap<u64, BuildingPaint>,
    // Each chunk's painted buildings, in publication order.
    chunk_buildings: HashMap<(i32, i32), BTreeSet<u64>>,
    published: PaintPublication,
}

// A painted building's chunk and rows, shared with the chunks published since.
type BuildingPaint = ((i32, i32), Arc<[f32]>);

/// One building's rebuilt paint: its id, its chunk (none without a painted place) and rows.
pub(crate) type BuiltPaint = (u64, Option<(i32, i32)>, Vec<f32>);

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn scale(a: [f32; 2], s: f32) -> [f32; 2] {
    [a[0] * s, a[1] * s]
}

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

/// Pushes the raw segments of `place`: a car place's two sides (entry to head) and its head line,
/// stretched half a line past each side so the corners close square; the entry stays open to
/// the aisle. A loading bay's four corners get two legs each, along its two edges, starting half
/// a line outside the corner. O(1).
pub(crate) fn place_segments(place: &PaintPlace, out: &mut Vec<Segment>) {
    let (s, c) = place.heading.sin_cos();
    let forward = [-s, -c];
    let across = [-forward[1], forward[0]];
    let (hl, hw, half_line) = (place.length * 0.5, place.width * 0.5, LINE_WIDTH_M * 0.5);
    let corner = |f: f32, a: f32| add(add(place.centre, scale(forward, f)), scale(across, a));
    if !place.dock {
        for side in [-hw, hw] {
            out.push([corner(-hl, side), corner(hl, side)]);
        }
        out.push([corner(hl, -hw - half_line), corner(hl, hw + half_line)]);
        return;
    }
    // Each corner with the inward directions of its two edges and their lengths.
    let legs = [
        (forward, place.length.min(CORNER_LEG_M * 2.0) * 0.5),
        (across, place.width.min(CORNER_LEG_M * 2.0) * 0.5),
    ];
    for (f, a) in [(-hl, -hw), (-hl, hw), (hl, -hw), (hl, hw)] {
        let p = corner(f, a);
        let inward = [-f.signum(), -a.signum()];
        for (axis, (direction, edge)) in legs.iter().enumerate() {
            let d = scale(*direction, inward[axis]);
            let leg = edge.min(CORNER_LEG_M);
            out.push([add(p, scale(d, -half_line)), add(p, scale(d, leg))]);
        }
    }
}

// A segment on its line: the line's key (rounded angle and offset), its interval along the
// line's direction, the direction and the segment's own offset.
#[derive(Clone, Copy)]
struct OnLine {
    key: (i64, i64),
    start: f32,
    end: f32,
    direction: [f32; 2],
    offset: f32,
}

impl OnLine {
    fn of([a, b]: Segment) -> Option<Self> {
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        if (dx * dx + dz * dz).sqrt() <= f32::EPSILON {
            return None;
        }
        // A line's direction, either way along it: angle in [0, pi).
        let mut angle = dz.atan2(dx);
        if angle < 0.0 {
            angle += std::f32::consts::PI;
        }
        if angle >= std::f32::consts::PI - ANGLE_STEP_RAD * 0.5 {
            angle = 0.0;
        }
        let angle_key = (angle / ANGLE_STEP_RAD).round() as i64;
        let snapped = angle_key as f32 * ANGLE_STEP_RAD;
        let direction = [snapped.cos(), snapped.sin()];
        let offset = dot([-direction[1], direction[0]], a);
        let (ta, tb) = (dot(direction, a), dot(direction, b));
        Some(Self {
            key: (angle_key, (offset / OFFSET_STEP_M).round() as i64),
            start: ta.min(tb),
            end: ta.max(tb),
            direction,
            offset,
        })
    }

    fn segment(&self) -> Segment {
        let base = scale([-self.direction[1], self.direction[0]], self.offset);
        [
            add(base, scale(self.direction, self.start)),
            add(base, scale(self.direction, self.end)),
        ]
    }
}

/// Merges collinear segments that overlap or touch (within `MERGE_GAP_M`) into one, in place,
/// and sorts the result into a canonical order, so the paint of a set of places does not depend
/// on their order. Lines are collinear when their directions agree to `ANGLE_STEP_RAD` and their
/// offsets to `OFFSET_STEP_M` (both rounded to those steps); a merged line keeps the offset of
/// its first segment in that order. O(S log S) for S segments.
pub(crate) fn merge_segments(segments: &mut Vec<Segment>) {
    let mut keyed: Vec<OnLine> = segments.iter().copied().filter_map(OnLine::of).collect();
    keyed.sort_unstable_by(|x, y| {
        x.key
            .cmp(&y.key)
            .then(x.start.total_cmp(&y.start))
            .then(x.end.total_cmp(&y.end))
            .then(x.offset.total_cmp(&y.offset))
    });
    segments.clear();
    let mut run: Option<OnLine> = None;
    for item in keyed {
        match run.as_mut() {
            Some(current) if current.key == item.key && item.start <= current.end + MERGE_GAP_M => {
                current.end = current.end.max(item.end);
            }
            _ => {
                if let Some(done) = run.replace(item) {
                    segments.push(done.segment());
                }
            }
        }
    }
    if let Some(done) = run {
        segments.push(done.segment());
    }
}

/// The merged marking lines of `places`. O(P log P) for P places.
pub(crate) fn paint_segments(places: impl IntoIterator<Item = PaintPlace>) -> Vec<Segment> {
    let mut segments = Vec::new();
    for place in places {
        place_segments(&place, &mut segments);
    }
    merge_segments(&mut segments);
    segments
}

/// The square a graded gather covers for every height `paint_rows` reads on `segments`: its
/// centre and half side, the segments' bounds plus a centimetre; none without a segment. Every
/// point read lies on a segment, inside it. O(segments).
pub(crate) fn gather_square(segments: &[Segment]) -> Option<([f64; 2], f64)> {
    let mut points = segments.iter().flatten();
    let first = *points.next()?;
    let [min_x, min_z, max_x, max_z] = points.fold(
        [first[0], first[1], first[0], first[1]],
        |[a, b, c, d], p| [a.min(p[0]), b.min(p[1]), c.max(p[0]), d.max(p[1])],
    );
    let centre = [(min_x + max_x) * 0.5, (min_z + max_z) * 0.5].map(f64::from);
    let half = ((max_x - min_x) * 0.5).max((max_z - min_z) * 0.5);
    Some((centre, f64::from(half) + 0.01))
}

/// Appends the instance rows of `segments` to `rows`, each line lying on the surface `height`
/// gives (world `x`, `z` to `y`) plus the decal lift. A line is one instance where the surface is
/// straight along it to `HEIGHT_TOLERANCE_M` (three inner samples), else it is halved, down to
/// `MIN_PIECE_M`. O(lines + pieces) height queries; a level pad gives one instance and five
/// queries a line.
pub(crate) fn paint_rows(
    segments: &[Segment],
    mut height: impl FnMut(f32, f32) -> f32,
    rows: &mut Vec<f32>,
) {
    for &[a, b] in segments {
        let (ha, hb) = (height(a[0], a[1]), height(b[0], b[1]));
        piece(a, b, ha, hb, &mut height, rows);
    }
}

fn piece(
    a: [f32; 2],
    b: [f32; 2],
    ha: f32,
    hb: f32,
    height: &mut impl FnMut(f32, f32) -> f32,
    rows: &mut Vec<f32>,
) {
    let at = |t: f32| add(a, scale([b[0] - a[0], b[1] - a[1]], t));
    let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    if length > MIN_PIECE_M * 2.0 {
        let mid = at(0.5);
        let hm = height(mid[0], mid[1]);
        let bent = (hm - (ha + hb) * 0.5).abs() > HEIGHT_TOLERANCE_M
            || [0.25, 0.75].into_iter().any(|t| {
                let p = at(t);
                (height(p[0], p[1]) - (ha + (hb - ha) * t)).abs() > HEIGHT_TOLERANCE_M
            });
        if bent {
            piece(a, mid, ha, hm, height, rows);
            piece(mid, b, hm, hb, height, rows);
            return;
        }
    }
    push_instance(a, b, ha, hb, rows);
}

// One line from `a` to `b` (heights `ha`, `hb`): the unit quad's x along the line (tilted with
// it), its z across it, horizontal and `LINE_WIDTH_M` long, and its y the unit normal of both,
// so the quad's up normal turns with the line.
fn push_instance(a: [f32; 2], b: [f32; 2], ha: f32, hb: f32, rows: &mut Vec<f32>) {
    let x_axis = [b[0] - a[0], hb - ha, b[1] - a[1]];
    let flat = (x_axis[0] * x_axis[0] + x_axis[2] * x_axis[2]).sqrt();
    if flat <= f32::EPSILON {
        return;
    }
    let z_axis = [
        -x_axis[2] / flat * LINE_WIDTH_M,
        0.0,
        x_axis[0] / flat * LINE_WIDTH_M,
    ];
    // z cross x, normalised.
    let y = [
        z_axis[1] * x_axis[2] - z_axis[2] * x_axis[1],
        z_axis[2] * x_axis[0] - z_axis[0] * x_axis[2],
        z_axis[0] * x_axis[1] - z_axis[1] * x_axis[0],
    ];
    let norm = (y[0] * y[0] + y[1] * y[1] + y[2] * y[2]).sqrt();
    let y_axis = y.map(|v| v / norm);
    let origin = [
        (a[0] + b[0]) * 0.5,
        (ha + hb) * 0.5 + LIFT_M,
        (a[1] + b[1]) * 0.5,
    ];
    rows.extend_from_slice(&[
        x_axis[0], y_axis[0], z_axis[0], origin[0], x_axis[1], y_axis[1], z_axis[1], origin[1],
        x_axis[2], y_axis[2], z_axis[2], origin[2],
    ]);
}

impl ParkingSystem {
    // Notes that the paint of the building hosting `source` must be rebuilt: one of its places or
    // docks was added, removed or changed. Kerb places draw no paint. O(log dirty).
    pub(super) fn mark_paint(&mut self, source: PlaceSource) {
        if let PlaceSource::Building { building, .. } | PlaceSource::Dock { building, .. } = source
        {
            self.paint.dirty.insert(building);
        }
    }

    /// Marks every building with places, docks or paint, rebuilt at the next snapshot at once:
    /// a load, a ground reset (a reloaded world, a dense terrain undo). O(buildings log
    /// buildings).
    pub(crate) fn mark_all_paint(&mut self) {
        self.paint.full = true;
        let buildings = self
            .building_sources
            .keys()
            .chain(self.building_docks.keys())
            .chain(self.paint.buildings.keys());
        self.paint.dirty.extend(buildings);
    }

    // The buildings with a place or dock centred within `rect` (world `[min_x, min_z, max_x,
    // max_z]`), whose ground changed. O(chunks the rectangle touches + their places and docks).
    pub(super) fn mark_paint_in(&mut self, rect: [f32; 4]) {
        let (first, last) = (chunk_at([rect[0], rect[1]]), chunk_at([rect[2], rect[3]]));
        for z in first.1..=last.1 {
            for x in first.0..=last.0 {
                let places = self.chunks.get(&(x, z)).into_iter().flat_map(|c| &c.places);
                let docks = self.paint.docks.get(&(x, z)).into_iter().flatten();
                for &handle in places.chain(docks) {
                    let p = self.place_handles.resolve(handle).expect("indexed place");
                    let [px, _, pz] = self.places.position[p];
                    if (rect[0]..=rect[2]).contains(&px)
                        && (rect[1]..=rect[3]).contains(&pz)
                        && let PlaceSource::Building { building, .. }
                        | PlaceSource::Dock { building, .. } = self.places.source[p]
                    {
                        self.paint.dirty.insert(building);
                    }
                }
            }
        }
    }

    // Lists a new dock row in its chunk; returns its slot. O(1) amortised.
    pub(super) fn list_dock(&mut self, handle: Handle<Place>, position: [f32; 3]) -> usize {
        let list = self
            .paint
            .docks
            .entry(chunk_at([position[0], position[2]]))
            .or_default();
        list.push(handle);
        list.len() - 1
    }

    // Takes dock row `p` off its chunk's list. O(1).
    pub(super) fn unlist_dock(&mut self, p: usize) {
        let [x, _, z] = self.places.position[p];
        let key = chunk_at([x, z]);
        let slot = self.places.chunk_slot[p];
        let list = self.paint.docks.get_mut(&key).expect("listed dock");
        list.swap_remove(slot);
        if let Some(&moved) = list.get(slot) {
            self.places.chunk_slot[self.place_handles.resolve(moved).expect("live dock")] = slot;
        }
        if list.is_empty() {
            self.paint.docks.remove(&key);
        }
    }

    /// The next marked buildings whose paint this snapshot rebuilds, in id order, up to
    /// `PLACES_PER_SNAPSHOT` places and docks (one building at least; one without any counts as
    /// one), or every one after [`Self::mark_all_paint`]; the store forgets them.
    /// O(taken x (log marked + their places)).
    pub(crate) fn take_paint_dirty(&mut self) -> Vec<u64> {
        if std::mem::take(&mut self.paint.full) {
            return std::mem::take(&mut self.paint.dirty).into_iter().collect();
        }
        let mut taken = Vec::new();
        let mut places = 0;
        while places < PLACES_PER_SNAPSHOT
            && let Some(building) = self.paint.dirty.pop_first()
        {
            // A building without places (a demolished lot clears its paint) counts as one.
            places += (self
                .source_rows(self.building_sources.get(&building))
                .count()
                + self.source_rows(self.building_docks.get(&building)).count())
            .max(1);
            taken.push(building);
        }
        taken
    }

    /// Marks one building's paint for a rebuild, as an edit of its lot does (tests, diagnostics).
    #[cfg(test)]
    pub(crate) fn repaint_building(&mut self, building: u64) {
        self.paint.dirty.insert(building);
    }

    // The place rows of every source in `list`. O(rows).
    fn source_rows<'a>(
        &'a self,
        list: Option<&'a Vec<PlaceSource>>,
    ) -> impl Iterator<Item = &'a Handle<Place>> + 'a {
        list.into_iter()
            .flatten()
            .filter_map(|source| self.sources.get(source))
            .flatten()
    }

    /// The merged marking lines of `building`'s active places and docks, and the chunk of its
    /// first painted place (none without one). O(P log P) for its P places and docks.
    pub(crate) fn paint_lines(&self, building: u64) -> (Vec<Segment>, Option<(i32, i32)>) {
        let car = self
            .source_rows(self.building_sources.get(&building))
            .map(|&h| (h, false));
        let docks = self
            .source_rows(self.building_docks.get(&building))
            .map(|&h| (h, true));
        let mut chunk = None;
        let lines = paint_segments(car.chain(docks).filter_map(|(handle, dock)| {
            let p = self.place_handles.resolve(handle)?;
            self.places.active[p].then(|| {
                let [x, _, z] = self.places.position[p];
                chunk.get_or_insert(chunk_at([x, z]));
                PaintPlace {
                    centre: [x, z],
                    heading: self.places.heading[p],
                    width: self.places.width[p],
                    length: self.places.length[p],
                    dock,
                }
            })
        }));
        (lines, chunk)
    }

    /// Takes rebuilt buildings and republishes the chunks whose rows changed, each as its
    /// buildings' rows in id order, under one new revision; nothing changing moves nothing.
    /// O(rebuilt rows + the changed chunks' buildings), plus one copy of the chunk map while a
    /// snapshot holds it: a chunk's rows are shared, never copied.
    pub(crate) fn install_paint(&mut self, built: Vec<BuiltPaint>) {
        let paint = &mut self.paint;
        let mut changed = BTreeSet::new();
        for (building, chunk, rows) in built {
            let same = |(old, old_rows): &BuildingPaint| {
                Some(*old) == chunk
                    && old_rows.len() == rows.len()
                    && old_rows
                        .iter()
                        .zip(&rows)
                        .all(|(a, b)| a.to_bits() == b.to_bits())
            };
            if paint.buildings.get(&building).is_some_and(same)
                || (rows.is_empty() && !paint.buildings.contains_key(&building))
            {
                continue;
            }
            if let Some((old, _)) = paint.buildings.remove(&building) {
                changed.insert(old);
                if let Some(set) = paint.chunk_buildings.get_mut(&old) {
                    set.remove(&building);
                }
            }
            if let (Some(chunk), false) = (chunk, rows.is_empty()) {
                changed.insert(chunk);
                paint
                    .chunk_buildings
                    .entry(chunk)
                    .or_default()
                    .insert(building);
                paint.buildings.insert(building, (chunk, rows.into()));
            }
        }
        if changed.is_empty() {
            return;
        }
        let revision = paint.published.revision + 1;
        paint.published.revision = revision;
        let chunks = Arc::make_mut(&mut paint.published.chunks);
        for key in changed {
            let buildings: Arc<[Arc<[f32]>]> = paint
                .chunk_buildings
                .get(&key)
                .into_iter()
                .flatten()
                .map(|building| Arc::clone(&paint.buildings[building].1))
                .collect();
            if paint
                .chunk_buildings
                .get(&key)
                .is_some_and(BTreeSet::is_empty)
            {
                paint.chunk_buildings.remove(&key);
            }
            chunks.insert(
                key,
                PaintChunk {
                    version: revision,
                    buildings,
                },
            );
        }
    }

    /// The paint half of a snapshot. O(1).
    pub(crate) fn paint_publication(&self) -> PaintPublication {
        self.paint.published.clone()
    }

    /// Whether any building's paint waits to be rebuilt. O(1).
    pub(crate) fn has_paint_dirty(&self) -> bool {
        !self.paint.dirty.is_empty()
    }
}

#[cfg(test)]
mod tests;
