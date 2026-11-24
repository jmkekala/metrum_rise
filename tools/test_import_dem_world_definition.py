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

from dem_water import DepthChart, SampleGrid, WaterBody, WaterRule, apply_hydrography, synthetic_bed
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
        # the 15.9 m river is dropped for its width, the 16.1 m river passes it, and the 6 m lake
        # is imported (the rule is for rivers only; it keeps one row of samples).
        grid = SampleGrid(west_e=1000.0, north_n=2320.0, cell_m=8.0, width=41, height=41)
        dem = np.full((41, 41), 60.0, dtype=np.float32)
        narrow = shapely.box(1020.0, 2040.0, 1300.0, 2055.9)
        wide = shapely.box(1020.0, 2120.0, 1300.0, 2136.1)
        lake = shapely.box(1020.0, 2203.0, 1300.0, 2209.0)
        bodies = [
            WaterBody(key=1, kind="river", level_mm=None, geometry=narrow),
            WaterBody(key=2, kind="river", level_mm=None, geometry=wide),
            WaterBody(key=3, kind="lake", level_mm=50_000, geometry=lake),
        ]
        result = apply_hydrography(dem, grid, bodies, [], WaterRule())

        dropped = result.report["rivers_dropped_narrow"]
        self.assertEqual([river["key"] for river in dropped], [1])
        self.assertAlmostEqual(dropped[0]["inscribed_width_m"], 15.9, delta=0.002)
        self.assertEqual(sorted(b.key for b in result.bodies), [2, 3])
        narrow_rows = slice((2320 - 2055) // 8, (2320 - 2040) // 8 + 1)
        self.assertTrue(np.all(result.heights[narrow_rows] == 60.0), "dry, the elevation model")
        # The wide river joins no lake, so the strait rule leaves it out in turn.
        self.assertEqual([r["key"] for r in result.report["rivers_not_joined"]], [2])
        self.assertEqual(len(result.fills), 1)

        # A width within the tolerance of the limit is not decided either way.
        edge = shapely.box(1020.0, 2040.0, 1300.0, 2055.9995)
        with self.assertRaises(ValueError):
            apply_hydrography(
                dem, grid, [WaterBody(key=4, kind="river", level_mm=None, geometry=edge)], [],
                WaterRule(),
            )


class RiverJoinTest(unittest.TestCase):
    # 41 x 41 samples at 8 m, source E 1000..1320, N 2000..2320. Two lakes, west and east, and
    # a 40 m wide river between them whose samples are 4-neighbours of both lakes' samples
    # (x 1096 lake, 1104 river; 1216 river, 1224 lake). The ground rises east 0.1 m a sample
    # from 60 m, so a flat river surface would lie under it.
    GRID = SampleGrid(west_e=1000.0, north_n=2320.0, cell_m=8.0, width=41, height=41)
    WEST = shapely.box(1020.0, 2100.0, 1100.0, 2260.0)
    EAST = shapely.box(1220.0, 2100.0, 1300.0, 2260.0)
    RIVER = shapely.box(1100.0, 2160.0, 1220.0, 2200.0)
    RIVER_ROWS = slice((2320 - 2192) // 8, (2320 - 2168) // 8 + 1)
    RIVER_COLS = slice((1104 - 1000) // 8, (1216 - 1000) // 8 + 1)

    def dem(self):
        return np.tile(np.float32(60.0) + np.arange(41, dtype=np.float32) * np.float32(0.1), (41, 1))

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

    def river(self, key=3, geometry=None):
        return WaterBody(key=key, kind="river", level_mm=None, geometry=geometry or self.RIVER)

    def test_strait_between_equal_lakes_takes_their_level(self):
        result = self.run_rule(self.lakes(50_000, 50_000) + [self.river()])

        self.assertEqual([(f.kind, f.surface_elevation_m, f.bodies) for f in result.fills],
                         [("lake", 50.0, [1, 2, 3])])
        self.assertEqual(result.report["rivers_not_joined"], [])
        entry = next(b for b in result.report["bodies"] if b["key"] == 3)
        self.assertEqual((entry["level_m"], entry["joined_lake_level_m"]), (None, 50.0))
        self.assertTrue(np.all(result.group_of_sample[self.RIVER_ROWS, self.RIVER_COLS] == 0))

    def test_river_between_lakes_at_different_levels_is_not_imported(self):
        result = self.run_rule(self.lakes(50_000, 53_000) + [self.river()])

        self.assertEqual(sorted((f.surface_elevation_m, f.bodies) for f in result.fills),
                         [(50.0, [1]), (53.0, [2])])
        self.assertEqual(result.report["rivers_not_joined"], [{
            "key": 3,
            "reason": "joins lake water at levels further apart than the tolerance",
            "joined_levels_m": [50.0, 53.0],
            "lakes": [1, 2],
            "system": [3],
        }])
        # No bed, no fill: the river keeps the elevation model.
        np.testing.assert_array_equal(result.heights[self.RIVER_ROWS, self.RIVER_COLS],
                                      self.dem()[self.RIVER_ROWS, self.RIVER_COLS])

    def test_inlet_joining_one_lake_is_not_imported_and_the_lake_is_unchanged(self):
        # The river runs from the west lake up the slope to the world's east edge.
        inlet = self.river(geometry=shapely.box(1100.0, 2160.0, 1330.0, 2200.0))
        lake_only = self.run_rule(self.lakes(50_000, 50_000)[:1])
        result = self.run_rule(self.lakes(50_000, 50_000)[:1] + [inlet])

        self.assertEqual([(r["key"], r["reason"], r["lakes"]) for r in result.report["rivers_not_joined"]],
                         [(3, "joins fewer than two lakes", [1])])
        np.testing.assert_array_equal(result.heights, lake_only.heights)
        self.assertEqual([f.__dict__ for f in result.fills], [f.__dict__ for f in lake_only.fills])

    def test_river_joining_no_lake_is_not_imported(self):
        result = self.run_rule([self.river()])

        self.assertEqual(result.fills, [])
        self.assertEqual([(r["key"], r["lakes"]) for r in result.report["rivers_not_joined"]], [(3, [])])
        np.testing.assert_array_equal(result.heights, self.dem())

    def test_river_mouth_touching_a_lake_polygon_without_neighbour_samples_counts(self):
        # The river's polygon meets the east lake's at E 1224, a sample on both boundaries and
        # inside neither: no river sample is a 4-neighbour of a lake sample, the polygons touch.
        river = self.river(geometry=shapely.box(1100.0, 2160.0, 1224.0, 2200.0))
        east = WaterBody(key=2, kind="lake", level_mm=50_000,
                         geometry=shapely.box(1224.0, 2100.0, 1300.0, 2260.0))
        result = self.run_rule(self.lakes(50_000, 50_000)[:1] + [east, river])

        self.assertEqual(result.report["rivers_not_joined"], [])
        self.assertEqual(sorted(f.bodies for f in result.fills), [[1, 3], [2]])
        self.assertEqual({f.surface_elevation_m for f in result.fills}, {50.0})

    def test_chain_of_rivers_is_decided_as_one_system_in_any_order(self):
        # River 3 touches the west lake and river 4, river 4 the east lake: each joins one lake,
        # together they are a strait. Within the tolerance (0.4 m) both are imported at the lower
        # lake's level; 3 m apart, neither.
        west_part = self.river(3, shapely.box(1100.0, 2160.0, 1164.0, 2200.0))
        east_part = self.river(4, shapely.box(1164.0, 2160.0, 1220.0, 2200.0))
        for east_mm, kept in ((53_000, False), (50_400, True)):
            orders = [
                self.lakes(50_000, east_mm) + [west_part, east_part],
                [east_part, west_part] + self.lakes(50_000, east_mm)[::-1],
            ]
            first, second = (self.run_rule(bodies) for bodies in orders)
            np.testing.assert_array_equal(first.heights, second.heights)
            self.assertEqual([f.__dict__ for f in first.fills], [f.__dict__ for f in second.fills])
            self.assertEqual(first.report["rivers_not_joined"], second.report["rivers_not_joined"])
            dropped = [river["key"] for river in first.report["rivers_not_joined"]]
            if kept:
                self.assertEqual(dropped, [])
                self.assertEqual([(f.surface_elevation_m, f.bodies) for f in first.fills],
                                 [(50.0, [1, 2, 3, 4])])
            else:
                self.assertEqual(dropped, [3, 4])
                self.assertEqual(first.report["rivers_not_joined"][0]["system"], [3, 4])
                self.assertEqual(sorted(f.surface_elevation_m for f in first.fills), [50.0, 53.0])

    def test_lake_without_interior_point_seeds_at_its_deepest_sample(self):
        # Samples E 1024..1096, N 2024..2096; the four central ones lie 36 m from the shore, and
        # the first in index order (northmost row, then westmost) seeds the fill.
        lake = WaterBody(key=1, kind="lake", level_mm=50_000,
                         geometry=shapely.box(1020.0, 2020.0, 1100.0, 2100.0))
        result = self.run_rule([lake])

        fill = result.fills[0]
        self.assertEqual((fill.world_x, fill.world_z), (1056.0 - 1160.0, 2160.0 - 2064.0))


class DepthChartTest(unittest.TestCase):
    # 41 x 41 samples at 8 m, source E 1000..1320, N 2000..2320; sample (i, j) at E 1000 + 8 i,
    # N 2320 - 8 j. Lakes stand at 50 m on ground 49.5 m (raised shores keep each fill in its
    # polygon). Depth is the water level minus the bed.
    GRID = SampleGrid(west_e=1000.0, north_n=2320.0, cell_m=8.0, width=41, height=41)

    def run_rule(self, bodies, depths):
        dem = np.full((41, 41), 49.5, dtype=np.float32)
        result = apply_hydrography(dem, self.GRID, bodies, [], WaterRule(), depths)
        level = np.full((41, 41), np.nan)
        for fill in result.fills:
            i = int(round((fill.world_x + 160.0) / 8.0))
            j = int(round((fill.world_z + 160.0) / 8.0))
            flooded = flood(result.heights, (i, j), np.float32(fill.surface_elevation_m))
            self.assertTrue(np.array_equal(flooded, result.group_of_sample == fill.group))
            level[result.group_of_sample == fill.group] = fill.surface_elevation_m
        water = result.group_of_sample >= 0
        self.assertTrue(np.all(result.heights[water] < np.float32(50.0)), "every water sample is wet")
        return result, np.where(water, level - result.heights, np.nan)

    @staticmethod
    def sample(e, n):
        return (2320 - n) // 8, (e - 1000) // 8

    def test_charted_lake_takes_the_depths_at_contours_and_soundings(self):
        lake = WaterBody(key=1, kind="lake", level_mm=50_000,
                         geometry=shapely.box(1020.0, 2020.0, 1300.0, 2300.0))
        ring3 = shapely.LinearRing([(1080, 2080), (1240, 2080), (1240, 2240), (1080, 2240)])
        ring6 = shapely.LinearRing([(1120, 2120), (1200, 2120), (1200, 2200), (1120, 2200)])
        depths = DepthChart(
            contours=((10, 3_000, shapely.LineString(ring3.coords)), (11, 6_000, shapely.LineString(ring6.coords))),
            soundings=((20, 9_500, shapely.Point(1160.0, 2160.0)),),
        )
        result, depth = self.run_rule([lake], depths)

        self.assertEqual(result.report["depth_chart"]["charted_keys"], [1])
        for along in range(0, 161, 8):
            for side in (0, 160):
                self.assertAlmostEqual(depth[self.sample(1080 + along, 2080 + side)], 3.0, places=4)
                self.assertAlmostEqual(depth[self.sample(1080 + side, 2080 + along)], 3.0, places=4)
        self.assertAlmostEqual(depth[self.sample(1120, 2160)], 6.0, places=4)
        self.assertAlmostEqual(depth[self.sample(1160, 2160)], 9.5, places=4)
        # Deeper than the 12 m cap would allow is kept: the cap is the synthetic bed's.
        self.assertGreater(np.nanmax(depth), 9.0)
        # Between the shore and the 3 m contour no deeper than 3 m.
        self.assertLessEqual(depth[self.sample(1048, 2160)], 3.0)

    def test_uncharted_lake_keeps_the_synthetic_bed_and_land_features_change_nothing(self):
        # Lake 1 holds a sounding; lake 2 only the grazing end of a contour that runs mostly on
        # land, so no polygon holds the larger part of it and lake 2 stays uncharted.
        charted = WaterBody(key=1, kind="lake", level_mm=50_000,
                            geometry=shapely.box(1020.0, 2020.0, 1140.0, 2300.0))
        uncharted = WaterBody(key=2, kind="lake", level_mm=50_000,
                              geometry=shapely.box(1180.0, 2020.0, 1300.0, 2300.0))
        grazing = shapely.LineString([(1150.0, 2160.0), (1185.0, 2160.0)])  # 5 of 35 m inside
        on_land = shapely.LineString([(1150.0, 2030.0), (1160.0, 2290.0)])
        depths = DepthChart(contours=((5, 1_500, grazing), (6, 3_000, on_land)),
                            soundings=((7, 4_000, shapely.Point(1080.0, 2160.0)),))
        plain, _ = self.run_rule([charted, uncharted], None)
        result, depth = self.run_rule([charted, uncharted], depths)

        self.assertEqual(result.report["depth_chart"]["charted_keys"], [1])
        self.assertEqual(result.report["depth_chart"]["contours_owned_by_no_kept_polygon"], 2)
        east = np.s_[:, (1180 - 1000) // 8 :]
        np.testing.assert_array_equal(result.heights[east], plain.heights[east])
        self.assertAlmostEqual(depth[self.sample(1080, 2160)], 4.0, places=4)
        self.assertEqual([f.__dict__ for f in result.fills], [f.__dict__ for f in plain.fills])

        # A chart whose features all lie on land charts nothing: the result is the plain one.
        land_only = DepthChart(contours=((6, 3_000, on_land),), soundings=((8, 2_000, shapely.Point(1160.0, 2100.0)),))
        result, _ = self.run_rule([charted, uncharted], land_only)
        np.testing.assert_array_equal(result.heights, plain.heights)
        self.assertEqual(result.report["depth_chart"]["charted_keys"], [])

    def test_partly_charted_lake_is_shallower_than_the_shallowest_contour_where_none_is_drawn(self):
        # A 312 m by 72 m lake charted only at its east end (a 3 m ring). Where no contour is
        # drawn, the bed is the shore ramp min(0.15 d, 1.5 m): no deeper than the chart's
        # shallowest contour level, which a chart would have drawn around deeper water.
        lake = WaterBody(key=1, kind="lake", level_mm=50_000,
                         geometry=shapely.box(1004.0, 2124.0, 1316.0, 2196.0))
        ring = shapely.LineString([(1240, 2144), (1296, 2144), (1296, 2176), (1240, 2176), (1240, 2144)])
        # A 1.5 m contour elsewhere (off the map) sets the chart's shallowest level.
        shallow = DepthChart(contours=((1, 1_500, shapely.LineString([(0, 0), (1, 0)])), (2, 3_000, ring)), soundings=())
        result, depth = self.run_rule([lake], shallow)

        self.assertEqual(result.report["depth_chart"]["shallowest_contour_m"], 1.5)
        self.assertAlmostEqual(depth[self.sample(1240, 2160)], 3.0, places=4)
        west = depth[:, : (1150 - 1000) // 8]
        west = west[~np.isnan(west)]
        self.assertLessEqual(float(west.max()), 1.5 + 1e-5)
        # 24 m or more from the shore (the lake's middle rows) the ramp has reached 1.5 m.
        middle = depth[self.sample(1104, 2160)[0], 4 : (1150 - 1000) // 8]
        np.testing.assert_allclose(middle, 1.5, atol=1e-5)
        self.assertTrue(np.all(depth[~np.isnan(depth)] > 0.0))

    def test_charted_lake_meets_uncharted_water_of_its_group_without_a_step(self):
        # Two lakes share the edge E 1164; the west one is charted with a 6 m ring 44 m from it.
        west = WaterBody(key=1, kind="lake", level_mm=50_000,
                         geometry=shapely.box(1020.0, 2020.0, 1164.0, 2300.0))
        east = WaterBody(key=2, kind="lake", level_mm=50_000,
                         geometry=shapely.box(1164.0, 2020.0, 1300.0, 2300.0))
        ring = shapely.LineString([(1056, 2120), (1120, 2120), (1120, 2216), (1056, 2216), (1056, 2120)])
        depths = DepthChart(contours=((2, 6_000, ring),), soundings=())
        result, depth = self.run_rule([west, east], depths)

        self.assertEqual(len(result.fills), 1)
        self.assertEqual(result.report["depth_chart"]["charted_keys"], [1])
        seam = (1168 - 1000) // 8  # the east lake's first column
        rows = slice(self.sample(0, 2200)[0], self.sample(0, 2140)[0] + 1)
        # The east side is the synthetic bed (capped at 12 m, 80 m+ from any shore there). The
        # seam carries that depth, so the west side's last sample, 4 m from the seam, is within
        # one cell's synthetic slope (1.2 m) of it, not 6 m (the ring) away.
        np.testing.assert_allclose(depth[rows, seam], 12.0, atol=1e-5)
        step = np.abs(depth[rows, seam - 1] - depth[rows, seam])
        self.assertLessEqual(float(step.max()), 1.2)


if __name__ == "__main__":
    unittest.main()
