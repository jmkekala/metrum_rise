#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-only
"""Turn tools/model_trees.py and tools/model_landscape.py output into the game's assets.

Usage: python3 tools/prepare_tree_models.py imgs/tree_models/out
       python3 tools/prepare_tree_models.py --landscape imgs/landscape_models/out

Writes godot/assets/models/vegetation/trees/ (or landscape/, and rocks/ for --landscape):
  <form>_<n>.glb      meshes with image, texture and sampler references removed; the game
                      assigns its own wind materials by the glTF material name.
  <form>_<n>_lod1.glb the reduced level, likewise.
  <form>_foliage.dds  512 px RGBA atlas, colour flooded into transparent texels, with a
                      complete mip chain that keeps each cell's alpha-tested coverage.
  <form>_bark.png     the bark tile, unchanged.
  trees.json          per form, the mean linear colour of the opaque foliage texels and of the
                      bark, and the mean depth term the card shader reads from mip 4.
                      landscape.json for the yard plants, which have one model and no reduced
                      level each.
  rocks/rock_<n>.glb  stripped like the rest, beside the shared rock_albedo.png and
                      rock_normal.png tiles, which the game's rock material samples.

The shaders divide texture colour by these means, so vertex colour stays the mean albedo:
the distant levels, the impostor bake and the crown integral read it without the textures.
Deterministic: NumPy only, fixed ordering, no timestamps.
"""

import argparse
import json
from pathlib import Path
import shutil
import struct
import zlib

import numpy as np

from bake_foliage_atlas import correct, coverage, write_dds

ROOT = Path(__file__).resolve().parents[1]
MODELS = ROOT / "godot/assets/models/vegetation"
FORMS = ("spruce", "pine", "birch", "aspen")
# Yard plants in the game's bush variant order; see LANDSCAPE_FORMS in tree_species.gd.
LANDSCAPE_FORMS = ("lilac", "spirea", "rose", "cotoneaster", "mugo", "juniper",
                   "hedge_low", "hedge_mid", "hedge_tall")
ROCKS = 6
SIZE = 512
THRESHOLD = 0.4
# The card shader samples mip 4 for its depth term; see vegetation_wind_cards.gdshader.
DEPTH_MIP = 4


def read_png(path):
    """Decode the 8-bit RGBA, non-interlaced PNGs model_trees.py writes (filter type 0 only)."""
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    offset, idat, width = 8, b"", 0
    while offset < len(data):
        length, kind = struct.unpack(">I4s", data[offset:offset + 8])
        payload = data[offset + 8:offset + 8 + length]
        if kind == b"IHDR":
            width, height, depth, colour, _, _, interlace = struct.unpack(">2I5B", payload)
            assert (depth, colour, interlace) == (8, 6, 0), path
        elif kind == b"IDAT":
            idat += payload
        offset += 12 + length
    rows = np.frombuffer(zlib.decompress(idat), np.uint8).reshape(height, 1 + width * 4)
    assert np.all(rows[:, 0] == 0), path
    return rows[:, 1:].reshape(height, width, 4)


def linear(srgb_bytes):
    c = srgb_bytes / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def srgb_bytes(value):
    encoded = np.where(value <= 0.0031308, value * 12.92, 1.055 * np.power(np.maximum(value, 0), 1 / 2.4) - 0.055)
    return np.clip(np.rint(encoded * 255), 0, 255).astype(np.uint8)


