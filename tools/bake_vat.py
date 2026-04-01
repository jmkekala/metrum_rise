#!/usr/bin/env python3
"""
Vertex Animation Texture (VAT) Baker for Metrum Rise pedestrian characters.

Runs headlessly via Blender:
  blender --background --python tools/bake_vat.py -- <char_fbx> <walk_fbx> <char_name> <output_dir> [num_frames]

Output files (all in <output_dir>/):
  <char_name>_walk_rest.gltf   — mesh in walk-cycle frame 0 (Godot-ready, Y-up)
  <char_name>_walk_rest.bin    — GLTF binary buffer
  <char_name>_vat_walk.exr     — float32 RGBA position-offset texture
  <char_name>_vat_meta.json    — vertex count, frame count, UV encoding info

Texture layout:
  width  = number of geometric vertices (VAT columns)
  height = num_frames (VAT rows)
  pixel[row=frame][col=vi] = (delta_x, delta_y, delta_z, 1.0) in Godot Y-up space

The rest-pose GLTF has a second UV layer ("UVMap_VertexID") where every loop
belonging to geometric vertex vi has UV1 = ((vi + 0.5) / num_verts, 0.5).
In Godot's vertex shader, UV2.x correctly indexes into the VAT regardless of
how the GLTF exporter splits vertices at UV0 seams (since UV1 is per-vertex,
not per-face-corner).
"""

import bpy
import os
import sys
import json
import math


# ---------------------------------------------------------------------------
def clear_scene():
    """Remove all objects, meshes, armatures and actions from the scene."""
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)
    for col in (bpy.data.meshes, bpy.data.armatures,
                bpy.data.actions, bpy.data.images, bpy.data.materials):
        for item in list(col):
            col.remove(item)


def import_fbx(filepath, use_anim=True):
    """Import an FBX and return the newly added objects."""
    before = {o.name for o in bpy.data.objects}
    bpy.ops.import_scene.fbx(
        filepath=filepath,
        use_anim=use_anim,
        ignore_leaf_bones=False,
        force_connect_children=False,
        automatic_bone_orientation=False,
    )
    after = {o.name for o in bpy.data.objects}
    return [bpy.data.objects[n] for n in (after - before)]


def blender_to_godot(dx, dy, dz):
    """Convert a position delta from Blender Z-up to Godot Y-up.
    Blender: X=right, Y=forward, Z=up
    Godot:   X=right, Y=up,     Z=back (right-handed Y-up = GLTF convention)
    Mapping: gX=bX, gY=bZ, gZ=-bY
    """
    return (dx, dz, -dy)


