"""
Lakes and rivers for a DEM-imported world, from national hydrography polygons.

Editor-time only, used by `import_dem_world_definition.py`. A laser-scanned elevation model is
flat at the water surface over lakes, so the lake bed is synthetic:

    bed = level - min(depth_cap, slope * d)

with `d` the distance from the sample to the shore of the water at the sample's level, and
`depth_cap` lowered to the shoal cap inside a shoal polygon. With a depth chart (`DepthChart`:
surveyed contours and soundings), a charted polygon's bed comes from the chart instead
(`_charted_polygons`, `_charted_depths`); uncharted water keeps the synthetic bed. Samples outside every water polygon
keep the elevation model. The runtime floods an authored fill 4-connected over samples at or
below its level (`FillFlood::flood`), so the shore rules below make each fill flood
exactly its own samples:

- A land sample beside water is raised to at least the water's level plus a margin.
- Water connected across polygon boundaries is one group with one fill at the lowest level
  among its polygons (the runtime holds one flat surface per connected body of water); a group
  touching the world edge is an open-water fill, any other a lake fill.
- A river is imported only as a strait: where it joins at least two lake polygons whose water
  stands at one level (within `river_join_tolerance_mm`), and then it takes that level. Any
  other river (one joining lakes at levels further apart, an inlet or outlet joining one lake,
  a river joining none) is not imported: its samples keep the elevation model. A flat surface
  on a river that slopes away from its lake would be carved under the ground upstream or held
  above it by raised shores downstream (`_river_joins` defines "touches" and "system").
- A river polygon narrower than `min_river_width_cells` terrain cells is not imported: no bed,
  no fill, its samples keep the elevation model. At a few samples across, a river breaks into
  fragments. Lakes are not subject to this rule.

A polygon's width is its inscribed width: twice the largest distance from a point inside it to
its boundary (the diameter of its largest inscribed circle), over the whole polygon, not the part
inside the world. GEOS's maximum inscribed circle returns a centre within `width_tolerance_m` of
the true one, so the true radius lies in `[r, r + tolerance]` of the returned `r`; a river whose
interval straddles the limit stops the import instead of being decided either way.

Everything here is deterministic: inputs are sorted, ties are broken by sample index, and no
step depends on hash or thread order.
"""

from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np
import shapely
from scipy import ndimage
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components

LAND = -1


@dataclass(frozen=True)
class WaterRule:
    """The synthetic bed and shore parameters."""

    slope: float = 0.15
    depth_cap_m: float = 12.0
    shoal_depth_cap_m: float = 1.0
    shore_margin_m: float = 0.05
    min_samples: int = 4
    min_river_width_cells: float = 2.0
    width_tolerance_m: float = 0.001
    # A strait is imported only when the lake levels it joins agree within this spread
    # (`_river_joins`). The levels are given to 0.1 m; one lake system's polygons span 0.3 m on
    # the shipped Kuopio and straits on the 60 km Kuopio join 0.1-0.4 m.
    river_join_tolerance_mm: int = 500


@dataclass(frozen=True)
class SampleGrid:
    """Terrain samples in source coordinates: sample (i, j) is at E = west + i * cell,
    N = north - j * cell; world X grows east and world Z south from the centre."""

    west_e: float
    north_n: float
    cell_m: float
    width: int
    height: int

    @property
    def east_e(self) -> float:
        return self.west_e + (self.width - 1) * self.cell_m

    @property
    def south_n(self) -> float:
        return self.north_n - (self.height - 1) * self.cell_m

    def world_xz(self, i: int, j: int) -> tuple[float, float]:
        half_w = (self.width - 1) * self.cell_m * 0.5
        half_h = (self.height - 1) * self.cell_m * 0.5
        return i * self.cell_m - half_w, j * self.cell_m - half_h

    def nearest_sample(self, e: float, n: float) -> tuple[int, int]:
        i = int(np.clip(round((e - self.west_e) / self.cell_m), 0, self.width - 1))
        j = int(np.clip(round((self.north_n - n) / self.cell_m), 0, self.height - 1))
        return i, j

    def box(self):
        return shapely.box(self.west_e, self.south_n, self.east_e, self.north_n)


