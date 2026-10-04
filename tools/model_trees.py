#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-only
"""Author deterministic Finnish trees, textures, GLBs and CPU Cycles previews.

blender --background --factory-startup --python tools/model_trees.py -- <out_dir>
Only Blender's bundled Python/NumPy are required. Metres, Z up internally;
Blender's glTF exporter converts to Y up. No simulation/runtime work is added.
Optional --round 1/2/3 labels the review pass (default: 3). Archived scripts preserve earlier geometry.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import random
import struct
import sys
import time
import zlib

import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from bpy_extras.object_utils import world_to_camera_view

TAU = math.tau
UP = Vector((0, 0, 1))
SIZE = 1024
# Measured near meshes, including the current generator's alpha-bounds crop.
# Source SHA256: 8f3d38ddd7155dc4c7f927b0e341f3bb366ba6863136358f06c2909e21b4e7ee
# Reduced level, drawn past the game's near-detail distance and baked into the impostor: only
# wood at least this thick, and one card in REDUCED_CARD_STRIDE, grown so the kept cards cover
# about the same crown area as all of them.
REDUCED_MIN_RADIUS = .06
REDUCED_CARD_STRIDE = 5
REDUCED_CARD_SCALE = math.sqrt(REDUCED_CARD_STRIDE)
BASELINE = {'spruce': (1085.442069, 910.410335), 'birch': (980.808011, 964.137400),
            'pine': (195.257490, 179.723204), 'aspen': (838.106413, 814.484856)}


def png(path, pixels):
    """Write deterministic RGBA8 PNG, without timestamps or colour conversion."""
    data = np.clip(np.rint(pixels * 255), 0, 255).astype(np.uint8)
    h, w = data.shape[:2]
    def chunk(kind, payload):
        return struct.pack('>I', len(payload)) + kind + payload + struct.pack('>I', zlib.crc32(kind + payload))
    rows = b''.join(b'\0' + row.tobytes() for row in data[::-1])
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>2I5B', w, h, 8, 6, 0, 0, 0))
                     + chunk(b'IDAT', zlib.compress(rows, 9)) + chunk(b'IEND', b''))


def atlas(species, out):
    """Rasterize connected shoots, individual serrated leaves/needles and relief."""
    rng = random.Random({'spruce': 817, 'birch': 611, 'pine': 912, 'aspen': 423}[species])
    rgb = np.zeros((SIZE, SIZE, 4), np.float32)
    rgb[..., :3] = (.16, .23, .065) if species == 'spruce' else (.32, .40, .10)
    normals = np.zeros_like(rgb)
    normals[:] = (.5, .5, 1, 1)
    def blade(a, b, width, color, needle=False, stem=False):
        a, b = np.array(a) * SIZE, np.array(b) * SIZE
        delta = b - a
        length = np.linalg.norm(delta)
        axis = delta / length
        side = np.array([-axis[1], axis[0]])
        w = width * SIZE
        lo = np.maximum(np.floor(np.minimum(a, b) - w - 1).astype(int), 0)
        hi = np.minimum(np.ceil(np.maximum(a, b) + w + 1).astype(int), SIZE)
        yy, xx = np.mgrid[lo[1]:hi[1], lo[0]:hi[0]]
        dx, dy = xx + .5 - a[0], yy + .5 - a[1]
        t = (dx * axis[0] + dy * axis[1]) / length
        q = (dx * side[0] + dy * side[1]) / w
        tc = np.clip(t, 0, 1)
        profile = np.ones_like(t) if stem else np.sin(np.pi * tc) ** (.32 if needle else .72)
        if not needle and not stem:
            profile *= 1 - (.07 if species == 'aspen' else .13) * (.5 + .5 * np.sin(tc * (12 if species == 'aspen' else 20) * np.pi))
            profile *= (1.0 if species == 'aspen' else 1.15 - .5 * tc)  # Broad basal half, pointed birch apex.
        mask = (t >= 0) & (t <= 1) & (np.abs(q) <= profile)
        region = rgb[lo[1]:hi[1], lo[0]:hi[0]]
        shade = .86 + .14 * tc - .10 * np.abs(q)
        vein = np.exp(-np.abs(q) * 30) * .12
        region[mask, :3] = (np.array(color)[None, :] * (shade + vein)[..., None])[mask]
        region[mask, 3] = 1
        nx = side[0] * q * .36 + axis[0] * (tc - .5) * .16
        ny = side[1] * q * .36 + axis[1] * (tc - .5) * .16
        nz = np.sqrt(np.maximum(.01, 1 - nx * nx - ny * ny))
        n = np.stack((nx, ny, nz), -1) * .5 + .5
        normals[lo[1]:hi[1], lo[0]:hi[0]][mask, :3] = n[mask]
    for cell in range(4):
        base = np.array((cell % 2 * .5, cell // 2 * .5))
        def pt(x, y):
            return base + np.array((.02 + x * .46, .02 + y * .46))
        def line(a, b, width=.0015):
            blade(pt(*a), pt(*b), width, (.20, .16, .075), stem=True)
        if species == 'spruce':
            # Unequal lateral shoots keep the alpha silhouette off the card boundary.
            line((.50, .02), (.49, .96), .002)
            for j in range(13):
                t = .08 + j * .064
                for side in (-1, 1):
                    start = np.array((.5 + .02 * math.sin(t * 5), t))
                    end = start + np.array((side * (.34 - .20 * t) * rng.uniform(.60, 1.12), rng.uniform(.09, .19)))
                    line(start, end, .0014)
                    axis = end - start
                    for k in range(23):
                        origin = start + axis * (k + rng.uniform(.05, .95)) / 23
                        for sign in (-1, 1):
                            tip = origin + np.array((side * rng.uniform(.010, .050), sign * rng.uniform(.020, .065)))
                            c = rng.uniform(.88, 1.10)
                            blade(pt(*origin), pt(*tip), .0027, np.array((.32, .43, .23)) * c, needle=True)
            for j in range(24):
                y = .78 + j * .008
                for side in (-1, 1):
                    blade(pt(.49, y), pt(.49 + side * .05, y + .026), .0022, (.38, .46, .21), needle=True)
        elif species == 'pine':
            # Three terminal brushes: bare shoot bases, paired slender needles at tips.
            # The outline is a needle fan, not the old seven-shoot leafy paddle.
            for j in range(3):
                root = np.array((.5, .96))
                phase = 1.5 * math.pi + (j - 1) * .42 + rng.uniform(-.06, .06)
                reach = rng.uniform(.53, .66)
                axis = np.array((math.cos(phase), math.sin(phase)))
                side = np.array((-axis[1], axis[0]))
                tip = root + reach * axis
                line(root, tip, .0013)
                for k in range(145):
                    at = tip - axis * rng.uniform(.01, .24)
                    spread = rng.uniform(-1.20, 1.20)
                    direction = axis * math.cos(spread) + side * math.sin(spread)
                    length = rng.uniform(.12, .23)
                    for paired in (-1, 1):
                        end = at + direction * length + side * paired * .009
                        c = rng.uniform(.80, 1.16)
                        blade(pt(*at), pt(*end), .0011,
                              np.array((.43, .53, .31)) * c, needle=True)
        elif species == 'aspen':
            line((.50, .97), (.47, .08), .002)
            for j in range(9):
                start = np.array((.49, .88 - j * .087))
                for sign in (-1, 1):
                    end = start + np.array((sign * rng.uniform(.19, .32), -.13))
                    line(start, end, .0011)
                    for k in range(7):
                        at = start + (end - start) * (k + 1) / 7
                        tip = at + np.array(((-1)**k * .055, -.11))
                        # Long flattened petiole; nearly round, shallow-toothed blade.
                        leafbase = at + (tip - at) * .34
                        line(at, leafbase, .0006)
                        c = rng.uniform(.88, 1.14)
                        blade(pt(*leafbase), pt(*tip), .019, np.array((.61, .68, .30)) * c)
        else:
            # Connected fine twigs with moderate terminal droop.
            tops = [(.08 + j * .084 + rng.uniform(-.014, .014),
                     .92 - .26 * ((j - 4.5) / 4.5) ** 2 + rng.uniform(-.035, .035)) for j in range(11)]
            for a, b in zip(tops, tops[1:]):
                line(a, b, .0018)
            for j, (x, y) in enumerate(tops):
                length = rng.uniform(.36, .62) * (1 - .25 * abs(j - 5) / 5)
                sway = rng.uniform(-.05, .05)
                count = rng.randrange(19, 25)
                prev = (x, y)
                for k in range(count):
                    t = (k + rng.uniform(.65, 1)) / count
                    at = (x + sway * t * t, y - length * t)
                    line(prev, at, .00065)
                    sign = (-1) ** k
                    tip = (at[0] + sign * rng.uniform(.030, .055), at[1] - rng.uniform(.025, .050))
                    c = rng.uniform(.88, 1.12)
                    blade(pt(*at), pt(*tip), rng.uniform(.010, .014), np.array((.60, .72, .30)) * c)
                    prev = at
    if species == 'spruce':
        # The card is attached at V=1: stem root at the top, pointed shoot tip below.
        for row in range(2):
            section = slice(row * SIZE // 2, (row + 1) * SIZE // 2)
            rgb[section] = rgb[section][::-1].copy()
            normals[section] = normals[section][::-1].copy()
            normals[section, :, 1] = 1 - normals[section, :, 1]
    png(out / f'{species}_foliage.png', rgb)
    png(out / f'{species}_normal.png', normals)
    return rgb[..., 3]


def bark_texture(species, out):
    n = 256
    y, x = np.mgrid[:n, :n] / n
    # Integer periodic frequencies make the field tile continuously at UV seams.
    grain = .5 + .22 * np.sin(TAU * (x * 17 + .17 * np.sin(TAU * y * 3))) + .12 * np.sin(TAU * (x * 43 + y * 2))
    if species == 'birch':
        value = .72 + .12 * grain
        marks = (np.sin(TAU * (y * 19 + .055 * np.sin(TAU * x * 3))) > .91) & (np.sin(TAU * (x * 7 + y * 3)) > .15)
        value[marks] *= .16
        color = np.stack((value, value * .985, value * .92), -1)
    elif species == 'aspen':
        value = .53 + .13 * grain
        scars = (np.sin(TAU * (y * 13 + .09 * np.sin(TAU * x * 4))) > .96) & (np.sin(TAU * x * 9) > .3)
        value[scars] *= .40
        color = np.stack((value * .91, value, value * .81), -1)
    elif species == 'pine':
        # U selects a continuous plate-to-flake transition; V repeats local detail.
        # This preserves texel density without a hard boundary between two tiles.
        rng = np.random.default_rng(5311)
        fy, fx = np.meshgrid(np.fft.fftfreq(n) * n, np.fft.fftfreq(n) * n, indexing='ij')
        def field(scale_x, scale_y):
            spectrum = np.fft.fft2(rng.standard_normal((n, n)))
            f = np.real(np.fft.ifft2(spectrum * np.exp(-(fx / scale_x) ** 2 - (fy / scale_y) ** 2)))
            return (f - f.mean()) / f.std()
        def plates(columns, rows):
            # Bounded neighbour search for the nearest two periodic jittered cells.
            u = x * columns + .22*field(20, 12)
            v = y * rows + .18*field(16, 8)
            nearest = np.full((n, n), np.inf)
            second = nearest.copy()
            tint = np.zeros((n, n))
            for dy in range(-2, 3):
                for dx in range(-2, 3):
                    cx, cy = np.floor(u) + dx, np.floor(v) + dy
                    key = (cx % columns) * 127.1 + (cy % rows) * 311.7
                    jitter = np.mod(np.sin(key) * 43758.5453, 1)
                    jy = np.mod(np.sin(key + 19.19) * 23421.631, 1)
                    distance = (u - cx - .05 - .9*jitter)**2 + (v - cy - .05 - .9*jy)**2
                    closer = distance < nearest
                    second = np.where(closer, nearest, np.minimum(second, distance))
                    tint = np.where(closer, .80 + .4*jitter, tint)
                    nearest = np.minimum(nearest, distance)
            return np.sqrt(second)-np.sqrt(nearest), tint
        edge, plate_tint = plates(28, 5)
        fissures = .18 + .82*np.clip(edge/.075, 0, 1)
        long_cracks = np.abs(np.sin(TAU*(x*40 + .09*field(18, 8))))
        fissures *= .25 + .75*np.clip(long_cracks/.15, 0, 1)
        fine = field(90, 90)
        low = plate_tint * fissures * (1 + .09*fine)
        flake_edge, flake_tint = plates(64, 16)
        # Small irregular flakes with dark edges, rather than isolated black spots.
        high = flake_tint * (.45 + .55*np.clip(flake_edge/.13, 0, 1)) * (1 + .045*fine)
        # Spatial islands retain grey plates through a several-metre band.
        fade = np.clip((x - .30 + .08*field(16, 8) + .035*(plate_tint-1)/.2) / .40, 0, 1)
        fade = fade*fade*(3-2*fade)
        lower = low[..., None] * np.array((.34, .30, .265))
        upper = high[..., None] * np.array((.59, .375, .29))
        upper *= np.where(flake_tint[..., None] < .88, np.array((.80, .91, 1.)), 1.)
        color = lower*(1-fade[..., None]) + upper*fade[..., None]
    else:
        value = .16 + .17 * grain
        value[np.sin(TAU * (x * 29 + .11 * np.sin(TAU * y * 7))) > .85] *= .55
        color = np.stack((value, value * .86, value * .68), -1)
    rgba = np.ones((n, n, 4), np.float32)
    rgba[..., :3] = color
    png(out / f'{species}_bark.png', rgba)


def material(species, out, foliage):
    name = f'{species}_{"foliage" if foliage else "bark"}'
    if name in bpy.data.materials:
        return bpy.data.materials[name]
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    mat.use_backface_culling = not foliage
    nodes, links = mat.node_tree.nodes, mat.node_tree.links
    bsdf = nodes.get('Principled BSDF')
    bsdf.inputs['Roughness'].default_value = .86
    bsdf.inputs['Specular IOR Level'].default_value = .15
    tex = nodes.new('ShaderNodeTexImage')
    tex.image = bpy.data.images.load(str(out / f'{species}_{"foliage" if foliage else "bark"}.png'))
    col = nodes.new('ShaderNodeVertexColor')
    col.layer_name = 'CrownAO'
    mul = nodes.new('ShaderNodeMixRGB')
    mul.blend_type = 'MULTIPLY'
    mul.inputs[0].default_value = 1
    links.new(tex.outputs['Color'], mul.inputs[1])
    links.new(col.outputs['Color'], mul.inputs[2])
    links.new(mul.outputs[0], bsdf.inputs['Base Color'])
    if foliage:
        links.new(tex.outputs['Alpha'], bsdf.inputs['Alpha'])
        normal = nodes.new('ShaderNodeTexImage')
        normal.image = bpy.data.images.load(str(out / f'{species}_normal.png'))
        normal.image.colorspace_settings.name = 'Non-Color'
        nm = nodes.new('ShaderNodeNormalMap')
        nm.inputs['Strength'].default_value = .5
        links.new(normal.outputs['Color'], nm.inputs['Color'])
        links.new(nm.outputs['Normal'], bsdf.inputs['Normal'])
    return mat


class Tree:
    def __init__(self, species, out, alpha):
        self.species, self.alpha = species, alpha
        self.name = species
        self.height = 22.
        self.vertices, self.faces, self.uv, self.normals, self.colors, self.slots = [], [], [], [], [], []
        self.cards = []
        self.area = 0.
        # Reduced level faces: (points, uv, normals, colours, slot, source card index or None).
        self.reduced = []
        self.materials = [material(species, out, False), material(species, out, True)]

    def face(self, points, uv, normals, colors, slot):
        start = len(self.vertices)
        self.vertices.extend(points)
        self.faces.append(tuple(range(start, start + len(points))))
        self.uv.extend(uv)
        self.normals.extend(normals)
        self.colors.extend(colors)
        self.slots.append(slot)

    def tube(self, path, radius, sides=4):
        path = [Vector(p) for p in path]
        rings, radial = [], []
        for j, p in enumerate(path):
            t = j / (len(path) - 1)
            axis = (path[min(j + 1, len(path) - 1)] - path[max(0, j - 1)]).normalized()
            right = axis.cross(Vector((0, 1, 0))).normalized()
            across = axis.cross(right).normalized()
            rr = radius * (1 - .96 * t)
            radial.append([right * math.cos(i * TAU / sides) + across * math.sin(i * TAU / sides) for i in range(sides)])
            rings.append([p + r * rr for r in radial[-1]])
            if j == 0 and p.z == 0:
                for vertex in rings[-1]:
                    vertex.z = 0
        for j in range(len(path) - 1):
            for i in range(sides):
                ids = [(j, i), (j, (i + 1) % sides), (j + 1, (i + 1) % sides), (j + 1, i)]
                points = [rings[k][v] for k, v in ids]
                colors = []
                for p in points:
                    # Birch's fissured dark foot has a spatially explicit vertical fade.
                    foot = max(0., 1 - p.z / 2.3) if self.species == 'birch' else 0
                    fissure = .5 + .5 * math.sin(math.atan2(p.y, p.x) * 13 + p.z * 2)
                    c = 1 - foot * (.60 + .34 * fissure)
                    if self.species == 'pine':
                        # Vertex tint separates the trunk-height colour transition from bark tiling.
                        fade = min(1., max(0., (p.z / self.height - .20) / .38))
                        fade = fade * fade * (3 - 2 * fade)
                        colors.append((.78 + .22 * fade, .90 - .16 * fade, 1 - .45 * fade, 1))
                    else:
                        colors.append((c, c, c, 1))
                def bark_v(z):
                    return z / (2.6 if self.species == 'pine' else 1.4)
                uv = [(i / sides, bark_v(path[j].z)), ((i + 1) / sides, bark_v(path[j].z)),
                      ((i + 1) / sides, bark_v(path[j + 1].z)), (i / sides, bark_v(path[j + 1].z))]
                normals = [radial[k][v] for k, v in ids]
                self.face(points, uv, normals, colors, 0)
                if radius >= REDUCED_MIN_RADIUS:
                    self.reduced.append((points, uv, normals, colors, 0, None))

    def card(self, top, axis, width, length, angle, cell, crown):
        axis = Vector(axis).normalized()
        right = axis.cross(UP)
        if right.length < .1:
            right = axis.cross(Vector((0, 1, 0)))
        right.normalize()
        right = math.cos(angle) * right + math.sin(angle) * axis.cross(right)
        top = Vector(top)
        bottom = top + axis * length
        face_normal = right.cross(-axis).normalized()
        if face_normal.dot((top + bottom) / 2 - Vector(crown)) < 0:
            right = -right
            face_normal = -face_normal
        points = [bottom - right * width / 2, bottom + right * width / 2,
                  top + right * width / 2, top - right * width / 2]
        origin = (cell % 2 * .5, cell // 2 * .5)
        uv = [(origin[0] + u * .5, origin[1] + v * .5) for u, v in [(0, 0), (1, 0), (1, 1), (0, 1)]]
        ns = []
        for p in points:
            outward = (p - Vector(crown)).normalized()
            n = outward * .60 + face_normal * .40 + UP * .25
            # Never supply a custom normal in the opposite geometric hemisphere.
            n += face_normal * max(0., .15 - n.dot(face_normal))
            ns.append(n.normalized())
        centre = (top + bottom) / 2
        if len(self.cards) % REDUCED_CARD_STRIDE == 0:
            grown = [centre + (p - centre) * REDUCED_CARD_SCALE for p in points]
            self.reduced.append((grown, uv, ns, None, 1, len(self.cards)))
        self.cards.append((len(self.faces), len(self.vertices), centre))
        self.face(points, uv, ns, [(1, 1, 1, 1)] * 4, 1)
        self.area += width * length

    def finish(self):
        # Offline alpha-aware geometric visibility: BVH plus 32 fixed sky directions.
        # O(F log F + C * D * H log F); D=32, H<=24. No runtime AO cost.
        bvh = BVHTree.FromPolygons(self.vertices, self.faces, all_triangles=False)
        directions = []
        for k in range(32):
            z = (k + .5) / 32
            phi = k * 2.399963229728653
            directions.append(Vector((math.sqrt(1 - z * z) * math.cos(phi), math.sqrt(1 - z * z) * math.sin(phi), z)))
        values = []
        for face_index, start, centre in self.cards:
            blocked = 0
            for direction in directions:
                pos = centre + direction * .035
                for _ in range(24):
                    hit, _, idx, distance = bvh.ray_cast(pos, direction, 30)
                    if hit is None:
                        break
                    opaque = self.slots[idx] == 0
                    if not opaque and idx != face_index:
                        f = self.faces[idx]
                        p0, p1, p3 = [self.vertices[f[i]] for i in (0, 1, 3)]
                        a, b, delta = p1 - p0, p3 - p0, hit - p0
                        u, v = delta.dot(a) / a.length_squared, delta.dot(b) / b.length_squared
                        uv0, uv1, uv3 = [Vector(self.uv[f[i]]) for i in (0, 1, 3)]
                        uv = uv0 + (uv1 - uv0) * u + (uv3 - uv0) * v
                        x, y = min(SIZE - 1, max(0, int(uv.x * SIZE))), min(SIZE - 1, max(0, int(uv.y * SIZE)))
                        opaque = self.alpha[y, x] >= .4
                    if opaque and idx != face_index:
                        blocked += 1
                        break
                    pos = hit + direction * .02
            ao = 1 - .40 * blocked / len(directions)
            values.append(ao)
            self.colors[start:start + 4] = [(ao, ao, ao, 1)] * 4
        obj = self.make_object(self.name, self.vertices, self.faces, self.uv, self.normals, self.colors, self.slots)
        mesh = obj.data
        # The reduced level takes each kept card's occlusion from the full tree.
        rv, rf, ruv, rn, rc, rs = [], [], [], [], [], []
        for points, uv, normals, colors, slot, source in self.reduced:
            if source is not None:
                ao = values[source]
                colors = [(ao, ao, ao, 1)] * 4
            rf.append(tuple(range(len(rv), len(rv) + len(points))))
            rv.extend(points); ruv.extend(uv); rn.extend(normals); rc.extend(colors); rs.append(slot)
        self.reduced_obj = self.make_object(self.name + '_lod1', rv, rf, ruv, rn, rc, rs)
        self.reduced_obj.data.calc_loop_triangles()
        mesh.calc_loop_triangles()
        triangles = len(mesh.loop_triangles)
        assert triangles <= 4000, (self.species, triangles)
        assert self.area <= 1.5 * BASELINE[self.species][1]
        assert min(p.z for p in self.vertices) >= -.01, (self.species, min(p.z for p in self.vertices))
        bounds = np.array([tuple(p) for p in self.vertices])
        self.stats = dict(triangles=triangles, cards=len(self.cards), card_area_m2=round(self.area, 3),
                          height_m=round(float(bounds[:, 2].max()), 3),
                          width_m=round(float(max(np.ptp(bounds[:, 0]), np.ptp(bounds[:, 1]))), 3),
                          crown_ratio=round(float((bounds[:, 2].max() - min(self.vertices[v].z for f, _, _ in self.cards for v in self.faces[f])) / bounds[:, 2].max()), 3),
                          ao_range=[round(min(values), 4), round(max(values), 4)],
                          atlas_coverage=round(float((self.alpha >= .4).mean()), 4),
                          reduced_triangles=len(self.reduced_obj.data.loop_triangles))
        return obj

    def make_object(self, name, vertices, faces, uvs, normals, colors, slots):
        mesh = bpy.data.meshes.new(name)
        mesh.from_pydata(vertices, [], faces)
        mesh.update()
        uv = mesh.uv_layers.new(name='UVMap')
        color = mesh.color_attributes.new(name='CrownAO', type='FLOAT_COLOR', domain='CORNER')
        for loop in mesh.loops:
            uv.data[loop.index].uv = uvs[loop.vertex_index]
            color.data[loop.index].color = colors[loop.vertex_index]
        for mat in self.materials:
            mesh.materials.append(mat)
        for i, poly in enumerate(mesh.polygons):
            poly.material_index = slots[i]
            poly.use_smooth = True
        mesh.normals_split_custom_set([normals[loop.vertex_index] for loop in mesh.loops])
        obj = bpy.data.objects.new(name, mesh)
        bpy.context.collection.objects.link(obj)
        return obj


def sample_path(path, t):
    f = min(len(path) - 1.000001, t * (len(path) - 1))
    i = int(f)
    return path[i].lerp(path[i + 1], f - i)


def setup_tree(species, out, alpha, variant, height):
    tree = Tree(species, out, alpha)
    tree.name = f'{species}_{variant}'
    tree.height = height
    return tree


def build_spruce(out, alpha, revision, variant):
    rng = random.Random(23371 + variant * 193)
    height, radius, base, lean = [(25., 1.45, 8.4, .18), (22.5, 1.35, 8.5, -.25), (17., 2.20, 2.05, .28)][variant]
    tree = setup_tree('spruce', out, alpha, variant, height)
    trunk = [Vector((lean * (i / 10)**1.5, .08 * math.sin(i), i * height / 10)) for i in range(11)]
    tree.tube(trunk, .34, 7)
    for branch in range(68):
        t = branch / 68
        z = base + (height * .96 - base) * t
        phi = branch * 2.39996323 + rng.uniform(-.30, .30)
        radial = Vector((math.cos(phi), math.sin(phi), 0))
        side = UP.cross(radial)
        reach = (radius * (1 - t) ** .92 + .07) * rng.uniform(.85, 1.10)
        start = sample_path(trunk, z / height)
        rise = reach * (-.20 + .7 * t)
        path = [start, start + radial * reach * .55 + UP * (rise * .6 - .25),
                start + radial * reach + UP * rise]
        tree.tube(path, .077 * (1 - t) + .006, 3)
        # Attached inner shoots hide the trunk without filling the outer crown's gaps.
        tree.card(start, radial * .32 - UP, .55, 1.0, phi, branch % 4, start - UP)
        for j in range(9):
            f = .08 + j * .108
            root = sample_path(path, f)
            sign = (-1)**j
            end = root + side * sign * reach * (.24 - .13 * f) + radial * .18 - UP * .24
            if j == 4:
                tree.tube([root, end], .017 * (1 - t) + .002, 3)
            for k in range(2):
                top = root.lerp(end, (k + .3) / 2)
                axis = radial * (.7 if k == 0 else .18) + side * sign * .18 - UP * (.35 if k == 0 else 1)
                width = (.45 + .30 * (1 - t)) * rng.uniform(.82, 1.08)
                length = (.60 + .48 * (1 - t)) * rng.uniform(.84, 1.12)
                tree.card(top, axis, width, length, rng.uniform(-1.4, 1.4), rng.randrange(4), (0, 0, z - 1))
    for i in range(12):
        z = height * (.95 + i * .004)
        phi = i * 2.39996323
        tree.card(sample_path(trunk, z / height), (math.cos(phi)*.2, math.sin(phi)*.2, -.8),
                  .28 * (1 - i/28), .38, phi, i%4, (0, 0, z-.5))
    if variant < 2:
        for j in range(4):
            root = sample_path(trunk, (2.5 + j * 1.2) / height)
            phi = j * 2.4
            tree.tube([root, root + Vector((math.cos(phi) * .55, math.sin(phi) * .55, -.18))], .025, 3)
    return tree, tree.finish()


def build_birch(out, alpha, revision, variant):
    rng = random.Random(73911 + variant * 317)
    height, radius, base, lean = [(22.5, 2.45, 10.6, .20), (20.5, 2.25, 9.4, -.25), (17.5, 3.6, 5.6, .30)][variant]
    tree = setup_tree('birch', out, alpha, variant, height)
    trunk = [Vector((lean * (i/8)**1.3, .10 * math.sin(i), height * .94 * i/8)) for i in range(9)]
    tree.tube(trunk, .25 if variant < 2 else .29, 8)
    for limb in range(22):
        t = limb / 21
        phi = limb * 2.39996323 + rng.uniform(-.35, .35)
        radial = Vector((math.cos(phi), math.sin(phi), 0)); side = UP.cross(radial)
        z = base + (height - base) * .70 * t
        start = sample_path(trunk, z / (height * .94))
        reach = radius * math.sin(.58 + t * 2.30) * rng.uniform(.85, 1.10)
        endz = base + (height - base) * (.17 + .80 * t) + rng.uniform(-.25, .25)
        end = Vector((0, 0, endz)) + radial * reach + Vector((lean, 0, 0))
        path = [start, start.lerp(end, .48) + UP * .35, end]
        tree.tube(path, .072 * (1 - t * .65), 5)
        for j in range(3):
            root = sample_path(path, .28 + j * .34)
            sign = (-1)**j
            offset = radial * rng.uniform(.15, .45) + side * sign * rng.uniform(.30, .65)
            secondary = [root, root + offset * .5 + UP * .35, root + offset + UP * .70]
            tree.tube(secondary, .030, 3)
            for k in range(3):
                top = sample_path(secondary, .35 + k * .31)
                length = rng.uniform(.45, 1.05) * (1.2 if variant == 2 else 1.)
                drift = radial * rng.uniform(.10, .30) + side * rng.uniform(-.20, .20)
                twig = [top, top + drift*.55 - UP*length*.35, top + drift - UP*length]
                if k == 1:
                    tree.tube([twig[0], twig[-1]], .009, 3)
                for q in range(5):
                    at = sample_path(twig, q/5)
                    axis = drift * (1.5 if q < 2 else .5) + UP * (.6 if q == 0 else -1)
                    tree.card(at, axis, rng.uniform(.72, .96), rng.uniform(.84, 1.14),
                              rng.uniform(-1.5, 1.5), rng.randrange(3), root + offset*.5 - UP*1.2)
    # Short terminal forks retain a light, ascending upper crown.
    for i in range(5):
        phi = i * 2.39996323 + variant * .6
        root = sample_path(trunk, .94 + i * .012)
        end = Vector((lean + math.cos(phi) * .50, math.sin(phi) * .50, height - .15))
        tree.tube([root, end], .018, 3)
        for q in range(8):
            at = root.lerp(end, (q + .5) / 8)
            tree.card(at, (math.cos(phi) * .2, math.sin(phi) * .2, -1),
                      .80, 1.0, phi + q * .85, (i + q) % 4, root - UP)
    return tree, tree.finish()


def build_pine(out, alpha, revision, variant):
    rng = random.Random(59281 + variant * 433)
    height, radius, base, lean = [(23., 2.00, 13.5, .16),
                                  (21., 2.16, 12.5, -.18),
                                  (9.5, 1.40, .40, .08)][variant]
    tree = setup_tree('pine', out, alpha, variant, height)
    transition = height * (.33 if variant < 2 else .22)

    def wood(path, radius, sides):
        # Pine-only remap keeps the common geometry/LOD thresholds and other species exact.
        first, reduced_first = len(tree.vertices), len(tree.reduced)
        tree.tube(path, radius, sides)
        def surface(points, uv):
            # Shift across the continuous bark atlas over the transition band.
            # Mirrored circumferential sampling closes the seam at every height.
            coords = []
            for p, (u, _) in zip(points, uv):
                h = p.z / height * (.33/.22 if variant == 2 else 1.)
                fade = min(1., max(0., (h-.23)/.20))
                coords.append((.015 + .22*(.5-.5*math.cos(TAU*u)) + .75*fade, p.z/1.8))
            colours = [(1., 1., 1., 1.) for _ in points]
            return coords, colours
        for start in range(first, len(tree.vertices), 4):
            coords, colours = surface(tree.vertices[start:start+4], tree.uv[start:start+4])
            tree.uv[start:start+4], tree.colors[start:start+4] = coords, colours
        for i in range(reduced_first, len(tree.reduced)):
            points, uv, normals, _, slot, source = tree.reduced[i]
            coords, colours = surface(points, uv)
            tree.reduced[i] = (points, coords, normals, colours, slot, source)

    levels = sorted(set([i / 12 for i in range(13)] + [transition / height]))
    trunk = [Vector((lean*t*t + .035*math.sin(t*17)*t,
                     .035*math.sin(t*12)*t, height*t)) for t in levels]
    wood(trunk, .28 if variant < 2 else .13, 8)
    # Interpolate by physical height: trunk ring spacing includes the bark boundary.
    def stem(z):
        for a, b in zip(trunk, trunk[1:]):
            if a.z <= z <= b.z:
                return a.lerp(b, (z-a.z)/(b.z-a.z))
        return trunk[-1].copy()

    count = (16, 16, 21)[variant]
    for limb in range(count):
        if variant == 2:
            tier = limb // 3
            t = tier / 7
            z = base + (height-base)*t
            phi = limb % 3 * TAU/3 + tier*.81 + rng.uniform(-.20, .20)
            reach = radius*(1-t)**.80*rng.uniform(.82, 1.05)
            rise = .30 + .10*t
        else:
            t = limb/(count-1)
            # Different crown scaffolds: a staggered leader versus an older spreading top.
            z = base + (height-base)*(.75*t if variant == 0 else .72*t)
            phi = limb*2.39996323 + variant*1.1 + rng.uniform(-.42, .42)
            reach = radius*(.76+.24*math.sin(t*math.pi))*(1-.90*t**3)
            reach *= rng.uniform(.80, 1.12)
            rise = rng.uniform(.30, .80) + (.35*t if variant == 0 else .70*t)
            if variant == 1:
                phi = (0.2,3.4,1.9,4.8,.8,3.,5.6,2.2,4.5,.3,3.6,5.4,1.5,4.,2.6,5.9)[limb]
                z = base + (height-base) * (0.,.06,.14,.21,.30,.39,.46,.52,.57,.63,.67,.71,.74,.77,.80,.83)[limb]
                reach = radius * (.85 if limb < 7 else .52) * rng.uniform(.72,1.12)
                rise = rng.uniform(.25,.65) if limb < 7 else rng.uniform(.55,.95)
        radial = Vector((math.cos(phi), math.sin(phi), 0))
        side = UP.cross(radial)
        start = stem(z)
        end = start + radial*reach + UP*rise
        elbow = start.lerp(end, .52) - UP*.14 + side*rng.uniform(-.14, .14)
        wood([start, elbow, end], (.072 if variant < 2 else .039)*(1-.55*t), 5 if variant < 2 else 3)
        for j in range(3):
            root = sample_path([start, elbow, end], .30+j*.31)
            taper = 1-.60*t if variant == 2 else 1.
            tip = root + radial*(.22*taper) + side*(j-1)*rng.uniform(.32, .48)*taper + UP*rng.uniform(.18, .42)
            wood([root, root.lerp(tip, .55), tip], .023 if variant < 2 else .015, 3)
            for k in range(3):
                angle = phi + (k-1)*.85
                shoot_axis = Vector((math.cos(angle)*.55, math.sin(angle)*.55, .60)).normalized()
                shoot = tip + side*(k-1)*.19*taper + shoot_axis*.21
                wood([tip, shoot], .009, 3)
                # Cards share shoot bases; needles project out along the growing shoot.
                # Small crossing brushes form discrete terminal clumps, leaving bare wood.
                for q in range((8 if t < .40 else 5) if variant < 2 else 4):
                    direction = (shoot_axis + side*rng.uniform(-.5, .5) + UP*rng.uniform(-.2, .3)).normalized()
                    at = shoot + radial*rng.uniform(-.07, .07) + side*rng.uniform(-.10, .10)
                    size = (.98, .98, 1.23)[variant] * (1-.40*t if variant == 2 else 1.)
                    tree.card(at, direction, rng.uniform(.47, .53)*size,
                              rng.uniform(.55, .65)*size, q*math.pi/3+.15*limb,
                              (limb+j+k+q)%4, tip-UP*.25)
    # The young tree retains a leader; old crown tops finish with short irregular shoots.
    for j in range(3):
        at = stem(height-.48)
        direction = Vector((math.cos(j*2.1)*.25, math.sin(j*2.1)*.25, 1))
        for q in range(15):
            tree.card(at + Vector((math.cos(q*2.4)*.12, math.sin(q*2.4)*.12, -.03*q)), direction, .40, .55, q*math.pi/3, j%4, at-UP*.3)
    for j in range(5 if variant < 2 else 2):
        z = max(.4, base-3+j*.55)
        root = stem(z)
        wood([root, root+Vector((math.cos(j*2.4)*.38, math.sin(j*2.4)*.38, .06))], .021, 3)
    obj = tree.finish()
    tree.stats['orange_transition_m'] = round(transition, 3)
    tree.stats['orange_transition_height_share'] = round(transition / tree.stats['height_m'], 4)
    tree.stats['orange_fade_m'] = [round(height*t, 3) for t in ((.23,.43) if variant < 2 else (.23*.22/.33,.43*.22/.33))]
    return tree, obj


def build_aspen(out, alpha, revision, variant):
    rng=random.Random(41881+variant*257)
    height,radius,base,lean=[(22.,2.35,11.0,.14),(20.,2.20,10.0,-.18),(23.5,2.45,12.2,.20)][variant]
    tree=setup_tree('aspen',out,alpha,variant,height)
    trunk=[Vector((lean*(i/9)**1.3,.045*math.sin(i),height*i/9)) for i in range(10)]
    tree.tube(trunk,.28,8)
    for limb in range(24):
        t=limb/23; phi=limb*2.39996323+rng.uniform(-.35,.35)
        radial=Vector((math.cos(phi),math.sin(phi),0)); side=UP.cross(radial)
        z=base+(height-base)*.76*t
        start=sample_path(trunk,z/height)
        reach=radius*(.70+.30*math.sin(t*math.pi))*(1-.45*t**3)*rng.uniform(.85,1.15)
        end=start+radial*reach+UP*(1.2+.6*t)
        path=[start,start.lerp(end,.55),end]
        tree.tube(path,.067*(1-.6*t),3)
        for j in range(3):
            root=sample_path(path,.43+j*.27)
            tip=root+side*(-1)**j*.50+radial*.20+UP*.50
            secondary=[root,root.lerp(tip,.6),tip]
            tree.tube(secondary,.018,3)
            for k in range(3):
                shoot=sample_path(secondary,.30+k*.34)
                for q in range(5):
                    angle=phi+q*2.4+k*.7
                    axis=Vector((math.cos(angle)*.6,math.sin(angle)*.6,-.8))
                    at=shoot+side*(k-1)*.30-UP*(q/5)*.70
                    tree.card(at,axis,rng.uniform(.55,.78),rng.uniform(.66,.92),rng.uniform(-1.6,1.6),rng.randrange(4),start-UP)
    return tree,tree.finish()


def export_glb(obj, out):
    bpy.ops.object.select_all(action='DESELECT')
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    path = out / f'{obj.name}.glb'
    bpy.ops.export_scene.gltf(filepath=str(path), export_format='GLB', use_selection=True,
                             export_yup=True, export_normals=True, export_texcoords=True,
                             export_vertex_color='NAME', export_vertex_color_name='CrownAO',
                             export_all_vertex_colors=False, export_attributes=False,
                             export_materials='EXPORT', export_image_format='AUTO')
    # Blender 5 uses dithered transparency; explicitly encode the runtime scissor contract.
    raw = path.read_bytes()
    size, kind = struct.unpack_from('<II', raw, 12)
    assert kind == 0x4E4F534A
    doc = json.loads(raw[20:20 + size])
    for mat in doc['materials']:
        if 'foliage' in mat['name']:
            mat['alphaMode'] = 'MASK'
            mat['alphaCutoff'] = .4
            mat['doubleSided'] = True
    assert len(doc['meshes']) == 1
    for prim in doc['meshes'][0]['primitives']:
        assert {'POSITION', 'NORMAL', 'TEXCOORD_0', 'COLOR_0'} <= prim['attributes'].keys()
    # External species PNGs give all three GLBs the same texture resource paths.
    # Remove embedded image buffers as well, retaining only geometry in the GLB.
    binary = raw[28 + size:]
    image_views = {im['bufferView'] for im in doc['images']}
    packed, views, remap = bytearray(), [], {}
    for index, view in enumerate(doc['bufferViews']):
        if index in image_views:
            continue
        packed.extend(b'\0' * (-len(packed) % 4))
        remap[index] = len(views)
        new_view = dict(view, byteOffset=len(packed))
        offset = view.get('byteOffset', 0)
        packed.extend(binary[offset:offset + view['byteLength']])
        views.append(new_view)
    for accessor in doc['accessors']:
        accessor['bufferView'] = remap[accessor['bufferView']]
    species = obj.name.split('_')[0]
    for im in doc['images']:
        suffix = 'normal' if 'normal' in im['name'] else ('foliage' if 'foliage' in im['name'] else 'bark')
        im.pop('bufferView')
        im['uri'] = f'{species}_{suffix}.png'
    doc['bufferViews'] = views
    doc['buffers'][0]['byteLength'] = len(packed)
    packed.extend(b'\0' * (-len(packed) % 4))
    payload = json.dumps(doc, separators=(',', ':')).encode()
    payload += b' ' * (-len(payload) % 4)
    tail = struct.pack('<II', len(packed), 0x004E4942) + packed
    path.write_bytes(struct.pack('<III', 0x46546C67, 2, 20 + len(payload) + len(tail))
                     + struct.pack('<II', len(payload), 0x4E4F534A) + payload + tail)



def preview_cutoff(mat):
    # Match the exported cutoff exactly in Cycles, including transparent shadows.
    nodes, links = mat.node_tree.nodes, mat.node_tree.links
    bsdf = nodes.get('Principled BSDF')
    source = bsdf.inputs['Alpha'].links[0].from_socket
    cutoff = nodes.new('ShaderNodeMath')
    cutoff.operation = 'GREATER_THAN'
    cutoff.inputs[1].default_value = .4
    links.new(source, cutoff.inputs[0])
    links.new(cutoff.outputs[0], bsdf.inputs['Alpha'])


def aim(obj, target):
    obj.rotation_euler = (Vector(target) - obj.location).to_track_quat('-Z', 'Y').to_euler()


def validate_glb(path, stats):
    """Check the serialized delivery, not just Blender's pre-export mesh."""
    raw = path.read_bytes()
    magic, version, total = struct.unpack_from('<III', raw)
    assert (magic, version, total) == (0x46546C67, 2, len(raw))
    length = struct.unpack_from('<I', raw, 12)[0]
    doc = json.loads(raw[20:20 + length])
    binary = raw[28 + length:]
    def accessor(index):
        a = doc['accessors'][index]
        view = doc['bufferViews'][a['bufferView']]
        dtype = np.dtype({5121: 'u1', 5123: '<u2', 5125: '<u4', 5126: '<f4'}[a['componentType']])
        width = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}[a['type']]
        offset = view.get('byteOffset', 0) + a.get('byteOffset', 0)
        values = np.ndarray((a['count'], width), dtype=dtype, buffer=binary, offset=offset,
                            strides=(view.get('byteStride', width * dtype.itemsize), dtype.itemsize))
        if a.get('normalized'):
            values = values / np.iinfo(dtype).max
        assert np.isfinite(values).all()
        return values
    assert len(doc['meshes']) == len(doc['nodes']) == 1
    assert len(doc['meshes'][0]['primitives']) == 2
    triangles, area, height = 0, 0., 0.
    for p in doc['meshes'][0]['primitives']:
        attr = p['attributes']
        v, n, col = [accessor(attr[k]) for k in ('POSITION', 'NORMAL', 'COLOR_0')]
        assert np.max(np.abs(np.linalg.norm(n, axis=1) - 1)) < .001
        assert np.allclose(col[:, 3], 1)
        assert col[:, :3].min() >= 0 and col[:, :3].max() <= 1
        assert v[:, 1].min() >= -.01  # Serialized geometry is Y up.
        height = max(height, float(v[:, 1].max()))
        indices = accessor(p['indices']).ravel().astype(int)
        assert indices.max() < len(v)
        tri = v[indices.reshape(-1, 3)]
        areas = np.linalg.norm(np.cross(tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0]), axis=1) * .5
        assert areas.min() > 1e-10
        triangles += len(areas)
        mat = doc['materials'][p['material']]
        if 'foliage' in mat['name']:
            assert col[:, :3].min() < .95
            assert mat['alphaMode'] == 'MASK' and mat['alphaCutoff'] == .4 and mat['doubleSided']
            assert 'normalTexture' in mat
            uv = accessor(attr['TEXCOORD_0'])
            assert uv.min() >= 0 and uv.max() <= 1
            area += float(areas.sum())
    assert triangles == stats['triangles'] <= 4000
    assert abs(area - stats['card_area_m2']) < .01, (path.name, area, stats['card_area_m2'])
    assert abs(height - stats['height_m']) < .01
    sizes = []
    for im in doc['images']:
        data = (path.parent / im['uri']).read_bytes()
        assert data[:8] == b'\x89PNG\r\n\x1a\n'
        sizes.append(struct.unpack_from('>II', data, 16))
    assert sorted(sizes) == [(256, 256), (1024, 1024), (1024, 1024)]
    return hashlib.sha256(raw).hexdigest()