# ---------------------------------------------------------------------------
def main():
    argv = sys.argv
    if "--" not in argv:
        print("ERROR: pass arguments after --")
        print("Usage: blender --background --python bake_vat.py -- "
              "<char_fbx> <walk_fbx> <char_name> <output_dir> [num_frames]")
        sys.exit(1)

    args = argv[argv.index("--") + 1:]
    if len(args) < 4:
        print("ERROR: need at least 4 arguments after --")
        sys.exit(1)

    char_fbx   = os.path.abspath(args[0])
    walk_fbx   = os.path.abspath(args[1])
    char_name  = args[2]
    output_dir = os.path.abspath(args[3])
    num_frames = int(args[4]) if len(args) > 4 else 30

    os.makedirs(output_dir, exist_ok=True)
    clear_scene()

    # ── 1. Import character (mesh + armature, no animation) ─────────────────
    print(f"[VAT] Importing character mesh: {char_fbx}")
    char_objects = import_fbx(char_fbx, use_anim=False)
    char_mesh_obj  = next((o for o in char_objects if o.type == 'MESH'),     None)
    char_armature  = next((o for o in char_objects if o.type == 'ARMATURE'), None)

    if not char_mesh_obj:
        print("ERROR: no MESH object found in character FBX")
        sys.exit(1)

    print(f"[VAT] Mesh: '{char_mesh_obj.name}'  "
          f"geo_verts={len(char_mesh_obj.data.vertices)}")

    # ── 2. Import walk animation ─────────────────────────────────────────────
    print(f"[VAT] Importing walk animation: {walk_fbx}")
    actions_before = {a.name for a in bpy.data.actions}
    walk_objs      = import_fbx(walk_fbx, use_anim=True)
    actions_after  = {a.name for a in bpy.data.actions}

    walk_armature = next((o for o in walk_objs if o.type == 'ARMATURE'), None)
    walk_mesh_obj = next((o for o in walk_objs if o.type == 'MESH'), None)

    if walk_armature:
        print(f"[VAT] Walk armature: '{walk_armature.name}'  "
              f"range={walk_armature.animation_data.action.frame_range[0]:.0f}–"
              f"{walk_armature.animation_data.action.frame_range[1]:.0f}"
              if walk_armature.animation_data and walk_armature.animation_data.action
              else f"[VAT] Walk armature: '{walk_armature.name}' (no action on it)")

        # ── Strategy: point the character mesh's Armature modifier at the walk
        # armature.  This avoids the bone-name mismatch problem entirely —
        # the walk FBX's armature already has its own animation natively bound.
        for mod in char_mesh_obj.modifiers:
            if mod.type == 'ARMATURE':
                mod.object = walk_armature
                print(f"[VAT] Redirected Armature modifier → '{walk_armature.name}'")
                break
        else:
            # No armature modifier yet; add one.
            mod = char_mesh_obj.modifiers.new("VAT_Armature", 'ARMATURE')
            mod.object = walk_armature
            print(f"[VAT] Added Armature modifier → '{walk_armature.name}'")

        # Also parent the mesh to the walk armature so transforms stay consistent.
        char_mesh_obj.parent = walk_armature

        # Read frame range from the walk armature's action.
        if walk_armature.animation_data and walk_armature.animation_data.action:
            act = walk_armature.animation_data.action
            fr_start = int(math.floor(act.frame_range[0]))
            fr_end   = int(math.ceil(act.frame_range[1]))
        else:
            fr_start, fr_end = 0, num_frames - 1
        # Remove the now-redundant character armature (mesh owns its own modifier).
        if char_armature and char_armature != walk_armature:
            bpy.data.objects.remove(char_armature, do_unlink=True)
    else:
        # Fallback: try to assign the action to the character armature.
        new_names   = actions_after - actions_before
        walk_action = bpy.data.actions[next(iter(new_names))] if new_names else None
        if walk_action and char_armature:
            print(f"[VAT] Walk action: '{walk_action.name}' assigned to character armature")
            char_armature.animation_data_create()
            char_armature.animation_data.action = walk_action
            fr_start = int(math.floor(walk_action.frame_range[0]))
            fr_end   = int(math.ceil(walk_action.frame_range[1]))
        else:
            print("[VAT] WARNING: no walk armature or action found; baking T-pose only")
            fr_start, fr_end = 0, num_frames - 1

    # Remove any walk mesh that came with walk.fbx (we use the character mesh).
    if walk_mesh_obj and walk_mesh_obj != char_mesh_obj:
        bpy.data.objects.remove(walk_mesh_obj, do_unlink=True)

    # Evenly sample num_frames across the action range
    span = max(fr_end - fr_start, 1)
    sample_frames = [
        fr_start + int(round(i * span / max(num_frames - 1, 1)))
        for i in range(num_frames)
    ]

    # ── 3. Apply object transforms so GLTF export has identity node rotation ──
    bpy.ops.object.select_all(action='DESELECT')
    active_armature = walk_armature if walk_armature else char_armature
    if active_armature:
        active_armature.select_set(True)
    char_mesh_obj.select_set(True)
    bpy.context.view_layer.objects.active = char_mesh_obj
    bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
    print(f"[VAT] Object transforms applied.")

    # ── 4. Add UV1 vertex-ID channel to the base mesh ───────────────────────
    # This survives UV0 seam–splits in the GLTF exporter and lets the
    # Godot shader address the correct VAT column via UV2.x.
    mesh_data     = char_mesh_obj.data
    num_geo_verts = len(mesh_data.vertices)

    vid_layer_name = "UVMap_VertexID"
    if vid_layer_name in mesh_data.uv_layers:
        mesh_data.uv_layers.remove(mesh_data.uv_layers[vid_layer_name])
    uv1 = mesh_data.uv_layers.new(name=vid_layer_name)

    for poly in mesh_data.polygons:
        for loop_idx in poly.loop_indices:
            vi = mesh_data.loops[loop_idx].vertex_index
            uv1.data[loop_idx].uv = ((vi + 0.5) / num_geo_verts, 0.5)

    print(f"[VAT] UV1 vertex-ID channel added  "
          f"num_geo_verts={num_geo_verts}")

    # ── 5. Snapshot rest-pose positions — use world space ───────────────────
    # After transform_apply, matrix_world should be close to identity, so
    # world_mat @ v.co == v.co.  We use world_mat anyway for robustness.
    scene = bpy.context.scene
    world_mat = char_mesh_obj.matrix_world   # identity after transform_apply
    scene.frame_set(sample_frames[0])
    depsgraph = bpy.context.evaluated_depsgraph_get()
    eval_obj  = char_mesh_obj.evaluated_get(depsgraph)
    mesh_rest = eval_obj.to_mesh()
    rest_world = [world_mat @ v.co for v in mesh_rest.vertices]
    eval_obj.to_mesh_clear()

    num_verts = len(rest_world)
    print(f"[VAT] Evaluated vertex count: {num_verts}  "
          f"frames to bake: {num_frames}")

    # ── 5. Bake position offsets into float EXR ──────────────────────────────
    # Layout: width=num_verts, height=num_frames
    # pixel[fi*num_verts + vi] = (gX, gY, gZ, 1.0) in Godot Y-up space
    # Delta is computed in Blender world space then converted to Godot Y-up.
    img = bpy.data.images.new(
        f"{char_name}_vat_walk",
        width=num_verts,
        height=num_frames,
        float_buffer=True,
        alpha=True,
    )
    pixels = [0.0] * (num_verts * num_frames * 4)

    for fi, fr in enumerate(sample_frames):
        print(f"[VAT] Baking frame {fr:4d}  ({fi+1}/{num_frames})")
        scene.frame_set(fr)
        depsgraph = bpy.context.evaluated_depsgraph_get()
        eval_obj  = char_mesh_obj.evaluated_get(depsgraph)
        eval_mesh = eval_obj.to_mesh()

        for vi in range(num_verts):
            # World-space position of this vertex at this frame
            world_pos  = world_mat @ eval_mesh.vertices[vi].co
            d_world    = world_pos - rest_world[vi]          # Blender world-space delta
            gx, gy, gz = blender_to_godot(d_world.x, d_world.y, d_world.z)
            base       = (fi * num_verts + vi) * 4
            pixels[base    ] = gx
            pixels[base + 1] = gy
            pixels[base + 2] = gz
            pixels[base + 3] = 1.0

        eval_obj.to_mesh_clear()

    img.pixels = pixels

    exr_path         = os.path.join(output_dir, f"{char_name}_vat_walk.exr")
    img.filepath_raw = exr_path
    img.file_format  = 'OPEN_EXR'
    img.save()
    print(f"[VAT] Saved texture → {exr_path}")

    # ── 6. Export rest-pose mesh as GLTF ────────────────────────────────────
    scene.frame_set(sample_frames[0])
    bpy.ops.object.select_all(action='DESELECT')
    char_mesh_obj.select_set(True)
    bpy.context.view_layer.objects.active = char_mesh_obj

    gltf_path = os.path.join(output_dir, f"{char_name}_walk_rest.gltf")
    bpy.ops.export_scene.gltf(
        filepath=gltf_path,
        export_format='GLTF_SEPARATE',
        use_selection=True,
        export_animations=False,
        export_apply=True,   # bake the armature deform at frame sample_frames[0]
    )
    print(f"[VAT] Saved mesh  → {gltf_path}")

    # ── 7. Write metadata ────────────────────────────────────────────────────
    meta = {
        "char_name":    char_name,
        "num_verts":    num_verts,
        "num_frames":   num_frames,
        "source_frames": [fr_start, fr_end],
        "sampled_frames": sample_frames,
        "texture":      f"{char_name}_vat_walk.exr",
        "mesh":         f"{char_name}_walk_rest.gltf",
        "shader_note":  (
            "In vertex shader: "
            "vec4 offset = texture(vat_tex, vec2(UV2.x, (phase*(num_frames-1)+0.5)/num_frames)); "
            "VERTEX += offset.xyz;"
        ),
    }
    meta_path = os.path.join(output_dir, f"{char_name}_vat_meta.json")
    with open(meta_path, "w") as f:
        json.dump(meta, f, indent=2)
    print(f"[VAT] Saved meta  → {meta_path}")
    print("[VAT] ✓ Done!")


main()
