#!/usr/bin/env python3
"""
Vertex Animation Texture (VAT) baker for Blender source files.

Usage:
  blender --background <char.blend> --python tools/bake_vat_blend.py -- \
      <anim.blend> <char_name> <output_dir> [num_frames] [idle.blend] \
      [--action Walk] [--idle-action Idle] [--target-height 1.8]

The character .blend is opened as the main scene. The animation .blend may be
the same file or a separate file. Runtime output is a static Godot Y-up rest
mesh plus a float EXR of per-vertex animation offsets.
"""

import argparse
import json
import math
import os
import sys

import bpy
from mathutils import Vector


VERTEX_ID_UV_LAYER = "UVMap_VertexID"


def blender_to_godot(dx, dy, dz):
    """Blender Z-up -> Godot/GLTF Y-up: gX=bX, gY=bZ, gZ=-bY."""
    return (dx, dz, -dy)


def parse_args(argv):
    parser = argparse.ArgumentParser(description="Bake a Blender character walk cycle to VAT.")
    parser.add_argument("walk_blend")
    parser.add_argument("char_name")
    parser.add_argument("output_dir")
    parser.add_argument("num_frames", nargs="?", type=int, default=30)
    parser.add_argument("idle_blend", nargs="?")
    parser.add_argument("--action", default=None, help="Exact walk action name to bake.")
    parser.add_argument("--idle-action", default=None, help="Exact idle/rest action name.")
    parser.add_argument(
        "--target-height",
        type=float,
        default=0.0,
        help="Normalize exported character height in metres. 0 keeps source scale.",
    )
    return parser.parse_args(argv)


def same_file(a, b):
    if not a or not b:
        return False
    try:
        return os.path.samefile(a, b)
    except OSError:
        return os.path.abspath(a) == os.path.abspath(b)


def load_actions_from_blend(path, label):
    """Returns action names available from path, loading them if needed."""
    path = os.path.abspath(path)
    current_path = os.path.abspath(bpy.data.filepath) if bpy.data.filepath else ""

    if same_file(path, current_path):
        names = set(bpy.data.actions.keys())
        print(f"[VAT] Using actions already present in current {label}: {sorted(names)}")
        return names

    print(f"[VAT] Loading actions from {label}: {path}")
    actions_before = set(bpy.data.actions.keys())
    with bpy.data.libraries.load(path, link=False) as (data_from, data_to):
        print(f"[VAT] Available actions in {label}: {data_from.actions}")
        data_to.actions = list(data_from.actions)

    new_names = set(bpy.data.actions.keys()) - actions_before
    print(f"[VAT] Imported {label} actions: {sorted(new_names)}")
    return new_names


def select_action(action_names, requested, suffix, label):
    if requested:
        if requested not in bpy.data.actions:
            print(f"ERROR: requested {label} action '{requested}' was not found")
            print(f"Available actions: {sorted(action_names)}")
            sys.exit(1)
        return bpy.data.actions[requested]

    suffix_lower = suffix.lower()
    candidates = [
        bpy.data.actions[name]
        for name in action_names
        if name.lower() == suffix_lower or name.lower().endswith("_" + suffix_lower)
    ]
    if not candidates:
        candidates = [bpy.data.actions[name] for name in action_names]

    if not candidates:
        print(f"ERROR: no {label} actions found")
        sys.exit(1)

    return max(candidates, key=lambda a: a.frame_range[1] - a.frame_range[0])


def assign_action(armature, action):
    if not armature:
        return

    armature.animation_data_create()
    anim_data = armature.animation_data
    anim_data.action = action

    if hasattr(anim_data, "action_slot"):
        slot = None
        for candidate in action.slots:
            if candidate.target_id_type == "OBJECT":
                slot = candidate
                break
        if slot is None and hasattr(action, "slots"):
            slot = action.slots.new(id_type="OBJECT", name=armature.name)
        if slot is not None:
            anim_data.action_slot = slot
            print(
                f"[VAT] Blender 5 slot assigned to {armature.name} "
                f"target_id_type={slot.target_id_type}"
            )
        else:
            print("[VAT] WARNING: could not create action slot")


