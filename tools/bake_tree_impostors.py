#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-only
"""Bake deterministic hemi-octahedral tree albedo/normal atlases on the CPU.

Usage: python3 tools/bake_tree_impostors.py /tmp/tree-meshes.json
Export with Godot's tests/vegetation_lod_measure.gd first. NumPy is the only
non-stdlib dependency. One form per near variant, baked from the reduced tree that the
impostor replaces; forms run in separate worker processes.
No lighting is baked. Colours are filtered in linear space and stored as sRGB;
normals retain the authored object-space direction on both sides of a card.
"""

import argparse
from concurrent.futures import ProcessPoolExecutor
import hashlib
import json
from multiprocessing import get_context
from pathlib import Path

import numpy as np

from bake_foliage_atlas import THRESHOLD, correct, coverage, write_dds
from vegetation_lod_measure import ROOT, atlas_mips, prepare, sample_alpha

SPECIES_NAMES = ("conifer", "broadleaf")
FRAMES = 8
FRAME_PX = 64


def hemi_decode(uv):
    uv = np.asarray(uv)
    x, z = (uv[..., 0] + uv[..., 1]) * .5, (uv[..., 0] - uv[..., 1]) * .5
    d = np.stack((x, 1 - np.abs(x) - np.abs(z), z), axis=-1)
    return d / np.linalg.norm(d, axis=-1, keepdims=True)


def hemi_encode(d):
    d = np.array(d, dtype=float, copy=True)
    d[..., 1] = np.maximum(d[..., 1], 0)
    d /= np.abs(d).sum(axis=-1, keepdims=True)
    return np.stack((d[..., 0] + d[..., 2], d[..., 0] - d[..., 2]), axis=-1)


def normalized(n):
    lengths = np.linalg.norm(n, axis=-1, keepdims=True)
    return np.where(lengths > 1e-12, n / np.maximum(lengths, 1e-12), [0., 1., 0.])


def srgb(linear):
    return np.where(linear <= .0031308, linear * 12.92,
                    1.055 * np.maximum(linear, 0) ** (1 / 2.4) - .055)


def raster_frame(mesh, direction, atlases, frame_px, samples):
    """Orthographic z buffer at samples x samples subpixels per output pixel."""
    size, centre = mesh["size"], np.asarray(mesh["centre"])
    right = normalized(np.cross([0., 1., 0.], direction))
    up = np.cross(direction, right)
    basis = np.array([right, -up, direction]).T  # Image Y increases downward.
    resolution = frame_px * samples
    depth = np.full(resolution * resolution, -np.inf)
    color = np.zeros((resolution * resolution, 3))
    normal = np.zeros_like(color)
    for surface in mesh["surfaces"]:
        projected = (surface["vertices"] - centre) @ basis
        projected[:, :2] = (projected[:, :2] / size + .5) * resolution
        cards = surface["shader"] == "vegetation_wind_cards.gdshader"
        if cards:
            mips = atlases[surface["parameters"]["foliage_mask"]]
        for tri in surface["indices"]:
            p = projected[tri]
            edges = (p[1:, :2] - p[0, :2]).T
            determinant = np.linalg.det(edges)
            # Godot uses clockwise front faces; the image Y inversion reverses the sign.
            if abs(determinant) < 1e-10 or (not cards and determinant <= 0):
                continue
            lo = np.maximum(np.ceil(p[:, :2].min(axis=0) - .5).astype(int), 0)
            hi = np.minimum(np.floor(p[:, :2].max(axis=0) - .5).astype(int), resolution - 1)
            if np.any(lo > hi):
                continue
            ys, xs = np.mgrid[lo[1]:hi[1]+1, lo[0]:hi[0]+1]
            ids = (ys * resolution + xs).ravel()
            inv = np.linalg.inv(edges)
            bc = (np.column_stack((xs.ravel()+.5, ys.ravel()+.5)) - p[0, :2]) @ inv.T
            bary = np.column_stack((1-bc.sum(axis=1), bc))
            z = bary @ p[:, 2]
            keep = np.all(bary >= -1e-9, axis=1) & (z > depth[ids])
            if cards:
                uv = surface["uv"][tri]
                derivatives = (uv[1:] - uv[0]).T @ inv
                keep &= sample_alpha(mips, bary @ uv, derivatives) >= .4
            if not np.any(keep):
                continue
            ids, bary = ids[keep], bary[keep]
            depth[ids] = z[keep]
            color[ids] = bary @ surface["colors"][tri, :3]
            normal[ids] = normalized(bary @ surface["normals"][tri])
    visible = np.isfinite(depth)
    mean = color[visible].mean(axis=0) if visible.any() else np.zeros(3)
    # Average visible samples only, keeping edge albedo independent of alpha.
    shape = (frame_px, samples, frame_px, samples)
    alpha = visible.reshape(shape).mean(axis=(1, 3))
    rgb = color.reshape(*shape, 3).mean(axis=(1, 3)) / np.maximum(alpha[..., None], 1e-12)
    n = normalized(normal.reshape(*shape, 3).mean(axis=(1, 3)))
    rgb[alpha == 0] = mean
    n[alpha == 0] = [0., 1., 0.]
    return rgb, n, np.rint(alpha * 255).astype(np.uint8)