@dataclass(frozen=True)
class WaterBody:
    """One hydrography polygon in source coordinates."""

    key: int
    kind: str  # "lake" or "river"
    # A lake's level; None for a river, which takes the level of the lakes it joins.
    level_mm: int | None
    geometry: object
    interior_en: tuple[float, float] | None = None

    @property
    def level_m(self) -> float | None:
        return None if self.level_mm is None else self.level_mm / 1000.0


@dataclass
class WaterFill:
    """One fill for the world definition, at a sample centre in world metres."""

    kind: str  # "lake" or "open_water"
    world_x: float
    world_z: float
    surface_elevation_m: float
    group: int
    bodies: list[int] = field(default_factory=list)


@dataclass
class WaterResult:
    heights: np.ndarray
    body_of_sample: np.ndarray  # index into the sorted bodies, or LAND
    group_of_sample: np.ndarray  # fill group per water sample, -1 elsewhere
    bodies: list[WaterBody]
    fills: list[WaterFill]
    report: dict
    # With a depth chart: per charted sample the share of its depth's weight held by shore points
    # (1 where no contour or sounding is in its triangle), NaN elsewhere; None without a chart.
    chart_shore_weight: np.ndarray | None = None


@dataclass(frozen=True)
class DepthChart:
    """Surveyed depths below the water surface (`--depths`): contour lines and soundings, each
    `(key, depth_mm, geometry)` in source coordinates, sorted by key."""

    contours: tuple
    soundings: tuple


def synthetic_depth(distance_m, cap_m, rule: WaterRule) -> np.ndarray:
    """`min(cap, slope * d)` in f64; `cap_m` is per sample (the shoal cap inside shoals)."""
    return np.minimum(np.asarray(cap_m, dtype=np.float64),
                      rule.slope * np.asarray(distance_m, dtype=np.float64))


def bed_below(level_m, depth_m) -> np.ndarray:
    """`level - depth` in f32. A bed that rounds to its level (a sample within micrometres of the
    shore, or a charted depth of 0) is moved one f32 step below it, so the runtime counts it wet
    rather than flooded at depth 0."""
    level = np.asarray(level_m, dtype=np.float64)
    bed = (level - np.asarray(depth_m, dtype=np.float64)).astype(np.float32)
    surface = level.astype(np.float32)
    return np.where(bed < surface, bed, np.nextafter(surface, np.float32(-np.inf)))


def synthetic_bed(level_m, distance_m, cap_m, rule: WaterRule) -> np.ndarray:
    """`level - min(cap, slope * d)` in f32 (`synthetic_depth`, `bed_below`)."""
    return bed_below(level_m, synthetic_depth(distance_m, cap_m, rule))


def inscribed_width_m(geometry, tolerance_m: float) -> float:
    """Twice the radius GEOS's maximum inscribed circle returns: a lower bound on the polygon's
    inscribed width, at most `2 * tolerance_m` below it."""
    return 2.0 * float(shapely.maximum_inscribed_circle(geometry, tolerance=tolerance_m).length)


def river_too_narrow(body: WaterBody, cell_m: float, rule: WaterRule) -> tuple[bool, float]:
    """Whether a river's inscribed width is under `min_river_width_cells` cells, and the width.
    Raises when the tolerance leaves the answer open (the true width may lie on either side)."""
    limit = rule.min_river_width_cells * cell_m
    width = inscribed_width_m(body.geometry, rule.width_tolerance_m)
    if width >= limit:
        return False, width
    if width + 2.0 * rule.width_tolerance_m < limit:
        return True, width
    raise ValueError(
        f"river {body.key}: inscribed width {width:.4f} m is within the "
        f"{rule.width_tolerance_m} m tolerance of the {limit} m limit"
    )


def sort_bodies(bodies: list[WaterBody]) -> list[WaterBody]:
    # A lake owns a sample it shares with a river, the lower lake one two lakes share; key breaks
    # ties.
    return sorted(bodies, key=lambda b: (b.kind != "lake", b.level_mm or 0, b.key))


