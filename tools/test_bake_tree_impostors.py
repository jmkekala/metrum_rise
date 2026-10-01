#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-only
"""Small analytic checks for CPU impostor baking and its shared DDS writer."""

import copy
import struct

import numpy as np

from bake_foliage_atlas import write_dds
from bake_tree_impostors import (bake, correct_filtered, filtered_coverage, hemi_decode, hemi_encode,
                                 mip_chain, raster_frame)
from vegetation_lod_measure import prepare


def mesh():
    return dict(centre=[0., 0., 0.], size=3., surfaces=[dict(
        vertices=[[-1., -1., 0.], [-1., 1., 0.], [1., 1., 0.], [1., -1., 0.]],
        normals=[[0., 0., 1.]]*4, colors=[[.1, .3, .05, 1.]]*4,
        uv=[[0., 0.], [0., 1.], [1., 1.], [1., 0.]], indices=[0, 1, 2, 0, 2, 3],
        shader="vegetation_wind_cards.gdshader", parameters=dict(foliage_mask="atlas"))])


def test_hemi_roundtrip_frame_centres():
    y, x = np.mgrid[:8, :8]
    uv = (np.stack((x, y), axis=-1) + .5) / 8 * 2 - 1
    np.testing.assert_allclose(hemi_encode(hemi_decode(uv)), uv, atol=1e-14)


def test_deterministic_dds_and_unit_normals(tmp_path):
    first = bake(mesh(), frames=2, frame_px=8, samples=4, load=lambda _: [np.ones((2, 2))])
    second = bake(mesh(), frames=2, frame_px=8, samples=4, load=lambda _: [np.ones((2, 2))])
    assert first[2] == second[2]
    for channel in range(2):
        a, b = tmp_path / f"a{channel}.dds", tmp_path / f"b{channel}.dds"
        write_dds(a, first[channel])
        write_dds(b, second[channel])
        assert a.read_bytes() == b.read_bytes()
        header = struct.unpack_from("<31I", a.read_bytes(), 4)
        assert header[2:4] == (16, 16) and header[6] == 5
    for level in first[1]:
        n = level[..., :3][level[..., 3] > 102].astype(float) / 255 * 2 - 1
        # RGBA8 quantization permits at most sqrt(3)/255 error in length.
        np.testing.assert_allclose(np.linalg.norm(n, axis=1), 1, atol=.007)
    assert np.any((first[0][0][..., 3] > 0) & (first[0][0][..., 3] < 255))


def test_coverage_per_frame_and_no_black_mips():
    rng = np.random.default_rng(42)
    alpha = np.where(rng.random((64, 64)) > .7, 255, 0).astype(np.uint8)
    rgb = np.full((64, 64, 3), .25)
    normal = np.zeros_like(rgb)
    normal[..., 1] = 1
    albedo, normals, report = mip_chain(rgb, normal, alpha, 2)
    for row in report:
        for index, cell in enumerate(row["cells"]):
            # Equal box-filter values cross together. The maximum tied group, rather
            # than one texel, is the representable coverage step for a binary source.
            pixels = (row["size"] // 2)**2
            level = row["level"]
            raw = alpha.astype(float)
            for _ in range(level):
                n = raw.shape[0]
                raw = raw.reshape(n//2, 2, n//2, 2).mean(axis=(1, 3))
            step = row["size"] // 2
            region = raw[index//2*step:(index//2+1)*step, index%2*step:(index%2+1)*step]
            _, counts = np.unique(region[region > 0], return_counts=True)
            tolerance = counts.max() / pixels if counts.size else 1.
            assert cell["corrected"] >= cell["target"]
            assert cell["corrected"] - cell["target"] <= tolerance + 1e-12
    for level in albedo:
        np.testing.assert_array_equal(level[..., :3], 137)  # Linear .25 encoded as sRGB.
    assert report[1]["corrected"] - report[0]["corrected"] < .08


def test_lone_texel_keeps_its_coverage_through_the_filter():
    # A crown one texel wide set just over the threshold filters below it almost everywhere.
    # At full alpha one texel filters over the threshold across about 0.94 of its area.
    cell = np.zeros((8, 8))
    cell[3, 4] = 60.
    target = .9 / 64
    corrected = correct_filtered(cell, target)
    assert filtered_coverage(corrected) >= target
    assert filtered_coverage(corrected) < 2 * target
    assert filtered_coverage(np.where(cell > 0, 103, 0)) < target / 4


def test_front_surface_mask_and_authored_back_normal():
    source = mesh()
    front = copy.deepcopy(source["surfaces"][0])
    front["vertices"] = (np.array(front["vertices"]) + [0, 0, .2]).tolist()
    front["shader"] = "standard"
    front["colors"] = [[.4, .1, .05, 1.]]*4
    source["surfaces"].append(front)
    rgb, n, alpha = raster_frame(prepare(source), np.array([0., 0., 1.]), {"atlas": [np.zeros((2, 2))]}, 8, 4)
    assert np.any(alpha > 102)
    np.testing.assert_allclose(rgb[alpha > 102], np.tile([.4, .1, .05], ((alpha > 102).sum(), 1)))
    # Looking at the reverse side of the card preserves +Z rather than flipping it.
    _, n, alpha = raster_frame(prepare(mesh()), np.array([0., 0., -1.]), {"atlas": [np.ones((2, 2))]}, 8, 4)
    assert np.any(alpha > 102)
    np.testing.assert_allclose(n[alpha > 102], np.tile([0, 0, 1], ((alpha > 102).sum(), 1)))