def previews(objects, out, revision):
    for mat in bpy.data.materials:
        if mat.name.endswith('_foliage'):
            preview_cutoff(mat)
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.cycles.device = 'CPU'
    scene.cycles.samples = 96
    scene.cycles.seed = 321
    scene.cycles.use_animated_seed = False
    scene.cycles.use_adaptive_sampling = False
    scene.cycles.use_denoising = False
    scene.cycles.transparent_max_bounces = 32
    scene.cycles.max_bounces = 4
    scene.render.threads_mode = 'FIXED'
    scene.render.threads = 12
    scene.render.image_settings.file_format = 'PNG'
    scene.view_settings.view_transform = 'AgX'
    scene.world.use_nodes = True
    bg = scene.world.node_tree.nodes.get('Background')
    bg.inputs[0].default_value = (.52, .69, .90, 1)
    bg.inputs[1].default_value = .8
    light = bpy.data.lights.new('Sun_40_degrees', 'SUN')
    light.energy = 2.5
    light.angle = math.radians(1.5)
    sun = bpy.data.objects.new('Sun_40_degrees', light)
    scene.collection.objects.link(sun)
    elevation = math.radians(40)
    sun.location = Vector((-math.cos(elevation) * 50, -math.cos(elevation) * 35, math.sin(elevation) * math.hypot(50, 35)))
    aim(sun, (0, 0, 0))
    bpy.ops.mesh.primitive_plane_add(size=2000)
    ground = bpy.context.object
    ground.name = 'Preview_ground'
    mat = bpy.data.materials.new('Ground')
    mat.use_nodes = True
    mat.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value = (.085, .125, .043, 1)
    mat.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value = 1
    ground.data.materials.append(mat)
    camdata = bpy.data.cameras.new('Camera')
    camera = bpy.data.objects.new('Camera', camdata)
    scene.collection.objects.link(camera)
    scene.camera = camera
    directory = out / 'previews'
    directory.mkdir(exist_ok=True)
    render_times = {}
    def render(name, width, height):
        scene.render.resolution_x, scene.render.resolution_y = width, height
        scene.render.resolution_percentage = 100
        scene.render.filepath = str(directory / f'{name}.png')
        started = time.perf_counter()
        bpy.ops.render.render(write_still=True)
        # Cycles writes dates and wall-clock timings even with visible stamps off.
        # Drop text metadata only; retain encoded pixels, colour space and CRCs.
        path = directory / f'{name}.png'
        raw = path.read_bytes()
        chunks, offset = [raw[:8]], 8
        while offset < len(raw):
            length = struct.unpack_from('>I', raw, offset)[0] + 12
            if raw[offset + 4:offset + 8] not in (b'tEXt', b'zTXt', b'iTXt'):
                chunks.append(raw[offset:offset + length])
            offset += length
        path.write_bytes(b''.join(chunks))
        render_times[name] = round(time.perf_counter() - started, 2)
    for obj in objects:
        for other in objects:
            other.hide_render = other != obj
        camera.location = (0, -25, 1.7)
        aim(camera, (0, 0, obj.dimensions.z * .49))
        camdata.lens = 34
        scene.render.resolution_x, scene.render.resolution_y = 640, 800
        bpy.context.view_layer.update()
        projected = [world_to_camera_view(scene, camera, obj.matrix_world @ v.co) for v in obj.data.vertices]
        assert all(.015 < p.x < .985 and .015 < p.y < .985 and p.z > 0 for p in projected), obj.name
        render(f'{obj.name}_eye', 640, 800)
    for obj in objects:
        obj.hide_render = obj != objects[0]
    camera.location = (0, -8, 11)
    aim(camera, (0, 0, 11))
    camdata.lens = 45
    render('spruce_sprays_detail', 900, 900)
    for obj in objects:
        obj.hide_render = True
    rng = random.Random(9911)
    stand = [6 + i % 3 for i in range(16)] + [i % 3 for i in range(12)] + [3 + i % 3 for i in range(9)] + [9, 10, 11]
    rng.shuffle(stand)
    instances = []
    for i, index in enumerate(stand):
        source = objects[index]
        instance = bpy.data.objects.new(f'Stand_{i:02}', source.data)
        scene.collection.objects.link(instance)
        instances.append(instance)
        instance.location = ((i % 8 - 3.5) * 7 + rng.uniform(-2.4, 2.4),
                             (i // 8 - 2) * 8 + rng.uniform(-2.4, 2.4), 0)
        scale = rng.uniform(.78, 1.14)
        instance.scale = (scale, scale, scale)
        instance.rotation_euler.z = rng.uniform(0, TAU)
    for distance, angle in [(150, 35), (60, 20)]:
        target = Vector((0, 0, 9))
        azimuth, elevation = math.radians(14), math.radians(angle)
        camera.location = target + Vector((distance * math.cos(elevation) * math.sin(azimuth),
                                            -distance * math.cos(elevation) * math.cos(azimuth), distance * math.sin(elevation)))
        aim(camera, target)
        camdata.lens = 52 if distance == 150 else 24
        render(f'mixed_{distance}m_{angle}deg', 1000, 700)
    for instance in instances:
        instance.hide_render = True
    # A 3 m road separates the two closed canopy strips; only forest habits.
    forest = [6, 7] * 5 + [0, 1] * 4 + [3, 4] * 2 + [9, 10, 11]
    rng.shuffle(forest)
    for i, index in enumerate(forest):
        instance = bpy.data.objects.new(f'Road_stand_{i:02}', objects[index].data)
        scene.collection.objects.link(instance)
        instance.location = ((-6.5, -3.3, 3.2, 6.2, 8.8)[i % 5] + rng.uniform(-.65, .65),
                             2 + (i // 5) * 3.6 + rng.uniform(-1.3, 1.3), 0)
        instance.rotation_euler.z = rng.uniform(0, TAU)
    bpy.ops.mesh.primitive_plane_add(size=1, location=(0, 15, .012))
    road = bpy.context.object
    road.scale = (3, 90, 1)
    roadmat = bpy.data.materials.new('Preview_gravel')
    roadmat.use_nodes = True
    roadmat.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value = (.26, .23, .18, 1)
    roadmat.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value = 1
    road.data.materials.append(roadmat)
    camera.location = (0, -7, 1.7)
    aim(camera, (0, 20, 1.7))
    camdata.lens = 18
    render('forest_road_eye', 1000, 900)
    started = time.perf_counter()
    contact_sheet(objects, directory)
    render_times['contact'] = round(time.perf_counter() - started, 2)
    return render_times


def contact_sheet(objects, directory):
    """Compose the 12 fresh Cycles eye renders; retain colour and add bitmap labels."""
    glyphs = {
        'A': '0e11111f111111', 'B': '1e11111e11111e', 'C': '0f10101010100f',
        'E': '1f10101e10101f', 'H': '1111111f111111', 'I': '1f04040404041f',
        'N': '11191915131311', 'P': '1e11111e101010', 'R': '1e11111e141211',
        'S': '0f10100e01011e', 'U': '1111111111110e', '_': '0000000000001f',
        '0': '0e11131519110e', '1': '040c040404040e', '2': '0e11010204081f',
    }
    sheet = np.ones((432 * 4, 320 * 3, 4), np.float32)
    sheet[..., :3] = (.70, .75, .77)
    for i, obj in enumerate(objects):
        img = bpy.data.images.load(str(directory / f'{obj.name}_eye.png'), check_existing=False)
        # Read encoded display RGB: these files already contain the AgX view transform.
        img.colorspace_settings.name = 'Non-Color'
        pixels = np.empty(640 * 800 * 4, np.float32)
        img.pixels.foreach_get(pixels)
        tile = pixels.reshape(400, 2, 320, 2, 4).mean(axis=(1, 3))
        x0, y0 = (i % 3) * 320, (3 - i // 3) * 432
        sheet[y0 + 28:y0 + 428, x0:x0 + 320] = tile
        label = obj.name.upper()
        left = x0 + (320 - len(label) * 12) // 2
        for k, letter in enumerate(label):
            rows = bytes.fromhex(glyphs[letter])
            for y, row in enumerate(rows):
                for x in range(5):
                    if row & (1 << (4 - x)):
                        xx, yy = left + k * 12 + x * 2, y0 + 7 + (6 - y) * 2
                        sheet[yy:yy + 2, xx:xx + 2, :3] = .07
        bpy.data.images.remove(img)
    png(directory / 'contact.png', sheet)


def report(out, stats, times, elapsed, revision):
    metrics = {'revision': revision, 'blender': bpy.app.version_string, 'trees': stats,
               'render_seconds': times, 'total_seconds': round(elapsed, 2),
               'script_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    (out / 'metrics.json').write_text(json.dumps(metrics, indent=2) + '\n')
    lines = ['# Finnish trees — brief 3', '',
             f'Blender {bpy.app.version_string}. Review pass {revision}.', '',
             '| Variant | Height m | Width m | Crown ratio | Triangles | Cards | Card area m² | Baseline min ratio |',
             '|---|---:|---:|---:|---:|---:|---:|---:|']
    for name, s in stats.items():
        species = name.rsplit('_', 1)[0]
        lines.append(f'| {name} | {s["height_m"]:.2f} | {s["width_m"]:.2f} | {s["crown_ratio"]:.3f} | {s["triangles"]} | {s["cards"]} | {s["card_area_m2"]:.2f} | {s["card_area_m2"]/BASELINE[species][1]:.3f}× |')
    lines += ['', ('Fresh previews were skipped in this build. The following lists the full-build outputs. ' if not times else '') +
              '12 GLBs, 12 species texture PNGs, 12 eye previews, two 40-tree stand views, a 25-tree forest road view, a spruce detail and contact.png. '
              'Each species shares one 1024² RGBA foliage atlas, one 1024² tangent normal map and one 256² bark texture. '
              'All three variants reference the same external species PNGs; keep them beside the GLBs. '
              'Blender also shares the species material datablocks during preview rendering.', '',
              'One mesh/node, two primitives, metres, +Y up, base origin, double-sided foliage MASK 0.4. '
              'Foliage CrownAO RGB is equal-channel geometric sky visibility, alpha 1; 32 deterministic rays/card '
              'traverse the alpha-tested geometry with a 0.60 floor. Bark RGB carries the birch foot and pine upper-trunk tints. '
              'Normals blend local crown direction and outward card normal with an upward bias; tangent normals model leaf curvature.', '',
              'Variants 0/1 are forest-grown for spruce, birch and pine; variant 2 is yard/roadside birch, yard spruce or younger edge pine. All aspen variants share a forest-grown habit. '
              'Width is the larger horizontal mesh bounding-box span. Crown ratio is (height minus lowest foliage-card corner) / height; '
              'transparent card margins are included, so it is a conservative geometric measurement rather than a pixel-derived live-crown estimate.', '',
              'Budget is one-sided geometric card area including transparent pixels, not measured GPU overdraw. '
              'baseline.json retains the original generator measurements; pine/aspen were calculated from the same archived '
              'round-one Godot mesh export, not a new runtime benchmark. Every variant is checked against 1.5× the smallest '
              'baseline tree of its species and 4000 triangles.', '',
              'Offline geometry O(vertices), BVH O(F log F), AO O(C × 32 × 24 × log F). No runtime algorithms or dependencies added. '
              'Serialized GLBs are checked for triangle count/area, nondegenerate faces, UVs, unit finite normals, vertex colours, '
              'Y-up bounds, materials and shared texture dimensions.', '',
              'Previews: Cycles CPU, 12 threads, 96 fixed samples, no denoising, 40° sun. '
              'Eye cameras are 25 m horizontally from each trunk and 1.7 m high. '
              'The contact sheet composes those fresh Cycles renders at half resolution with labels. '
              'Stand: 16 pine, 12 spruce, nine birch, three aspen; every variant represented. '
              'Stand camera distances and depression angles are relative to the look-at point: 150 m/35°, 60 m/20°.', '',
              f'Total build/render time: {elapsed:.2f} s. Render seconds: {json.dumps(times)}.', '',
              'Reproduce on the Mac: `imgs/tree_models/mac_build.sh`. Never run Blender locally.', '']
    review = out / 'review_notes.txt'
    if review.exists():
        lines += [review.read_text()]
    (out / 'REPORT.md').write_text('\n'.join(lines))
    print('TREE_BUILD_SUMMARY ' + json.dumps(metrics, sort_keys=True), flush=True)
    print('Files: spruce/birch/pine/aspen_0..2.glb; 12 shared PNG textures; previews/; REPORT.md; metrics.json.', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('out_dir', type=Path)
    parser.add_argument('--round', type=int, choices=(1, 2, 3), default=3)
    parser.add_argument('--skip-previews', action='store_true', help='Asset-only determinism check')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    started = time.perf_counter()
    out = args.out_dir.resolve()
    out.mkdir(parents=True, exist_ok=True)
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)
    objects, stats = [], {}
    for name, builder in [('spruce', build_spruce), ('birch', build_birch), ('pine', build_pine), ('aspen', build_aspen)]:
        alpha = atlas(name, out)
        bark_texture(name, out)
        for variant in range(3):
            tree, obj = builder(out, alpha, args.round, variant)
            stats[obj.name] = tree.stats
            export_glb(obj, out)
            stats[obj.name]['glb_sha256'] = validate_glb(out / f'{obj.name}.glb', tree.stats)
            export_glb(tree.reduced_obj, out)
            # Previews and stands render the full level only.
            bpy.data.objects.remove(tree.reduced_obj)
            objects.append(obj)
    times = {} if args.skip_previews else previews(objects, out, args.round)
    report(out, stats, times, time.perf_counter() - started, args.round)


if __name__ == '__main__':
    main()
