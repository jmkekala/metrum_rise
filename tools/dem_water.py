"""
Lakes and rivers for a DEM-imported world, from national hydrography polygons.

Editor-time only, used by `import_dem_world_definition.py`. A laser-scanned elevation model is
flat at the water surface over lakes, so the lake bed is synthetic:

    bed = level - min(depth_cap, slope * d)

with `d` the distance from the sample to the shore of the water at the sample's level, and
`depth_cap` lowered to the shoal cap inside a shoal polygon. Samples outside every water polygon
keep the elevation model. The runtime floods an authored fill 4-connected over samples at or
below its level (`FillFlood::flood`), so the shore rules below make each fill flood
exactly its own samples:

- A land sample beside water is raised to at least the water's level plus a margin.
- Water connected across polygon boundaries is one group with one fill at the lowest level
  among its polygons (the runtime holds one flat surface per connected body of water); a group
  touching the world edge is an open-water fill, any other a lake fill.

Everything here is deterministic: inputs are sorted, ties are broken by sample index, and no
step depends on hash or thread order.
"""

from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np
import shapely
from scipy import ndimage

LAND = -1


@dataclass(frozen=True)
class WaterRule:
    """The synthetic bed and shore parameters."""

    slope: float = 0.15
    depth_cap_m: float = 12.0
    shoal_depth_cap_m: float = 1.0
    shore_margin_m: float = 0.05
    min_samples: int = 4


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
    level_mm: int
    geometry: object
    interior_en: tuple[float, float] | None = None

    @property
    def level_m(self) -> float:
        return self.level_mm / 1000.0


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


def synthetic_bed(level_m, distance_m, cap_m, rule: WaterRule) -> np.ndarray:
    """`level - min(cap, slope * d)` in f32; `cap_m` is per sample (the shoal cap inside shoals).
    A bed that rounds to its level (a sample within micrometres of the shore) is moved one f32
    step below it, so the runtime counts it wet rather than flooded at depth 0."""
    depth = np.minimum(np.asarray(cap_m, dtype=np.float64),
                       rule.slope * np.asarray(distance_m, dtype=np.float64))
    level = np.asarray(level_m, dtype=np.float64)
    bed = (level - depth).astype(np.float32)
    surface = level.astype(np.float32)
    return np.where(bed < surface, bed, np.nextafter(surface, np.float32(-np.inf)))


def sort_bodies(bodies: list[WaterBody]) -> list[WaterBody]:
    # The lower level owns a sample two polygons share; kind and key break ties.
    return sorted(bodies, key=lambda b: (b.level_mm, b.kind != "lake", b.key))


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


def _shore_distance(grid: SampleGrid, shore, rows, cols, max_distance: float) -> np.ndarray:
    """Distance from each sample to the nearest segment of `shore`; `inf` beyond `max_distance`."""
    coords = []
    for line in shapely.get_parts(shore):
        xy = shapely.get_coordinates(line)
        if len(xy) >= 2:
            coords.append(np.stack([xy[:-1], xy[1:]], axis=1))
    out = np.full(rows.shape, np.inf)
    if not coords or rows.size == 0:
        return out
    segments = shapely.linestrings(np.concatenate(coords))
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


def apply_hydrography(
    dem: np.ndarray,
    grid: SampleGrid,
    bodies: list[WaterBody],
    shoals: list,
    rule: WaterRule,
) -> WaterResult:
    """Carves the synthetic beds, raises the shores and returns one fill per water group."""
    if dem.shape != (grid.height, grid.width):
        raise ValueError(f"DEM shape {dem.shape} does not match the sample grid")
    world = grid.box()
    bodies = sort_bodies([b for b in bodies if b.geometry.intersects(world)])
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
    level_of_body = np.array([b.level_mm for b in bodies] + [0], dtype=np.int64)
    group_level_mm = np.full(group_count + 1, np.iinfo(np.int64).max, dtype=np.int64)
    np.minimum.at(group_level_mm, groups[water], level_of_body[body_of[water]])
    level_mm = np.where(water, group_level_mm[groups], 0)

    # Distance to the shore: the boundary of all kept polygons together, so an edge two polygons
    # share is not a shore, and a polygon cut by the world edge keeps its real shore beyond it.
    distance = np.full(dem.shape, np.inf)
    max_distance = rule.depth_cap_m / rule.slope
    kept = [body.geometry for body in bodies if body.key not in dropped]
    if kept:
        reach = world.buffer(max_distance + 2.0 * grid.cell_m, join_style="mitre")
        union = shapely.intersection(shapely.union_all(kept), reach)
        rows, cols = np.nonzero(water)
        distance[rows, cols] = _shore_distance(grid, shapely.boundary(union), rows, cols, max_distance)

    shoal_mask = np.zeros(dem.shape, dtype=bool)
    for shoal in shoals:
        if shoal.intersects(world):
            rows, cols = _samples_inside(grid, shoal)
            shoal_mask[rows, cols] = True
    cap = np.where(shoal_mask, rule.shoal_depth_cap_m, rule.depth_cap_m)

    heights = np.array(dem, dtype=np.float32, copy=True)
    level_m = level_mm / 1000.0
    bed = synthetic_bed(level_m, np.where(water, distance, 0.0), cap, rule)
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
        "bodies_in_world": len(bodies),
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
                "clipped_area_m2": float(shapely.intersection(b.geometry, world).area),
                "clipped_perimeter_m": float(shapely.intersection(b.geometry, world).length),
                "cut_by_world_edge": bool(not b.geometry.within(world)),
            }
            for k, b in enumerate(bodies)
        ],
    }
    group_of = np.where(water, groups - 1, -1).astype(np.int32)
    return WaterResult(heights, body_of, group_of, bodies, fills, report)


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
