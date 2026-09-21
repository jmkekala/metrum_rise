#!/usr/bin/env python3
"""Headless canopy raster measurements from the live Godot mesh catalogue.

Usage: python3 tools/vegetation_lod_measure.py /tmp/render02/phase1

Uses NumPy, already used by the foliage atlas tool. This is a CPU reference raster,
not a Godot framebuffer: orthographic pixel-centre samples, z buffer, linear mip
filtering, 0.4 alpha scissor, Burley diffuse plus palette-gated backlight. RGB is
linear, before exposure/tonemapping. No shadows, AO, sky, specular or antialiasing.
The deliberately isolated lighting model is documented in docs/terrain.md.
"""

import argparse
import csv
from concurrent.futures import ProcessPoolExecutor
import hashlib
import json
import math
from multiprocessing import get_context
from pathlib import Path
import re
import struct
import subprocess
import sys

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
LIGHT = np.array([0.3, 0.5, -0.8])
LIGHT /= np.linalg.norm(LIGHT)
BACKLIGHT = np.array([0.080, 0.150, 0.040])
# Shader source_color uniforms are sRGB; mesh vertex RGB is linear.
BACKLIGHT = np.where(BACKLIGHT <= 0.04045, BACKLIGHT / 12.92,
                     ((BACKLIGHT + 0.055) / 1.055) ** 2.4)
MODES = ("baseline", "opaque_cards", "no_backlight", "wind_2s")


def distant_coverage():
    source = (ROOT / "godot/scripts/shaders/vegetation_distant.gdshader").read_text()
    return float(re.search(r"uniform float crown_coverage[^;]*=\s*([0-9.]+);", source)[1])


