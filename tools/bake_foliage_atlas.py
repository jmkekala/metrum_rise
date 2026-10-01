#!/usr/bin/env python3
"""Bake four foliage silhouettes with Blender's orthographic CPU renderer.

Usage: blender --background --python tools/bake_foliage_atlas.py -- <output_png>

Top row: generic broadleaf (left), birch (right). Bottom row: conifer sprays. A same-stem DDS carries
coverage-corrected mipmaps because PNG cannot store a mip chain. A JSON companion
records measured alpha coverage (> 0.4), before and after correction, per cell.
RGB carries a grey leaf shade from a second render, flooded outward into transparent
texels so filtering never pulls in black. Each cell is scaled to the same mean linear
shade over its opaque texels, LEAF_SHADE_MEAN in the card shader, which divides it back
out: the pattern varies the colour but not its mean, so the distant levels and their bake
stay calibrated. Species colour remains owned by the vertex palette.

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
import re
import struct
import sys
import tempfile
import zlib

import numpy as np

# 256 px per cell. At 1024 the uncompressed atlas cost 5.33 MB of VRAM and made the
# scatter partly fill-bound for the first time, measured at +0.43 ms and +0.72 ms at 1.5x
# render scale. This is still four times the per-cluster resolution of the 128 px mask it
# replaced, across four distinct clusters instead of one.
SIZE = 512
THRESHOLD = 0.4


# Each polygon carries a shade per corner. A leaf drawn later lies above the earlier ones
# at a slightly greater height, and the shade pass reads the topmost.
SHADES = []


def polygon(vertices, faces, points, shades=None):
    start = len(vertices)
    z = len(faces) * 1e-5
    vertices.extend((x, y, z) for x, y in points)
    faces.append(tuple(range(start, len(vertices))))
    SHADES.extend(shades if shades is not None else [1.0] * len(points))


def leaf_shades(profile, level):
    """A leaf brightens from its base to its tip, around its own level."""
    return [level * (0.84 + 0.16 * t) for t, _ in profile]


def blade(vertices, faces, origin, angle, length, width, needle=False, level=1.0):
    """Pointed leaf or narrow lanceolate needle, with a shared planar root."""
    ux, uy = math.cos(angle), math.sin(angle)
    profile = [(0, 0), (0.28, -0.65), (0.58, -1), (0.83, -0.6),
               (1, 0), (0.83, 0.6), (0.58, 1), (0.28, 0.65)]
    if needle:
        profile = [(0, 0), (0.12, -1), (0.78, -0.65), (1, 0),
                   (0.78, 0.65), (0.12, 1)]
    polygon(vertices, faces, [(origin[0] + ux * t * length - uy * w * width,
                              origin[1] + uy * t * length + ux * w * width)
                             for t, w in profile], leaf_shades(profile, level))


def birch_leaf(vertices, faces, origin, angle, length, width, level=1.0):
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
                             for t, w in profile], leaf_shades(profile, level))


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
    # A separate stream, so the shade leaves the geometry and the alpha pass untouched.
    shade = random.Random(52379 + column * 997 + int(conifer) * 7919)

    def level(rank):
        # Leaves lower in the stack sit in the shade of those drawn over them.
        return shade.uniform(0.72, 1.0) * (0.60 + 0.40 * rank)
    # Blender's image origin is bottom-left; exported PNG is flipped to top-left.
    cx, cy = column + 0.5, 0.5 if conifer else 1.5

    def stem(points, width, value):
        """A twig as a chain of narrow blades, one per segment."""
        for (x0, y0), (x1, y1) in zip(points, points[1:]):
            blade(vertices, faces, (x0, y0), math.atan2(y1 - y0, x1 - x0),
                  math.hypot(x1 - x0, y1 - y0) * 1.15, width, True, value)

    # A cell is drawn at the size its card is shown: about 2.5 m for a broadleaf spray, 1.5 m
    # for a spruce frond and 0.7 m for a pine brush. Leaves are drawn a little above their real
    # size, which at 256 px per cell is the smallest that still reads as separate leaves.
    if not conifer and column == 1:
        # Birch: a curtain of long pendulous twigs hanging from arching shoots, with small
        # toothed leaves alternating along them and sky between the twigs.
        tops = []
        for twig in range(19):
            x0 = cx - 0.43 + twig * 0.048 + rng.uniform(-0.012, 0.012)
            y0 = cy + 0.44 - (x0 - cx) ** 2 * 1.4 + rng.uniform(-0.02, 0.02)
            tops.append((x0, y0))
        stem(tops, 0.004, 0.30)
        for twig, (x0, y0) in enumerate(tops):
            length = rng.uniform(0.55, 0.85) * (1.0 - abs(x0 - cx) * 0.7)
            sway = rng.uniform(-0.10, 0.10)
            points = [(x0 + sway * (i / 10) ** 2, y0 - length * i / 10) for i in range(11)]
            stem(points, 0.0022, 0.30)
            count = int(length / 0.021)
            for step in range(count):
                t = (step + 0.7) / count
                at = (x0 + sway * t * t, y0 - length * t)
                side = 1 if step % 2 else -1
                birch_leaf(vertices, faces, at, -math.pi / 2 + side * rng.uniform(0.45, 1.0),
                           rng.uniform(0.030, 0.040), rng.uniform(0.0085, 0.0115),
                           level((twig + t) / 20))
    elif not conifer:
        # Aspen and other broadleaves: twigs fan up from the base, and round leaves on long
        # stalks crowd toward the twig ends in open rosettes.
        for twig in range(13):
            angle = math.pi / 2 + (twig - 6) * 0.18 + rng.uniform(-0.05, 0.05)
            origin = (cx + rng.uniform(-0.03, 0.03), cy - 0.46)
            length = rng.uniform(0.70, 0.92) * (1.0 - abs(twig - 6) * 0.035)
            ux, uy = math.cos(angle), math.sin(angle)
            stem([origin, (origin[0] + ux * length, origin[1] + uy * length)], 0.004, 0.30)
            count = int(length / 0.036)
            for step in range(count):
                t = 0.30 + 0.70 * (step + 1) / count
                at = (origin[0] + ux * length * t, origin[1] + uy * length * t)
                for side in (-1, 1):
                    stalk = angle + side * rng.uniform(0.5, 1.1)
                    reach = rng.uniform(0.012, 0.022)
                    tip = (at[0] + math.cos(stalk) * reach, at[1] + math.sin(stalk) * reach)
                    stem([at, tip], 0.0012, 0.30)
                    size = rng.uniform(0.034, 0.048)
                    blade(vertices, faces, tip, stalk + rng.uniform(-0.5, 0.5), size, size * 0.42,
                          False, level((twig + t) / 14))
    elif column == 0:
        # Spruce: a flat frond, an axis with alternating side shoots, each fringed on both sides
        # with short needles. From outside the tree this fringe is what a spruce shows.
        axis = [(cx + 0.03 * math.sin(i / 12 * math.pi), cy - 0.47 + 0.94 * i / 12) for i in range(13)]
        stem(axis, 0.006, 0.35)
        shoots = [(axis[0], math.pi / 2, 0.94, 0.0)]
        for index in range(17):
            t = 0.06 + 0.88 * index / 16
            side = 1 if index % 2 else -1
            at = (cx + 0.03 * math.sin(t * math.pi), cy - 0.47 + 0.94 * t)
            angle = math.pi / 2 + side * rng.uniform(0.85, 1.05)
            shoots.append((at, angle, (0.46 * (1.0 - 0.75 * t) + 0.05) * rng.uniform(0.85, 1.1), t))
        # Each side shoot carries its own alternating shoots, which close the frond into a mass.
        for at, angle, length, t0 in list(shoots[1:]):
            ux, uy = math.cos(angle), math.sin(angle)
            for index in range(int(length / 0.07)):
                t = (index + 0.6) / int(length / 0.07)
                side = 1 if index % 2 else -1
                shoots.append(((at[0] + ux * length * t, at[1] + uy * length * t),
                               angle + side * rng.uniform(0.7, 0.95), length * 0.32 * (1.0 - 0.5 * t), t0))
        for number, (at, angle, length, t0) in enumerate(shoots):
            ux, uy = math.cos(angle), math.sin(angle)
            if number:
                stem([at, (at[0] + ux * length, at[1] + uy * length)], 0.0035, 0.35)
            count = int(length / 0.009)
            for step in range(count):
                t = (step + 0.5) / count
                base = (at[0] + ux * length * t, at[1] + uy * length * t)
                for side in (-1, 1):
                    # Young needles at the shoot tip are lighter than the older ones inside.
                    blade(vertices, faces, base, angle + side * rng.uniform(0.7, 1.1),
                          rng.uniform(0.020, 0.030) * (1.0 - 0.35 * t), rng.uniform(0.0035, 0.005),
                          True, level(0.30 + 0.70 * t))
    else:
        # Pine: bare twigs end in brushes of long needles, with sky between the brushes.
        for twig in range(9):
            angle = math.pi / 2 + (twig - 4) * 0.25 + rng.uniform(-0.07, 0.07)
            origin = (cx + rng.uniform(-0.04, 0.04), cy - 0.40)
            length = rng.uniform(0.45, 0.62)
            ux, uy = math.cos(angle), math.sin(angle)
            stem([origin, (origin[0] + ux * length, origin[1] + uy * length)], 0.007, 0.35)
            for needle in range(70):
                t = 0.58 + 0.42 * (needle + 0.5) / 70
                base = (origin[0] + ux * length * t, origin[1] + uy * length * t)
                spread = angle + (rng.uniform(-1.0, 1.0) * 1.25) * (0.5 + 0.5 * t)
                blade(vertices, faces, base, spread, rng.uniform(0.07, 0.11), rng.uniform(0.0035, 0.005),
                      True, level(0.35 + 0.65 * (needle / 69)))
    fit_cell(vertices, start, cx, cy)


def render(shaded):
    """Alpha mask of the white clusters, or the linear leaf shade premultiplied by it."""
    import bpy

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
    scene.render.image_settings.file_format = 'OPEN_EXR' if shaded else 'PNG'
    scene.render.image_settings.color_mode = 'RGBA'
    scene.render.image_settings.color_depth = '32' if shaded else '8'
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
    SHADES.clear()
    vertices, faces = [], []
    for conifer in (False, True):
        for column in range(2):
            cluster(vertices, faces, column, conifer)
    mesh = bpy.data.meshes.new('Foliage source')
    mesh.from_pydata(vertices, [], faces)
    if shaded:
        attribute = mesh.color_attributes.new('shade', 'FLOAT_COLOR', 'CORNER')
        for loop in mesh.loops:
            value = SHADES[loop.vertex_index]
            attribute.data[loop.index].color = (value, value, value, 1.0)
        source = nodes.new('ShaderNodeAttribute')
        source.attribute_name = 'shade'
        material.node_tree.links.new(source.outputs['Color'], emission.inputs['Color'])
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
        scene.render.filepath = str(Path(directory) / ('render.exr' if shaded else 'render.png'))
        bpy.ops.render.render(write_still=True)
        rendered = bpy.data.images.load(scene.render.filepath)
        pixels = np.empty(SIZE * SIZE * 4, dtype=np.float32)
        rendered.pixels.foreach_get(pixels)
        pixels = pixels.reshape(SIZE, SIZE, 4)[::-1]
        if shaded:
            return pixels[:, :, 0].astype(np.float64)
        return np.rint(pixels[:, :, 3] * 255).astype(np.uint8)


def leaf_shade_mean():
    source = (Path(__file__).resolve().parents[1]
              / 'godot/scripts/shaders/vegetation_wind_cards.gdshader').read_text()
    return float(re.search(r'const float LEAF_SHADE_MEAN\s*=\s*([0-9.]+);', source)[1])


def flood(shade, known):
    """Extend known shade outward, one texel ring per step, then fill the rest with the mean."""
    shade, known = np.where(known, shade, 0.0), known.astype(np.float64)
    for _ in range(24):
        total = sum(np.roll(np.roll(shade * known, dy, 0), dx, 1)
                    for dy in (-1, 0, 1) for dx in (-1, 0, 1))
        count = sum(np.roll(np.roll(known, dy, 0), dx, 1) for dy in (-1, 0, 1) for dx in (-1, 0, 1))
        grow = (known == 0) & (count > 0)
        shade = np.where(grow, total / np.maximum(count, 1), shade)
        known = np.where(grow, 1.0, known)
    mean = shade[known > 0].mean()
    return np.where(known > 0, shade, mean)


def shade_cells(premultiplied, alpha):
    """Straight leaf shade per texel, each cell scaled to LEAF_SHADE_MEAN over its opaque texels."""
    target = leaf_shade_mean()
    straight = premultiplied / np.maximum(alpha / 255.0, 1e-6)
    shade = np.zeros_like(straight)
    half = SIZE // 2
    for y in (0, half):
        for x in (0, half):
            cell = straight[y:y + half, x:x + half]
            cover = alpha[y:y + half, x:x + half]
            opaque = cover >= THRESHOLD * 255
            filled = flood(cell, cover >= 0.5 * 255)
            shade[y:y + half, x:x + half] = np.clip(filled * target / filled[opaque].mean(), 0.0, 1.0)
    return shade


def srgb_bytes(linear):
    encoded = np.where(linear <= 0.0031308, linear * 12.92, 1.055 * np.power(linear, 1 / 2.4) - 0.055)
    return np.clip(np.rint(encoded * 255), 0, 255).astype(np.uint8)


def rgba(alpha, shade=None):
    pixels = np.full((*alpha.shape, 4), 255, dtype=np.uint8)
    if shade is not None:
        pixels[:, :, :3] = srgb_bytes(shade)[:, :, None]
    pixels[:, :, 3] = alpha
    return pixels


def write_png(path, pixels):
    def chunk(tag, data):
        return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data))
    rows = pixels.reshape(SIZE, SIZE * 4)
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


def shade_chain(shade, alpha, count):
    """Alpha-weighted linear box filter, so a mip keeps the shade of the leaves it covers."""
    levels, weight = [shade], alpha.astype(np.float64) + 1e-3
    while len(levels) < count:
        size = shade.shape[0] // 2
        total = (shade * weight).reshape(size, 2, size, 2).sum(axis=(1, 3))
        weight = weight.reshape(size, 2, size, 2).sum(axis=(1, 3))
        shade = total / weight
        levels.append(shade)
    return levels


# The sprays are open at full size, but from farther away the gaps between their leaves are
# closed by the leaves behind them, which a single card cannot show. The coverage target
# therefore climbs from each cell's own coverage at the second level to FAR_COVERAGE over
# DENSIFY_LEVELS levels: a crown reads as open close up and as closed from the game camera.
FAR_COVERAGE = 0.55
DENSIFY_LEVELS = 4


def mip_target(base, level):
    return base + (max(FAR_COVERAGE, base) - base) * min(max(level - 1, 0) / DENSIFY_LEVELS, 1.0)


def mip_chain(alpha):
    bases = [coverage(alpha[y:y + SIZE // 2, x:x + SIZE // 2])
             for y in (0, SIZE // 2) for x in (0, SIZE // 2)]
    raw = alpha.astype(np.float64)
    levels, report = [], []
    while True:
        size = raw.shape[0]
        targets = [mip_target(base, len(levels)) for base in bases]
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
    height, width = levels[0].shape[:2]
    header = [124, 0x2100F, height, width, width * 4, 0, len(levels)] + [0] * 11
    header += [32, 0x41, 0, 32, 0xFF, 0xFF00, 0xFF0000, 0xFF000000]
    header += [0x401008, 0, 0, 0, 0]
    path.write_bytes(b'DDS ' + struct.pack('<31I', *header) + b''.join(
        rgba(level).tobytes() if level.ndim == 2 else level.astype(np.uint8).tobytes() for level in levels))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output_png', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else [])
    path = args.output_png.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    alpha = render(False)
    shade = shade_cells(render(True), alpha)
    write_png(path, rgba(alpha, shade))
    levels, report = mip_chain(alpha)
    shades = shade_chain(shade, alpha, len(levels))
    write_dds(path.with_suffix('.dds'), [rgba(a, s) for a, s in zip(levels, shades)])
    path.with_suffix('.coverage.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
