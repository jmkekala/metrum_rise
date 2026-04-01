"""
Print all bone names in character armature and all bone names
driven by the Walk action from walk.blend, to check for name match.

Usage:
  blender --background <char.blend> --python tools/check_bones.py -- <walk.blend>
"""
import bpy, sys, os

argv       = sys.argv
walk_blend = os.path.abspath(argv[argv.index("--") + 1])

char_arm  = next(o for o in bpy.data.objects if o.type == 'ARMATURE')
char_mesh = next((o for o in bpy.data.objects
                  if o.type == 'MESH' and len(o.vertex_groups) > 5), None)

print(f"\n=== Character armature '{char_arm.name}' — ALL {len(char_arm.data.bones)} bones ===")
for b in sorted(char_arm.data.bones, key=lambda b: b.name):
    print(f"  use_deform={b.use_deform}  '{b.name}'")

if char_mesh:
    print(f"\n=== Mesh '{char_mesh.name}' — vertex groups ===")
    for vg in char_mesh.vertex_groups:
        print(f"  '{vg.name}'")

with bpy.data.libraries.load(walk_blend, link=False) as (src, dst):
    dst.actions = [a for a in src.actions if a == 'Walk']
    print(f"\n=== walk.blend actions: {src.actions} ===")
    dst.actions = list(src.actions)

walk_action = max(bpy.data.actions, key=lambda a: a.frame_range[1])
print(f"\n=== Walk action '{walk_action.name}' — driven bone names ===")
bone_names = set()
try:
    for fc in walk_action.fcurves:
        dp = fc.data_path
        if '["' in dp:
            bone_names.add(dp.split('["')[1].split('"]')[0])
        elif "['" in dp:
            bone_names.add(dp.split("['")[1].split("']")[0])
    for bn in sorted(bone_names):
        in_char = bn in {b.name for b in char_arm.data.bones}
        in_vg   = char_mesh and bn in {vg.name for vg in char_mesh.vertex_groups}
        print(f"  '{bn}'  in_armature={in_char}  in_vertex_groups={in_vg}")
except AttributeError:
    print("  (Blender 4+ action — no direct fcurves attribute)")
    # Try through layers/strips
    for layer in getattr(walk_action, 'layers', []):
        for strip in getattr(layer, 'strips', []):
            for fc in getattr(strip, 'fcurves', []):
                dp = fc.data_path
                if '["' in dp:
                    bone_names.add(dp.split('["')[1].split('"]')[0])
    print(f"  Animated bones via layers: {sorted(bone_names)[:20]}")

print("\n=== END ===")
