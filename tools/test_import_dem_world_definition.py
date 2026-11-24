"""DEM importer regressions: the world frame and the synthetic lake rule; no GDAL or Godot."""

from collections import deque
import sys
import unittest

# The importer's third-party modules are optional on a gate machine: without them the whole
# module skips, and says so, instead of failing the gate before any Rust or Godot stage.
try:
    import numpy as np
    import scipy  # noqa: F401 (dem_water imports scipy.ndimage)
    import shapely
except ImportError as missing:
    print(
        f"test_import_dem_world_definition: SKIPPED, Python module {missing.name or missing} "
        "is missing (the DEM importer tests need numpy, scipy and shapely)",
        file=sys.stderr,
    )
    raise unittest.SkipTest(f"missing Python module {missing.name or missing}") from None

from dem_water import SampleGrid, WaterBody, WaterRule, apply_hydrography, synthetic_bed
from import_dem_world_definition import world_frame


def flood(heights, seed, level):
    # The runtime's fill (`FillFlood::flood`): 4-connected over samples at or below the
    # level, from the seed sample.
    i, j = seed
    seen = np.zeros(heights.shape, dtype=bool)
    if heights[j, i] > level:
        return seen
    seen[j, i] = True
    queue = deque([(i, j)])
    while queue:
        i, j = queue.popleft()
        for ni, nj in ((i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)):
            if 0 <= ni < heights.shape[1] and 0 <= nj < heights.shape[0]:
                if not seen[nj, ni] and heights[nj, ni] <= level:
                    seen[nj, ni] = True
                    queue.append((ni, nj))
    return seen


class WorldFrameTest(unittest.TestCase):
    def test_square_world_around_centre(self):
        self.assertEqual(
            world_frame(18432.0, 533000.0, 6975000.0, 8.0),
            (523784.0, 6965784.0, 542216.0, 6984216.0),
        )

    def test_side_must_be_whole_cells(self):
        with self.assertRaises(ValueError):
            world_frame(18004.0, 533000.0, 6975000.0, 8.0)