def _subgrid(grid: SampleGrid, bounds) -> tuple[int, int, int, int] | None:
    min_e, min_n, max_e, max_n = bounds
    i0 = max(0, int(np.ceil((min_e - grid.west_e) / grid.cell_m)))
    i1 = min(grid.width - 1, int(np.floor((max_e - grid.west_e) / grid.cell_m)))
    j0 = max(0, int(np.ceil((grid.north_n - max_n) / grid.cell_m)))
    j1 = min(grid.height - 1, int(np.floor((grid.north_n - min_n) / grid.cell_m)))
    if i0 > i1 or j0 > j1:
        return None
    return i0, i1, j0, j1


def _samples_inside(grid: SampleGrid, geometry) -> tuple[np.ndarray, np.ndarray]:
    """Row and column indices of samples strictly inside `geometry` (sample centre test)."""
    window = _subgrid(grid, geometry.bounds)
    if window is None:
        return np.empty(0, dtype=np.int64), np.empty(0, dtype=np.int64)
    i0, i1, j0, j1 = window
    jj, ii = np.mgrid[j0 : j1 + 1, i0 : i1 + 1]
    e = grid.west_e + ii * grid.cell_m
    n = grid.north_n - jj * grid.cell_m
    shapely.prepare(geometry)
    inside = shapely.contains_xy(geometry, e, n)
    return jj[inside].astype(np.int64), ii[inside].astype(np.int64)


def _shore_segments(shore) -> list:
    """`shore`'s lines as two-point segments."""
    coords = []
    for line in shapely.get_parts(shore):
        xy = shapely.get_coordinates(line)
        if len(xy) >= 2:
            coords.append(np.stack([xy[:-1], xy[1:]], axis=1))
    return list(shapely.linestrings(np.concatenate(coords))) if coords else []


def _shore_distance(grid: SampleGrid, segments, rows, cols, max_distance: float) -> np.ndarray:
    """Distance from each sample to the nearest shore segment; `inf` beyond `max_distance`."""
    out = np.full(rows.shape, np.inf)
    if not segments or rows.size == 0:
        return out
    tree = shapely.STRtree(segments)
    points = shapely.points(grid.west_e + cols * grid.cell_m, grid.north_n - rows * grid.cell_m)
    (point_idx, seg_idx), dist = tree.query_nearest(
        points, max_distance=max_distance, return_distance=True, all_matches=True
    )
    np.minimum.at(out, point_idx, dist)
    return out


def _neighbours(array: np.ndarray, fill):
    """The four 4-neighbour views of `array` (west, east, north, south), padded with `fill`."""
    padded = np.pad(array, 1, constant_values=fill)
    return (padded[1:-1, :-2], padded[1:-1, 2:], padded[:-2, 1:-1], padded[2:, 1:-1])


def _east_south_pairs(array: np.ndarray):
    """Each 4-neighbour sample pair once: (west, east) and (north, south) views of `array`."""
    return ((array[:, :-1], array[:, 1:]), (array[:-1, :], array[1:, :]))