def weighted_box(colour, weight):
    """Alpha-weighted 2x2 mean, so transparent texels never darken a mip."""
    h, w = weight.shape
    total = (colour * weight[..., None]).reshape(h // 2, 2, w // 2, 2, 3).sum(axis=(1, 3))
    count = weight.reshape(h // 2, 2, w // 2, 2).sum(axis=(1, 3))
    return total / np.maximum(count, 1e-9)[..., None], count / 4


def flood(colour, known):
    """Extend known colour outward ring by ring, then fill the rest with the opaque mean."""
    colour, known = np.where(known[..., None], colour, 0.0), known.astype(np.float64)
    for _ in range(32):
        total = sum(np.roll(np.roll(colour * known[..., None], dy, 0), dx, 1)
                    for dy in (-1, 0, 1) for dx in (-1, 0, 1))
        count = sum(np.roll(np.roll(known, dy, 0), dx, 1) for dy in (-1, 0, 1) for dx in (-1, 0, 1))
        grow = (known == 0) & (count > 0)
        colour = np.where(grow[..., None], total / np.maximum(count, 1)[..., None], colour)
        known = np.where(grow, 1.0, known)
    mean = colour[known > 0].mean(axis=0)
    return np.where(known[..., None] > 0, colour, mean)


def foliage_levels(source):
    """512 px base from the 1024 px atlas, then a mip chain that keeps each cell's coverage."""
    alpha = source[..., 3].astype(np.float64)
    colour, _ = weighted_box(linear(source[..., :3].astype(np.float64)), alpha / 255.0)
    alpha = alpha.reshape(SIZE, 2, SIZE, 2).mean(axis=(1, 3))
    colour = flood(colour, alpha >= THRESHOLD * 255)
    half = SIZE // 2
    cells = [(y, x) for y in (0, half) for x in (0, half)]
    targets = [coverage(alpha[y:y + half, x:x + half]) for y, x in cells]
    levels, raw = [], alpha
    while True:
        size = raw.shape[0]
        if size == SIZE:
            corrected = np.rint(raw).astype(np.uint8)
        elif size > 1:
            step = size // 2
            corrected = np.zeros((size, size), np.uint8)
            for (y, x), target in zip(cells, targets):
                y, x = y * size // SIZE, x * size // SIZE
                corrected[y:y + step, x:x + step] = correct(raw[y:y + step, x:x + step], target)
        else:
            corrected = correct(raw, sum(targets) / 4)
        levels.append(np.dstack((srgb_bytes(colour), corrected)))
        if size == 1:
            return levels
        colour, _ = weighted_box(colour, raw / 255.0 + 1e-3)
        raw = raw.reshape(size // 2, 2, size // 2, 2).mean(axis=(1, 3))


def foliage_means(levels):
    base = levels[0]
    opaque = base[..., 3] >= THRESHOLD * 255
    leaf = linear(base[..., :3][opaque].astype(np.float64)).mean(axis=0)
    # Nearest mip-4 texel under each opaque base texel, as the shader's textureLod reads it.
    depth_mip = levels[DEPTH_MIP][..., 3] / 255.0
    scale = SIZE // depth_mip.shape[0]
    ys, xs = np.nonzero(opaque)
    depth = float(depth_mip[ys // scale, xs // scale].mean())
    return [round(float(v), 6) for v in leaf], round(depth, 6)


def strip_glb(source, target):
    """Drop images, textures and samplers; keep material names, which select the game material."""
    data = source.read_bytes()
    magic, version, _ = struct.unpack_from("<4sII", data, 0)
    assert magic == b"glTF" and version == 2
    json_length, json_kind = struct.unpack_from("<I4s", data, 12)
    assert json_kind == b"JSON"
    document = json.loads(data[20:20 + json_length])
    rest = data[20 + json_length:]
    for key in ("images", "textures", "samplers"):
        document.pop(key, None)
    for material in document.get("materials", []):
        pbr = material.get("pbrMetallicRoughness", {})
        pbr.pop("baseColorTexture", None)
        for key in ("normalTexture", "occlusionTexture", "emissiveTexture"):
            material.pop(key, None)
    encoded = json.dumps(document, separators=(",", ":"), sort_keys=True).encode()
    encoded += b" " * (-len(encoded) % 4)
    body = struct.pack("<I4s", len(encoded), b"JSON") + encoded + rest
    target.write_bytes(struct.pack("<4sII", b"glTF", 2, 12 + len(body)) + body)


def prepare_forms(source, output, forms, models, levels):
    """Strip every model of each form and write its atlas, bark and measured means."""
    output.mkdir(parents=True, exist_ok=True)
    metadata = {}
    for form in forms:
        for n in range(models):
            for suffix in levels:
                strip_glb(source / f"{form}_{n}{suffix}.glb", output / f"{form}_{n}{suffix}.glb")
        levels_ = foliage_levels(read_png(source / f"{form}_foliage.png"))
        write_dds(output / f"{form}_foliage.dds", levels_)
        shutil.copyfile(source / f"{form}_bark.png", output / f"{form}_bark.png")
        leaf, depth = foliage_means(levels_)
        bark = linear(read_png(source / f"{form}_bark.png")[..., :3].astype(np.float64)).mean(axis=(0, 1))
        metadata[form] = {"leaf_mean": leaf, "leaf_depth_mean": depth,
                          "bark_mean": [round(float(v), 6) for v in bark],
                          "foliage_coverage": round(coverage(levels_[0][..., 3]), 4)}
    return metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--landscape", action="store_true",
                        help="yard plants and rocks from tools/model_landscape.py")
    args = parser.parse_args()
    if args.landscape:
        # The game draws a yard plant at one level; the reduced models are not shipped.
        output = MODELS / "landscape"
        metadata = prepare_forms(args.source, output, LANDSCAPE_FORMS, 1, ("",))
        (output / "landscape.json").write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n")
        rocks = MODELS / "rocks"
        rocks.mkdir(parents=True, exist_ok=True)
        for n in range(ROCKS):
            strip_glb(args.source / f"rock_{n}.glb", rocks / f"rock_{n}.glb")
        for tile in ("rock_albedo.png", "rock_normal.png"):
            shutil.copyfile(args.source / tile, rocks / tile)
    else:
        output = MODELS / "trees"
        metadata = prepare_forms(args.source, output, FORMS, 3, ("", "_lod1"))
        (output / "trees.json").write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n")
    print(json.dumps(metadata, indent=1))


if __name__ == "__main__":
    main()
