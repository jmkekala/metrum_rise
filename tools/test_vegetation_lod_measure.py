#!/usr/bin/env python3
"""Analytic checks for the headless canopy measurement instrument."""

import unittest

import numpy as np

from vegetation_lod_measure import bilinear, distant_coverage, prepare, raster, sample_alpha


class RasterTests(unittest.TestCase):
    def test_distant_coverage_at_small_apparent_size(self):
        # An isolated canopy plane spans the same derivative scales as a tree.
        # Average pixel phases/headings; a single four-pixel mask is quantized.
        mesh = prepare(dict(surfaces=[dict(
            vertices=[[-4, -8, 0], [-4, 8, 0], [4, 8, 0], [4, -8, 0]],
            normals=[[0, 0, 1]]*4, colors=[[.1, .2, .05, 1]]*4,
            uv=[[0, 0]]*4, indices=[0, 1, 2, 0, 2, 3],
            shader="vegetation_distant.gdshader", coverage=distant_coverage())]))
        coverage = {}
        for height in (64, 4):
            coverage[height] = np.mean([
                raster(mesh, yaw, height, np.array(offset), "baseline", [])[0]["coverage"]
                for yaw in (-30, 0, 30) for offset in ((0, 0), (.5, .5))])
        self.assertGreaterEqual(coverage[4], coverage[64] - .02)

    def test_mip_filtering_and_repeat(self):
        mip = np.array([[0., 1.], [1., 0.]])
        uv = np.array([[.25, .25], [1.75, .25], [.5, .5]])
        np.testing.assert_allclose(bilinear(mip, uv), [0, 1, .5])
        np.testing.assert_allclose(sample_alpha([mip, np.ones((1, 1))], uv, np.eye(2)), 1)

    def test_opaque_square_and_scissored_occluder(self):
        surface = dict(vertices=[[-1, -2**.5, 0], [-1, 2**.5, 0],
                                 [1, 2**.5, 0], [1, -2**.5, 0]],
                       normals=[[0, 0, 1]]*4, colors=[[.1, .2, .05, 0]]*4,
                       uv=[[0, 0], [0, 1], [1, 1], [1, 0]], indices=[0, 1, 2, 0, 2, 3],
                       shader="standard")
        mesh = prepare(dict(surfaces=[surface]))
        row, mask, image = raster(mesh, 0, 16, np.zeros(2), "baseline", [np.zeros((2, 2))])
        self.assertEqual(row["coverage"], 1)
        self.assertEqual(row["covered_pixels"], 256)
        # Light is behind this face: only fixed ambient .2 remains.
        np.testing.assert_allclose(row["rgb"], [.02, .04, .01], atol=1e-10)
        front = {k: v.copy() if isinstance(v, np.ndarray) else v for k, v in surface.items()}
        front["vertices"] = front["vertices"] + np.array([0, .1, .1])
        front["shader"] = "vegetation_wind_cards.gdshader"
        mesh["surfaces"].append(front)
        after, mask_after, image_after = raster(mesh, 0, 16, np.zeros(2), "baseline", [np.zeros((2, 2))])
        self.assertEqual(after["coverage"], 1)
        np.testing.assert_array_equal(mask, mask_after)
        np.testing.assert_allclose(image, image_after, atol=1e-10)
        _, _, lit = raster(mesh, 0, 16, np.zeros(2), "baseline", [np.ones((2, 2))])
        self.assertGreater(lit.sum(), image.sum())


if __name__ == "__main__":
    unittest.main()
