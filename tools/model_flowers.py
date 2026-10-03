# SPDX-License-Identifier: GPL-2.0-only

"""Deterministic Finnish yard flowers: perennials, flowering shrubs and a summer bedding clump.

blender --background --factory-startup --python tools/model_flowers.py -- <out_dir>
Then: python3 tools/prepare_tree_models.py --flowers <out_dir>

Each form is a shrub as tools/model_landscape.py builds one: wood tubes on a bark tile and
alpha-tested cards on a four-cell atlas, exported with the same contract and checks. Two atlas
cells carry leaves and two carry blooms, so a bloom card shows a whole flower head. Metres, Z up
internally; the glTF exporter converts to Y up. Offline only: O(cards + texture pixels).
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import random
import sys
import time

import bpy
import numpy as np
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
import model_landscape as landscape
import model_trees as trees

TAU = math.tau
N = trees.SIZE


class Form:
    """One flower: size, card counts and atlas styles. Colours are sRGB."""
    def __init__(self, height, width, leaves, leaf_m, leaf_band, leaf_style, leaf, blooms,
                 bloom_m, bloom_style, bloom_colours, wood, dome_base=.45, lowest=.15, cross=True,
                 tilt=(0., .7)):
        self.height, self.width = height, width
        self.leaves, self.leaf_m, self.leaf_band = leaves, leaf_m, leaf_band
        self.leaf_style, self.leaf = leaf_style, leaf
        self.blooms, self.bloom_m, self.bloom_style = blooms, bloom_m, bloom_style
        self.bloom_colours, self.wood = bloom_colours, wood
        # Blooms sit on a half-ellipsoid from dome_base of the height up, no lower than `lowest`
        # of the way up it; a crossed bloom is two cards, so it reads from the side as well.
        self.dome_base, self.lowest, self.cross, self.tilt = dome_base, lowest, cross, tilt


# Peony (pioni), garden lupine (lupiini), garden phlox (syysleimu), smooth hydrangea
# 'Annabelle' (pallohortensia), rhododendron and a clump of summer bedding plants (kesäkukat:
# marigold and petunia). Sizes are a mature clump's; the planting places them near this size.
FLOWERS = {
    'peony': Form(.85, (.95, .90), 150, (.22, .32), (.12, .86), 'pinnate', (.20, .34, .11), 12,
                  (.17, .22), 'peony', [(.93, .45, .62), (.97, .82, .86)], (.30, .36, .16),
                  dome_base=.30, lowest=.40, tilt=(0., .55)),
    'lupine': Form(1.05, (.70, .64), 90, (.18, .26), (.05, .45), 'palmate', (.25, .40, .15), 9,
                   (.16, .40), 'spike', [(.38, .34, .80), (.86, .44, .66)], (.30, .40, .18)),
    'phlox': Form(.90, (.62, .58), 140, (.15, .22), (.12, .78), 'pinnate', (.24, .40, .13), 12,
                  (.20, .26), 'panicle', [(.90, .38, .66), (.95, .62, .82)], (.28, .38, .16),
                  dome_base=.55, lowest=.40),
    'hydrangea': Form(1.10, (1.30, 1.20), 170, (.22, .30), (.22, .85), 'broad', (.30, .46, .16),
                      16, (.22, .28), 'ball', [(.95, .96, .88), (.88, .93, .78)], (.42, .36, .27),
                      dome_base=.40, lowest=.10),
    'rhododendron': Form(1.30, (1.45, 1.30), 210, (.20, .28), (.10, .90), 'whorl',
                         (.13, .26, .09), 22, (.18, .24), 'truss',
                         [(.80, .24, .55), (.88, .48, .72)], (.36, .30, .24),
                         dome_base=.35, lowest=.05),
    'summer_flowers': Form(.32, (.55, .50), 60, (.10, .14), (.08, .65), 'round',
                           (.24, .40, .14), 26, (.09, .12), 'bedding',
                           [(.98, .55, .08), (.55, .22, .70)], (.30, .40, .18),
                           dome_base=.30, lowest=.50, cross=False, tilt=(0., .45)),
}


class Painter:
    """An RGBA atlas and its normal map, painted with shaded strokes and petals."""
    def __init__(self, base):
        self.rgba = np.zeros((N, N, 4), np.float32)
        self.rgba[..., :3] = base
        self.normal = np.ones((N, N, 4), np.float32)
        self.normal[..., :3] = (.5, .5, 1)

    def _window(self, lo, hi):
        lo = np.maximum(0, np.floor(np.array(lo) * N - 1).astype(int))
        hi = np.minimum(N, np.ceil(np.array(hi) * N + 1).astype(int))
        if np.any(hi <= lo):
            return None
        yy, xx = np.mgrid[lo[1]:hi[1], lo[0]:hi[0]]
        return (slice(lo[1], hi[1]), slice(lo[0], hi[0])), (xx + .5) / N, (yy + .5) / N

    def _put(self, window, mask, tint, shade, nx, ny):
        (rows, cols), _, _ = window
        region = self.rgba[rows, cols]
        region[mask, :3] = np.clip(np.array(tint) * shade[..., None], 0, 1)[mask]
        region[mask, 3] = 1
        nz = np.sqrt(np.maximum(.02, 1 - nx * nx - ny * ny))
        self.normal[rows, cols][mask, :3] = (np.stack((nx, ny, nz), -1) * .5 + .5)[mask]

    def blade(self, a, b, width, tint, stem=False):
        """A pointed leaf from a to b, or a constant-width stem."""
        a, b = np.array(a, float), np.array(b, float)
        delta = b - a
        length = np.linalg.norm(delta)
        if length < 1e-4:
            return
        axis = delta / length
        side = np.array((-axis[1], axis[0]))
        window = self._window(np.minimum(a, b) - width, np.maximum(a, b) + width)
        if window is None:
            return
        _, x, y = window
        t = ((x - a[0]) * axis[0] + (y - a[1]) * axis[1]) / length
        q = ((x - a[0]) * side[0] + (y - a[1]) * side[1]) / width
        tc = np.clip(t, 0, 1)
        profile = np.ones_like(t) if stem else np.sin(np.pi * tc) ** .72
        mask = (t >= 0) & (t <= 1) & (np.abs(q) <= profile)
        # A paler midrib and a darker edge read as a leaf rather than a flat splash.
        shade = .88 + .14 * tc - .14 * np.abs(q) + .10 * np.exp(-np.abs(q) * 22)
        self._put(window, mask, tint, shade, side[0] * q * .3, side[1] * q * .3)

    def petal(self, centre, rx, ry, angle, tint, relief=.30):
        """A filled ellipse with a domed normal, its x axis along `angle`."""
        c = np.array(centre, float)
        reach = max(rx, ry)
        window = self._window(c - reach, c + reach)
        if window is None:
            return
        _, x, y = window
        ca, sa = math.cos(angle), math.sin(angle)
        u = ((x - c[0]) * ca + (y - c[1]) * sa) / rx
        v = (-(x - c[0]) * sa + (y - c[1]) * ca) / ry
        d = u * u + v * v
        mask = d <= 1
        shade = 1 - relief * np.sqrt(np.minimum(d, 1)) + .06 * np.cos(u * 9)
        self._put(window, mask, tint, shade, (u * ca - v * sa) * .35, (u * sa + v * ca) * .35)

    def floret(self, centre, radius, petals, tint, eye, angle=0., relief=.30):
        """A flat flower seen from the front: `petals` petals round a small eye."""
        for k in range(petals):
            a = angle + k * TAU / petals
            p = np.array(centre) + radius * .55 * np.array((math.cos(a), math.sin(a)))
            self.petal(p, radius * .55, radius * .36, a, tint, relief)
        self.petal(centre, radius * .22, radius * .22, 0, eye, .2)


def cell_point(cell):
    """Maps a cell's own 0..1 square into the atlas, with the margin the trees keep."""
    origin = np.array((cell % 2 * .5, cell // 2 * .5))
    return lambda x, y: origin + np.array((.025 + .45 * x, .025 + .45 * y))


def jitter(rng, colour, spread):
    return np.clip(np.array(colour) * rng.uniform(1 - spread, 1 + spread), 0, 1)


def leaf_cell(paint, rng, cell, style, colour, stem_colour):
    """One leafy shoot hanging from the top of the cell, as the shrub cards attach."""
    point = cell_point(cell)
    size = .45
    if style == 'palmate':
        # Lupine: three hand-shaped leaves, nine narrow leaflets each round a petiole.
        for centre in ((.30, .62), (.70, .58), (.50, .30)):
            paint.blade(point(.50, .97), point(*centre), .0016, stem_colour, True)
            turn = rng.uniform(0, TAU)
            for k in range(9):
                a = turn + k * TAU / 9
                tip = np.array(centre) + .21 * np.array((math.cos(a), math.sin(a)))
                paint.blade(point(*centre), point(*tip), .022 * size * 2, jitter(rng, colour, .15))
        return
    if style == 'round':
        # Bedding plants: small rounded leaves heaped round short stems.
        for _ in range(26):
            c = (rng.uniform(.12, .88), rng.uniform(.10, .90))
            paint.petal(point(*c), rng.uniform(.045, .07) * size, rng.uniform(.03, .05) * size,
                        rng.uniform(0, TAU), jitter(rng, colour, .18), .35)
        return
    paint.blade(point(.50, .97), point(.49, .06), .0018, stem_colour, True)
    pairs, length, width = {'pinnate': (6, .30, .030), 'broad': (4, .30, .060),
                            'whorl': (3, .40, .040)}[style]
    for j in range(pairs):
        root = np.array((.495, .90 - j * .78 / pairs))
        for sign in ((-1, 1) if style != 'whorl' else (-1, -.35, .35, 1)):
            reach = length * rng.uniform(.80, 1.10)
            tip = root + np.array((sign * reach * .80, -reach * .55))
            tip = np.clip(tip, .04, .96)
            paint.blade(point(*root), point(*tip), width * rng.uniform(.85, 1.15) * size,
                        jitter(rng, colour, .18))
            if style == 'pinnate':
                # Peony and phlox leaves are divided: a narrower leaflet off each leaf.
                side = root + (tip - root) * .45
                end = np.clip(side + np.array((sign * .10, .10)), .04, .96)
                paint.blade(point(*side), point(*end), width * .7 * size, jitter(rng, colour, .18))


def bloom_cell(paint, rng, cell, style, colour, leaf):
    """One flower head filling its cell; a spike's base is at the bottom of the cell."""
    point = cell_point(cell)
    s = .45
    if style == 'peony':
        # A double bloom: rings of broad petals, paler and ruffled towards the centre.
        for ring, (count, r, size) in enumerate(((11, .30, .17), (9, .20, .15), (7, .11, .12),
                                                 (5, .04, .08))):
            for k in range(count):
                a = k * TAU / count + ring * .4 + rng.uniform(-.15, .15)
                p = np.array((.5, .5)) + r * np.array((math.cos(a), math.sin(a)))
                tint = jitter(rng, np.array(colour) * (.86 + .07 * ring), .06)
                paint.petal(point(*p), size * s, size * .75 * s, a, tint, .40)
        return
    if style == 'spike':
        # Lupine: staggered whorls of pea flowers up a tapering column, each a coloured keel
        # under a paler banner, then green buds at the tip. The card is about a third as wide
        # as it is long, so the cell paints each flower three times as wide as it will show.
        paint.blade(point(.50, .02), point(.50, .98), .012, (.30, .40, .18), True)
        rows = 17
        for j in range(rows):
            t = j / (rows - 1)
            half = .30 * (1 - t) ** .5 + .08
            y = .05 + .90 * t
            for k in (-1, 0, 1):
                p = (.5 + k * half * .62 + (j % 2 - .5) * .05, y + abs(k) * .012)
                if t > .78:
                    paint.petal(point(*p), .07 * s, .022 * s, 0, jitter(rng, leaf, .15), .3)
                    continue
                tint = jitter(rng, colour, .10)
                # Back whorls first and darker, so the front flowers overlap them.
                if k:
                    tint = tint * .82
                paint.petal(point(*p), .17 * s, .040 * s, k * .30, tint, .45)
                paint.petal(point(p[0], p[1] + .026), .11 * s, .024 * s, k * .30,
                            np.clip(tint * .45 + .50, 0, 1), .30)
        return
    if style == 'ball':
        # Hydrangea: a sphere of four-petalled florets, darker towards its underside.
        for _ in range(170):
            r = math.sqrt(rng.random()) * .42
            a = rng.uniform(0, TAU)
            p = np.array((.5 + r * math.cos(a), .52 + r * math.sin(a)))
            facing = math.sqrt(max(0., 1 - (r / .44) ** 2))
            light = .70 + .22 * facing + .10 * (p[1] - .5)
            paint.floret(point(*p), .055 * s, 4, jitter(rng, np.array(colour) * light, .04),
                         np.array(colour) * .8, rng.uniform(0, TAU), .25)
        return
    if style == 'panicle':
        # Phlox: a rounded head of five-petalled florets with darker eyes.
        for _ in range(55):
            r = math.sqrt(rng.random())
            a = rng.uniform(0, TAU)
            p = (.5 + .40 * r * math.cos(a), .58 + .32 * r * math.sin(a))
            paint.floret(point(*p), .07 * s, 5, jitter(rng, colour, .07),
                         np.array(colour) * .55, rng.uniform(0, TAU))
        return
    if style == 'truss':
        # Rhododendron: a domed truss of five-lobed funnels, each with a spotted upper lobe.
        for _ in range(15):
            r = math.sqrt(rng.random()) * .30
            a = rng.uniform(0, TAU)
            p = np.array((.5 + r * math.cos(a), .55 + r * math.sin(a) * .85))
            turn = rng.uniform(0, TAU)
            paint.floret(point(*p), .17 * s, 5, jitter(rng, colour, .07),
                         np.array(colour) * .65, turn, .35)
            paint.petal(point(*(p + .045 * np.array((math.cos(turn), math.sin(turn))))),
                        .018 * s, .018 * s, 0, np.array(colour) * .45, .1)
        return
    # Bedding: marigold pompoms or petunia trumpets among the leaves of the clump.
    for _ in range(12):
        c = (rng.uniform(.10, .90), rng.uniform(.10, .90))
        paint.petal(point(*c), .07 * s, .045 * s, rng.uniform(0, TAU), jitter(rng, leaf, .15), .35)
    for _ in range(9):
        p = np.array((rng.uniform(.18, .82), rng.uniform(.18, .82)))
        if cell == 2:
            for ring, scale in enumerate((1., .70, .40)):
                tint = jitter(rng, np.array(colour) * (1.05 - .12 * ring), .05)
                paint.floret(point(*p), .13 * s * scale, 9, tint, tint, rng.uniform(0, TAU), .3)
        else:
            paint.floret(point(*p), .14 * s, 5, jitter(rng, colour, .08), (.20, .07, .25),
                         rng.uniform(0, TAU), .3)


def atlas(name, form, out):
    """Two leaf cells and two bloom cells, the bark tile and the normal map."""
    rng = random.Random(sum(map(ord, name)) + 4021)
    paint = Painter(form.leaf)
    stem = np.array(form.wood) * 1.1
    for cell in (0, 1):
        leaf_cell(paint, rng, cell, form.leaf_style, form.leaf, stem)
    for cell, colour in zip((2, 3), form.bloom_colours):
        bloom_cell(paint, rng, cell, form.bloom_style, colour, form.leaf)
    trees.png(out / f'{name}_foliage.png', paint.rgba)
    trees.png(out / f'{name}_normal.png', paint.normal)
    bark = np.ones((256, 256, 4), np.float32)
    grain = landscape.field(np.random.default_rng(len(name) * 97 + 11), 256, 26)
    bark[..., :3] = np.array(form.wood) * (1 + .06 * grain)[..., None]
    trees.png(out / f'{name}_bark.png', bark)
    return paint.rgba[..., 3]


def dome_point(rng, form):
    """A point on the upper part of the form's bloom dome, and its outward direction."""
    wx, wy = form.width
    while True:
        d = Vector((rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(form.lowest, 1)))
        if .2 < d.length <= 1:
            d.normalize()
            break
    rise = form.height * (1 - form.dome_base)
    p = Vector((d.x * wx * .44, d.y * wy * .44, form.height * form.dome_base + d.z * rise))
    return p, d


def build_flower(name, form, out, alpha):
    """Stems to every bloom, a mound of leaf cards and the bloom cards over it."""
    tree = landscape.Shrub(name, out, alpha)
    tree.height = form.height
    rng = random.Random(5200 + list(FLOWERS).index(name))
    wx, wy = form.width
    h = form.height
    crown = Vector((0, 0, h * .35))
    woody = name in ('hydrangea', 'rhododendron')
    blooms = []
    for _ in range(form.blooms):
        if form.bloom_style == 'spike':
            a = rng.uniform(0, TAU)
            r = rng.uniform(.05, .32) * wx
            base = Vector((math.cos(a) * r, math.sin(a) * r, h * rng.uniform(.38, .52)))
            lean = Vector((math.cos(a) * .12, math.sin(a) * .12, 1)).normalized()
            blooms.append((base, lean))
        else:
            blooms.append(dome_point(rng, form))
    for p, d in blooms:
        root = Vector((d.x * wx * .06, d.y * wy * .06, 0))
        mid = root.lerp(p, .5) + Vector((d.x, d.y, 0)) * .04
        tree.tube([root, mid, p], (.016 if woody else .007) * rng.uniform(.8, 1.2), 3)
    lo, hi = form.leaf_band
    for i in range(form.leaves):
        t = rng.uniform(lo, hi)
        z = h * t
        # Leaves fill an ellipsoid mound: widest a third of the way up, narrower at the top.
        bulge = math.sqrt(max(.05, 1 - ((t - .35) / .75) ** 2))
        a = rng.uniform(0, TAU)
        r = math.sqrt(rng.random()) * bulge
        p = Vector((math.cos(a) * r * wx * .44, math.sin(a) * r * wy * .44, z))
        length = rng.uniform(*form.leaf_m)
        width = length * rng.uniform(.82, 1.10)
        direction = Vector((math.cos(a), math.sin(a), rng.uniform(-.3, .9))).normalized()
        axis = Vector((direction.x, direction.y, -.35 + direction.z * .35)).normalized()
        # Half the card either way of its centre, so no corner dips under the lawn.
        p.z = max((length + width) * .5 + .02, p.z)
        tree.card(p - axis * length / 2, axis, width, length, rng.uniform(0, TAU), i % 2, crown)
    for i, (p, d) in enumerate(blooms):
        cell = 2 + i % 2 if rng.random() < .7 else 2 + (i + 1) % 2
        if form.bloom_style == 'spike':
            width, length = form.bloom_m
            # A spike card hangs from its tip, so its base, at v = 0, meets the stem.
            top = p + d * length
            for turn in (0, math.pi / 2):
                tree.card(top, -d, width, length, turn + rng.uniform(0, .3), cell, crown)
            continue
        size = rng.uniform(*form.bloom_m)
        # A head faces outward from the dome, tipped up by `tilt`: near flat for a peony, which
        # opens to the sky, and towards the side for a hydrangea ball.
        lean = rng.uniform(*form.tilt)
        facing = Vector((d.x * math.sin(lean), d.y * math.sin(lean), 1)).normalized() \
            if form.tilt[1] < .6 else d.lerp(Vector((0, 0, 1)), 1 - lean).normalized()
        across = facing.cross(Vector((0, 0, 1)))
        if across.length < .05:
            across = Vector((1, 0, 0))
        across.normalize()
        axis = facing.cross(across).normalized()
        centre = p + facing * .02
        tree.card(centre - axis * size / 2, axis, size, size, rng.uniform(-.2, .2), cell, crown)
        if form.cross:
            # The second card stands across the first through the head's centre.
            tree.card(centre - facing * size / 2, facing, size, size, rng.uniform(-.2, .2), cell,
                      crown)
    obj = tree.finish()
    landscape.fit_mesh(obj, (wx, wy, h))
    bpy.data.objects.remove(tree.reduced_obj, do_unlink=True)
    return obj


def previews(objects, out):
    """One eye-level view of each flower and a bed of them beside a house wall."""
    for mat in bpy.data.materials:
        if mat.name.endswith('_foliage'):
            trees.preview_cutoff(mat)
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.cycles.device = 'CPU'
    scene.cycles.samples = 32
    scene.cycles.seed = 321
    scene.cycles.use_adaptive_sampling = False
    scene.cycles.use_denoising = False
    scene.cycles.transparent_max_bounces = 32
    scene.render.image_settings.file_format = 'PNG'
    scene.view_settings.view_transform = 'AgX'
    scene.world.use_nodes = True
    background = scene.world.node_tree.nodes.get('Background')
    background.inputs[0].default_value = (.65, .75, .9, 1)
    background.inputs[1].default_value = .65
    light = bpy.data.lights.new('Preview_sun', 'SUN')
    light.energy = 2.4
    light.angle = math.radians(4)
    sun = bpy.data.objects.new('Preview_sun', light)
    scene.collection.objects.link(sun)
    sun.location = (-20, -30, 40)
    trees.aim(sun, (0, 0, 0))
    landscape.box('Preview_ground', (0, 0, -.1), (60, 60, .2),
                  landscape.simple_material('Preview_lawn', (.075, .13, .035)))
    camera_data = bpy.data.cameras.new('Preview_camera')
    camera = bpy.data.objects.new('Preview_camera', camera_data)
    scene.collection.objects.link(camera)
    scene.camera = camera
    directory = out / 'previews'
    directory.mkdir(exist_ok=True)

    def render(name, width, height):
        scene.render.resolution_x, scene.render.resolution_y = width, height
        scene.render.filepath = str(directory / f'{name}.png')
        bpy.ops.render.render(write_still=True)
        landscape.strip_png_metadata(directory / f'{name}.png')

    for obj in objects:
        for other in objects:
            other.hide_render = other != obj
        reach = max(obj.dimensions) * 2.6 + 1
        camera.location = (reach * .38, -reach * .92, 1.2)
        trees.aim(camera, (0, 0, obj.dimensions.z * .45))
        camera_data.lens = 50
        render(obj.name + '_eye', 640, 640)
    for obj in objects:
        obj.hide_render = True
    wall = landscape.simple_material('Preview_red_wood', (.29, .035, .021))
    landscape.box('Preview_house', (0, 3.2, 1.6), (9, 4, 3.2), wall)
    rng = random.Random(77)
    sources = {o.name: o for o in objects}
    bed = [('rhododendron_0', -3.4, .7), ('hydrangea_0', 3.4, .7), ('peony_0', -1.8, .6),
           ('phlox_0', 1.6, .5), ('lupine_0', 0, .7), ('summer_flowers_0', -.8, -.4),
           ('summer_flowers_0', .7, -.5), ('peony_0', 2.4, -.5), ('lupine_0', -2.6, -.3)]
    for name, x, y in bed:
        o = bpy.data.objects.new('Preview_' + name, sources[name].data)
        scene.collection.objects.link(o)
        o.location = (x, y, 0)
        o.rotation_euler.z = rng.uniform(0, TAU)
    camera.location = (2.5, -8, 2.2)
    trees.aim(camera, (0, .5, .6))
    camera_data.lens = 32
    render('flower_bed_eye', 1200, 700)
    camera.location = (6, -14, 14)
    trees.aim(camera, (0, .5, 0))
    camera_data.lens = 45
    render('flower_bed_above', 1200, 700)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('out_dir', type=Path)
    parser.add_argument('--skip-previews', action='store_true')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    start = time.perf_counter()
    out = args.out_dir.resolve()
    out.mkdir(parents=True, exist_ok=True)
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)
    objects, stats = [], {}
    for name, form in FLOWERS.items():
        obj = build_flower(name, form, out, atlas(name, form, out))
        s = landscape.measure(obj)
        landscape.export_glb(obj, out)
        s['glb_sha256'] = landscape.validate(out / f'{obj.name}.glb', s)
        stats[obj.name] = s
        objects.append(obj)
    if not args.skip_previews:
        previews(objects, out)
    metrics = {'blender': bpy.app.version_string,
               'script_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
               'models': stats, 'total_seconds': round(time.perf_counter() - start, 3)}
    (out / 'flower_metrics.json').write_text(json.dumps(metrics, indent=2) + '\n')
    print('FLOWER_BUILD ' + json.dumps({'models': len(stats)}), flush=True)


if __name__ == '__main__':
    main()
