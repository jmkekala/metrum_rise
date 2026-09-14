#!/usr/bin/env python3
"""Bake four foliage silhouettes with Blender's orthographic CPU renderer.

Usage: blender --background --python tools/bake_foliage_atlas.py -- <output_png>

Top row: generic broadleaf (left), birch (right). Bottom row: conifer sprays. A same-stem DDS carries
coverage-corrected mipmaps because PNG cannot store a mip chain. A JSON companion
records measured alpha coverage (> 0.4), before and after correction, per cell.
RGB is white throughout (including transparent pixels): the exact flood extension
of the white opaque mask; species colour remains owned by the vertex palette.

Determinism on the same Blender/NumPy build: isolated factory scene, explicit
per-cell random.Random seeds, fixed Cycles seed/samples, CPU with one thread,
no adaptive sampling/denoising, fixed camera and emission material. PNG uses a
fixed stdlib encoder without timestamps or Blender metadata; DDS and JSON also
have fixed ordering. No unseeded random source is used.
"""

import argparse
import json
import math
from pathlib import Path
import random
import struct
import sys
import tempfile
import zlib

import bpy
import numpy as np

# 256 px per cell. The shader samples only alpha, so the three constant-white RGB bytes
# per texel are padding; at 1024 the uncompressed atlas cost 5.33 MB of VRAM and made the
# scatter partly fill-bound for the first time, measured at +0.43 ms and +0.72 ms at 1.5x
# render scale. This is still four times the per-cluster resolution of the 128 px mask it
# replaced, across four distinct clusters instead of one.
SIZE = 512
THRESHOLD = 0.4


def polygon(vertices, faces, points):
    start = len(vertices)
    vertices.extend((x, y, 0.0) for x, y in points)
    faces.append(tuple(range(start, len(vertices))))


def blade(vertices, faces, origin, angle, length, width, needle=False):
    """Pointed leaf or narrow lanceolate needle, with a shared planar root."""
    ux, uy = math.cos(angle), math.sin(angle)
    profile = [(0, 0), (0.28, -0.65), (0.58, -1), (0.83, -0.6),
               (1, 0), (0.83, 0.6), (0.58, 1), (0.28, 0.65)]
    if needle:
        profile = [(0, 0), (0.12, -1), (0.78, -0.65), (1, 0),
                   (0.78, 0.65), (0.12, 1)]
    polygon(vertices, faces, [(origin[0] + ux * t * length - uy * w * width,
                              origin[1] + uy * t * length + ux * w * width)
                             for t, w in profile])


def birch_leaf(vertices, faces, origin, angle, length, width):
    """Small ovate leaf with a pointed apex and alternating teeth on both margins."""
    ux, uy = math.cos(angle), math.sin(angle)
    profile = [(0, 0)]
    for side in (-1, 1):
        steps = range(1, 12) if side == -1 else range(11, 0, -1)
        for step in steps:
            t = step / 12
            margin = math.sin(math.pi * t) ** 0.7 * (1.0 if step % 2 else 0.80)
            profile.append((t, side * margin))
        if side == -1:
            profile.append((1, 0))
    polygon(vertices, faces, [(origin[0] + ux * t * length - uy * w * width,
                              origin[1] + uy * t * length + ux * w * width)
                             for t, w in profile])


def fit_cell(vertices, start, cx, cy, margin=0.02):
    """Scale one cluster about its cell centre until it fills the cell.

    A card quad already stretches the square atlas cell to its own aspect, so the
    empty margin around a cluster is wasted card area rather than a shape the
    renderer preserves. Scaling about the centre keeps the cluster aligned with the
    branch tip the card hangs from.
    """
    xs = [vertices[i][0] for i in range(start, len(vertices))]
    ys = [vertices[i][1] for i in range(start, len(vertices))]
    scale_x = (0.5 - margin) / max(max(xs) - cx, cx - min(xs))
    scale_y = (0.5 - margin) / max(max(ys) - cy, cy - min(ys))
    for index in range(start, len(vertices)):
        x, y, z = vertices[index]
        vertices[index] = (cx + (x - cx) * scale_x, cy + (y - cy) * scale_y, z)