def ensure_vat_uv_layers(mesh_data):
    """Ensures UV0 exists and UV1 stores stable geometric vertex IDs."""
    while len(mesh_data.uv_layers) > 1:
        mesh_data.uv_layers.remove(mesh_data.uv_layers[-1])

    if len(mesh_data.uv_layers) == 0:
        mesh_data.uv_layers.new(name="UVMap")

    primary = mesh_data.uv_layers[0]
    for loop in mesh_data.loops:
        primary.data[loop.index].uv = (0.5, 0.5)

    if VERTEX_ID_UV_LAYER in mesh_data.uv_layers:
        mesh_data.uv_layers.remove(mesh_data.uv_layers[VERTEX_ID_UV_LAYER])

    uv1 = mesh_data.uv_layers.new(name=VERTEX_ID_UV_LAYER)
    num_geo_verts = len(mesh_data.vertices)
    for poly in mesh_data.polygons:
        for loop_idx in poly.loop_indices:
            vi = mesh_data.loops[loop_idx].vertex_index
            uv1.data[loop_idx].uv = ((vi + 0.5) / num_geo_verts, 0.5)
    print(f"[VAT] UV layers ready. num_geo_verts={num_geo_verts}")


def evaluated_mesh_at_frame(mesh_obj, frame):
    scene = bpy.context.scene
    scene.frame_set(frame)
    depsgraph = bpy.context.evaluated_depsgraph_get()
    eval_obj = mesh_obj.evaluated_get(depsgraph)
    eval_mesh = eval_obj.to_mesh()
    positions = [eval_obj.matrix_world @ v.co for v in eval_mesh.vertices]
    return eval_obj, eval_mesh, positions


def compute_normalization(rest_positions, target_height):
    min_x = min(v.x for v in rest_positions)
    max_x = max(v.x for v in rest_positions)
    min_y = min(v.y for v in rest_positions)
    max_y = max(v.y for v in rest_positions)
    min_z = min(v.z for v in rest_positions)
    max_z = max(v.z for v in rest_positions)
    source_height = max_z - min_z
    scale = target_height / source_height if target_height > 0.0 and source_height > 0.0 else 1.0

    return {
        "center_x": (min_x + max_x) * 0.5,
        "center_y": (min_y + max_y) * 0.5,
        "min_z": min_z,
        "source_height": source_height,
        "scale": scale,
    }


def normalized_blender_rest_vertex(pos, norm):
    x = (pos.x - norm["center_x"]) * norm["scale"]
    y = (pos.y - norm["center_y"]) * norm["scale"]
    z = (pos.z - norm["min_z"]) * norm["scale"]
    return Vector((x, y, z))


def material_color(material):
    if material:
        color = material.diffuse_color
        return (color[0], color[1], color[2], color[3])
    return (1.0, 1.0, 1.0, 1.0)


def source_material_colors(source_obj):
    colors = [material_color(material) for material in source_obj.data.materials]
    return colors if colors else [(1.0, 1.0, 1.0, 1.0)]


def build_static_rest_mesh(char_name, source_obj, eval_mesh, rest_positions, norm):
    verts = [normalized_blender_rest_vertex(pos, norm) for pos in rest_positions]
    faces = [[vi for vi in poly.vertices] for poly in eval_mesh.polygons]

    out_mesh = bpy.data.meshes.new(f"{char_name}_walk_rest")
    out_mesh.from_pydata(verts, [], faces)
    out_mesh.update()

    runtime_material = bpy.data.materials.new(f"{char_name}_vat_vertex_color")
    runtime_material.diffuse_color = (1.0, 1.0, 1.0, 1.0)
    out_mesh.materials.append(runtime_material)

    for src_poly, dst_poly in zip(eval_mesh.polygons, out_mesh.polygons):
        dst_poly.material_index = 0
        dst_poly.use_smooth = True

    uv0 = out_mesh.uv_layers.new(name="UVMap")
    uv1 = out_mesh.uv_layers.new(name=VERTEX_ID_UV_LAYER)
    for poly in out_mesh.polygons:
        source_poly = eval_mesh.polygons[poly.index]
        material_index = min(source_poly.material_index, max(len(source_obj.data.materials) - 1, 0))
        palette_u = (material_index + 0.5) / max(len(source_obj.data.materials), 1)
        for loop_idx in poly.loop_indices:
            loop = out_mesh.loops[loop_idx]
            uv0.data[loop_idx].uv = (palette_u, 0.5)
            uv1.data[loop_idx].uv = ((loop.vertex_index + 0.5) / len(out_mesh.vertices), 0.5)

    out_obj = bpy.data.objects.new(f"{char_name}_walk_rest", out_mesh)
    bpy.context.collection.objects.link(out_obj)
    return out_obj, source_material_colors(source_obj)