def _river_joins(body_of: np.ndarray, bodies: list[WaterBody], rule: WaterRule):
    """The river rule, on the polygons that hold samples after the minimum-sample drop: a river is
    imported only as a strait.

    Two such polygons touch when they intersect (a shared edge or vertex counts) or when a sample
    of one is a 4-neighbour of a sample of the other. The second makes the rule match the groups
    (water samples connected 4-way); the first catches a river mouth whose samples miss the lake's
    by a cell. A river system is a set of rivers linked by touches between rivers (not through a
    lake). The water a lake polygon joins a system at is every lake component its samples lie in
    (4-connected lake samples, each at the lowest level among its lakes: the level that water
    stands at). A system is a strait when it touches at least two lake polygons and their
    components' levels span at most `river_join_tolerance_mm`: every river of it takes the lowest
    of those levels, the level its group stands at. No river of any other system is imported. Two
    lakes are counted as polygons, not components, because a strait between two parts of one lake
    system joins a single component.

    Returns `{body index: level_mm}` for the rivers of straits and `{body index: report entry}`
    for the rivers not imported. Touches, systems and components are symmetric relations and
    their connected components, so the result does not depend on the order of the polygons."""
    is_river = np.array([b.kind == "river" for b in bodies] + [False])
    if not is_river.any():
        return {}, {}
    count = len(bodies)
    level = np.array([b.level_mm or 0 for b in bodies] + [0], dtype=np.int64)
    lake = (body_of >= 0) & ~is_river[body_of]  # LAND (-1) reads the trailing False
    lake_comp, comp_count = ndimage.label(lake)
    comp_level = np.full(comp_count + 1, np.iinfo(np.int64).max, dtype=np.int64)
    np.minimum.at(comp_level, lake_comp[lake], level[body_of[lake]])
    lake_of_comp, comp_of_lake = np.divmod(
        np.unique(body_of[lake].astype(np.int64) * (comp_count + 1) + lake_comp[lake]), comp_count + 1
    )

    touches = []
    for a, b in _east_south_pairs(body_of):
        side = (a != b) & (a >= 0) & (b >= 0) & (is_river[a] | is_river[b])
        touches.append(np.unique(a[side].astype(np.int64) * count + b[side]))
    holding = np.unique(body_of[body_of >= 0]).astype(np.int64)
    rivers = holding[is_river[holding]]
    tree = shapely.STRtree([bodies[i].geometry for i in holding])
    river_hit, hit = tree.query([bodies[i].geometry for i in rivers], predicate="intersects")
    touches.append(rivers[river_hit] * count + holding[hit])
    a, b = np.divmod(np.unique(np.concatenate(touches)), count)
    a, b = a[a != b], b[a != b]

    # River systems: connected components of the river-to-river touches.
    both = is_river[a] & is_river[b]
    links = coo_matrix((np.ones(int(both.sum())), (a[both], b[both])), shape=(count, count))
    _, system_of = connected_components(links, directed=False)
    one = is_river[a] != is_river[b]
    touch_river = np.where(is_river[a], a, b)[one]
    touch_lake = np.where(is_river[a], b, a)[one]

    joined = {}
    dropped = {}
    for system in np.unique(system_of[rivers]):
        members = rivers[system_of[rivers] == system]
        lakes = np.unique(touch_lake[np.isin(touch_river, members)])
        levels = np.unique(comp_level[comp_of_lake[np.isin(lake_of_comp, lakes)]])
        if lakes.size >= 2 and levels[-1] - levels[0] <= rule.river_join_tolerance_mm:
            joined.update({int(index): int(levels[0]) for index in members})
            continue
        reason = (
            "joins lake water at levels further apart than the tolerance" if lakes.size >= 2
            else "joins fewer than two lakes"
        )
        system_keys = sorted(bodies[int(index)].key for index in members)
        lake_keys = sorted(bodies[int(index)].key for index in lakes)
        for index in members:
            dropped[int(index)] = {
                "key": bodies[int(index)].key,
                "reason": reason,
                "joined_levels_m": [int(v) / 1000.0 for v in levels],
                "lakes": lake_keys,
                "system": system_keys,
            }
    return joined, dropped


def _charted_polygons(bodies: list[WaterBody], kept: list[int], depths: DepthChart):
    """The kept polygons the depth chart covers: a polygon is charted when a sounding lies inside
    it or it holds more than half of some contour's length. A contour is drawn for one lake: on
    the 60 km maps 3,422 of Kuopio's 3,463 contours and 1,568 of Vieremä's 1,601 lie within one
    polygon, and the majority keeps a contour that grazes a neighbour's shore (by 0.24 mm on one
    5 km2 Kuopio polygon) from charting that neighbour. Returns the charted indices into `bodies`
    and the count of contours and soundings no kept polygon takes."""
    geometries = [bodies[k].geometry for k in kept]
    tree = shapely.STRtree(geometries)
    charted = set()
    contours = np.array([geometry for _, _, geometry in depths.contours], dtype=object)
    contour_hit, polygon_hit = tree.query(contours, predicate="intersects")
    inside = shapely.length(shapely.intersection(
        contours[contour_hit], np.array(geometries, dtype=object)[polygon_hit]
    ))
    owner = inside * 2.0 > shapely.length(contours[contour_hit])
    charted.update(kept[int(p)] for p in polygon_hit[owner])
    unowned_contours = len(contours) - int(np.unique(contour_hit[owner]).size)
    points = np.array([geometry for _, _, geometry in depths.soundings], dtype=object)
    point_hit, polygon_hit = tree.query(points, predicate="within")
    charted.update(kept[int(p)] for p in polygon_hit)
    unowned_soundings = len(points) - int(np.unique(point_hit).size)
    return charted, unowned_contours, unowned_soundings


