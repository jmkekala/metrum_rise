"""
Diagnose a .blend file to find objects, rigs, animations, and bone names.
Usage: blender --background <blend_file> --python tools/diagnose_blend.py
(The blend file is the FIRST argument after --background, before --python)
"""
import bpy, sys, math

print(f"\n===== BLEND DIAGNOSTIC: {bpy.data.filepath} =====\n")

for obj in bpy.data.objects:
    print(f"OBJECT  name='{obj.name}'  type={obj.type}")
    rot = [math.degrees(v) for v in obj.rotation_euler]
    print(f"        rotation=({rot[0]:.1f}°, {rot[1]:.1f}°, {rot[2]:.1f}°)  scale={tuple(f'{v:.4f}' for v in obj.scale)}")

    if obj.type == 'ARMATURE':
        deform_bones = [b.name for b in obj.data.bones if b.use_deform]
        ctrl_bones   = [b.name for b in obj.data.bones if not b.use_deform]
        print(f"        deform bones ({len(deform_bones)}): {deform_bones[:12]}")
        if len(deform_bones) > 12:
            print(f"          ... and {len(deform_bones)-12} more")
        print(f"        control bones ({len(ctrl_bones)}): {ctrl_bones[:8]}")

        # Actions
        for action in bpy.data.actions:
            print(f"        ACTION '{action.name}' frames "
                  f"{action.frame_range[0]:.0f}-{action.frame_range[1]:.0f}")
            # Collect driven bone names from F-curves
            bone_names = set()
            for layer in action.layers if hasattr(action, 'layers') else []:
                pass
            # Try legacy fcurves
            try:
                for fc in action.fcurves:
                    dp = fc.data_path
                    if '["' in dp:
                        bone_names.add(dp.split('["')[1].split('"]')[0])
                    elif "['" in dp:
                        bone_names.add(dp.split("['")[1].split("']")[0])
                print(f"          driven bones: {sorted(bone_names)[:12]}")
            except AttributeError:
                # Blender 4+ uses layers/strips instead of fcurves on Action
                print(f"          (Blender 4+ action format — fcurves not directly accessible)")

        if obj.animation_data and obj.animation_data.action:
            act = obj.animation_data.action
            print(f"        active action='{act.name}' frames "
                  f"{act.frame_range[0]:.0f}-{act.frame_range[1]:.0f}")
        else:
            print(f"        no active action on object")

    elif obj.type == 'MESH':
        vg_names = [vg.name for vg in obj.vertex_groups]
        print(f"        vertices={len(obj.data.vertices)}  vertex_groups ({len(vg_names)}): {vg_names[:12]}")
        mod_types = [m.type for m in obj.modifiers]
        print(f"        modifiers: {mod_types}")
        if hasattr(obj, 'parent') and obj.parent:
            print(f"        parent: '{obj.parent.name}' ({obj.parent.type})")

print("\n===== ALL ACTIONS =====")
for action in bpy.data.actions:
    print(f"  '{action.name}' frames {action.frame_range[0]:.0f}-{action.frame_range[1]:.0f}")

print("\n===== END DIAGNOSTIC =====")
