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
    # The runtime's fill (`apply_lake_fill_to_depth`): 4-connected over samples at or below the
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


if __name__ == "__main__":
    unittest.main()