def _line_points(lines, cell_m: float) -> np.ndarray:
    """The vertices of `lines` with every segment split to at most one terrain cell."""
    if lines is None or shapely.is_empty(lines):
        return np.empty((0, 2))
    return shapely.get_coordinates(shapely.segmentize(lines, cell_m))


def _charted_depths(grid, bodies, body_of, groups, charted, kept, shore_segments, depths, rule):
    """Two arrays over the samples of charted polygons (NaN elsewhere): the depth the chart's
    points give, linear inside their Delaunay triangles, and the share of that depth's weight
    held by shore points (the sum of the shore vertices' barycentric weights). One triangulation
    per group and its charted polygons together (their union `R`), from:

    - the shore (the boundary of `R` outside every other kept polygon) at depth 0;
    - the seam (the boundary of `R` shared with uncharted kept water) at the synthetic depth
      without the shoal cap, so the bed meets the uncharted bed without a step;
    - contours clipped to `R` at their depth (a part outside it, on land or in uncharted water,
      is not used) and soundings inside `R` at theirs.

    Lines are split to one terrain cell so the triangles follow them; a point given twice takes
    the first in that order (shore, seam, contours, soundings, each by key). The polygons are not
    clipped to the world, so a lake cut by the world edge keeps its shore beyond it. Qhull's
    Delaunay is deterministic for one input order; a sample outside every triangle stops the
    import."""
    from scipy.spatial import Delaunay

    depth = np.full(body_of.shape, np.nan)
    shore_weight = np.full(body_of.shape, np.nan)
    charted_mask = np.isin(body_of, sorted(charted))
    if not charted_mask.any():
        return depth, shore_weight
    kept_tree = shapely.STRtree([bodies[k].geometry for k in kept])
    contour_tree = shapely.STRtree([geometry for _, _, geometry in depths.contours])
    sounding_tree = shapely.STRtree([geometry for _, _, geometry in depths.soundings])
    shore_tree = shapely.STRtree(shore_segments)
    slices = ndimage.find_objects(np.where(charted_mask, groups, 0))
    origin = np.array([grid.west_e, grid.north_n])
    for label, window in enumerate(slices, start=1):
        if window is None:
            continue
        mask = charted_mask[window] & (groups[window] == label)
        members = [int(b) for b in np.unique(body_of[window][mask])]
        region = shapely.union_all([bodies[b].geometry for b in members])
        others = [kept[int(i)] for i in kept_tree.query(region, predicate="intersects")]
        other = shapely.union_all([bodies[b].geometry for b in others if b not in members])
        boundary = shapely.boundary(region)
        shore = _line_points(shapely.difference(boundary, other), grid.cell_m)
        seam = _line_points(shapely.intersection(boundary, other), grid.cell_m)
        seam_distance = np.full(len(seam), np.inf)
        if len(seam):
            (point_idx, _), dist = shore_tree.query_nearest(
                shapely.points(seam), max_distance=rule.depth_cap_m / rule.slope,
                return_distance=True, all_matches=True,
            )
            np.minimum.at(seam_distance, point_idx, dist)
        parts = [(shore, np.zeros(len(shore))), (seam, synthetic_depth(seam_distance, rule.depth_cap_m, rule))]
        for index in sorted(int(i) for i in contour_tree.query(region, predicate="intersects")):
            _, depth_mm, line = depths.contours[index]
            xy = _line_points(shapely.intersection(line, region), grid.cell_m)
            parts.append((xy, np.full(len(xy), depth_mm / 1000.0)))
        for index in sorted(int(i) for i in sounding_tree.query(region, predicate="contains")):
            _, depth_mm, point = depths.soundings[index]
            parts.append((shapely.get_coordinates(point), np.array([depth_mm / 1000.0])))
        xy = np.concatenate([p for p, _ in parts]) - origin
        value = np.concatenate([v for _, v in parts])
        on_shore = np.arange(len(xy)) < len(shore)
        _, first = np.unique(xy, axis=0, return_index=True)
        first.sort()
        triangulation = Delaunay(xy[first])
        value, on_shore = value[first], on_shore[first]

        rows, cols = np.nonzero(mask)
        samples = np.stack([(cols + window[1].start) * grid.cell_m,
                            -(rows + window[0].start) * grid.cell_m], axis=1)
        simplex = triangulation.find_simplex(samples)
        if np.any(simplex < 0):
            raise RuntimeError(f"charted water group {label}: {int(np.sum(simplex < 0))} samples outside the triangulation")
        transform = triangulation.transform[simplex]
        b = np.einsum("nij,nj->ni", transform[:, :2], samples - transform[:, 2])
        weights = np.concatenate([b, 1.0 - b.sum(axis=1, keepdims=True)], axis=1)
        vertices = triangulation.simplices[simplex]
        at = (rows + window[0].start, cols + window[1].start)
        depth[at] = np.maximum(np.einsum("ni,ni->n", weights, value[vertices]), 0.0)
        shore_weight[at] = np.clip(np.einsum("ni,ni->n", weights, on_shore[vertices].astype(np.float64)), 0.0, 1.0)
    return depth, shore_weight


