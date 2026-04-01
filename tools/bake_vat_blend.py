#!/usr/bin/env python3
"""
Vertex Animation Texture (VAT) Baker — uses original .blend source files.

Usage (Blender opens the character .blend as the main scene):
  blender --background <char.blend> --python tools/bake_vat_blend.py -- <anim.blend> <char_name> <output_dir> [num_frames] [idle.blend]

The animation action is imported from anim.blend via bpy.data.libraries.load()
(no full scene open → no linked-library hang).

Optional idle.blend: if provided, the rest pose (exported GLTF + VAT delta base)
is taken from idle.blend frame 0 instead of frame 0 of anim.blend.  This gives
characters naturally hanging arms in the rest mesh, so the animation deltas keep
hands at the correct idle height throughout the cycle.

Output files (all in <output_dir>/):
  <char_name>_walk_rest.gltf   — rest-pose mesh (Godot Y-up, clean coordinates)
  <char_name>_walk_rest.bin
  <char_name>_vat_walk.exr     — float32 RGBA position-offset texture
  <char_name>_vat_meta.json
"""

import bpy
import os
import sys
import json
import math


def blender_to_godot(dx, dy, dz):
    """Blender Z-up → Godot/GLTF Y-up: gX=bX, gY=bZ, gZ=-bY."""
    return (dx, dz, -dy)