def box(image):
    h, w = image.shape[:2]
    return image.reshape(h//2, 2, w//2, 2, *image.shape[2:]).mean(axis=(1, 3))


def covered_box(value, weight):
    """Coverage-weighted 2x2 mean. Empty texels hold only fill values; an unweighted mean mixed
    the fill "up" normal into every crown edge and turned distant impostor normals toward the sky."""
    total = box(weight)[..., None]
    mean = box(value * weight[..., None]) / np.maximum(total, 1e-12)
    return np.where(total > 0, mean, box(value))


def bilinear_rows(n, k):
    """Weights that reconstruct n texels at k points each, as the sampler filters them:
    linear between texel centres, clamped to the edge texel. The shader's frame inset
    clamps sampling to the edge texel's centre, so a frame never reads its neighbour."""
    p = (np.arange(n * k) + .5) / k - .5
    low = np.floor(p).astype(int)
    t = p - low
    weights = np.zeros((n * k, n))
    rows = np.arange(n * k)
    np.add.at(weights, (rows, np.clip(low, 0, n - 1)), 1 - t)
    np.add.at(weights, (rows, np.clip(low + 1, 0, n - 1)), t)
    return weights


def filtered_coverage(alpha, k=8):
    """Share of a frame the alpha test keeps once the sampler filters it bilinearly."""
    weights = bilinear_rows(alpha.shape[0], k)
    return float(np.mean(weights @ alpha.astype(float) @ weights.T >= THRESHOLD * 255))


def correct_filtered(alpha, target):
    """Scale alpha until the filtered frame keeps its target coverage.

    `correct` counts texels over the threshold, which is the coverage a nearest
    sampler draws. The impostor samples bilinearly, and a crown one or two texels
    wide that `correct` sets just over the threshold filters below it almost
    everywhere: seen from above the small pine crowns vanished at 900 m while the
    larger spruce crowns stayed, and the stand drew 0.79 of its near level's colour
    against 0.90 from the side.
    Coverage only grows with the scale, so a bisection finds the smallest one.
    """
    nonzero = alpha[alpha > 0]
    if not nonzero.size or target <= 0:
        return np.zeros_like(alpha, dtype=np.uint8)
    scaled = lambda scale: np.clip(np.rint(alpha * scale), 0, 255).astype(np.uint8)
    low, high = 0., 255. / nonzero.min()
    if filtered_coverage(scaled(high)) < target:
        return scaled(high)
    for _ in range(32):
        middle = (low + high) * .5
        low, high = (low, middle) if filtered_coverage(scaled(middle)) >= target else (middle, high)
    return scaled(high)


def mip_chain(rgb, normal, alpha, frames):
    """Correct each frame independently until one texel; tail mips use mean coverage.

    Coverage is quantized: the correction chooses the smallest reachable coverage
    at or above the target. Above one texel per frame it is the coverage the
    bilinear sampler draws; at one texel per frame every nonempty frame must become
    opaque. The report records these unavoidable coarse-mip errors rather than
    claiming exact coverage.
    """
    size = alpha.shape[0]
    frame_px = size // frames
    targets = [coverage(alpha[y:y+frame_px, x:x+frame_px])
               for y in range(0, size, frame_px) for x in range(0, size, frame_px)]
    albedos, normals, report = [], [], []
    raw = alpha.astype(float)
    while True:
        size = raw.shape[0]
        corrected = np.rint(raw).astype(np.uint8)
        cells = []
        if size >= frames:
            step = size // frames
            for index, (y, x) in enumerate(( (y, x) for y in range(0, size, step)
                                            for x in range(0, size, step) )):
                cell = raw[y:y+step, x:x+step]
                if not albedos:
                    result = corrected[y:y+step, x:x+step]
                elif step > 1:
                    result = correct_filtered(cell, targets[index])
                else:
                    result = correct(cell, targets[index])
                corrected[y:y+step, x:x+step] = result
                drawn = filtered_coverage if step > 1 else coverage
                cells.append(dict(target=targets[index], raw=drawn(np.rint(cell)),
                                  corrected=drawn(result)))
        else:
            corrected = correct(raw, float(np.mean(targets)))
        albedos.append(np.dstack((np.rint(np.clip(srgb(rgb), 0, 1)*255).astype(np.uint8), corrected)))
        # Mips keep the averaged normal unnormalized: its length records how far the normals
        # under one texel disagreed, which the shader turns into a volume response.
        encoded = np.rint((np.clip(normal, -1, 1)*.5+.5)*255).astype(np.uint8)
        encoded[corrected == 0] = [128, 255, 128]
        normals.append(np.dstack((encoded, corrected)))
        report.append(dict(level=len(report), size=size, raw=coverage(np.rint(raw)),
                           corrected=coverage(corrected), cells=cells))
        if size == 1:
            break
        # Always reduce the uncorrected source, as in the foliage atlas tool.
        raw, rgb, normal = box(raw), covered_box(rgb, raw), covered_box(normal, raw)
    return albedos, normals, report


def frame_bounds(alpha, frames):
    """Per view, the frame UV rectangle holding every texel with any coverage, as
    [u0, v0, u1, v1]. Coarser mips only scale texels this rectangle already holds, so the
    impostor crops its quad to it and pads by the mip it samples. An empty view is a point."""
    step = alpha.shape[0] // frames
    bounds = []
    for y in range(0, alpha.shape[0], step):
        for x in range(0, alpha.shape[1], step):
            rows, cols = np.nonzero(alpha[y:y+step, x:x+step])
            if not rows.size:
                bounds.append([.5, .5, .5, .5])
                continue
            bounds.append([cols.min() / step, rows.min() / step,
                           (cols.max() + 1) / step, (rows.max() + 1) / step])
    return bounds


def bake(mesh, frames=FRAMES, frame_px=FRAME_PX, samples=4, load=atlas_mips):
    mesh = prepare(mesh)
    # Each card surface names its own foliage atlas; read each file once per form.
    atlases = {s["parameters"]["foliage_mask"]: None for s in mesh["surfaces"]
               if s["shader"] == "vegetation_wind_cards.gdshader"}
    atlases = {path: load(path) for path in atlases}
    size = frames * frame_px
    rgb, normal = np.zeros((size, size, 3)), np.zeros((size, size, 3))
    alpha = np.zeros((size, size), dtype=np.uint8)
    for cy in range(frames):
        for cx in range(frames):
            direction = hemi_decode((np.array([cx, cy])+.5) / frames * 2 - 1)
            color, n, a = raster_frame(mesh, direction, atlases, frame_px, samples)
            region = np.s_[cy*frame_px:(cy+1)*frame_px, cx*frame_px:(cx+1)*frame_px]
            rgb[region], normal[region], alpha[region] = color, n, a
    return mip_chain(rgb, normal, alpha, frames) + (frame_bounds(alpha, frames),)


def bake_form(form, mesh, output):
    albedo, normal, report, bounds = bake(mesh)
    write_dds(output / f"tree_impostor_{form}_albedo.dds", albedo)
    write_dds(output / f"tree_impostor_{form}_normal.dds", normal)
    return dict(centre=mesh["centre"], size=mesh["size"], frame_bounds=bounds, coverage=report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("export", type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "godot/assets/textures/vegetation")
    args = parser.parse_args()
    data = json.loads(args.export.read_text())
    source = data["impostor_source_json"]
    meshes = json.loads(source)
    # Form names match tree_species.gd impostor_form().
    names = [f"{SPECIES_NAMES[m['species']]}_{m['variant']:02d}" for m in meshes]
    assert all(m["lod"] == 1 for m in meshes) and len(set(names)) == len(names)
    args.output.mkdir(parents=True, exist_ok=True)
    with ProcessPoolExecutor(max_workers=4, mp_context=get_context("spawn")) as pool:
        pending = [pool.submit(bake_form, form, mesh, args.output) for form, mesh in zip(names, meshes)]
        forms = {form: future.result() for form, future in zip(names, pending)}
    metadata = dict(frames=FRAMES, frame_px=FRAME_PX, supersample=4,
                    source_sha256=hashlib.sha256(source.encode()).hexdigest(), forms=forms)
    (args.output / "tree_impostors.json").write_text(json.dumps(metadata, separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