def apply_hydrography(
    dem: np.ndarray,
    grid: SampleGrid,
    bodies: list[WaterBody],
    shoals: list,
    rule: WaterRule,
    depths: DepthChart | None = None,
) -> WaterResult:
    """Carves the beds (synthetic, or from `depths` in charted polygons), raises the shores and
    returns one fill per water group."""
    if dem.shape != (grid.height, grid.width):
        raise ValueError(f"DEM shape {dem.shape} does not match the sample grid")
    world = grid.box()
    bodies = [b for b in bodies if b.geometry.intersects(world)]
    bodies_in_world = len(bodies)
    river_width = {}
    narrow_rivers = []
    for body in sorted(bodies, key=lambda b: b.key):
        if body.kind == "river":
            narrow, width = river_too_narrow(body, grid.cell_m, rule)
            river_width[body.key] = width
            if narrow:
                narrow_rivers.append({"key": body.key, "inscribed_width_m": width})
    narrow_keys = {river["key"] for river in narrow_rivers}
    bodies = sort_bodies([b for b in bodies if b.key not in narrow_keys])
    body_of = np.full(dem.shape, LAND, dtype=np.int32)
    body_samples = []
    for index, body in enumerate(bodies):
        rows, cols = _samples_inside(grid, body.geometry)
        free = body_of[rows, cols] == LAND
        overlap = int(np.count_nonzero(~free))
        rows, cols = rows[free], cols[free]
        body_of[rows, cols] = index
        body_samples.append({"samples": int(rows.size), "overlap_samples": overlap})

    dropped = []
    for index, body in enumerate(bodies):
        if body_samples[index]["samples"] < rule.min_samples:
            body_of[body_of == index] = LAND
            dropped.append(body.key)

    level_of_body = np.array([b.level_mm or 0 for b in bodies] + [0], dtype=np.int64)
    joined_rivers, not_joined = _river_joins(body_of, bodies, rule)
    body_of[np.isin(body_of, sorted(not_joined))] = LAND
    for index, level in joined_rivers.items():
        level_of_body[index] = level
    not_joined_rivers = sorted(not_joined.values(), key=lambda river: river["key"])
    not_joined_keys = {river["key"] for river in not_joined_rivers}

    # Groups: water connected 4-way across polygon boundaries. The runtime holds one flat surface
    # per connected water body, so a group takes the lowest level among its polygons (the polygons
    # of one lake system differ by a few decimetres). Fragments below the minimum return to land
    # with the elevation model's heights.
    water = body_of >= 0
    groups, group_count = ndimage.label(water)
    sizes = np.bincount(groups.ravel(), minlength=group_count + 1)
    fragment = water & (sizes[groups] < rule.min_samples)
    fragment_samples = int(np.count_nonzero(fragment))
    body_of[fragment] = LAND
    water = body_of >= 0
    groups, group_count = ndimage.label(water)
    group_level_mm = np.full(group_count + 1, np.iinfo(np.int64).max, dtype=np.int64)
    np.minimum.at(group_level_mm, groups[water], level_of_body[body_of[water]])
    level_mm = np.where(water, group_level_mm[groups], 0)

    # Distance to the shore: the boundary of all kept polygons together, so an edge two polygons
    # share is not a shore, and a polygon cut by the world edge keeps its real shore beyond it.
    distance = np.full(dem.shape, np.inf)
    max_distance = rule.depth_cap_m / rule.slope
    kept_indices = [
        k for k, body in enumerate(bodies)
        if body.key not in dropped and body.key not in not_joined_keys
    ]
    shore_segments = []
    if kept_indices:
        reach = world.buffer(max_distance + 2.0 * grid.cell_m, join_style="mitre")
        union = shapely.intersection(shapely.union_all([bodies[k].geometry for k in kept_indices]), reach)
        shore_segments = _shore_segments(shapely.boundary(union))
        rows, cols = np.nonzero(water)
        distance[rows, cols] = _shore_distance(grid, shore_segments, rows, cols, max_distance)

    shoal_mask = np.zeros(dem.shape, dtype=bool)
    for shoal in shoals:
        if shoal.intersects(world):
            rows, cols = _samples_inside(grid, shoal)
            shoal_mask[rows, cols] = True
    cap = np.where(shoal_mask, rule.shoal_depth_cap_m, rule.depth_cap_m)

    heights = np.array(dem, dtype=np.float32, copy=True)
    level_m = level_mm / 1000.0
    depth_m = synthetic_depth(np.where(water, distance, 0.0), cap, rule)
    chart_report = None
    shore_weight = None
    if depths is not None:
        charted, unowned_contours, unowned_soundings = _charted_polygons(bodies, kept_indices, depths)
        point_depth, shore_weight = _charted_depths(
            grid, bodies, body_of, groups, charted, kept_indices, shore_segments, depths, rule
        )
        # Where no contour is drawn the chart says the water is shallower than its shallowest
        # contour level: a shore point stands for the shore ramp, the synthetic bed capped at that
        # level, which is 0 at the shore itself and leaves every contour and sounding exact.
        shallowest_m = min(depth_mm for _, depth_mm, _ in depths.contours) / 1000.0
        ramp = np.minimum(depth_m, shallowest_m)
        on_chart = water & ~np.isnan(point_depth)
        charted_depth = point_depth + shore_weight * ramp
        depth_m = np.where(on_chart, charted_depth, depth_m)
        chart_report = {
            "charted_keys": sorted(bodies[k].key for k in charted),
            "charted_samples": int(np.count_nonzero(on_chart)),
            "shallowest_contour_m": shallowest_m,
            "shore_only_samples": int(np.count_nonzero(on_chart & (shore_weight > 1.0 - 1e-9))),
            "contours": len(depths.contours),
            "soundings": len(depths.soundings),
            "contours_owned_by_no_kept_polygon": unowned_contours,
            "soundings_in_no_kept_polygon": unowned_soundings,
            "deepest_charted_m": float(np.max(charted_depth[on_chart])) if on_chart.any() else 0.0,
        }
    bed = bed_below(level_m, depth_m)
    heights[water] = bed[water]

    # Shores: land beside water stands above the highest level it touches, so no fill floods
    # past its polygons' samples (islands and polygon holes are land like any other).
    shore_level = np.full(dem.shape, -np.inf)
    water_level_m = np.where(water, level_m, -np.inf)
    for view in _neighbours(water_level_m, -np.inf):
        shore_level = np.maximum(shore_level, view)
    shore = (body_of == LAND) & np.isfinite(shore_level)
    raised_to = (shore_level + rule.shore_margin_m).astype(np.float32)
    lifted = shore & (heights < raised_to)
    lift_amount = np.where(lifted, raised_to.astype(np.float64) - heights, 0.0)
    heights[lifted] = raised_to[lifted]

    fills = _group_fills(grid, bodies, body_of, groups, group_count, distance, level_mm)
    body_group_level = {}
    for k in range(len(bodies)):
        levels = np.unique(level_mm[body_of == k])
        body_group_level[k] = [int(v) / 1000.0 for v in levels]

    report = {
        "rule": rule.__dict__,
        "bodies_in_world": bodies_in_world,
        "rivers_dropped_narrow": narrow_rivers,
        "rivers_not_joined": not_joined_rivers,
        "bodies_dropped_under_min_samples": len(dropped),
        "dropped_keys": dropped,
        "fragment_samples_returned_to_land": fragment_samples,
        "shore_samples_raised": int(np.count_nonzero(lifted)),
        "shore_raise_max_m": float(lift_amount.max()) if lifted.any() else 0.0,
        "shore_raise_mean_m": float(lift_amount[lifted].mean()) if lifted.any() else 0.0,
        "water_samples": int(np.count_nonzero(water)),
        "shoal_water_samples": int(np.count_nonzero(water & shoal_mask)),
        "groups": group_count,
        "lake_fills": sum(1 for f in fills if f.kind == "lake"),
        "open_water_fills": sum(1 for f in fills if f.kind == "open_water"),
        "bodies": [
            {
                "key": b.key,
                "kind": b.kind,
                "level_m": b.level_m,
                "water_level_m": body_group_level[k],
                "samples_assigned": body_samples[k]["samples"],
                "samples_water": int(np.count_nonzero(body_of == k)),
                "overlap_samples": body_samples[k]["overlap_samples"],
                "dropped": b.key in dropped,
                "inscribed_width_m": river_width.get(b.key),
                "joined_lake_level_m": joined_rivers[k] / 1000.0 if k in joined_rivers else None,
                "clipped_area_m2": float(shapely.intersection(b.geometry, world).area),
                "clipped_perimeter_m": float(shapely.intersection(b.geometry, world).length),
                "cut_by_world_edge": bool(not b.geometry.within(world)),
            }
            for k, b in enumerate(bodies)
        ],
    }
    if chart_report is not None:
        report["depth_chart"] = chart_report
    group_of = np.where(water, groups - 1, -1).astype(np.int32)
    return WaterResult(heights, body_of, group_of, bodies, fills, report, shore_weight)


