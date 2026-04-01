"""
Diagnose the contents of a character or animation FBX file.
Usage: blender --background --python tools/diagnose_fbx.py -- <fbx_file>
"""
import bpy, sys, math

argv = sys.argv
fbx_path = argv[argv.index("--") + 1]

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.fbx(filepath=fbx_path, use_anim=True,
                          ignore_leaf_bones=False,
                          force_connect_children=False,
                          automatic_bone_orientation=False)

print(f"\n===== FBX DIAGNOSTIC: {fbx_path} =====\n")
for obj in bpy.context.scene.objects:
    print(f"OBJECT  name='{obj.name}'  type={obj.type}")
    print(f"        location={tuple(f'{v:.3f}' for v in obj.location)}")
    print(f"        rotation={tuple(f'{math.degrees(v):.1f}°' for v in obj.rotation_euler)}")
    print(f"        scale={tuple(f'{v:.3f}' for v in obj.scale)}")

    if obj.type == 'ARMATURE':
        print(f"        bones ({len(obj.data.bones)}):")
        for b in obj.data.bones[:15]:
            print(f"          '{b.name}'")
        if len(obj.data.bones) > 15:
            print(f"          ... and {len(obj.data.bones)-15} more")

        if obj.animation_data and obj.animation_data.action:
            act = obj.animation_data.action
            print(f"        action='{act.name}'  frames {act.frame_range[0]:.0f}-{act.frame_range[1]:.0f}")
            bone_names = set()
            for fc in act.fcurves:
                dp = fc.data_path
                if '["' in dp:
                    bone_names.add(dp.split('["')[1].split('"]')[0])
                elif "['" in dp:
                    bone_names.add(dp.split("['")[1].split("']")[0])
            print(f"        animated bones ({len(bone_names)}):")
            for bn in sorted(bone_names)[:15]:
                print(f"          '{bn}'")
            if len(bone_names) > 15:
                print(f"          ... and {len(bone_names)-15} more")
        else:
            print(f"        NO animation data")

    elif obj.type == 'MESH':
        print(f"        vertices={len(obj.data.vertices)}")
        print(f"        vertex_groups ({len(obj.vertex_groups)}):")
        for vg in obj.vertex_groups[:15]:
            print(f"          '{vg.name}'")
        if len(obj.vertex_groups) > 15:
            print(f"          ... and {len(obj.vertex_groups)-15} more")
        modifiers = [m.type for m in obj.modifiers]
        print(f"        modifiers: {modifiers}")

print("\n===== END DIAGNOSTIC =====")