def write_palette_texture(char_name, output_dir, colors):
    width = len(colors)
    image = bpy.data.images.new(
        f"{char_name}_palette",
        width=width,
        height=1,
        alpha=True,
    )
    pixels = []
    for color in colors:
        pixels.extend(color)
    image.pixels = pixels

    palette_path = os.path.join(output_dir, f"{char_name}_palette.png")
    image.filepath_raw = palette_path
    image.file_format = "PNG"
    image.save()
    print(f"[VAT] Saved palette -> {palette_path}")


def bake_vat_texture(char_name, output_dir, sample_frames, mesh_obj, rest_positions, norm):
    import OpenEXR
    import Imath
    import array as _arr

    num_frames = len(sample_frames)
    num_verts = len(rest_positions)
    r_ch = [0.0] * (num_verts * num_frames)
    g_ch = [0.0] * (num_verts * num_frames)
    b_ch = [0.0] * (num_verts * num_frames)

    for fi, frame in enumerate(sample_frames):
        print(f"[VAT] Baking frame {frame:4d}  ({fi + 1}/{num_frames})")
        eval_obj, eval_mesh, frame_positions = evaluated_mesh_at_frame(mesh_obj, frame)

        max_d = 0.0
        for vi in range(num_verts):
            d_world = (frame_positions[vi] - rest_positions[vi]) * norm["scale"]
            gx, gy, gz = blender_to_godot(d_world.x, d_world.y, d_world.z)
            idx = fi * num_verts + vi
            r_ch[idx] = gx
            g_ch[idx] = gy
            b_ch[idx] = gz
            max_d = max(max_d, d_world.length)

        eval_obj.to_mesh_clear()
        print(
            f"           max_delta={max_d:.4f}m  "
            + ("animated" if max_d > 0.001 else "near-zero")
        )

    r_out = _arr.array("f")
    g_out = _arr.array("f")
    b_out = _arr.array("f")
    for fi in range(num_frames - 1, -1, -1):
        base = fi * num_verts
        r_out.extend(r_ch[base:base + num_verts])
        g_out.extend(g_ch[base:base + num_verts])
        b_out.extend(b_ch[base:base + num_verts])

    exr_path = os.path.join(output_dir, f"{char_name}_vat_walk.exr")
    header = OpenEXR.Header(num_verts, num_frames)
    pixel_type = Imath.PixelType(Imath.PixelType.FLOAT)
    header["channels"] = {c: Imath.Channel(pixel_type) for c in ("R", "G", "B")}
    exr = OpenEXR.OutputFile(exr_path, header)
    exr.writePixels({"R": r_out.tobytes(), "G": g_out.tobytes(), "B": b_out.tobytes()})
    exr.close()
    print(f"[VAT] Saved texture -> {exr_path}")


def export_rest_mesh(char_name, output_dir, rest_obj):
    bpy.ops.object.select_all(action="DESELECT")
    rest_obj.select_set(True)
    bpy.context.view_layer.objects.active = rest_obj

    gltf_path = os.path.join(output_dir, f"{char_name}_walk_rest.gltf")
    bpy.ops.export_scene.gltf(
        filepath=gltf_path,
        export_format="GLTF_SEPARATE",
        use_selection=True,
        export_animations=False,
        export_apply=False,
    )
    print(f"[VAT] Saved mesh -> {gltf_path}")