def atlas_mips():
    data = (ROOT / "godot/assets/textures/vegetation/foliage_atlas.dds").read_bytes()
    header = struct.unpack_from("<31I", data, 4)
    assert data[:4] == b"DDS " and header[0] == 124 and header[21] == 32
    h, w, count = header[2], header[3], header[6]
    result, offset = [], 128
    for _ in range(count):
        result.append(np.frombuffer(data, np.uint8, w*h*4, offset).reshape(h, w, 4)[:, :, 3] / 255.)
        offset += w*h*4
        h, w = max(1, h//2), max(1, w//2)
    assert offset == len(data)
    return result


def bilinear(mip, uv):
    xy = uv * np.array(mip.shape[::-1]) - 0.5
    ij = np.floor(xy).astype(int)
    f = xy - ij
    x, y = ij.T
    h, w = mip.shape
    return ((mip[y % h, x % w]*(1-f[:, 0]) + mip[y % h, (x+1) % w]*f[:, 0])*(1-f[:, 1])
            + (mip[(y+1) % h, x % w]*(1-f[:, 0]) + mip[(y+1) % h, (x+1) % w]*f[:, 0])*f[:, 1])


def sample_alpha(mips, uv, derivatives):
    footprint = max(np.linalg.norm(derivatives[:, 0]), np.linalg.norm(derivatives[:, 1]))
    level = float(np.clip(math.log2(max(footprint*mips[0].shape[0], 1)), 0, len(mips)-1))
    lo = int(level)
    return bilinear(mips[lo], uv)*(1-level+lo) + bilinear(mips[min(lo+1, len(mips)-1)], uv)*(level-lo)


def prepare(mesh):
    for surface in mesh["surfaces"]:
        for key in ("vertices", "normals", "colors", "uv", "indices"):
            surface[key] = np.asarray(surface[key])
        surface["indices"] = surface["indices"].astype(int).reshape(-1, 3)
    return mesh


def wind(vertices, weights, time):
    direction = np.array([1., .35]) / math.hypot(1, .35)
    across = np.array([-direction[1], direction[0]])
    phase = time * .8  # Fixed instance origin (0,0).
    bend = .36*math.sin(phase) + .08*math.sin(phase*.5)
    flutter = .06*math.sin(phase*4)*weights
    result = vertices.copy()
    result[:, [0, 2]] += (direction*bend + across*flutter[:, None])*weights[:, None]
    return result


def crown_sample(position, cell_size):
    cell = np.floor(position/cell_size).astype(np.int64).astype(np.uint32)
    h = cell[:, 0]*np.uint32(374761393) + cell[:, 1]*np.uint32(668265263) + cell[:, 2]*np.uint32(2246822519)
    h = (h ^ (h >> 13))*np.uint32(1274126177)
    return ((h ^ (h >> 16)) & 16777215).astype(float)/16777216


def filtered_sample(position, footprint):
    octave = max(math.log2(footprint*4), 0.)
    cell_size = 2**math.floor(octave)*.25
    t = octave % 1
    x = crown_sample(position, cell_size)*(1-t) + crown_sample(position, cell_size*2)*t
    a, b = max(t, 1-t), min(t, 1-t)
    if b < .00001:
        return x
    return np.where(x < b, x*x/(2*a*b),
                    np.where(x > a, 1-(1-x)**2/(2*a*b), (x-.5*b)/a))


def raster(mesh, yaw, height, offset, mode, mips, elevation=45.):
    # Right/up/toward-camera basis at the given elevation. Light stays world-fixed.
    angle = math.radians(yaw)
    tilt = math.radians(elevation)
    view = np.array([math.sin(angle)*math.cos(tilt), math.sin(tilt), math.cos(angle)*math.cos(tilt)])
    right = np.array([math.cos(angle), 0., -math.sin(angle)])
    up = np.cross(view, right)
    basis = np.array([right, up, view]).T
    # Bounds belong to foliage geometry, not the trunk or a rasterized occupied bbox.
    crown = np.concatenate([s["vertices"][s["colors"][:, 1] > s["colors"][:, 0]]
                            for s in mesh["surfaces"]]) @ basis
    lower, upper = crown[:, :2].min(axis=0), crown[:, :2].max(axis=0)
    scale = height / (upper[1]-lower[1])
    width = (upper[0]-lower[0])*scale
    # Fixed 64-pixel frame preserves equal apparent height and silhouette positions.
    centre = (upper+lower)*.5
    shift = np.array([32., 32.]) + offset
    yy, xx = np.mgrid[:64, :64]
    pixels = np.column_stack((xx.ravel()+.5, yy.ravel()+.5))
    # A sub-pixel silhouette can contain no pixel centre at all, which leaves the
    # coverage ratio with an empty denominator. Hold the frame at one pixel either
    # side of the centre so the smallest sweep steps still measure something.
    extent = np.maximum(np.array([width, height])*.5, 1.)
    in_box = np.all(np.abs(pixels-shift) < extent, axis=1)
    depth = np.full(4096, -np.inf)
    image = np.zeros((4096, 3))
    half = (LIGHT+view) / np.linalg.norm(LIGHT+view)
    fd90 = 2*np.dot(LIGHT, half)**2-.5
    for surface in mesh["surfaces"]:
        shader = surface["shader"]
        cards = shader == "vegetation_wind_cards.gdshader"
        vertices = surface["vertices"]
        if mode == "wind_2s" and shader in ("vegetation_wind.gdshader", "vegetation_wind_cards.gdshader"):
            vertices = wind(vertices, surface["colors"][:, 3], 2.)
        projected = vertices @ basis
        projected[:, :2] = (projected[:, :2]-centre)*scale + shift
        for tri in surface["indices"]:
            p = projected[tri]
            edges = (p[1:, :2]-p[0, :2]).T
            determinant = np.linalg.det(edges)
            if abs(determinant) < 1e-10 or (not cards and determinant >= 0):
                continue
            lo = np.maximum(np.ceil(p[:, :2].min(axis=0)-.5).astype(int), 0)
            hi = np.minimum(np.floor(p[:, :2].max(axis=0)-.5).astype(int), 63)
            if np.any(lo > hi):
                continue
            ys, xs = np.mgrid[lo[1]:hi[1]+1, lo[0]:hi[0]+1]
            ids = (ys*64+xs).ravel()
            inv = np.linalg.inv(edges)
            bc = (pixels[ids]-p[0, :2]) @ inv.T
            bary = np.column_stack((1-bc.sum(axis=1), bc))
            z = bary @ p[:, 2]
            keep = np.all(bary >= -1e-9, axis=1) & in_box[ids] & (z > depth[ids])
            if cards and mode != "opaque_cards":
                uv = surface["uv"][tri]
                derivatives = (uv[1:]-uv[0]).T @ inv
                keep &= sample_alpha(mips, bary @ uv, derivatives) >= .4
            if shader == "vegetation_distant.gdshader":
                color = bary @ surface["colors"][tri, :3]
                derivatives = (vertices[tri[1:]]-vertices[tri[0]]).T @ inv
                footprint = max(np.linalg.norm(derivatives[:, 0]), np.linalg.norm(derivatives[:, 1]))
                t = float(np.clip((footprint-4)/4, 0, 1))
                t = t*t*(3-2*t)
                coverage = np.where(color[:, 1] > color[:, 0], surface["coverage"]*(1-t)+t, .04)
                keep &= filtered_sample(bary @ vertices[tri], footprint) <= coverage
            if not np.any(keep):
                continue
            ids, bary, z = ids[keep], bary[keep], z[keep]
            color = bary @ surface["colors"][tri, :3]
            normal = bary @ surface["normals"][tri]
            normal /= np.maximum(np.linalg.norm(normal, axis=1)[:, None], 1e-12)
            nl = np.maximum(normal @ LIGHT, 0)
            nv = np.maximum(normal @ view, 1e-4)
            # Burley, roughness=1, white key radiance=pi, ambient irradiance=.2.
            diffuse = (1+fd90*(1-nv)**5)*(1+fd90*(1-nl)**5)*nl
            illumination = np.repeat((.2+diffuse)[:, None], 3, axis=1)
            if shader != "standard" and mode != "no_backlight":
                gate = np.clip((color[:, 1]-color[:, 0])/.05, 0, 1)
                gate = gate*gate*(3-2*gate)
                illumination += (1-diffuse)[:, None]*gate[:, None]*BACKLIGHT
            if shader == "vegetation_distant.gdshader":
                color = np.where((color[:, 1] > color[:, 0])[:, None], color, [.025, .030, .010])
            image[ids] = color*illumination
            depth[ids] = z
    covered = np.isfinite(depth)
    assert in_box.sum() > 0
    # Coverage is the fill of the crown's OWN box, and every level is rastered at one apparent
    # height, so both of a crown's size terms are divided out of it. Two levels can match on
    # coverage while one hides a third of the ground the other does. `footprint` is the metre
    # box the crown fills, in world units and at true relative scale, so it is comparable
    # across levels: coverage says how solid a crown is, footprint how much sky it takes.
    box = upper-lower
    return {"coverage": float(covered.sum()/in_box.sum()),
            "rgb": (image[covered].mean(axis=0) if covered.any() else np.zeros(3)).tolist(), "bbox_width": float(width),
            "footprint": float(covered.sum()/in_box.sum()*box[0]*box[1]),
            "crown_m": [float(box[0]), float(box[1])],
            "covered_pixels": int(covered.sum()), "bbox_pixels": int(in_box.sum())}, covered, image


def measure(meshes, height, mips, elevation=45.):
    rows = []
    for mesh in meshes:
        vertices = sum(len(s["vertices"]) for s in mesh["surfaces"])
        triangles = sum(len(s["indices"]) for s in mesh["surfaces"])
        for yaw in (0, 90, 180, 270):
            for offset in ((0., 0.), (.5, .5)):
                reference = None
                for mode in MODES:
                    row, mask, image = raster(mesh, yaw, height, np.array(offset), mode, mips, elevation)
                    if reference is None:
                        reference = mask, image
                    row.update(species=mesh["species"], variant=mesh["variant"], lod=mesh["lod"],
                               yaw=yaw, offset=offset, mode=mode, vertices=vertices, triangles=triangles,
                               silhouette_change=float(np.count_nonzero(mask != reference[0])/row["bbox_pixels"]),
                               image_change=float(np.abs(image-reference[1]).sum()/row["bbox_pixels"]/3))
                    rows.append(row)
    return rows


def summarize(rows):
    result = []
    for species in (0, 1):
        for lod in range(3):
            for mode in MODES:
                group = [r for r in rows if (r["species"], r["lod"], r["mode"]) == (species, lod, mode)]
                result.append(dict(species=species, lod=lod, mode=mode, samples=len(group),
                                   coverage=float(np.mean([r["coverage"] for r in group])),
                                   rgb=np.mean([r["rgb"] for r in group], axis=0).tolist(),
                                   bbox_width=float(np.mean([r["bbox_width"] for r in group])),
                                   footprint=float(np.mean([r["footprint"] for r in group])),
                                   crown_m=np.mean([r["crown_m"] for r in group], axis=0).tolist(),
                                   vertices=[min(r["vertices"] for r in group), max(r["vertices"] for r in group)],
                                   triangles=[min(r["triangles"] for r in group), max(r["triangles"] for r in group)],
                                   silhouette_change=float(np.mean([r["silhouette_change"] for r in group])),
                                   image_change=float(np.mean([r["image_change"] for r in group]))))
    return result


def transitions(summary):
    result = []
    for species in (0, 1):
        levels = [r for r in summary if r["species"] == species and r["mode"] == "baseline"]
        for a, b in zip(levels, levels[1:]):
            result.append(dict(species=species, transition=f'{a["lod"]}->{b["lod"]}',
                               coverage_pp=100*(b["coverage"]-a["coverage"]),
                               # Against the NEAR crown, not the previous level: the near crown
                               # is what the player is standing in when the switch happens, and
                               # two small steps that agree with each other still open the
                               # ground up if both sit below it.
                               footprint_pct=100*b["footprint"]/levels[0]["footprint"],
                               rgb_delta=(np.array(b["rgb"])-a["rgb"]).tolist()))
    return result


def height_sweep(value):
    try:
        heights = [int(v) for v in value.split(",")]
        if not heights or any(h < 2 or h > 64 for h in heights):
            raise ValueError
        return heights
    except ValueError:
        raise argparse.ArgumentTypeError("sweep heights must be integers from 2 to 64 pixels")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--height", type=int, default=16)
    parser.add_argument("--elevation", type=float, default=45.,
                        help="Camera elevation above the horizon in degrees, 5..89. The near crown "
                             "spreads its foliage sideways and the distant lathe is a solid of "
                             "revolution, so the two agree on footprint at one elevation and not at "
                             "another; a canopy seen from a high camera is the steep end of that.")
    parser.add_argument("--height-sweep", type=height_sweep, default="64,48,32,24,16,12,8,6,4,3,2",
                        help="Comma-separated crown heights, 2..64 pixels (64-pixel frame)")
    parser.add_argument("--reuse-export", action="store_true", help="Reuse the named export; never validates current sources")
    args = parser.parse_args()
    if not 5. <= args.elevation <= 89.:
        raise SystemExit("elevation must be between 5 and 89 degrees")
    args.output.mkdir(parents=True, exist_ok=True)
    export = args.output / "meshes.json"
    if not args.reuse_export:
        command = ["godot", "--headless", "--path", str(ROOT / "godot"), "--log-file",
                   str(args.output.resolve() / "engine.log"), "--script", "res://tests/vegetation_lod_measure.gd",
                   "--", str(export.resolve())]
        run = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (args.output / "export.log").write_text(run.stdout)
        errors = run.stdout.count("SCRIPT ERROR")
        sys.stdout.write(f"export exit={run.returncode} SCRIPT ERROR={errors}\n")
        if run.returncode or errors or not export.exists():
            raise SystemExit(run.stdout)
    data = json.loads(export.read_text())
    sources = [ROOT / "godot/scripts/renderers/tree_species.gd", Path(__file__),
               ROOT / "godot/tests/vegetation_lod_measure.gd"]
    sources += list((ROOT / "godot/scripts/shaders").glob("vegetation*"))
    sources.append(ROOT / "godot/assets/textures/vegetation/foliage_atlas.dds")
    hashes = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
              for p in sources if p.suffix != ".uid"}
    meshes = [prepare(m) for m in data["meshes"]]
    for mesh in meshes:
        for surface in mesh["surfaces"]:
            if surface["shader"] == "vegetation_distant.gdshader":
                surface["coverage"] = surface.get("parameters", {}).get("crown_coverage", distant_coverage())
    rows = measure(meshes, args.height, atlas_mips(), args.elevation)
    summary = summarize(rows)
    deltas = transitions(summary)
    (args.output / "measurements.json").write_text(json.dumps(dict(height=args.height,
        elevation=args.elevation, source_sha256=hashes,
        reused_export=args.reuse_export, export_sha256=hashlib.sha256(export.read_bytes()).hexdigest(),
        build_us=data["build_us"], summary=summary, transitions=deltas, samples=rows), indent=2)+"\n")
    with (args.output / "summary.csv").open("w") as stream:
        writer = csv.DictWriter(stream, fieldnames=summary[0].keys())
        writer.writeheader()
        writer.writerows(summary)
    with (args.output / "transitions.csv").open("w") as stream:
        writer = csv.DictWriter(stream, fieldnames=deltas[0].keys())
        writer.writeheader()
        writer.writerows(deltas)
    # Spawn avoids inheriting NumPy's thread state; ordered futures keep CSV deterministic.
    with ProcessPoolExecutor(max_workers=min(4, len(set(args.height_sweep))),
                             mp_context=get_context("spawn")) as pool, (args.output / "sweep.csv").open("w") as stream:
        pending = {height: pool.submit(measure, meshes, height, atlas_mips(), args.elevation)
                   for height in dict.fromkeys(args.height_sweep) if height != args.height}
        fields = ["species", "lod", "mode", "height", "coverage", "footprint", "rgb", "covered_pixels", "bbox_pixels"]
        writer = csv.DictWriter(stream, fieldnames=fields)
        writer.writeheader()
        for height in args.height_sweep:
            samples = rows if height == args.height else pending[height].result()
            for row in summarize(samples):
                group = [r for r in samples if all(r[k] == row[k] for k in ("species", "lod", "mode"))]
                writer.writerow(dict(species=row["species"], lod=row["lod"], mode=row["mode"], height=height,
                                     coverage=row["coverage"], footprint=row["footprint"], rgb=row["rgb"],
                                     covered_pixels=float(np.mean([r["covered_pixels"] for r in group])),
                                     bbox_pixels=float(np.mean([r["bbox_pixels"] for r in group]))))
            stream.flush()
    for row in summary:
        if row["mode"] == "baseline":
            sys.stdout.write(json.dumps(row)+"\n")
    for row in deltas:
        sys.stdout.write(json.dumps(row)+"\n")


if __name__ == "__main__":
    main()