def cluster(vertices, faces, column, conifer):
    start = len(vertices)
    rng = random.Random(81371 + column * 997 + int(conifer) * 7919)
    # Blender's image origin is bottom-left; exported PNG is flipped to top-left.
    cx, cy = column + 0.5, 0.5 if conifer else 1.5
    if not conifer and column == 1:
        # Fine hanging twigs connect smaller, rounder leaves into an open cluster.
        for twig in range(7):
            angle = -math.pi / 2 + (twig - 3) * 0.18
            origin = (cx + (twig - 3) * 0.040, cy + 0.28 - abs(twig - 3) * 0.035)
            length = rng.uniform(0.40, 0.50)
            blade(vertices, faces, origin, angle, length, 0.003, True)
            for step in range(8):
                t = (step + 1) / 9
                at = (origin[0] + math.cos(angle) * length * t,
                      origin[1] + math.sin(angle) * length * t)
                for side in (-1, 1):
                    birch_leaf(vertices, faces, at, angle + side * rng.uniform(0.8, 1.3),
                               rng.uniform(0.062, 0.099), rng.uniform(0.036, 0.055))
    elif not conifer:
        for i in range(78):
            angle = i * 2.399963229728653 + column * 0.4
            reach = math.sqrt(rng.random()) * 0.28
            origin = (cx + math.cos(angle) * reach, cy + math.sin(angle) * reach)
            blade(vertices, faces, origin, angle + rng.uniform(-0.8, 0.8),
                  rng.uniform(0.09, 0.16), rng.uniform(0.049, 0.083))
    else:
        # Five ascending twigs, each carrying paired, individually tapered needles.
        for twig in range(7):
            angle = math.pi / 2 + (twig - 3) * (0.30 if column == 0 else 0.36)
            origin = (cx, cy - 0.28 + abs(twig - 3) * 0.030)
            length = 0.64 - abs(twig - 3) * 0.048
            blade(vertices, faces, origin, angle, length, 0.009, True)
            for step in range(26):
                t = (step + 1) / 28
                at = (origin[0] + math.cos(angle) * length * t,
                      origin[1] + math.sin(angle) * length * t)
                for side in (-1, 1):
                    blade(vertices, faces, at, angle + side * rng.uniform(0.65, 1.2),
                          rng.uniform(0.09, 0.15) * (1 - t * 0.45),
                          rng.uniform(0.0148, 0.0246), True)
    fit_cell(vertices, start, cx, cy)


def render_alpha():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.cycles.device = 'CPU'
    scene.cycles.seed = 137
    scene.cycles.samples = 32
    scene.cycles.use_adaptive_sampling = False
    scene.cycles.use_denoising = False
    scene.render.threads_mode = 'FIXED'
    scene.render.threads = 1
    scene.render.resolution_x = scene.render.resolution_y = SIZE
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = 'PNG'
    scene.render.image_settings.color_mode = 'RGBA'
    scene.render.image_settings.color_depth = '8'
    scene.view_settings.view_transform = 'Standard'
    scene.view_settings.look = 'None'
    scene.view_settings.exposure = 0
    scene.view_settings.gamma = 1
    material = bpy.data.materials.new('White mask')
    material.use_nodes = True
    nodes = material.node_tree.nodes
    nodes.clear()
    emission = nodes.new('ShaderNodeEmission')
    emission.inputs['Color'].default_value = (1, 1, 1, 1)
    output = nodes.new('ShaderNodeOutputMaterial')
    material.node_tree.links.new(emission.outputs[0], output.inputs['Surface'])
    vertices, faces = [], []
    for conifer in (False, True):
        for column in range(2):
            cluster(vertices, faces, column, conifer)
    mesh = bpy.data.meshes.new('Foliage source')
    mesh.from_pydata(vertices, [], faces)
    mesh.materials.append(material)
    scene.collection.objects.link(bpy.data.objects.new('Foliage source', mesh))
    camera = bpy.data.cameras.new('Atlas camera')
    camera.type = 'ORTHO'
    camera.ortho_scale = 2
    camera_object = bpy.data.objects.new('Atlas camera', camera)
    scene.collection.objects.link(camera_object)
    camera_object.location = (1, 1, 3)
    scene.camera = camera_object
    with tempfile.TemporaryDirectory(prefix='foliage-render-') as directory:
        scene.render.filepath = str(Path(directory) / 'render.png')
        bpy.ops.render.render(write_still=True)
        rendered = bpy.data.images.load(scene.render.filepath)
        pixels = np.empty(SIZE * SIZE * 4, dtype=np.float32)
        rendered.pixels.foreach_get(pixels)
        return np.rint(pixels.reshape(SIZE, SIZE, 4)[::-1, :, 3] * 255).astype(np.uint8)


