# SPDX-License-Identifier: GPL-2.0-only

"""Deterministic Finnish shrubs, repeating hedges and granite boulders.

Run with Blender 5.2 on the Mac through imgs/landscape_models/mac_build.sh.
Uses only Blender, its NumPy, and the unchanged model_trees geometry helpers.
All geometry is authored in metres/Z up, exported in metres/Y up.
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

import bpy
import numpy as np
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
import model_trees as trees

TAU = math.tau
# Shrubs have more wood relative to foliage than trees; retain one third of cards.
trees.REDUCED_CARD_STRIDE = 3
trees.REDUCED_CARD_SCALE = math.sqrt(3)
FORMS = {
    'lilac': (3.45, 3.55, 3.30, 640, 16, (.40, .53, .24)),
    'spirea': (1.26, 1.65, 1.48, 390, 12, (.43, .57, .25)),
    'rose': (1.20, 1.48, 1.36, 370, 12, (.29, .43, .19)),
    'cotoneaster': (1.82, 1.96, 1.78, 400, 12, (.28, .40, .17)),
    'mugo': (1.42, 2.22, 1.98, 390, 12, (.30, .43, .25)),
    'juniper': (3.05, .91, .83, 410, 10, (.36, .46, .35)),
}
HEDGES = {'hedge_low': (.90, .60, (.34, .48, .22)),
          'hedge_mid': (1.40, .80, (.45, .59, .27)),
          'hedge_tall': (1.90, .90, (.29, .43, .32))}
ROCK_DIMS = [(.58, .46, .39), (.68, .54, .48), (1.24, .97, .92),
             (1.56, 1.13, 1.08), (2.72, 1.91, 1.85), (1.52, 1.21, .50)]


def field(rng, n, frequency):
    """Periodic band-limited noise: UV wrap has no discontinuity."""
    fy, fx = np.meshgrid(np.fft.fftfreq(n) * n, np.fft.fftfreq(n) * n, indexing='ij')
    a = np.fft.ifft2(np.fft.fft2(rng.standard_normal((n, n))) *
                    np.exp(-(fx * fx + fy * fy) / frequency ** 2)).real
    return a / max(a.std(), 1e-8)


def foliage_atlas(form, color, out):
    """Four connected shoot tiles, or two shoot tiles above an opaque hedge swatch."""
    n = 1024
    seed = sum(map(ord, form)) + 817
    rng = random.Random(seed)
    pixels = np.zeros((n, n, 4), np.float32)
    pixels[..., :3] = color
    normal = np.ones_like(pixels)
    normal[..., :3] = (.5, .5, 1)
    needle = form in ('mugo', 'juniper', 'hedge_tall')
    def blade(a, b, width, tint, stem=False):
        a, b = np.array(a) * n, np.array(b) * n
        delta = b - a
        length = np.linalg.norm(delta)
        if length < .1:
            return
        axis = delta / length
        side = np.array((-axis[1], axis[0]))
        w = width * n
        lo = np.maximum(0, np.floor(np.minimum(a, b) - w - 1).astype(int))
        hi = np.minimum(n, np.ceil(np.maximum(a, b) + w + 1).astype(int))
        if np.any(hi <= lo):
            return
        yy, xx = np.mgrid[lo[1]:hi[1], lo[0]:hi[0]]
        dx, dy = xx + .5 - a[0], yy + .5 - a[1]
        t = (dx * axis[0] + dy * axis[1]) / length
        q = (dx * side[0] + dy * side[1]) / w
        tc = np.clip(t, 0, 1)
        profile = np.ones_like(t) if stem else np.sin(np.pi * tc) ** (.33 if needle else .72)
        if form == 'lilac' and not stem:
            profile *= (1.42 - .95 * tc)
            profile *= .78 + .22 * np.clip(tc * 12 + np.abs(q), 0, 1)
        if form == 'rose' and not stem:
            profile *= .94 + .06 * np.sin(tc * 50)
        mask = (t >= 0) & (t <= 1) & (np.abs(q) <= profile)
        relief = .88 + .14 * tc - .13 * np.abs(q)
        relief += .08 * np.exp(-np.abs(q) * 24)
        if form == 'rose':
            relief += .07 * np.cos((tc * 11 - np.abs(q) * 2) * TAU)
        region = pixels[lo[1]:hi[1], lo[0]:hi[0]]
        region[mask, :3] = (np.array(tint) * relief[..., None])[mask]
        region[mask, 3] = 1
        nx, ny = side[0] * q * .30, side[1] * q * .30
        nz = np.sqrt(np.maximum(.02, 1 - nx * nx - ny * ny))
        normals = np.stack((nx, ny, nz), -1) * .5 + .5
        normal[lo[1]:hi[1], lo[0]:hi[0]][mask, :3] = normals[mask]
    hedge = form in HEDGES
    if hedge:
        # Bottom half is fully opaque, including a gutter. This is the closed body UV region.
        texrng = np.random.default_rng(seed)
        noise = field(texrng, n, 12)
        pixels[:512, :, :3] = np.array(color) * (.57 + .08 * noise[:512, :, None])
        pixels[:512, :, 3] = 1
        # Each 5–15 cm shoot has overlapping 2–5 cm leaves and a shared light value.
        # Author in metres before mapping to the swatch, including for the tall hedge.
        height = HEDGES[form][0]
        def swatch(p):
            return (p[0], .012 + p[1] / height * .44)
        for _ in range(round(460 * height)):
            root = np.array((rng.random(), rng.uniform(0, height)))
            angle = rng.uniform(0, TAU)
            axis = np.array((math.cos(angle), math.sin(angle)))
            across = np.array((-axis[1], axis[0]))
            tint = np.array(color) * rng.uniform(.65, 1.45)
            shoot = rng.uniform(.055, .13)
            for wrap in (-1, 0, 1):
                r = root + (wrap, 0)
                blade(swatch(r), swatch(r + axis * shoot), .0012, tint * .60, True)
                for j in range(18 if needle else 7):
                    p = r + axis * shoot * (j / (18 if needle else 7))
                    tip = p + axis * .023 + across * (-1)**j * (.018 if needle else .028)
                    blade(swatch(p), swatch(tip), .003 if needle else .010,
                          tint * (.80 + .35 * (j % 3) / 2))
        # Identical texture phase at the body end rings; reserve a thin wood strip.
        pixels[:480, -21:] = pixels[:480, :21]
        normal[:480, -21:] = normal[:480, :21]
        pixels[490:511, :, :3] = (.26, .23, .16)
    for cell in (range(2, 4) if hedge else range(4)):
        origin = np.array((cell % 2 * .5, cell // 2 * .5))
        def point(x, y):
            return origin + np.array((.025 + .45 * x, .025 + .45 * y))
        def stem(a, b, width=.0010):
            blade(point(*a), point(*b), width, (.26, .24, .12), True)
        if not hedge or needle:
            stem((.50, .96), (.48, .09), .0018)
        if hedge and not needle:
            # A clipped shoot viewed through its overlapping leaves, not a flat
            # pinnate/fern frond. Jittered leaf angles break the card's central axis.
            for j in range(16):
                centre = np.array((.18+(j%4)*.21,.18+(j//4)*.21))
                centre += (rng.uniform(-.05,.05),rng.uniform(-.05,.05))
                angle = rng.uniform(0,TAU)
                axis = np.array((math.cos(angle),math.sin(angle)))
                root = np.clip(centre-axis*.10,.035,.965)
                tip = np.clip(centre+axis*rng.uniform(.13,.18),.035,.965)
                stem((.5,.52),root,.0009)
                blade(point(*root),point(*tip),rng.uniform(.025,.034),
                      np.array(color)*rng.uniform(.74,1.38))
        elif needle:
            for j in range(9 if form != 'mugo' else 7):
                side = (-1) ** j
                root = np.array((.49, .85 - j * .08))
                tip = root + np.array((side * rng.uniform(.19, .32), -.18))
                stem(root, tip)
                axis = (tip - root) / np.linalg.norm(tip - root)
                perp = np.array((-axis[1], axis[0]))
                for k in range(45 if form == 'mugo' else 65):
                    p = root + (tip - root) * rng.random()
                    sign = (-1) ** k
                    end = p + axis * rng.uniform(.07, .16) + perp * sign * rng.uniform(.025, .12)
                    blade(point(*p), point(*end), .0019 if form == 'mugo' else .00145,
                          np.array(color) * rng.uniform(.80, 1.25))
        else:
            count = 7 if form in ('lilac', 'rose') else 11
            for j in range(count):
                for sign in (-1, 1):
                    root = np.array((.49, .86 - j * .70 / count))
                    silhouette = .48 + .52 * math.sin(math.pi * (j + .5) / count)
                    tip = root + np.array((sign * rng.uniform(.19, .32) * silhouette, -.09))
                    stem(root, tip)
                    for k in range(3 if form == 'lilac' else 5):
                        p = root + (tip - root) * (.25 + .75 * k / (2 if form == 'lilac' else 4))
                        end = p + np.array(((-1) ** k * .035, -rng.uniform(.075, .15)))
                        width = .022 if form == 'lilac' else (.016 if form == 'rose' else .010)
                        blade(point(*p), point(*end), width, np.array(color) * rng.uniform(.82, 1.22))
    trees.png(out / f'{form}_foliage.png', pixels)
    trees.png(out / f'{form}_normal.png', normal)
    # Grey, finely fissured multi-stem bark; no reused pine/orange trunk texture.
    b = np.ones((256, 256, 4), np.float32)
    rr = np.random.default_rng(seed + 3)
    grain = field(rr, 256, 26)
    yy, xx = np.mgrid[:256, :256] / 256
    value = .40 + .035 * grain - .10 * (np.sin(TAU * (xx * 31 + .15 * np.sin(yy * TAU))) > .90)
    b[..., :3] = value[..., None] * np.array((1., .94, .83))
    trees.png(out / f'{form}_bark.png', b)
    return pixels[..., 3]


class Shrub(trees.Tree):
    """Tree card/branch authoring and alpha-aware crown AO, with shrub-sized LOD wood."""
    def __init__(self, form, out, alpha):
        super().__init__(form, out, alpha)
        self.name = form + '_0'
        self.wood_count = 0
        trees.BASELINE[form] = (1000., 1000.)

    def tube(self, path, radius, sides=4):
        previous = len(self.reduced)
        super().tube(path, radius, sides)
        del self.reduced[previous:]
        # Keep every third main stem, with three-sided two-segment wood in the reduced level.
        if radius >= .018:
            if self.wood_count % 3 == 0:
                tmp = trees.Tree(self.species, Path('.'), self.alpha)
                tmp.tube([path[0], path[len(path) // 2], path[-1]], radius, 3)
                for face in tmp.faces:
                    self.reduced.append(([tmp.vertices[i] for i in face], [tmp.uv[i] for i in face],
                                         [tmp.normals[i] for i in face], [tmp.colors[i] for i in face], 0, None))
            self.wood_count += 1


def fit_mesh(obj, dims, base=0.):
    a = np.array([tuple(v.co) for v in obj.data.vertices])
    low, high = a.min(0), a.max(0)
    scale = np.array(dims) / (high - low)
    offset = np.array(((low[0] + high[0]) / 2, (low[1] + high[1]) / 2, low[2]))
    for v in obj.data.vertices:
        v.co = (np.array(v.co) - offset) * scale + (0, 0, base)
    # Preserve authored card normals under the anisotropic fit.
    ns = [Vector(np.array(n.vector) / scale).normalized() for n in obj.data.corner_normals]
    obj.data.normals_split_custom_set(ns)
    obj.data.update()


def build_shrub(form, out, alpha):
    height, wx, wy, cards, stems, _ = FORMS[form]
    tree = Shrub(form, out, alpha)
    tree.height = height
    rng = random.Random(3100 + list(FORMS).index(form))
    crown = Vector((0, 0, height * (.65 if form == 'lilac' else .52)))
    centres = []
    for j in range(stems):
        angle = j * 2.399963 + rng.uniform(-.18, .18)
        reach = .22 + .70 * math.sqrt((j + .5) / stems)
        dx, dy = math.cos(angle), math.sin(angle)
        if form == 'juniper':
            z = height * (.20 + .73 * j / (stems - 1))
            r = wx * .28 * (1 - .58 * j / stems)
        else:
            z = height * (.50 + .37 * math.sqrt(1 - reach * reach))
            r = wx * .40 * reach
        root = Vector((dx * wx * .055, dy * wy * .055, 0))
        tip = Vector((dx * r, dy * r * wy / wx, z))
        mid = root.lerp(tip, .52)
        if form == 'mugo':
            mid.z *= .48
        elif form == 'spirea':
            mid.z += height * .14
        tree.tube([root, mid, root.lerp(tip, .82), tip],
                  (.042 if form == 'lilac' else .023) * rng.uniform(.8, 1.2), 4 if form == 'lilac' else 3)
        if form == 'rose':
            thornbase = root.lerp(mid, .8)
            thorn = thornbase + Vector((dx * .038, dy * .038, .025))
            side = Vector((-dy, dx, 0)) * .011
            points = [thornbase-side, thornbase+side, thorn]
            normal = (points[1]-points[0]).cross(points[2]-points[0]).normalized()
            tree.face(points, [(0,0),(1,0),(.5,1)], [normal]*3, [(1,1,1,1)]*3, 0)
        for k in range(3):
            turn = angle + (k - 1) * .85
            # Leaf-bearing side branches start low; these are free-growing shrubs,
            # not a tree crown lifted onto an exposed fan of stems.
            fraction = (.50 + .23 * k) if form == 'lilac' else (.26 + .34 * k)
            radial = (1.02 - .10 * k) if form != 'lilac' else (.82 + .08 * k)
            centre = Vector((tip.x * radial + math.cos(turn) * wx * .105,
                             tip.y * radial + math.sin(turn) * wy * .105, tip.z * fraction))
            if form == 'juniper':
                t = (j * 3 + k + .5) / (stems * 3)
                radial = wx * .23 * math.sin(math.pi * t) ** .40
                centre = Vector((math.cos(turn) * radial, math.sin(turn) * radial, height * (.09 + .83 * t)))
            start = root.lerp(tip, max(.15, fraction - .22))
            bend = start.lerp(centre, .60)
            if form == 'spirea':
                bend.z += .10
                centre.z -= .08
            tree.tube([start, bend, centre], .008 if form != 'lilac' else .015, 3)
            centres.append(centre)
    for i in range(cards):
        c = centres[i % len(centres)]
        phi = rng.uniform(0, TAU)
        z = rng.uniform(-1, 1)
        direction = Vector((math.sqrt(1 - z*z)*math.cos(phi), math.sqrt(1-z*z)*math.sin(phi), z))
        spread = .46 if form == 'lilac' else (.22 if form != 'juniper' else .15)
        p = c + direction * spread * rng.random() ** .5
        length = rng.uniform(.47, .70) if form == 'lilac' else rng.uniform(.28, .43)
        width = length * rng.uniform(.82, 1.12)
        if form == 'juniper':
            axis = Vector((direction.x * .25, direction.y * .25, .85)).normalized()
            length *= 1.4
            width *= .82
        elif form == 'mugo':
            axis = Vector((direction.x * .5, direction.y * .5, .65)).normalized()
        else:
            axis = Vector((direction.x, direction.y, -.35 + direction.z * .35)).normalized()
        p.z = max(length * .80 + .025, p.z)
        tree.card(p - axis * length / 2, axis, width, length, rng.uniform(0, TAU), i % 4, crown)
    obj = tree.finish()
    fit_mesh(obj, (wx, wy, height))
    fit_mesh(tree.reduced_obj, (wx, wy, height))
    return obj, tree.reduced_obj


def body_builder(form, out, alpha):
    # Reuse the tree's mesh assembler; only the foliage material is used by the hedge.
    mesh = trees.Tree(form, out, alpha)
    return mesh


def build_hedge(form, out, alpha):
    height, width, _ = HEDGES[form]
    mesh = body_builder(form, out, alpha)
    rng = random.Random(660 + list(HEDGES).index(form))
    # 116 body triangles, 24 stem triangles, 756 leaf triangles = 896 total.
    # Recessed foot, bulging sides, soft crown; end cross-sections remain identical.
    section = [(-width*.28, .075), (-width*.43, .27), (-width*.46, height*.55),
               (-width*.43, height-.14), (-width*.30, height-.07), (-width*.08, height-.045),
               (width*.08, height-.045), (width*.30, height-.07), (width*.43, height-.14),
               (width*.46, height*.55), (width*.43, .27), (width*.28, .075)]
    count = len(section)
    section_normals = []
    for k in range(count):
        before = Vector((0, *section[k])) - Vector((0, *section[k-1]))
        after = Vector((0, *section[(k+1) % count])) - Vector((0, *section[k]))
        section_normals.append(Vector((0, -(before.normalized()+after.normalized()).z,
                                       (before.normalized()+after.normalized()).y)).normalized())
    def ao(z):
        return .65 + .33 * min(1., max(0., z / .28))
    rings = []
    for x in (-.5, -.25, 0., .25, .5):
        # Fine corrugation is zero at both joins; no metre-scale hump.
        rings.append([Vector((x, y + (rng.uniform(-.012,.012) if abs(x)<.5 else 0), z))
                      for y, z in section])
    for j in range(4):
        for k in range(count):
            l = (k + 1) % count
            points = [rings[j][k], rings[j+1][k], rings[j+1][l], rings[j][l]]
            uv = [(.01 + (p.x+.5)*.98, .01 + p.z/height*.44) for p in points]
            if k in (4, 5, 6, 11):
                uv = [(.01+(p.x+.5)*.98, .01+(p.y+width*.5)/height*.44) for p in points]
            colors = [(ao(p.z),)*3+(1,) for p in points]
            mesh.face(points, uv, [section_normals[t] for t in (k,k,l,l)], colors, 1)
    for end in (0, 4):
        points = rings[end] if end == 0 else list(reversed(rings[end]))
        n = Vector((-1 if end == 0 else 1, 0, 0))
        uv = [(.01+(p.y+width*.5)*.98, .01+p.z/height*.44) for p in points]
        mesh.face(points, uv, [n]*count, [(ao(p.z),)*3+(1,) for p in points], 1)
    # Four narrow stems use an opaque wood gutter in the foliage atlas, preserving
    # the existing single primitive/material contract and the open ground-level foot.
    for x, y in ((-.36,-.07),(-.12,.08),(.15,-.06),(.39,.07)):
        first_vertex, first_face = len(mesh.vertices), len(mesh.faces)
        mesh.tube([(x,y,0),(x+.015,y,.23)], .011, 3)
        for i in range(first_vertex, len(mesh.vertices)):
            mesh.uv[i] = (.5, .489)
            mesh.colors[i] = (.78,.78,.78,1)
        mesh.slots[first_face:] = [1] * (len(mesh.faces)-first_face)
    # Area-weighted stratification covers both long faces, top, and closed ends.
    weights = np.array((height-.15,height-.15,width,width*height*.60,width*height*.60))
    edges = np.cumsum(weights / weights.sum())
    surfaces = np.searchsorted(edges,(np.arange(378)+.5)/378)
    counts = np.bincount(surfaces,minlength=5)
    seen = [0]*5
    for i in range(378):
        surface = int(surfaces[i])
        local = seen[surface];seen[surface] += 1
        # Bounded low-discrepancy coverage avoids random bare patches repeating
        # every metre. Small independent jitter removes visible diagonal bands.
        s = ((local+.5)/counts[surface]+rng.uniform(-.012,.012)) % 1
        t = (local*.61803398875+.31+surface*.17+rng.uniform(-.022,.022)) % 1
        x = s-.5
        if surface < 2:
            z = .09+t*(height-.155)
            # Interpolate the actual cross-section so no card is buried in the core.
            y = float(np.interp(z, [p[1] for p in section[:6]], [-p[0] for p in section[:6]]))
            y *= -1 if surface == 0 else 1
            p = Vector((x,y,z))
            shoulder = max(0.,(z-(height-.20))/.135)
            n = Vector((0,-1 if surface == 0 else 1,.20+.65*shoulder)).normalized()
        elif surface == 2:
            y = (t-.5)*width*.70
            p = Vector((x,y,height-.025-.025*(abs(y)/(width*.35))**2))
            n = Vector((0,y/width*.7,1)).normalized()
        else:
            p = Vector((-.5 if surface == 3 else .5,(s-.5)*width*.64,.11+t*(height-.20)))
            n = Vector((-1 if surface == 3 else 1,0,.12)).normalized()
        p += n * rng.uniform(.018, .047 if surface < 3 else .025)
        tangent = n.cross(Vector((0,0,1)))
        if tangent.length < .1:
            tangent = Vector((1,0,0))
        tangent.normalize()
        across = n.cross(tangent).normalized()
        angle = rng.uniform(0,TAU)
        u = (tangent*math.cos(angle)+across*math.sin(angle)).normalized()
        v = n.cross(u).normalized()
        # Geometric tilt gives parallax; lighting follows the enclosing soft volume.
        u = (u+n*rng.uniform(-.20,.20)).normalized()
        v = (v+n*rng.uniform(-.20,.20)).normalized()
        size = rng.uniform(.16,.23) * (1.12 if form=='hedge_tall' else 1.)
        points = [p+u*s*size*.5+v*t*size*.5 for s,t in ((-1,-1),(1,-1),(1,1),(-1,1))]
        for q in points:
            q.x = max(-.55,min(.55,q.x))
            q.y = max(-width*.5-.04,min(width*.5+.04,q.y))
            q.z = max(.025,min(height+.035,q.z))
        cell = i % 2
        # Stay away from the opaque lower half when bilinear/mip filtering the cards.
        uv = [(cell*.5+.005+a*.49,.51+b*.48) for a,b in ((0,0),(1,0),(1,1),(0,1))]
        shade = rng.uniform(.96,1.)
        mesh.face(points,uv,[n]*4,[(ao(q.z)*shade,)*3+(1,) for q in points],1)
    obj = mesh.make_object(form+'_0',mesh.vertices,mesh.faces,mesh.uv,mesh.normals,mesh.colors,mesh.slots)
    bsdf = mesh.materials[1].node_tree.nodes.get('Principled BSDF')
    bsdf.inputs['Roughness'].default_value = .48 if form=='hedge_low' else .73
    bsdf.inputs['Specular IOR Level'].default_value = .27 if form=='hedge_low' else .15
    return obj


def rock_textures(out):
    n = 1024
    rng = np.random.default_rng(99712)
    broad, grain, fine = [field(rng,n,f) for f in (6,220,400)]
    mineral = field(rng,n,310)
    # Pale quartz, grey feldspar with a restrained pink cast and dark biotite grains.
    value = .60 + .045*broad + .052*grain + .035*fine
    value -= .14*(mineral < -1.0)
    pink = .018 + np.clip(field(rng,n,48)-.2,0,1)*.09
    rgba = np.ones((n,n,4),np.float32)
    rgba[...,:3] = np.stack((value+pink,value+pink*.12,value-pink*.08),-1)
    trees.png(out/'rock_albedo.png',rgba)
    relief = .00040*grain + .00020*fine + .00030*(mineral < -1.)
    dx = (np.roll(relief,-1,axis=1)-np.roll(relief,1,axis=1))*n/3
    dy = (np.roll(relief,-1,axis=0)-np.roll(relief,1,axis=0))*n/3
    normals = np.stack((-dx,-dy,np.ones_like(dx)),-1)
    normals /= np.linalg.norm(normals,axis=2)[...,None]
    rgba[...,:3] = normals*.5+.5
    trees.png(out/'rock_normal.png',rgba)


def rock_material(out):
    mat = bpy.data.materials.new('rock_stone')
    mat.use_nodes = True
    mat.use_backface_culling = True
    nodes,links = mat.node_tree.nodes,mat.node_tree.links
    bsdf = nodes.get('Principled BSDF')
    bsdf.inputs['Roughness'].default_value = .87
    tex = nodes.new('ShaderNodeTexImage')
    tex.image = bpy.data.images.load(str(out/'rock_albedo.png'))
    col = nodes.new('ShaderNodeVertexColor');col.layer_name='CrownAO'
    mul = nodes.new('ShaderNodeMixRGB');mul.blend_type='MULTIPLY';mul.inputs[0].default_value=1
    links.new(tex.outputs['Color'],mul.inputs[1]);links.new(col.outputs['Color'],mul.inputs[2])
    links.new(mul.outputs[0],bsdf.inputs['Base Color'])
    texn = nodes.new('ShaderNodeTexImage');texn.image=bpy.data.images.load(str(out/'rock_normal.png'))
    texn.image.colorspace_settings.name='Non-Color'
    nm = nodes.new('ShaderNodeNormalMap')
    links.new(texn.outputs['Color'],nm.inputs['Color']);links.new(nm.outputs['Normal'],bsdf.inputs['Normal'])
    return mat


def build_rock(index, mat):
    rng = random.Random(740+index)
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=3,radius=1)
    obj=bpy.context.object;obj.name=f'rock_{index}'
    mesh=obj.data
    # Intersect random broad fracture planes radially, then retain a rounded shoulder.
    planes=[(Vector((1,rng.uniform(-.4,.4),rng.uniform(-.25,.35))).normalized(),rng.uniform(.72,.94)),
            (Vector((-1,rng.uniform(-.4,.4),rng.uniform(-.2,.4))).normalized(),rng.uniform(.70,.98)),
            (Vector((rng.uniform(-.4,.4),1,.15)).normalized(),rng.uniform(.72,.94)),
            (Vector((rng.uniform(-.4,.4),-1,.25)).normalized(),rng.uniform(.72,.94)),
            (Vector((rng.uniform(-.6,.6),rng.uniform(-.5,.5),1)).normalized(),rng.uniform(.68,.9))]
    for _ in range(6):
        n=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.6,1))).normalized()
        planes.append((n,rng.uniform(.77,.98)))
    for v in mesh.vertices:
        p=v.co.normalized()
        limit=min([1.15]+[d/p.dot(n) for n,d in planes if p.dot(n)>.01])
        fracture=(.68,.91,.68,.73,.94,.90)[index]
        r=fracture*limit+(1-fracture)
        rough=.018*math.sin(p.x*13+index)*math.cos(p.y*11+p.z*9)
        v.co=p*(r+rough)
        v.co.x += .12*p.z+.06*p.y*p.z
        v.co.z=max(-.65,v.co.z)
    wx, wy, height = ROCK_DIMS[index]
    fit_mesh(obj, (wx, wy, height + .075), -.075)
    mesh.update()
    uv=mesh.uv_layers.new(name='UVMap')
    color=mesh.color_attributes.new(name='CrownAO',type='FLOAT_COLOR',domain='CORNER')
    normals=[]
    adjacent=[[] for _ in mesh.vertices]
    for poly in mesh.polygons:
        for vi in poly.vertices:adjacent[vi].append(poly.normal.copy())
    h=ROCK_DIMS[index][2]
    for poly in mesh.polygons:
        poly.use_smooth=True
        axis=max(range(3),key=lambda k:abs(poly.normal[k]))
        pair=((1,2),(0,2),(0,1))[axis]
        for li in poly.loop_indices:
            v=mesh.vertices[mesh.loops[li].vertex_index]
            p=v.co
            uv.data[li].uv=(p[pair[0]]/1.5,p[pair[1]]/1.5)
            candidates=[n for n in adjacent[v.index] if n.dot(poly.normal)>(.78 if index in (2,3) else .92)]
            normals.append(sum(candidates,Vector()).normalized())
            patch=math.sin(p.x*4.3+p.y*2.7+index)*math.cos(p.y*5.2+p.z*3.8)+.20*math.sin(p.x*15-p.z*11)
            lichen=max(0,min(1,(patch-.02)*2.7))
            stone=np.array((.48,.46,.43)) if index%2==0 else np.array((.56,.46,.42))
            tint=stone*(1-lichen)+np.array((.99,1.,.92))*lichen
            moss=max(0,1-(p.z+.075)/(h*.65))*max(0,min(1,(patch+.45)*1.8))
            tint=tint*(1-moss*.90)+np.array((.15,.32,.065))*moss*.90
            ao=.70+.30*min(1,max(0,(p.z+.075)/(h*.35)))
            color.data[li].color=(*np.clip(tint*ao,0,1),1)
    mesh.normals_split_custom_set(normals)
    mesh.materials.append(mat)
    return obj


def export_glb(obj,out):
    """Tree export contract, retaining full form names and shared external rock maps."""
    bpy.ops.object.select_all(action='DESELECT');obj.select_set(True)
    bpy.context.view_layer.objects.active=obj
    path=out/f'{obj.name}.glb'
    bpy.ops.export_scene.gltf(filepath=str(path),export_format='GLB',use_selection=True,
        export_yup=True,export_normals=True,export_texcoords=True,
        export_vertex_color='NAME',export_vertex_color_name='CrownAO',
        export_all_vertex_colors=False,export_attributes=False,export_materials='EXPORT',export_image_format='AUTO')
    raw=path.read_bytes();size,kind=struct.unpack_from('<II',raw,12)
    assert kind==0x4E4F534A
    doc=json.loads(raw[20:20+size]);binary=raw[28+size:]
    for mat in doc['materials']:
        if mat['name'].endswith('_foliage'):
            mat.update(alphaMode='MASK',alphaCutoff=.4,doubleSided=True)
    image_views={im['bufferView'] for im in doc['images']}
    packed,views,remap=bytearray(),[],{}
    for index,view in enumerate(doc['bufferViews']):
        if index in image_views:continue
        packed.extend(b'\0'*(-len(packed)%4));remap[index]=len(views)
        views.append(dict(view,byteOffset=len(packed)))
        offset=view.get('byteOffset',0);packed.extend(binary[offset:offset+view['byteLength']])
    for accessor in doc['accessors']:accessor['bufferView']=remap[accessor['bufferView']]
    form=obj.name.removesuffix('_lod1').rsplit('_',1)[0]
    for im in doc['images']:
        suffix='normal' if 'normal' in im['name'] else ('albedo' if form=='rock' else ('foliage' if 'foliage' in im['name'] else 'bark'))
        im.pop('bufferView');im['uri']=f'{form}_{suffix}.png'
    doc['bufferViews']=views;doc['buffers'][0]['byteLength']=len(packed)
    packed.extend(b'\0'*(-len(packed)%4))
    payload=json.dumps(doc,separators=(',',':')).encode();payload+=b' '*(-len(payload)%4)
    tail=struct.pack('<II',len(packed),0x004E4942)+packed
    path.write_bytes(struct.pack('<III',0x46546C67,2,20+len(payload)+len(tail))+
                     struct.pack('<II',len(payload),0x4E4F534A)+payload+tail)


def measure(obj):
    obj.data.calc_loop_triangles()
    a=np.array([tuple(v.co) for v in obj.data.vertices])
    return {'height_m':round(float(a[:,2].max()),4),'vertical_extent_m':round(float(np.ptp(a[:,2])),4),
            'width_x_m':round(float(np.ptp(a[:,0])),4),'width_y_m':round(float(np.ptp(a[:,1])),4),
            'base_z_m':round(float(a[:,2].min()),4),'triangles':len(obj.data.loop_triangles)}


def validate(path,stats):
    """Read back exported attributes, bounds, materials, indices and external PNGs."""
    raw=path.read_bytes();magic,version,total=struct.unpack_from('<III',raw)
    assert (magic,version,total)==(0x46546C67,2,len(raw))
    size=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+size]);binary=raw[28+size:]
    def accessor(index):
        a=doc['accessors'][index];view=doc['bufferViews'][a['bufferView']]
        dtype=np.dtype({5121:'u1',5123:'<u2',5125:'<u4',5126:'<f4'}[a['componentType']])
        width={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']]
        offset=view.get('byteOffset',0)+a.get('byteOffset',0)
        v=np.ndarray((a['count'],width),dtype=dtype,buffer=binary,offset=offset,
                     strides=(view.get('byteStride',width*dtype.itemsize),dtype.itemsize))
        if a.get('normalized'):v=v/np.iinfo(dtype).max
        assert np.isfinite(v).all()
        return v
    form=path.stem.removesuffix('_lod1').rsplit('_',1)[0]
    assert len(doc['meshes'])==len(doc['nodes'])==1
    assert 'matrix' not in doc['nodes'][0] and 'rotation' not in doc['nodes'][0]
    prims=doc['meshes'][0]['primitives']
    # A hedge is one closed foliage body and a rock one stone; every other form has wood and cards.
    assert len(prims)==(1 if form in HEDGES or form=='rock' else 2),(path,len(prims))
    triangles=0;positions=[]
    for prim in prims:
        attr=prim['attributes'];assert {'POSITION','NORMAL','TEXCOORD_0','COLOR_0'}<=attr.keys()
        v,n,c,uv=[accessor(attr[k]) for k in ('POSITION','NORMAL','COLOR_0','TEXCOORD_0')]
        assert np.max(np.abs(np.linalg.norm(n,axis=1)-1))<.002
        assert c.min()>=0 and c.max()<=1 and np.allclose(c[:,3],1)
        indices=accessor(prim['indices']).ravel().astype(int)
        assert len(indices)%3==0 and indices.min()>=0 and indices.max()<len(v)
        tri=v[indices.reshape(-1,3)]
        area=np.linalg.norm(np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0]),axis=1)/2
        assert area.min()>1e-11,(path,area.min())
        triangles+=len(area);positions.append(v)
        mat=doc['materials'][prim['material']]
        assert mat['name'] in ({'rock_stone'} if form=='rock' else {form+'_bark',form+'_foliage'})
        if mat['name'].endswith('_foliage'):
            assert mat['alphaMode']=='MASK' and mat['alphaCutoff']==.4 and mat['doubleSided']
            assert uv.min()>=0 and uv.max()<=1 and c[:,:3].min()<.96
            assert np.allclose(c[:,0],c[:,1]) and np.allclose(c[:,1],c[:,2])
    assert triangles==stats['triangles']
    limit=900 if form in HEDGES else (600 if form=='rock' else (2500 if form=='lilac' else 1500))
    assert triangles<=limit,(path,triangles)
    if form=='rock':assert 250<=triangles<=600
    v=np.concatenate(positions)
    for value,key in [(v[:,1].max(),'height_m'),(v[:,1].min(),'base_z_m'),
                      (np.ptp(v[:,0]),'width_x_m'),(np.ptp(v[:,2]),'width_y_m')]:
        assert abs(value-stats[key])<.002,(path,key,value,stats[key])
    for im in doc['images']:
        assert 'bufferView' not in im and im['uri'].startswith(form+'_')
        data=(path.parent/im['uri']).read_bytes()
        assert data[:8]==b'\x89PNG\r\n\x1a\n'
        assert max(struct.unpack_from('>II',data,16))<=1024
    return hashlib.sha256(raw).hexdigest()


def simple_material(name,color):
    mat=bpy.data.materials.new(name);mat.use_nodes=True
    bsdf=mat.node_tree.nodes.get('Principled BSDF')
    bsdf.inputs['Base Color'].default_value=(*color,1);bsdf.inputs['Roughness'].default_value=.88
    return mat


def box(name,location,scale,mat):
    bpy.ops.mesh.primitive_cube_add(size=1,location=location)
    obj=bpy.context.object;obj.name=name;obj.scale=scale;obj.data.materials.append(mat)
    return obj


def strip_png_metadata(path):
    raw=path.read_bytes();chunks=[raw[:8]];offset=8
    while offset<len(raw):
        length=struct.unpack_from('>I',raw,offset)[0]+12
        if raw[offset+4:offset+8] not in (b'tEXt',b'zTXt',b'iTXt'):
            chunks.append(raw[offset:offset+length])
        offset+=length
    path.write_bytes(b''.join(chunks))


def previews(objects,out):
    for mat in bpy.data.materials:
        if mat.name.endswith('_foliage'):trees.preview_cutoff(mat)
    scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.device='CPU'
    scene.cycles.samples=32;scene.cycles.seed=321;scene.cycles.use_animated_seed=False
    scene.cycles.use_adaptive_sampling=False;scene.cycles.use_denoising=False
    scene.cycles.transparent_max_bounces=32;scene.cycles.max_bounces=4
    scene.render.threads_mode='FIXED';scene.render.threads=12
    scene.render.image_settings.file_format='PNG';scene.view_settings.view_transform='AgX'
    scene.world.use_nodes=True
    bg=scene.world.node_tree.nodes.get('Background');bg.inputs[0].default_value=(.65,.75,.9,1);bg.inputs[1].default_value=.65
    light=bpy.data.lights.new('Preview_sun','SUN');light.energy=2.4;light.angle=math.radians(4)
    sun=bpy.data.objects.new('Preview_sun',light);scene.collection.objects.link(sun)
    sun.location=(-20,-30,40);trees.aim(sun,(0,0,0))
    groundmat=simple_material('Preview_lawn',(.075,.13,.035))
    ground=box('Preview_ground',(0,0,-.12),(180,180,.20),groundmat)
    # Fine ground variation gives the rocks scale without hiding their silhouettes.
    nodes=groundmat.node_tree.nodes;links=groundmat.node_tree.links
    noise=nodes.new('ShaderNodeTexNoise');noise.inputs['Scale'].default_value=3
    position=nodes.new('ShaderNodeNewGeometry')
    links.new(position.outputs['Position'],noise.inputs['Vector'])
    ramp=nodes.new('ShaderNodeValToRGB')
    ramp.color_ramp.elements[0].color=(.035,.058,.015,1);ramp.color_ramp.elements[1].color=(.12,.17,.048,1)
    ramp.color_ramp.elements[0].position=.25;ramp.color_ramp.elements[1].position=.75
    links.new(noise.outputs['Fac'],ramp.inputs[0]);links.new(ramp.outputs[0],nodes.get('Principled BSDF').inputs['Base Color'])
    camdata=bpy.data.cameras.new('Preview_camera');camera=bpy.data.objects.new('Preview_camera',camdata)
    scene.collection.objects.link(camera);scene.camera=camera
    directory=out/'previews';directory.mkdir(exist_ok=True)
    times={}
    def render(name,width=800,height=600):
        scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.resolution_percentage=100
        scene.render.filepath=str(directory/f'{name}.png')
        start=time.perf_counter();bpy.ops.render.render(write_still=True)
        strip_png_metadata(directory/f'{name}.png');times[name]=round(time.perf_counter()-start,3)
        print('PREVIEW '+name,flush=True)
    for obj in objects:
        for other in objects:other.hide_render=other!=obj
        rock=obj.name.startswith('rock_');distance=5 if rock else 8
        camera.location=(distance*.38,-distance*math.sqrt(1-.38**2),1.65)
        trees.aim(camera,(0,0,max(.20,obj.dimensions.z*.45)))
        camdata.lens=48 if rock else 52
        render(obj.name+'_eye',640,640)
    # Contact sheet uses a five-column ordering matching the report, plus image labels.
    glyphs={'A':'0e11111f111111','B':'1e111e1111111e','C':'0f10101010100f','D':'1e11111111111e',
            'E':'1f10101e10101f','F':'1f10101e101010','G':'0e11101711110f','H':'1111111f111111',
            'I':'0e04040404040e','J':'0702020212120c','K':'11121418141211','L':'1010101010101f',
            'M':'111b1515111111','N':'11191915131311','O':'0e11111111110e','P':'1e11111e101010',
            'R':'1e11111e141211','S':'0f10100e01011e','T':'1f040404040404','U':'1111111111110e',
            'W':'11111115151b11','_':'0000000000001f','0':'0e11131519110e','1':'040c040404040e',
            '2':'0e11010204081f','3':'1e01010601011e','4':'02060a121f0202','5':'1f101e0101110e'}
    sheet=np.ones((3*350,5*320,4),np.float32);sheet[...,:3]=(.75,.78,.80)
    for i,obj in enumerate(objects):
        im=bpy.data.images.load(str(directory/f'{obj.name}_eye.png'),check_existing=False)
        im.colorspace_settings.name='Non-Color'
        pixels=np.empty(640*640*4,np.float32);im.pixels.foreach_get(pixels)
        tile=pixels.reshape(320,2,320,2,4).mean(axis=(1,3));x0=i%5*320;y0=(2-i//5)*350
        sheet[y0+28:y0+348,x0:x0+320]=tile
        label=obj.name.upper();left=x0+(320-len(label)*12)//2
        for k,letter in enumerate(label):
            for y,row in enumerate(bytes.fromhex(glyphs[letter])):
                for x in range(5):
                    if row&(1<<(4-x)):sheet[y0+7+(6-y)*2:y0+9+(6-y)*2,left+k*12+x*2:left+k*12+x*2+2,:3]=.07
        bpy.data.images.remove(im)
    trees.png(directory/'contact.png',sheet)
    for obj in objects:obj.hide_render=True
    sources={o.name:o for o in objects};yard=[]
    def instance(name,location,yaw=0):
        src=sources[name];o=bpy.data.objects.new('Preview_'+name,src.data);scene.collection.objects.link(o)
        o.location=location;o.rotation_euler.z=yaw;yard.append(o);return o
    # Actual one-metre instancing, unchanged yaw, eye 1.65 m above the ground.
    # Both views include the entire 12 m row; 6 m uses a wider lens.
    for i in range(12):instance('hedge_low_0',(i-5.5,0,0))
    for distance in (6,15):
        camera.location=(0,-distance,1.65);trees.aim(camera,(0,0,.48));camdata.lens=16 if distance==6 else 40
        render(f'hedge_low_row_12m_eye_{distance}m',1400,700)
    for o in yard:bpy.data.objects.remove(o,do_unlink=True)
    yard.clear()
    red=simple_material('Preview_red_wood',(.29,.035,.021));white=simple_material('Preview_trim',(.76,.76,.68))
    roofmat=simple_material('Preview_roof',(.09,.09,.085));glass=simple_material('Preview_windows',(.12,.23,.27))
    gravel=simple_material('Preview_gravel',(.27,.25,.21));roadmat=simple_material('Preview_street',(.12,.13,.13))
    yard.append(box('Preview_house',(0,5,1.6),(8,6,3.2),red))
    verts=[(-4.5,1.5,3.2),(4.5,1.5,3.2),(-4.5,8.5,3.2),(4.5,8.5,3.2),(-4.5,5,5),(4.5,5,5)]
    mesh=bpy.data.meshes.new('Preview_roof');mesh.from_pydata(verts,[],[(0,1,5,4),(4,5,3,2),(0,4,2),(1,3,5)])
    roof=bpy.data.objects.new('Preview_roof',mesh);scene.collection.objects.link(roof);mesh.materials.append(roofmat);yard.append(roof)
    for x in (-2.5,2.5):
        yard.append(box('Preview_window_frame',(x,1.975,1.9),(1.5,.08,1.5),white))
        yard.append(box('Preview_window',(x,1.925,1.9),(1.24,.03,1.24),glass))
    yard.append(box('Preview_door',(0,1.94,1.1),(1.05,.09,2.2),white))
    yard.append(box('Preview_drive',(7,2,-.005),(3.2,18,.025),gravel))
    yard.append(box('Preview_street',(0,-8,-.005),(65,5,.025),roadmat))
    for i in range(20):instance('hedge_low_0',(-11+i,-5,0))
    for i in range(8):instance('hedge_tall_0',(-11, -4.5+i,0),math.pi/2)
    for name,pos,yaw in [('lilac_0',(-4.7,1.5,0),.4),('spirea_0',(-2.8,1,0),.1),
        ('rose_0',(2.8,.9,0),-.4),('cotoneaster_0',(-7,-1,0),.2),('juniper_0',(4.7,5,0),0),
        ('mugo_0',(4.5,-1.6,0),.4),('mugo_0',(4.3,-3.1,0),1.8),('mugo_0',(6,-3.4,0),2.7),
        ('rock_0',(2,-3,0),0),('rock_2',(-5.8,-2.8,0),.4),('rock_3',(3,-1.8,0),1.1),('rock_5',(-7,-3.4,0),.7)]:
        instance(name,pos,yaw)
    camera.location=(3,-15,1.65);trees.aim(camera,(0,1,1.5));camdata.lens=23
    render('yard_street_15m',1000,700)
    target=Vector((0,1,0));elevation=math.radians(35);az=math.radians(18)
    camera.location=target+Vector((40*math.cos(elevation)*math.sin(az),-40*math.cos(elevation)*math.cos(az),40*math.sin(elevation)))
    trees.aim(camera,target);camdata.lens=43;render('yard_40m_35deg',1000,700)
    for o in yard:o.hide_render=True
    # Same six rocks, no scaling, nestled into a mottled moss/needle floor.
    ramp.color_ramp.elements[0].color=(.032,.027,.013,1);ramp.color_ramp.elements[1].color=(.105,.14,.035,1)
    for i in range(6):instance(f'rock_{i}',((-2.4,0,2.1,-2,1.2,3)[i],(0,-1,0,2,3,2)[i],0),i*.72)
    camera.location=(6,-8,2.1);trees.aim(camera,(.4,1,.5));camdata.lens=37
    render('forest_floor_rocks',1000,700)
    return times


def report(out,stats,times,elapsed):
    metrics={'blender':bpy.app.version_string,'script_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
             'models':stats,'render_seconds':times,'total_seconds':round(elapsed,3)}
    (out/'metrics.json').write_text(json.dumps(metrics,indent=2)+'\n')
    lines=['# Finnish yard landscape assets','',
           'Metres; +Y up in glTF, ground origin. Width Y below refers to Blender Y (glTF Z). '
           'Height is the top above ground; rock vertical extent includes the 7.5 cm buried foot.','',
           '| Model | Height m | Width X m | Width Y m | Triangles |','|---|---:|---:|---:|---:|']
    for name,s in stats.items():
        lines.append(f'| {name} | {s["height_m"]:.3f} | {s["width_x_m"]:.3f} | {s["width_y_m"]:.3f} | {s["triangles"]} |')
    lines+=['','21 GLBs: six shrubs and six reduced levels, three closed hedge modules, six rocks. '
            'Keep all 29 external PNG textures beside the GLBs. Nine foliage atlases and normal maps are 1024²; '
            'nine bark maps 256²; the two shared rock maps 1024². Hedge bark maps are delivered but unused.',
            '', 'Foliage is double-sided MASK 0.4. Shrub crown AO uses the existing tree helper’s 32 fixed '
            'alpha-aware sky rays per card; LOD cards inherit AO. Hedge vertex shading darkens the lower body. '
            'Rock vertex RGB combines a base-occlusion factor, pale lichen and lower moss multipliers, all in [0,1]. '
            'Stone uses face-dominant box UV projection at one repeat per 1.5 m, with an OpenGL +Y normal map.',
            '', 'The hedge body is a closed bevelled extrusion with identical end cross-sections exactly at X ±0.5 m. '
            '378 leaf cards use 756 of 896 triangles, with at most 5 cm X overhang. '
            'The rounded core uses 116 triangles; four basal stems use 24 and sample the foliage atlas wood gutter. '
            'Above 28 cm the AO multiplier is 0.94–0.98, independent of side orientation. '
            'No per-module macro noise is added. '
            'Rock shapes use multiple fracture planes blended into rounded shoulders, with selective normal splitting.',
            '', 'Offline geometry generation is O(vertices + texture pixels); crown AO uses the tree helper’s bounded '
            'BVH ray traversal. No game runtime code or dependencies changed. Export validation reads serialized '
            'GLBs and checks budgets, primitive counts, material names, attributes, finite unit normals, nondegenerate '
            'triangles, colours, Y-up bounds and external PNG dimensions.',
            '', 'Cycles CPU: 12 threads, 32 fixed samples, no adaptive sampling/denoising, 32 transparent bounces. '
            'Solo cameras are 1.65 m high, 8 m from shrubs/hedges and 5 m from rocks. Yard uses 20 identical low '
            'hedge modules at 1 m spacing and eight tall modules at 1 m spacing. Forest-floor view contains all six rocks. '
            'Two additional 1400×700 previews show a 12 m low hedge row at eye level from 6 m and 15 m.',
            '', f'Fresh build and render time: {elapsed:.3f} seconds. Per-render times are in metrics.json.',
            '', 'Reproduce: `imgs/landscape_models/mac_build.sh`. Asset-only check: add `--skip-previews`.',
            '', '## Visual review','', 'See review_notes.txt for the authored comparison with the reference photos. '
            'Procedural textures are original; reference photographs supply shape and colour targets only.','']
    (out/'REPORT.md').write_text('\n'.join(lines))
    print('LANDSCAPE_BUILD '+json.dumps({'models':len(stats),'seconds':round(elapsed,3)}),flush=True)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('out_dir',type=Path)
    parser.add_argument('--skip-previews',action='store_true')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);start=time.perf_counter()
    out=args.out_dir.resolve();out.mkdir(parents=True,exist_ok=True)
    bpy.ops.object.select_all(action='SELECT');bpy.ops.object.delete(use_global=False)
    objects=[];stats={}
    for form in FORMS:
        alpha=foliage_atlas(form,FORMS[form][-1],out)
        obj,lod=build_shrub(form,out,alpha)
        for asset in (obj,lod):
            s=measure(asset);export_glb(asset,out);s['glb_sha256']=validate(out/f'{asset.name}.glb',s);stats[asset.name]=s
        ratio=stats[lod.name]['triangles']/stats[obj.name]['triangles']
        assert .16<=ratio<=.34,(form,ratio)
        bpy.data.objects.remove(lod,do_unlink=True);objects.append(obj)
    for form in HEDGES:
        alpha=foliage_atlas(form,HEDGES[form][-1],out);obj=build_hedge(form,out,alpha)
        s=measure(obj);export_glb(obj,out);s['glb_sha256']=validate(out/f'{obj.name}.glb',s);stats[obj.name]=s;objects.append(obj)
    rock_textures(out);mat=rock_material(out)
    for i in range(6):
        obj=build_rock(i,mat);s=measure(obj);export_glb(obj,out)
        s['glb_sha256']=validate(out/f'{obj.name}.glb',s);stats[obj.name]=s;objects.append(obj)
    times={} if args.skip_previews else previews(objects,out)
    report(out,stats,times,time.perf_counter()-start)


if __name__=='__main__':
    main()
