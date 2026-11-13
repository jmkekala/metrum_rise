# SPDX-License-Identifier: GPL-2.0-only
"""Reads the stream benchmark's draw census (`RENDER-44`, `godot/scripts/benchmarks/draw_census.gd`;
`tools/stream_benchmark.py census`): per pose, the main pass's primitives by layer and distance
band, the instances drawn off-screen inside drawn batches, triangles per screen pixel and the
level of detail drawn against each instance's own, and with a depth image the drawn instances
hidden behind nearer opaque geometry.

It mirrors what Godot 4's Forward+ renderer does with each geometry instance (unmodified engine,
`renderer_scene_cull.cpp`, `render_forward_clustered.cpp`):

- An instance is culled whole by its world bounds (a MultiMesh's bounds are those of every
  instance it holds, so a batch draws all its instances or none) against the camera frustum, and
  by its visibility range from the camera to the bounds' centre.
- A drawn surface's primitives are its index count over three, times the MultiMesh's visible
  instances, at the level of detail picked once for the whole instance: the coarsest whose edge
  length stays within `mesh_lod_threshold` pixels at the distance from the camera to the bounds'
  nearest point along the view direction (zero when the camera is inside the bounds).
- The census's own prediction of the main pass's primitives is checked against Godot's counter
  (`godot.visible`) and each layer's against the counter with the layer hidden (`layers`).

A band is the distance from the camera to an instance's bounds' centre. An instance is off-screen
when its own bounds miss the frustum; its "own" level of detail is the one Godot would pick for it
alone. Screen pixels are the instance bounds' projected rectangle clipped to the screen, so
triangles per pixel is a lower bound. Occluded: every depth pixel of that rectangle is nearer than
the bounds' nearest point (the depth image holds the opaque geometry; the water, transparent,
does not occlude here). The graded cells move their vertices in the shader and carry custom
bounds the census cannot read, so their band is right and their culling approximate.
"""

import base64
import json
import math
from pathlib import Path

BANDS = [0, 50, 100, 200, 300, 500, 1000, 2000, 4000, math.inf]
CORNERS = [(x, y, z) for x in (0, 1) for y in (0, 1) for z in (0, 1)]


def band_names():
    return [f"{a}-{b}" if b != math.inf else f"{a}+" for a, b in zip(BANDS, BANDS[1:])]