def _group_fills(grid, bodies, body_of, groups, group_count, distance, level_mm) -> list[WaterFill]:
    if group_count == 0:
        return []
    flat_groups = groups.ravel()
    order = np.argsort(flat_groups, kind="stable")
    starts = np.searchsorted(flat_groups[order], np.arange(1, group_count + 2))
    flat_body = body_of.ravel()
    flat_distance = distance.ravel()
    flat_level = level_mm.ravel()
    fills = []
    for group in range(1, group_count + 1):
        members = order[starts[group - 1] : starts[group]]  # ascending sample index
        rows, cols = np.divmod(members, grid.width)
        levels = np.unique(flat_level[members])
        if levels.size != 1:
            raise RuntimeError(f"water group {group} spans levels {levels.tolist()}")
        member_bodies = np.unique(flat_body[members])
        counts = np.array([np.count_nonzero(flat_body[members] == b) for b in member_bodies])
        # The seed: the largest member polygon's interior point when it lands in the group,
        # else the group's deepest sample (first in index order on a tie).
        largest = int(member_bodies[np.lexsort((member_bodies, -counts))[0]])
        seed = None
        point = bodies[largest].interior_en
        if point is not None:
            i, j = grid.nearest_sample(*point)
            if groups[j, i] == group:
                seed = (i, j)
        if seed is None:
            best = int(np.argmax(flat_distance[members]))
            seed = (int(cols[best]), int(rows[best]))
        edge = bool(
            np.any(rows == 0) or np.any(cols == 0)
            or np.any(rows == grid.height - 1) or np.any(cols == grid.width - 1)
        )
        world_x, world_z = grid.world_xz(*seed)
        fills.append(
            WaterFill(
                kind="open_water" if edge else "lake",
                world_x=world_x,
                world_z=world_z,
                surface_elevation_m=float(levels[0]) / 1000.0,
                group=group - 1,
                bodies=sorted(bodies[int(b)].key for b in member_bodies),
            )
        )
    return fills
