# Writes the parking pack's lot asset.toml files; layout numbers match parking_lot_meshes.py.
# Usage: python3 tools/parking_lot_assets.py godot/bootstrap/mods/parking/assets
import sys

AISLE_HALF, BAY_LEN, PITCH = 3.5, 5.0, 2.5


def lot(asset_id, name, depth_cells, z_rear, bays, markings):
    half = depth_cells * 5.0
    front, rear = half, -half
    head = AISLE_HALF + BAY_LEN
    out = [f'''asset_id = "{asset_id}"
display_name = "{name}"

[building]
placement_mode = "explicit"
service_class = "parking"
lot_width_cells = 2
lot_depth_cells = {depth_cells}
frontage_forward = [0, 0, 1]
worker_capacity = 0
household_capacity = 0
window_brightness = 0

[[mesh_parts]]
name = "markings"
position = [0, 0, 0]
scale = 1
[[mesh_parts.lods]]
file = "{markings}"
distance_min_m = 0

[[mesh_parts]]
name = "p_sign"
position = [{head + 0.8}, 0, {front - 0.6}]
scale = 1
[[mesh_parts.lods]]
file = "p_sign.glb"
distance_min_m = 0
''']
    # One two-socket heater post between each pair of bays, on the strip behind the kerb.
    for side, sign in (("left", -1), ("right", 1)):
        for k in range(bays // 2):
            z = z_rear + PITCH * (2 * k + 1)
            out.append(f'''
[[mesh_parts]]
name = "heater_post_{side}_{k + 1}"
position = [{sign * (head + 0.6)}, 0, {z}]
scale = 1
[[mesh_parts.lods]]
file = "heating_post.glb"
distance_min_m = 0
''')
    out.append(f'''
[[anchors]]
type = "entrance"
name = "main"
position = [0, 0, {front}]
forward = [0, 0, 1]

[[anchors]]
type = "driveway"
position = [0, 0, {front}]
forward = [0, 0, -1]
width_m = 6
vehicle_class = "car"

[[site_surfaces]]
material = "asphalt"
y_m = 0.01
vertices = [[{-head}, {rear}], [{head}, {rear}], [{head}, {front}], [{-head}, {front}]]
''')
    for side, sign in (("left", -1), ("right", 1)):
        for k in range(bays):
            z = z_rear + PITCH * (k + 0.5)
            out.append(f'''
[[anchors]]
type = "parking"
name = "{side} {k + 1}"
position = [{sign * AISLE_HALF}, 0, {z}]
forward = [{sign}, 0, 0]
width_m = 2.5
length_m = 5
vehicle_class = "car"
''')
    return "".join(out)


root = sys.argv[1]
for asset_id, name, depth, z_rear, bays, markings in (
    ("building.parking.small_lot", "Small parking lot (12 bays)", 2, -8.5, 6,
     "small_lot_markings.glb"),
    ("building.parking.medium_lot", "Parking lot (20 bays)", 3, -13.5, 10,
     "medium_lot_markings.glb"),
):
    with open(f"{root}/{asset_id}/asset.toml", "w") as f:
        f.write(lot(asset_id, name, depth, z_rear, bays, markings))