def mesh_info(surfaces):
    """Merged local bounds (min, max) and per surface (triangles, [(edge, triangles)...])."""
    import numpy as np
    lo = np.full(3, np.inf)
    hi = np.full(3, -np.inf)
    parts = []
    for surface in surfaces:
        box = surface["aabb"]
        lo = np.minimum(lo, box[:3])
        hi = np.maximum(hi, np.add(box[:3], box[3:]))
        if surface["primitive"] == 3:
            triangles = (surface["indices"] or surface["vertices"]) // 3
        elif surface["primitive"] == 4:
            triangles = max(0, (surface["indices"] or surface["vertices"]) - 2)
        else:
            triangles = 0
        parts.append((triangles, [(edge, count // 3) for edge, count in surface["lods"]]))
    if not surfaces:
        lo = hi = np.zeros(3)
    return lo, hi, parts


class Camera:
    def __init__(self, data):
        import numpy as np
        t = np.array(data["transform"], dtype=float).reshape(3, 4)
        self.basis = t[:, :3]
        self.origin = t[:, 3]
        self.width, self.height = data["size"]
        self.near, self.far = data["near"], data["far"]
        aspect = self.width / self.height
        half = math.radians(data["fov"]) / 2
        if data.get("keep_aspect", 1) == 1:
            self.tan_v = math.tan(half)
            self.tan_h = self.tan_v * aspect
        else:
            self.tan_h = math.tan(half)
            self.tan_v = self.tan_h / aspect
        self.threshold = data.get("lod_threshold", 1.0)

    def view(self, points):
        """World points (..., 3) to view space (x right, y up, z back)."""
        return (points - self.origin) @ self.basis

    def outside(self, view_corners):
        """Per box (N, 8, 3 in view space): all corners beyond one frustum plane."""
        x, y, z = view_corners[..., 0], view_corners[..., 1], view_corners[..., 2]
        depth = -z
        tests = [depth < self.near, depth > self.far, x > depth * self.tan_h, x < -depth * self.tan_h,
                 y > depth * self.tan_v, y < -depth * self.tan_v]
        return sum(test.all(axis=-1).astype(int) for test in tests) > 0

    def screen_rect(self, view_corners):
        """Per box: [x0, y0, x1, y1] in pixels clipped to the screen (the whole screen when a
        corner lies behind the near plane) and the nearest view depth."""
        import numpy as np
        depth = -view_corners[..., 2]
        safe = np.maximum(depth, self.near)
        sx = (view_corners[..., 0] / (safe * self.tan_h) * 0.5 + 0.5) * self.width
        sy = (0.5 - view_corners[..., 1] / (safe * self.tan_v) * 0.5) * self.height
        rect = np.stack([sx.min(-1), sy.min(-1), sx.max(-1), sy.max(-1)], axis=-1)
        behind = (depth < self.near).any(-1)
        rect[behind] = [0, 0, self.width, self.height]
        rect[:, 0::2] = rect[:, 0::2].clip(0, self.width)
        rect[:, 1::2] = rect[:, 1::2].clip(0, self.height)
        return rect, depth.min(-1)

    def lod_distance(self, world_lo, world_hi):
        """Godot's distance for a level of detail: to the bounds' support point along the view's
        back axis, zero when the camera is inside the bounds."""
        import numpy as np
        back = self.basis[:, 2]
        support = np.where(back >= 0, world_hi, world_lo)
        distance = np.linalg.norm(support - self.origin, axis=-1)
        inside = ((self.origin >= world_lo) & (self.origin <= world_hi)).all(-1)
        return np.where(inside, 0.0, distance)

    def pick(self, lods, distance, scale=1.0):
        """Triangles of the level Godot picks at `distance` (the full mesh's when it has none)."""
        import numpy as np
        out = np.full(np.shape(distance), -1, dtype=int)
        limit = self.threshold / self.width
        lod_multiplier = 2.0 * self.tan_h
        with np.errstate(divide="ignore"):
            for index, (edge, _) in enumerate(lods):
                size = edge * scale / (np.asarray(distance) * lod_multiplier)
                out = np.where((size <= limit) & (out == index - 1), index, out)
        return out


def box_corners(lo, hi):
    import numpy as np
    return np.array([[hi[i] if c[i] else lo[i] for i in range(3)] for c in CORNERS])


def transform_boxes(rows, lo, hi):
    """Boxes (N rows of 12 floats, Godot's row-major basis with the origin in column 4) of the
    local box: world corners (N, 8, 3) and world bounds."""
    import numpy as np
    m = rows.reshape(-1, 3, 4)
    corners = np.einsum("nij,kj->nki", m[:, :, :3], box_corners(lo, hi)) + m[:, None, :, 3]
    return corners, corners.min(1), corners.max(1)


def analyse(path, depth_path=None):
    """One pose's census as tables (see the module's docstring)."""
    import numpy as np
    census = json.loads(Path(path).read_text())
    camera = Camera(census["camera"])
    meshes = {key: mesh_info(value) for key, value in census["meshes"].items()}
    depth = None
    if depth_path and Path(depth_path).exists():
        from PIL import Image
        image = np.asarray(Image.open(depth_path).convert("RGB")).astype(np.float64)
        depth = np.exp2((image[..., 0] * 256 + image[..., 1]) / 65535.0 * 20.0 - 4.0)
    nbands = len(BANDS) - 1
    layers = {}

    def entry(layer):
        return layers.setdefault(layer, {
            "batches": 0, "batches_drawn": 0, "surfaces_drawn": 0, "instances": 0, "drawn": 0,
            "offscreen": 0, "offscreen_prims": 0.0, "prims": np.zeros(nbands), "own_prims": np.zeros(nbands),
            "lod0_prims": np.zeros(nbands), "pixels": np.zeros(nbands), "onscreen": np.zeros(nbands),
            "subpixel": np.zeros(nbands), "subpixel_prims": np.zeros(nbands), "occluded": 0,
            "occluded_prims": np.zeros(nbands), "tested": 0, "batch_sizes": [], "batches_occluded": 0,
            "batch_occluded_prims": 0.0})

    for item in census["items"]:
        if item.get("kind") not in ("mesh", "multimesh") or str(item.get("mesh")) not in meshes:
            continue
        rs = "instance" in item
        shadow_only = item.get("cast") == (2 if rs else 3)
        if shadow_only or item.get("count", 0) <= 0:
            continue
        lo, hi, parts = meshes[str(item["mesh"])]
        stats = entry(item["layer"])
        stats["batches"] += 1
        transform = np.array(item["transform"], dtype=float)
        box = item["aabb"]
        margin = item.get("margin", 0.0)
        batch_lo = np.subtract(box[:3], margin)
        batch_hi = np.add(np.add(box[:3], box[3:]), margin)
        corners, world_lo, world_hi = transform_boxes(transform, batch_lo, batch_hi)
        centre = (world_lo + world_hi) * 0.5
        distance = float(np.linalg.norm(centre[0] - camera.origin))
        begin, end = item.get("range", [0.0, 0.0])
        in_range = not ((end > 0 and distance >= end) or (begin > 0 and distance < begin))
        drawn = in_range and not camera.outside(camera.view(corners))[0]
        scale = float(np.linalg.norm(transform.reshape(3, 4)[:, :3], axis=0).max())
        batch_distance = camera.lod_distance(world_lo, world_hi)[0]
        # Instances: a MultiMesh's rows composed with the item's transform, or the node itself.
        if item["kind"] == "multimesh":
            rows = np.frombuffer(base64.b64decode(item["rows"]), dtype=np.float32).astype(float)
            rows = rows.reshape(-1, 12)
            outer = transform.reshape(3, 4)
            local = rows.reshape(-1, 3, 4)
            basis = np.einsum("ij,njk->nik", outer[:, :3], local[:, :, :3])
            origin = np.einsum("ij,nj->ni", outer[:, :3], local[:, :, 3]) + outer[:, 3]
            rows = np.concatenate([basis, origin[:, :, None]], axis=2).reshape(-1, 12)
        else:
            rows = transform[None, :]
        count = rows.shape[0]
        stats["instances"] += count
        if not drawn:
            continue
        stats["batches_drawn"] += 1
        stats["surfaces_drawn"] += len(parts)
        stats["drawn"] += count
        stats["batch_sizes"].append(count)
        icorners, ilo, ihi = transform_boxes(rows, lo, hi)
        icentre = (ilo + ihi) * 0.5
        idist = np.linalg.norm(icentre - camera.origin, axis=1)
        bands = np.clip(np.searchsorted(BANDS, idist, side="right") - 1, 0, nbands - 1)
        view_corners = camera.view(icorners)
        off = camera.outside(view_corners)
        own_distance = camera.lod_distance(ilo, ihi)
        prims = np.zeros(count)
        own = np.zeros(count)
        full = np.zeros(count)
        for triangles, lods in parts:
            batch_pick = camera.pick(lods, batch_distance, scale)
            own_pick = camera.pick(lods, own_distance, scale)
            counts = np.array([triangles] + [t for _, t in lods], dtype=float)
            prims += counts[int(batch_pick) + 1]
            own += counts[own_pick + 1]
            full += triangles
        np.add.at(stats["prims"], bands, prims)
        np.add.at(stats["own_prims"], bands, own)
        np.add.at(stats["lod0_prims"], bands, full)
        stats["offscreen"] += int(off.sum())
        stats["offscreen_prims"] += float(prims[off].sum())
        on = ~off
        rect, nearest = camera.screen_rect(view_corners)
        area = (rect[:, 2] - rect[:, 0]) * (rect[:, 3] - rect[:, 1])
        np.add.at(stats["pixels"], bands[on], area[on])
        np.add.at(stats["onscreen"], bands[on], 1)
        small = on & (np.maximum(rect[:, 2] - rect[:, 0], rect[:, 3] - rect[:, 1]) < 1.0)
        np.add.at(stats["subpixel"], bands[small], 1)
        np.add.at(stats["subpixel_prims"], bands[small], prims[small])
        if depth is not None:
            # Sub-pixel instances are counted apart: whether they cover a pixel centre at all is
            # rasterization, not occlusion.
            hidden = 0
            for index in np.nonzero(on & ~small)[0]:
                x0, y0, x1, y1 = rect[index]
                xa, ya = int(math.floor(x0)), int(math.floor(y0))
                xb, yb = max(xa + 1, int(math.ceil(x1))), max(ya + 1, int(math.ceil(y1)))
                window = depth[min(ya, depth.shape[0] - 1):yb, min(xa, depth.shape[1] - 1):xb]
                stats["tested"] += 1
                if window.size and window.max() < nearest[index] * 0.995 - 0.25:
                    hidden += 1
                    stats["occluded"] += 1
                    stats["occluded_prims"][bands[index]] += prims[index]
            # A whole batch hidden (every instance off-screen or occluded, none sub-pixel): what
            # Godot's own occlusion culling, which culls an instance whole, could remove at best.
            if hidden and hidden + int(off.sum()) == count:
                stats["batches_occluded"] += 1
                stats["batch_occluded_prims"] += float(prims.sum())
    return census, layers


def godot_layers(census):
    """Per layer, the main pass's primitives and draws Godot lost with that layer hidden."""
    counters = census.get("layers", {})
    full = counters.get("full", {}).get("visible", [0, 0, 0])
    # The GPU reference: the frames before the toggles and after the restore.
    if "restored" in counters:
        counters = dict(counters)
        counters["full"] = dict(counters["full"], gpu_ms=(counters["full"]["gpu_ms"] + counters["restored"]["gpu_ms"]) / 2)
    out = {}
    for key, value in counters.items():
        if not key.startswith("hidden_") or "counters" not in value:
            continue
        hidden = value["counters"]["visible"]
        out[", ".join(value["layers"])] = (full[0] - hidden[0], full[2] - hidden[2],
                                           counters["full"]["gpu_ms"] - value["counters"]["gpu_ms"])
    return out


def mega(value):
    return f"{value / 1e6:.2f}"


def report(directory, write=print):
    """Prints every pose's tables of the census directory."""
    import numpy as np
    names = band_names()
    for path in sorted(Path(directory).glob("*.census.json")):
        label = path.name[: -len(".census.json")]
        census, layers = analyse(path, path.with_name(label + ".depth.png"))
        g = census["godot"]
        predicted = sum(stats["prims"].sum() for stats in layers.values())
        write(f"## {label}: Godot main pass {g['visible'][0]:.0f} draws, {g['visible'][1]:.0f} objects, "
              f"{mega(g['visible'][2])} M primitives (census {mega(predicted)} M); shadow pass "
              f"{g['shadow'][0]:.0f} draws, {mega(g['shadow'][2])} M; GPU {g['gpu_ms']:.2f} ms")
        write("")
        write("Main-pass primitives (M) by layer and band (census, Godot's level of detail); Godot's own "
              "primitives and draws of the layer (its counters with the layer hidden from the camera) and the "
              "GPU time that saves (the layer also stops casting into the sun's shadows):")
        write("")
        write("| Layer | " + " | ".join(names) + " | total | Godot M | Godot draws | GPU ms |")
        write("|---" * (len(names) + 5) + "|")
        hidden = godot_layers(census)
        ordered = sorted(layers.items(), key=lambda kv: -kv[1]["prims"].sum())
        for layer, stats in ordered:
            match = next((value for key, value in hidden.items() if layer in key.split(", ")), None)
            godot = f"{mega(match[1])} | {match[0]:.0f} | {match[2]:.2f}" if match else "- | - | -"
            write(f"| {layer} | " + " | ".join(mega(v) for v in stats["prims"]) + f" | {mega(stats['prims'].sum())} | {godot} |")
        write("")
        write("Sub-pixel instances are left out of the occlusion test (`occluded` counts instances of a pixel or more).")
        write("")
        write("| Layer | batches | instances drawn | off-screen | off-screen M | batch size | tri/px | sub-px M | over own LOD M | occluded | occluded M | batches occluded (M) |")
        write("|---" * 12 + "|")
        for layer, stats in ordered:
            sizes = sorted(stats["batch_sizes"]) or [0]
            pixels = stats["pixels"].sum()
            tri_px = stats["prims"].sum() / pixels if pixels else float("nan")
            occluded = f"{stats['occluded']}/{stats['tested']}" if stats["tested"] else "-"
            write(f"| {layer} | {stats['batches_drawn']}/{stats['batches']} ({stats['surfaces_drawn']} draws) | {stats['drawn']} | {stats['offscreen']} | "
                  f"{mega(stats['offscreen_prims'])} | {sizes[len(sizes) // 2]}, {sizes[-1]} | {tri_px:.2f} | "
                  f"{mega(stats['subpixel_prims'].sum())} | {mega(stats['prims'].sum() - stats['own_prims'].sum())} | "
                  f"{occluded} | {mega(stats['occluded_prims'].sum())} | "
                  f"{stats['batches_occluded']} ({mega(stats['batch_occluded_prims'])}) |")
        write("")
        write("Triangles a screen pixel by band (all layers; sub-pixel instances, M primitives over own LOD):")
        write("")
        prims = sum(stats["prims"] for stats in layers.values())
        pixels = sum(stats["pixels"] for stats in layers.values())
        over = sum(stats["prims"] - stats["own_prims"] for stats in layers.values())
        sub = sum(stats["subpixel_prims"] for stats in layers.values())
        occ = sum(stats["occluded_prims"] for stats in layers.values())
        write("| | " + " | ".join(names) + " |")
        write("|---" * (len(names) + 1) + "|")
        write("| primitives M | " + " | ".join(mega(v) for v in prims) + " |")
        write("| screen px (M, boxes) | " + " | ".join(f"{v / 1e6:.2f}" for v in pixels) + " |")
        write("| tri/px | " + " | ".join(f"{p / x:.1f}" if x else "-" for p, x in zip(prims, pixels)) + " |")
        write("| sub-pixel M | " + " | ".join(mega(v) for v in sub) + " |")
        write("| over own LOD M | " + " | ".join(mega(v) for v in over) + " |")
        if any(stats["tested"] for stats in layers.values()):
            write("| occluded M | " + " | ".join(mega(v) for v in occ) + " |")
        write("")
    return 0