def rgba(alpha):
    # All opaque RGB is white, so its flood fill has the constant solution white.
    pixels = np.full((*alpha.shape, 4), 255, dtype=np.uint8)
    pixels[:, :, 3] = alpha
    return pixels.tobytes()


def write_png(path, alpha):
    def chunk(tag, data):
        return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data))
    rows = np.frombuffer(rgba(alpha), dtype=np.uint8).reshape(SIZE, SIZE * 4)
    scanlines = b''.join(b'\0' + row.tobytes() for row in rows)
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>2I5B', SIZE, SIZE, 8, 6, 0, 0, 0))
                     + chunk(b'IDAT', zlib.compress(scanlines, 9)) + chunk(b'IEND', b''))


def coverage(alpha):
    return float(np.count_nonzero(alpha > THRESHOLD * 255) / alpha.size)


def correct(alpha, target):
    # Evaluate all distinct crossing scales and take the smallest representable
    # coverage that still reaches the target. Rounding down instead was measured to
    # erase a cell outright once a mip holds one texel per cell: the closest
    # representable coverage to 0.44 is then 0, the card disappears at about 800 m,
    # and the conifer crown lost 46% of its fill between 64 and 16 apparent pixels.
    # A cluster whose cell no longer resolves must fill, not vanish. On the larger
    # mips the representable ratios are dense, so this stays within a texel of the
    # closest choice. Quantization is included in the decision.
    values, counts = np.unique(alpha, return_counts=True)
    nonzero = values > 0
    values, counts = values[nonzero], counts[nonzero]
    if not len(values) or target <= 0:
        return np.zeros_like(alpha, dtype=np.uint8)
    scales = 102.50001 / values
    ratios = np.cumsum(counts[::-1])[::-1] / alpha.size
    reaching = [(float(ratio), float(scale)) for ratio, scale in zip(ratios, scales)
                if float(ratio) >= target]
    scale = min(reaching)[1] if reaching else float(scales[0])
    return np.clip(np.rint(alpha * scale), 0, 255).astype(np.uint8)


def mip_chain(alpha):
    targets = [coverage(alpha[y:y + SIZE // 2, x:x + SIZE // 2])
               for y in (0, SIZE // 2) for x in (0, SIZE // 2)]
    raw = alpha.astype(np.float64)
    levels, report = [], []
    while True:
        size = raw.shape[0]
        corrected = raw.astype(np.uint8).copy()
        cells = []
        if size > 1:
            half = size // 2
            for index, (y, x) in enumerate(( (y, x) for y in (0, half) for x in (0, half) )):
                cell = raw[y:y + half, x:x + half]
                result = cell.astype(np.uint8) if size == SIZE else correct(cell, targets[index])
                corrected[y:y + half, x:x + half] = result
                cells.append({'raw': coverage(np.rint(cell)), 'corrected': coverage(result)})
        else:
            corrected = correct(raw, sum(targets) / 4)
        levels.append(corrected)
        report.append({'level': len(levels) - 1, 'size': size,
                       'raw': coverage(np.rint(raw)), 'corrected': coverage(corrected), 'cells': cells})
        if size == 1:
            break
        raw = raw.reshape(size // 2, 2, size // 2, 2).mean(axis=(1, 3))
    return levels, report


def write_dds(path, levels):
    # Legacy uncompressed RGBA8 DDS with an explicit complete mip chain.
    header = [124, 0x2100F, SIZE, SIZE, SIZE * 4, 0, len(levels)] + [0] * 11
    header += [32, 0x41, 0, 32, 0xFF, 0xFF00, 0xFF0000, 0xFF000000]
    header += [0x401008, 0, 0, 0, 0]
    path.write_bytes(b'DDS ' + struct.pack('<31I', *header) + b''.join(rgba(level) for level in levels))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output_png', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else [])
    path = args.output_png.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    alpha = render_alpha()
    write_png(path, alpha)
    levels, report = mip_chain(alpha)
    write_dds(path.with_suffix('.dds'), levels)
    path.with_suffix('.coverage.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