class LakeRuleTest(unittest.TestCase):
    def test_bed_follows_shore_distance_and_caps(self):
        rule = WaterRule(slope=0.15, depth_cap_m=12.0, shoal_depth_cap_m=1.0)
        bed = synthetic_bed(82.1, np.array([2.0, 40.0, 200.0, 200.0]),
                            np.array([12.0, 12.0, 12.0, 1.0]), rule)
        np.testing.assert_allclose(bed, np.float32([81.8, 76.1, 70.1, 81.1]), atol=1e-5)

    def test_lake_floods_its_polygon_keeps_island_dry_and_edge_water_is_open(self):
        # 41 x 41 samples at 8 m; source E 1000..1320, N 2000..2320. The ground is 0.5 m below
        # the lake level everywhere, so without raised shores every fill would flood the map.
        grid = SampleGrid(west_e=1000.0, north_n=2320.0, cell_m=8.0, width=41, height=41)
        level_mm = 50_000
        dem = np.full((41, 41), 49.5, dtype=np.float32)
        lake = shapely.Polygon(
            [(1060, 2060), (1200, 2060), (1200, 2200), (1060, 2200)],
            holes=[[(1110, 2110), (1150, 2110), (1150, 2150), (1110, 2150)]],
        )
        # A bay cut by the west world edge, at another level.
        bay = shapely.box(950.0, 2250.0, 1030.0, 2300.0)
        bodies = [
            WaterBody(key=1, kind="lake", level_mm=level_mm, geometry=lake, interior_en=(1080.0, 2080.0)),
            WaterBody(key=2, kind="lake", level_mm=51_000, geometry=bay),
            WaterBody(key=3, kind="lake", level_mm=50_000, geometry=shapely.box(1300.0, 2000.0, 1305.0, 2005.0)),
        ]
        rule = WaterRule()
        result = apply_hydrography(dem, grid, bodies, [], rule)

        self.assertEqual(result.report["dropped_keys"], [3])
        kinds = sorted((fill.kind, fill.surface_elevation_m) for fill in result.fills)
        self.assertEqual(kinds, [("lake", 50.0), ("open_water", 51.0)])
        for fill in result.fills:
            i = int(round((fill.world_x + 160.0) / 8.0))
            j = int(round((fill.world_z + 160.0) / 8.0))
            flooded = flood(result.heights, (i, j), np.float32(fill.surface_elevation_m))
            water = result.group_of_sample == fill.group
            self.assertTrue(np.array_equal(flooded, water), fill.kind)
        lake_water = result.group_of_sample == next(f.group for f in result.fills if f.kind == "lake")
        # Sample centres strictly inside the lake and outside the island.
        self.assertEqual(int(lake_water.sum()), 17 * 17 - 5 * 5)
        self.assertFalse(lake_water[(2320 - 2130) // 8, (1130 - 1000) // 8], "island stays dry")
        # Nearest sample to the interior point seeds the lake, and the bed deepens off the shore.
        lake_fill = next(f for f in result.fills if f.kind == "lake")
        self.assertEqual((lake_fill.world_x, lake_fill.world_z), (-80.0, 80.0))
        row = result.heights[(2320 - 2096) // 8]
        self.assertLess(row[(1096 - 1000) // 8], row[(1064 - 1000) // 8])

    def test_river_narrower_than_two_cells_is_not_imported(self):
        # 41 x 41 samples at 8 m. Three 280 m strips across the map, 15.9, 16.1 and 6 m wide:
        # the 15.9 m river is not imported, the 16.1 m river is, and the 6 m lake is (the rule
        # is for rivers only; it keeps one row of samples).
        grid = SampleGrid(west_e=1000.0, north_n=2320.0, cell_m=8.0, width=41, height=41)
        dem = np.full((41, 41), 60.0, dtype=np.float32)
        narrow = shapely.box(1020.0, 2040.0, 1300.0, 2055.9)
        wide = shapely.box(1020.0, 2120.0, 1300.0, 2136.1)
        lake = shapely.box(1020.0, 2203.0, 1300.0, 2209.0)
        bodies = [
            WaterBody(key=1, kind="river", level_mm=50_000, geometry=narrow),
            WaterBody(key=2, kind="river", level_mm=50_000, geometry=wide),
            WaterBody(key=3, kind="lake", level_mm=50_000, geometry=lake),
        ]
        result = apply_hydrography(dem, grid, bodies, [], WaterRule())

        dropped = result.report["rivers_dropped_narrow"]
        self.assertEqual([river["key"] for river in dropped], [1])
        self.assertAlmostEqual(dropped[0]["inscribed_width_m"], 15.9, delta=0.002)
        self.assertEqual(sorted(b.key for b in result.bodies), [2, 3])
        narrow_rows = slice((2320 - 2055) // 8, (2320 - 2040) // 8 + 1)
        self.assertTrue(np.all(result.heights[narrow_rows] == 60.0), "dry, the elevation model")
        self.assertEqual(len(result.fills), 2)

        # A width within the tolerance of the limit is not decided either way.
        edge = shapely.box(1020.0, 2040.0, 1300.0, 2055.9995)
        with self.assertRaises(ValueError):
            apply_hydrography(
                dem, grid, [WaterBody(key=4, kind="river", level_mm=50_000, geometry=edge)], [],
                WaterRule(),
            )


class RiverJoinTest(unittest.TestCase):
    # 41 x 41 samples at 8 m, source E 1000..1320, N 2000..2320, ground at 60 m. Two lakes,
    # west and east, and a 40 m wide river between them whose samples are 4-neighbours of both
    # lakes' samples (x 1096 lake, 1104 river; 1216 river, 1224 lake).
    GRID = SampleGrid(west_e=1000.0, north_n=2320.0, cell_m=8.0, width=41, height=41)
    WEST = shapely.box(1020.0, 2100.0, 1100.0, 2260.0)
    EAST = shapely.box(1220.0, 2100.0, 1300.0, 2260.0)
    RIVER = shapely.box(1100.0, 2160.0, 1220.0, 2200.0)
    RIVER_ROWS = slice((2320 - 2192) // 8, (2320 - 2168) // 8 + 1)
    RIVER_COLS = slice((1104 - 1000) // 8, (1216 - 1000) // 8 + 1)

    def dem(self):
        dem = np.full((41, 41), 60.0, dtype=np.float32)
        # The elevation model's lowest sample in the river, 0.3 m under the lakes: the level a
        # river without vertex heights would take by itself.
        dem[self.RIVER_ROWS, self.RIVER_COLS][1, 3] = 49.7
        return dem

    def run_rule(self, bodies):
        result = apply_hydrography(self.dem(), self.GRID, bodies, [], WaterRule())
        for fill in result.fills:
            i = int(round((fill.world_x + 160.0) / 8.0))
            j = int(round((fill.world_z + 160.0) / 8.0))
            flooded = flood(result.heights, (i, j), np.float32(fill.surface_elevation_m))
            self.assertTrue(np.array_equal(flooded, result.group_of_sample == fill.group))
        return result

    def lakes(self, west_mm, east_mm):
        return [
            WaterBody(key=1, kind="lake", level_mm=west_mm, geometry=self.WEST),
            WaterBody(key=2, kind="lake", level_mm=east_mm, geometry=self.EAST),
        ]

    def test_strait_between_equal_lakes_takes_their_level(self):
        # A river without vertex heights alone would stand at 49.7 m and pull both lakes down.
        river = WaterBody(key=3, kind="river", level_mm=None, geometry=self.RIVER)
        result = self.run_rule(self.lakes(50_000, 50_000) + [river])

        self.assertEqual([(f.kind, f.surface_elevation_m, f.bodies) for f in result.fills],
                         [("lake", 50.0, [1, 2, 3])])
        self.assertEqual(result.report["rivers_dropped_level"], [])
        entry = next(b for b in result.report["bodies"] if b["key"] == 3)
        self.assertEqual((entry["level_m"], entry["joined_lake_level_m"]), (49.7, 50.0))
        self.assertTrue(np.all(result.group_of_sample[self.RIVER_ROWS, self.RIVER_COLS] == 0))

    def test_river_between_lakes_at_different_levels_is_not_imported(self):
        river = WaterBody(key=3, kind="river", level_mm=49_700, geometry=self.RIVER)
        result = self.run_rule(self.lakes(50_000, 53_000) + [river])

        self.assertEqual(sorted((f.surface_elevation_m, f.bodies) for f in result.fills),
                         [(50.0, [1]), (53.0, [2])])
        self.assertEqual(result.report["rivers_dropped_level"], [{
            "key": 3,
            "reason": "joins lake water at levels further apart than the tolerance",
            "joined_levels_m": [50.0, 53.0],
            "lakes": [1, 2],
            "system": [3],
        }])
        # No bed, no fill: the river keeps the elevation model.
        np.testing.assert_array_equal(result.heights[self.RIVER_ROWS, self.RIVER_COLS],
                                      self.dem()[self.RIVER_ROWS, self.RIVER_COLS])

    def test_river_mouth_touching_a_lake_polygon_without_neighbour_samples_counts(self):
        # The river's polygon meets the east lake's at E 1224, a sample on both boundaries and
        # inside neither: no river sample is a 4-neighbour of a lake sample, the polygons touch.
        river = WaterBody(key=3, kind="river", level_mm=49_700,
                          geometry=shapely.box(1100.0, 2160.0, 1224.0, 2200.0))
        east = WaterBody(key=2, kind="lake", level_mm=53_000,
                         geometry=shapely.box(1224.0, 2100.0, 1300.0, 2260.0))
        result = self.run_rule(self.lakes(50_000, 53_000)[:1] + [east, river])

        self.assertEqual([river["lakes"] for river in result.report["rivers_dropped_level"]], [[1, 2]])
        self.assertEqual(sorted(f.surface_elevation_m for f in result.fills), [50.0, 53.0])

    def test_chain_of_rivers_is_decided_as_one_system_in_any_order(self):
        # River 3 touches the west lake and river 4, river 4 the east lake: neither touches two
        # lakes, but together they join both, so neither is imported. Within the tolerance
        # (0.4 m) both are, at the lower lake's level.
        west_part = WaterBody(key=3, kind="river", level_mm=None,
                              geometry=shapely.box(1100.0, 2160.0, 1164.0, 2200.0))
        east_part = WaterBody(key=4, kind="river", level_mm=None,
                              geometry=shapely.box(1164.0, 2160.0, 1220.0, 2200.0))
        for east_mm, kept in ((53_000, False), (50_400, True)):
            orders = [
                self.lakes(50_000, east_mm) + [west_part, east_part],
                [east_part, west_part] + self.lakes(50_000, east_mm)[::-1],
            ]
            first, second = (self.run_rule(bodies) for bodies in orders)
            np.testing.assert_array_equal(first.heights, second.heights)
            self.assertEqual([f.__dict__ for f in first.fills], [f.__dict__ for f in second.fills])
            self.assertEqual(first.report["rivers_dropped_level"], second.report["rivers_dropped_level"])
            dropped = [river["key"] for river in first.report["rivers_dropped_level"]]
            if kept:
                self.assertEqual(dropped, [])
                self.assertEqual([(f.surface_elevation_m, f.bodies) for f in first.fills],
                                 [(50.0, [1, 2, 3, 4])])
            else:
                self.assertEqual(dropped, [3, 4])
                self.assertEqual(first.report["rivers_dropped_level"][0]["system"], [3, 4])
                self.assertEqual(sorted(f.surface_elevation_m for f in first.fills), [50.0, 53.0])

    def test_river_without_vertex_heights_touching_no_lake_takes_its_lowest_sample(self):
        river = WaterBody(key=3, kind="river", level_mm=None, geometry=self.RIVER)
        result = self.run_rule([river])

        self.assertEqual([(f.kind, f.surface_elevation_m) for f in result.fills], [("lake", 49.7)])
        entry = result.report["bodies"][0]
        self.assertEqual((entry["level_m"], entry["joined_lake_level_m"]), (49.7, None))
        self.assertEqual(int(np.count_nonzero(result.group_of_sample == 0)), 4 * 15)

    def test_lake_without_interior_point_seeds_at_its_deepest_sample(self):
        # Samples E 1024..1096, N 2024..2096; the four central ones lie 36 m from the shore, and
        # the first in index order (northmost row, then westmost) seeds the fill.
        lake = WaterBody(key=1, kind="lake", level_mm=50_000,
                         geometry=shapely.box(1020.0, 2020.0, 1100.0, 2100.0))
        result = self.run_rule([lake])

        fill = result.fills[0]
        self.assertEqual((fill.world_x, fill.world_z), (1056.0 - 1160.0, 2160.0 - 2064.0))


if __name__ == "__main__":
    unittest.main()