def main():
    argv = sys.argv
    if "--" not in argv:
        print("ERROR: pass arguments after --")
        sys.exit(1)

    args = parse_args(argv[argv.index("--") + 1:])
    walk_blend = os.path.abspath(args.walk_blend)
    output_dir = os.path.abspath(args.output_dir)
    idle_blend = os.path.abspath(args.idle_blend) if args.idle_blend else None
    os.makedirs(output_dir, exist_ok=True)

    char_mesh_obj = next(
        (o for o in bpy.data.objects if o.type == "MESH" and len(o.vertex_groups) > 5),
        None,
    )
    char_armature = next((o for o in bpy.data.objects if o.type == "ARMATURE"), None)
    if not char_mesh_obj:
        print("ERROR: no rigged MESH found in character .blend")
        sys.exit(1)

    print(
        f"[VAT] Character mesh: '{char_mesh_obj.name}' "
        f"verts={len(char_mesh_obj.data.vertices)}"
    )
    print(f"[VAT] Armature: '{char_armature.name if char_armature else 'NONE'}'")

    walk_names = load_actions_from_blend(walk_blend, "walk blend")
    walk_action = select_action(walk_names, args.action, "walk", "walk")
    fr_start = int(math.floor(walk_action.frame_range[0]))
    fr_end = int(math.ceil(walk_action.frame_range[1]))
    print(f"[VAT] Using walk action: '{walk_action.name}' frames {fr_start}-{fr_end}")
    assign_action(char_armature, walk_action)

    ensure_vat_uv_layers(char_mesh_obj.data)

    idle_action = None
    idle_frame = fr_start
    if idle_blend:
        idle_names = load_actions_from_blend(idle_blend, "idle blend")
        idle_action = select_action(idle_names, args.idle_action, "idle", "idle")
        idle_frame = int(math.floor(idle_action.frame_range[0]))
        print(f"[VAT] Using idle action: '{idle_action.name}' frame {idle_frame}")

    if idle_action is not None:
        assign_action(char_armature, idle_action)
        rest_frame_label = f"idle frame {idle_frame}"
        rest_frame = idle_frame
    else:
        rest_frame_label = f"walk frame {fr_start}"
        rest_frame = fr_start

    rest_eval_obj, rest_eval_mesh, rest_positions = evaluated_mesh_at_frame(char_mesh_obj, rest_frame)
    norm = compute_normalization(rest_positions, args.target_height)
    print(
        f"[VAT] Rest pose: {rest_frame_label} verts={len(rest_positions)} "
        f"source_height={norm['source_height']:.4f}m scale={norm['scale']:.4f} "
        f"target_height={args.target_height:.4f}m"
    )

    rest_obj, palette_colors = build_static_rest_mesh(
        args.char_name,
        char_mesh_obj,
        rest_eval_mesh,
        rest_positions,
        norm,
    )
    rest_eval_obj.to_mesh_clear()

    assign_action(char_armature, walk_action)
    span = max(fr_end - fr_start, 1)
    sample_frames = [
        fr_start + int(round(i * span / max(args.num_frames - 1, 1)))
        for i in range(args.num_frames)
    ]
    bake_vat_texture(args.char_name, output_dir, sample_frames, char_mesh_obj, rest_positions, norm)
    write_palette_texture(args.char_name, output_dir, palette_colors)
    export_rest_mesh(args.char_name, output_dir, rest_obj)

    meta = {
        "char_name": args.char_name,
        "num_verts": len(rest_positions),
        "num_frames": args.num_frames,
        "source_frames": [fr_start, fr_end],
        "walk_action": walk_action.name,
        "target_height_m": args.target_height,
        "source_height_m": norm["source_height"],
        "source_to_runtime_scale": norm["scale"],
        "texture": f"{args.char_name}_vat_walk.exr",
        "palette": f"{args.char_name}_palette.png",
        "mesh": f"{args.char_name}_walk_rest.gltf",
        "shader_note": (
            "VERTEX += texture(vat_tex, vec2(UV2.x, "
            "(phase*(num_frames-1)+0.5)/num_frames)).rgb;"
        ),
    }
    meta_path = os.path.join(output_dir, f"{args.char_name}_vat_meta.json")
    with open(meta_path, "w", encoding="utf-8") as f:
        json.dump(meta, f, indent=2)
    print(f"[VAT] Saved meta -> {meta_path}")
    print("[VAT] Done!")


main()