def main():
    argv = sys.argv
    if "--" not in argv:
        print("ERROR: pass arguments after --")
        sys.exit(1)

    args       = argv[argv.index("--") + 1:]
    walk_blend = os.path.abspath(args[0])
    char_name  = args[1]
    output_dir = os.path.abspath(args[2])
    num_frames = int(args[3]) if len(args) > 3 else 30
    idle_blend = os.path.abspath(args[4]) if len(args) > 4 else None

    os.makedirs(output_dir, exist_ok=True)

    # ── 1. Find the mesh and armature in the pre-opened character .blend ────
    char_mesh_obj = next((o for o in bpy.data.objects if o.type == 'MESH'
                          and len(o.vertex_groups) > 5), None)
    char_armature = next((o for o in bpy.data.objects if o.type == 'ARMATURE'), None)

    if not char_mesh_obj:
        print("ERROR: no rigged MESH found in character .blend")
        sys.exit(1)

    print(f"[VAT] Character mesh: '{char_mesh_obj.name}'  "
          f"verts={len(char_mesh_obj.data.vertices)}")
    print(f"[VAT] Armature:      '{char_armature.name if char_armature else 'NONE'}'")

    # ── 2. Import the walk action from walk.blend (no scene linking) ────────
    print(f"[VAT] Loading actions from: {walk_blend}")
    actions_before = set(bpy.data.actions.keys())

    with bpy.data.libraries.load(walk_blend, link=False) as (data_from, data_to):
        print(f"[VAT] Available actions in walk.blend: {data_from.actions}")
        data_to.actions = list(data_from.actions)

    new_action_names = set(bpy.data.actions.keys()) - actions_before
    print(f"[VAT] Imported actions: {new_action_names}")

    if not new_action_names:
        print("ERROR: no actions found in walk.blend")
        sys.exit(1)

    # Pick the action with the longest frame range (most animation data)
    walk_action = max(
        (bpy.data.actions[n] for n in new_action_names),
        key=lambda a: a.frame_range[1] - a.frame_range[0]
    )
    fr_start = int(math.floor(walk_action.frame_range[0]))
    fr_end   = int(math.ceil(walk_action.frame_range[1]))
    print(f"[VAT] Using action: '{walk_action.name}'  frames {fr_start}-{fr_end}")

    # ── 3. Assign action to the character armature (Blender 5.1 slot API) ────
    if char_armature:
        char_armature.animation_data_create()
        anim_data = char_armature.animation_data

        # Blender 5+ uses a slot-based action binding system.
        # The legacy `animation_data.action = ...` sets the action but does NOT
        # create or select a slot, so the armature bones are never driven.
        # We must: (a) assign the action, (b) create/use a slot for this armature.
        anim_data.action = walk_action

        if hasattr(anim_data, 'action_slot'):
            # Blender 5+ path
            slot = None
            # Try to find an existing slot for this armature type
            for s in walk_action.slots:
                if s.target_id_type == 'OBJECT':
                    slot = s
                    break
            if slot is None and hasattr(walk_action, 'slots'):
                slot = walk_action.slots.new(id_type='OBJECT', name=char_armature.name)
            if slot is not None:
                anim_data.action_slot = slot
                print(f"[VAT] Blender 5 slot assigned to {char_armature.name}  "
                      f"target_id_type={slot.target_id_type}")
            else:
                print("[VAT] WARNING: could not create action slot")
        else:
            # Blender 3/4 legacy
            print(f"[VAT] Legacy action assigned to armature '{char_armature.name}'")

        # Verify the assignment actually drives bones at frame 1
        scene = bpy.context.scene
        scene.frame_set(fr_start + 1)
        dg   = bpy.context.evaluated_depsgraph_get()
        ev   = char_mesh_obj.evaluated_get(dg)
        m    = ev.to_mesh()
        # T-pose check: just get the max Y (head position) as a proxy
        max_y = max(v.co.z for v in m.vertices)
        ev.to_mesh_clear()
        print(f"[VAT] Post-assignment max Z at frame {fr_start+1}: {max_y:.4f}m "
              f"(expect ~0.8-1.0 for upright character)")
    else:
        print("[VAT] WARNING: no armature found; baking T-pose only")

    # ── 4. Add UV1 vertex-ID channel ────────────────────────────────────────
    # Encode geometric vertex index as UV1.x so the shader can address the VAT
    # column correctly even after UV0 seam-splits during GLTF export.
    mesh_data     = char_mesh_obj.data
    num_geo_verts = len(mesh_data.vertices)

    # Clear all but the primary albedo UV layer before adding the ID layer
    # to ensure our ID layer becomes UV2 in Godot.
    while len(mesh_data.uv_layers) > 1:
        mesh_data.uv_layers.remove(mesh_data.uv_layers[-1])

    vid_layer_name = "UVMap_VertexID"
    uv1 = mesh_data.uv_layers.new(name=vid_layer_name)
    for poly in mesh_data.polygons:
        for loop_idx in poly.loop_indices:
            vi = mesh_data.loops[loop_idx].vertex_index
            uv1.data[loop_idx].uv = ((vi + 0.5) / num_geo_verts, 0.5)
    print(f"[VAT] UV1 (VertexID) layer added at index 1. num_geo_verts={num_geo_verts}")

    # ── 5. Sample rest-pose positions ───────────────────────────────────────
    # Sample in WORLD space so that deltas match the GLTF export which uses
    # export_apply=True (baking the object's scale/rotation into vertex positions).
    # v.co is LOCAL space; multiplying by matrix_world converts to world space.
    #
    # If idle.blend is provided, the rest pose is taken from its first frame so
    # the exported mesh has naturally hanging arms.  Deltas in the VAT then span
    # the full distance from idle to each animation frame — this is intentional.
    world_mat = char_mesh_obj.matrix_world
    scene = bpy.context.scene

    if idle_blend:
        print(f"[VAT] Loading idle action from: {idle_blend}")
        idle_before = set(bpy.data.actions.keys())
        with bpy.data.libraries.load(idle_blend, link=False) as (data_from, data_to):
            print(f"[VAT] Available idle actions: {data_from.actions}")
            data_to.actions = list(data_from.actions)
        idle_action_names = set(bpy.data.actions.keys()) - idle_before
        if not idle_action_names:
            print("ERROR: no actions found in idle.blend")
            sys.exit(1)
        idle_action = max(
            (bpy.data.actions[n] for n in idle_action_names),
            key=lambda a: a.frame_range[1] - a.frame_range[0]
        )
        print(f"[VAT] Idle action: '{idle_action.name}'")

        # Assign idle action to armature using the same slot logic as above
        if char_armature:
            anim_data = char_armature.animation_data
            anim_data.action = idle_action
            if hasattr(anim_data, 'action_slot'):
                slot = None
                for s in idle_action.slots:
                    if s.target_id_type == 'OBJECT':
                        slot = s
                        break
                if slot is None and hasattr(idle_action, 'slots'):
                    slot = idle_action.slots.new(id_type='OBJECT', name=char_armature.name)
                if slot is not None:
                    anim_data.action_slot = slot

        idle_fr = int(math.floor(idle_action.frame_range[0]))
        scene.frame_set(idle_fr)
        depsgraph = bpy.context.evaluated_depsgraph_get()
        eval_obj  = char_mesh_obj.evaluated_get(depsgraph)
        mesh_rest = eval_obj.to_mesh()
        rest_co   = [world_mat @ v.co for v in mesh_rest.vertices]
        eval_obj.to_mesh_clear()
        rest_frame_label = f"idle frame {idle_fr}"

        # Now re-assign the walk/run action for baking below
        if char_armature:
            anim_data = char_armature.animation_data
            anim_data.action = walk_action
            if hasattr(anim_data, 'action_slot'):
                slot = None
                for s in walk_action.slots:
                    if s.target_id_type == 'OBJECT':
                        slot = s
                        break
                if slot is None and hasattr(walk_action, 'slots'):
                    slot = walk_action.slots.new(id_type='OBJECT', name=char_armature.name)
                if slot is not None:
                    anim_data.action_slot = slot
    else:
        scene.frame_set(fr_start)
        depsgraph = bpy.context.evaluated_depsgraph_get()
        eval_obj  = char_mesh_obj.evaluated_get(depsgraph)
        mesh_rest = eval_obj.to_mesh()
        rest_co   = [world_mat @ v.co for v in mesh_rest.vertices]
        eval_obj.to_mesh_clear()
        rest_frame_label = f"anim frame {fr_start}"

    num_verts = len(rest_co)
    print(f"[VAT] Rest pose: {rest_frame_label}  verts={num_verts}  frames to bake: {num_frames}")

    # Evenly sample num_frames across the action range
    span = max(fr_end - fr_start, 1)
    sample_frames = [
        fr_start + int(round(i * span / max(num_frames - 1, 1)))
        for i in range(num_frames)
    ]

    # ── 6. Bake position offsets ──────────────────────────────────────────────
    # Per-frame, per-vertex offsets stored as flat (R,G,B) per pixel.
    # Using plain lists avoids any Blender image colour-management transforms.
    r_ch = [0.0] * (num_verts * num_frames)
    g_ch = [0.0] * (num_verts * num_frames)
    b_ch = [0.0] * (num_verts * num_frames)

    for fi, fr in enumerate(sample_frames):
        print(f"[VAT] Baking frame {fr:4d}  ({fi+1}/{num_frames})")
        scene.frame_set(fr)
        depsgraph = bpy.context.evaluated_depsgraph_get()
        eval_obj  = char_mesh_obj.evaluated_get(depsgraph)
        eval_mesh = eval_obj.to_mesh()

        max_d = 0.0
        for vi in range(num_verts):
            d_world    = world_mat @ eval_mesh.vertices[vi].co - rest_co[vi]
            gx, gy, gz = blender_to_godot(d_world.x, d_world.y, d_world.z)
            idx = fi * num_verts + vi
            r_ch[idx] = gx
            g_ch[idx] = gy
            b_ch[idx] = gz
            max_d = max(max_d, d_world.length)

        eval_obj.to_mesh_clear()
        print(f"           max_delta={max_d:.4f}m  "
              + ("✓ animated" if max_d > 0.001 else "⚠ near-zero (T-pose?)"))

    # Write EXR directly with Python's OpenEXR to bypass Blender's sRGB colour
    # management (which would otherwise multiply near-zero values by ~12.92×).
    # Rows are written in REVERSE fi order (fi=num_frames-1 first) to match
    # Blender's EXR Y-flip convention so the shader's vat_v mapping is correct.
    import OpenEXR, Imath, array as _arr
    r_out = _arr.array('f'); g_out = _arr.array('f'); b_out = _arr.array('f')
    for fi in range(num_frames - 1, -1, -1):
        base = fi * num_verts
        r_out.extend(r_ch[base:base + num_verts])
        g_out.extend(g_ch[base:base + num_verts])
        b_out.extend(b_ch[base:base + num_verts])

    exr_path = os.path.join(output_dir, f"{char_name}_vat_walk.exr")
    _hdr = OpenEXR.Header(num_verts, num_frames)
    _ft  = Imath.PixelType(Imath.PixelType.FLOAT)
    _hdr['channels'] = {c: Imath.Channel(_ft) for c in ('R', 'G', 'B')}
    _exr = OpenEXR.OutputFile(exr_path, _hdr)
    _exr.writePixels({'R': r_out.tobytes(), 'G': g_out.tobytes(), 'B': b_out.tobytes()})
    _exr.close()
    print(f"[VAT] Saved texture → {exr_path}")

    # ── 7. Export rest-pose mesh as GLTF ────────────────────────────────────
    # Set armature to the rest pose frame before exporting so export_apply=True
    # bakes the same pose that was used as the VAT delta base.
    if idle_blend:
        if char_armature:
            anim_data = char_armature.animation_data
            anim_data.action = idle_action
            if hasattr(anim_data, 'action_slot'):
                slot = None
                for s in idle_action.slots:
                    if s.target_id_type == 'OBJECT':
                        slot = s
                        break
                if slot is not None:
                    anim_data.action_slot = slot
        scene.frame_set(idle_fr)
    else:
        scene.frame_set(fr_start)
    bpy.ops.object.select_all(action='DESELECT')
    char_mesh_obj.select_set(True)
    bpy.context.view_layer.objects.active = char_mesh_obj

    gltf_path = os.path.join(output_dir, f"{char_name}_walk_rest.gltf")
    bpy.ops.export_scene.gltf(
        filepath=gltf_path,
        export_format='GLTF_SEPARATE',
        use_selection=True,
        export_animations=False,
        export_apply=True,   # bake armature deform at fr_start
    )
    print(f"[VAT] Saved mesh  → {gltf_path}")

    # ── 8. Metadata ──────────────────────────────────────────────────────────
    meta = {
        "char_name":     char_name,
        "num_verts":     num_verts,
        "num_frames":    num_frames,
        "source_frames": [fr_start, fr_end],
        "walk_action":   walk_action.name,
        "texture":       f"{char_name}_vat_walk.exr",
        "mesh":          f"{char_name}_walk_rest.gltf",
        "shader_note": (
            "VERTEX += texture(vat_tex, vec2(UV2.x, "
            "(phase*(num_frames-1)+0.5)/num_frames)).rgb;"
        ),
    }
    meta_path = os.path.join(output_dir, f"{char_name}_vat_meta.json")
    with open(meta_path, "w") as f:
        json.dump(meta, f, indent=2)
    print(f"[VAT] Saved meta  → {meta_path}")
    print("[VAT] ✓ Done!")


main()
