# Blender script: builds the parking pack's lot meshes as glTF binaries.
# Usage: blender -b -P tools/parking_lot_meshes.py -- <out_dir>
# Game axes: +Y up, lot front (street) at +Z. Blender exports (x, y, z) as glTF (x, z, -y),
# so a game point (gx, gy, gz) is built at Blender (gx, -gz, gy).
import math
import os
import sys

import bmesh
import bpy

OUT = sys.argv[sys.argv.index("--") + 1]

PAINT = (0.92, 0.92, 0.88, 1.0)
KERB = (0.55, 0.55, 0.53, 1.0)
POST = (0.62, 0.64, 0.66, 1.0)
POST_HEAD = (0.30, 0.32, 0.34, 1.0)
# Blender colours are linear. The sign takes the road-sign atlas's values (godot/assets/models/
# road_signs): Väylä's sign blue #005EB8, the pole grey #9A9FA0, and its finish.
SIGN_BLUE = (0.0, 0.112, 0.479, 1.0)
SIGN_POLE = (0.323, 0.347, 0.352, 1.0)
SIGN_ROUGHNESS = 0.46
SIGN_METALLIC = 0.18
WHITE = (1.0, 1.0, 1.0, 1.0)


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def material(name, colour, rough=0.85, metal=0.0):
    mat = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = colour
    bsdf.inputs["Roughness"].default_value = rough
    bsdf.inputs["Metallic"].default_value = metal
    return mat


def box(name, mat, x0, x1, y0, y1, z0, z1):
    # Game-space box: x0..x1 across, y0..y1 up, z0..z1 along the lot.
    bpy.ops.mesh.primitive_cube_add(size=1.0)
    ob = bpy.context.active_object
    ob.name = name
    ob.scale = (x1 - x0, z1 - z0, y1 - y0)
    ob.location = ((x0 + x1) / 2, -(z0 + z1) / 2, (y0 + y1) / 2)
    ob.data.materials.append(mat)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    return ob


def export(path):
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.join()
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB", use_selection=True,
                              export_apply=True)


def lot_markings(path, aisle_half, bay_len, z_rear, bays, asphalt_rear, lot_front):
    # Painted bay separators on both sides of a central aisle, kerbs along the asphalt edge.
    reset()
    paint = material("paint", PAINT, 0.7)
    kerb = material("kerb", KERB)
    for side in (-1, 1):
        inner, outer = side * aisle_half, side * (aisle_half + bay_len)
        # Lines stop 0.3 m short of the kerb, as painted lines usually do.
        lo, hi = sorted((inner, outer - side * 0.3))
        for k in range(bays + 1):
            z = z_rear + k * 2.5
            box(f"line_{side}_{k}", paint, lo, hi, 0.012, 0.022, z - 0.05, z + 0.05)
        x0, x1 = sorted((outer, outer + side * 0.15))
        box(f"kerb_{side}", kerb, x0, x1, 0.0, 0.12, asphalt_rear - 0.15, lot_front)
    box("kerb_rear", kerb, -(aisle_half + bay_len), aisle_half + bay_len, 0.0, 0.12,
        asphalt_rear - 0.15, asphalt_rear)
    export(path)


def heating_post(path):
    # A two-socket engine-heater post (lämmitystolppa): steel post and a socket box on top.
    reset()
    steel = material("post", POST, 0.5, 0.6)
    head = material("post_head", POST_HEAD, 0.6, 0.3)
    box("post", steel, -0.05, 0.05, 0.0, 1.0, -0.05, 0.05)
    box("head", head, -0.11, 0.11, 1.0, 1.32, -0.09, 0.09)
    box("cap", steel, -0.12, 0.12, 1.32, 1.36, -0.10, 0.10)
    export(path)


# DIN-style face like Finnish road signs; any bold sans works if this one is missing.
SIGN_FONT = "/System/Library/Fonts/Supplemental/DIN Alternate Bold.ttf"


def rounded_panel(name, mat, width, height, radius, y_front, depth, centre_up):
    # A rounded-corner panel facing game +Z, its front face at game z = y_front.
    bpy.ops.mesh.primitive_plane_add(size=1.0)
    ob = bpy.context.active_object
    ob.name = name
    ob.scale = (width, height, 1.0)
    bpy.ops.object.transform_apply(scale=True)
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    # bmesh's bevel profile defaults to 0, a square notch; 0.5 is a round corner.
    bmesh.ops.bevel(bm, geom=list(bm.verts), offset=radius, segments=6, profile=0.5,
                    affect="VERTICES")
    bm.to_mesh(ob.data)
    bm.free()
    # The plane lies in Blender XY; stand it up to face Blender -Y (game +Z), then give it depth.
    ob.rotation_euler = (math.pi / 2, 0.0, 0.0)
    ob.location = (0.0, -y_front, centre_up)
    bpy.ops.object.transform_apply(location=True, rotation=True)
    solid = ob.modifiers.new("depth", "SOLIDIFY")
    solid.thickness = depth
    solid.offset = -1.0
    bpy.ops.object.modifier_apply(modifier=solid.name)
    ob.data.materials.append(mat)
    return ob


def p_sign(path):
    # Finnish parking sign (E1): white-bordered rounded blue square with a white P, on a pole.
    reset()
    steel = material("pole", SIGN_POLE, SIGN_ROUGHNESS, SIGN_METALLIC)
    blue = material("sign_blue", SIGN_BLUE, SIGN_ROUGHNESS, SIGN_METALLIC)
    white = material("sign_white", WHITE, SIGN_ROUGHNESS, SIGN_METALLIC)
    box("pole", steel, -0.03, 0.03, 0.0, 2.6, -0.03, 0.03)
    up = 2.3
    rounded_panel("plate", white, 0.62, 0.62, 0.05, 0.06, 0.02, up)
    rounded_panel("face", blue, 0.57, 0.57, 0.035, 0.065, 0.004, up)
    bpy.ops.object.text_add()
    text = bpy.context.active_object
    if os.path.exists(SIGN_FONT):
        text.data.font = bpy.data.fonts.load(SIGN_FONT)
    text.data.body = "P"
    text.data.size = 0.66
    text.data.align_x = "CENTER"
    text.data.align_y = "CENTER"
    text.data.extrude = 0.002
    # Finnish sign lettering is heavier than the font; grow the outline.
    text.data.offset = 0.012
    text.data.resolution_u = 8
    text.rotation_euler = (math.pi / 2, 0.0, 0.0)
    text.location = (0.015, -0.071, up - 0.005)
    bpy.ops.object.convert(target="MESH")
    text.data.materials.clear()
    text.data.materials.append(white)
    export(path)


lot_markings(f"{OUT}/small_lot_markings.glb", 3.5, 5.0, -8.5, 6, -10.0, 10.0)
lot_markings(f"{OUT}/medium_lot_markings.glb", 3.5, 5.0, -13.5, 10, -15.0, 15.0)
heating_post(f"{OUT}/heating_post.glb")
p_sign(f"{OUT}/p_sign.glb")
