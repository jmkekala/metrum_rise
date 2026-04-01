"""
Validates that the Walk action from walk.blend actually deforms
the character mesh in characterLarge*.blend.

Usage:
  blender --background <char.blend> --python tools/validate_vat.py -- <walk.blend>
"""
import bpy, sys, math, os

argv  = sys.argv
walk_blend = os.path.abspath(argv[argv.index("--") + 1]) if "--" in argv else None

print(f"\n===== VAT VALIDATION =====")
print(f"Character: {bpy.data.filepath}")
print(f"Walk file: {walk_blend}")

# Find objects
char_mesh = next((o for o in bpy.data.objects
                  if o.type == 'MESH' and len(o.vertex_groups) > 5), None)
char_arm  = next((o for o in bpy.data.objects if o.type == 'ARMATURE'), None)

if not char_mesh or not char_arm:
    print("ERROR: could not find mesh or armature")
    sys.exit(1)

print(f"Mesh:      '{char_mesh.name}'  verts={len(char_mesh.data.vertices)}")
print(f"Armature:  '{char_arm.name}'")

# 1. Rest (T-pose): clear any animation so armature is in bind pose
char_arm.animation_data_create()
char_arm.animation_data.action = None
bpy.context.scene.frame_set(1)
depsgraph  = bpy.context.evaluated_depsgraph_get()
ev         = char_mesh.evaluated_get(depsgraph)
mv         = ev.to_mesh()
rest_co    = [v.co.copy() for v in mv.vertices]
ev.to_mesh_clear()
print(f"\nRest (T-pose) head Y: {max(v.y for v in rest_co):.4f}  "
      f"foot Y: {min(v.y for v in rest_co):.4f}")

# 2. Import walk action
if walk_blend:
    actions_before = set(bpy.data.actions.keys())
    with bpy.data.libraries.load(walk_blend, link=False) as (src, dst):
        print(f"Actions in walk.blend: {src.actions}")
        dst.actions = list(src.actions)
    new_names = set(bpy.data.actions.keys()) - actions_before
    print(f"Imported: {new_names}")

    if new_names:
        walk_action = max((bpy.data.actions[n] for n in new_names),
                          key=lambda a: a.frame_range[1] - a.frame_range[0])
        print(f"Using action: '{walk_action.name}'  "
              f"frames {walk_action.frame_range[0]:.0f}-{walk_action.frame_range[1]:.0f}")

        # Blender 5 slot-based binding
        anim_data = char_arm.animation_data
        anim_data.action = walk_action
        if hasattr(anim_data, 'action_slot'):
            slot = None
            for s in walk_action.slots:
                if s.target_id_type == 'OBJECT':
                    slot = s
                    break
            if slot is None:
                slot = walk_action.slots.new(id_type='OBJECT', name=char_arm.name)
            if slot:
                anim_data.action_slot = slot
                print(f"Blender 5 slot assigned for {char_arm.name}")
    else:
        print("ERROR: no actions imported")
        sys.exit(1)
else:
    print("No walk.blend supplied — checking existing actions")

# 3. Sample frames and report max displacement
scene = bpy.context.scene
print(f"\nFrame  MaxDisp(m)  Status")
print(f"-----  ----------  ------")
for fr in [0, 4, 8, 12, 16, 20, 24, 28, 32]:
    scene.frame_set(fr)
    depsgraph = bpy.context.evaluated_depsgraph_get()
    ev        = char_mesh.evaluated_get(depsgraph)
    mv        = ev.to_mesh()
    max_d = max((mv.vertices[i].co - rest_co[i]).length
                for i in range(len(rest_co)))
    ev.to_mesh_clear()
    status = "✓ ANIMATED" if max_d > 0.005 else "⚠ near-zero"
    print(f"  {fr:3d}  {max_d:10.4f}  {status}")

print("\n===== END VALIDATION =====")
