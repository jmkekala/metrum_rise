// SPDX-License-Identifier: GPL-2.0-only

//! The encoder against bytes Godot 4.7.2 stored for the same arrays (read back with
//! `RenderingServer.mesh_get_surface` by CB's `format_probe.gd` and CB2's `aabb_probe.gd`; the
//! Godot test `road_surface_bytes_test.gd` checks whole chunks).

use super::*;

fn hex(text: &str) -> Vec<u8> {
    let text: String = text.split_whitespace().collect();
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

fn normalized_vector(x: f64, y: f64, z: f64) -> Vector3 {
    let [x, y, z] = normalized([x as f32, y as f32, z as f32]);
    Vector3::new(x, y, z)
}

// `format_probe.gd`'s six vertices, as GDScript makes them.
fn probe_arrays() -> (Vec<Vector3>, Vec<Vector3>, Vec<Vector2>, Vec<Color>, Vec<Vector4>) {
    let mut arrays = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..6 {
        let f = f64::from(i);
        arrays.0.push(Vector3::new((1.5 + f) as f32, (-2.25 * f) as f32, (0.125 * f * f) as f32));
        arrays.1.push(normalized_vector(0.3 - 0.2 * f, 0.9, -0.1 * f));
        arrays.2.push(Vector2::new((0.25 * f) as f32, (-1.5 + f) as f32));
        arrays.3.push(Color::from_rgba((0.1 * f) as f32, 0.5, (1.0 - 0.15 * f) as f32, 0.333));
        let lane = |k: f64| (0.5 * f + k) as f32;
        arrays.4.push(Vector4::new(lane(0.0), lane(1.0), lane(2.0), lane(3.0)));
    }
    arrays
}

const PROBE_VERTEX_DATA: &str = "0000c03f000000800000000000002040000010c00000003e00006040000090c00000003f000090400000d8c00000903f0000b040000010c1000000400000d040000034c100004840ff9fffdf6227c4ce45975cf4aa0aa9caff5f54f5b90e01afcc4c65e6ff1fffa7ff3f70dc212d5ca5db3654d59d38eba4";

#[test]
fn a_layer_is_the_bytes_godot_stores() {
    let (vertices, normals, uvs, colors, _) = probe_arrays();
    let surface = encode_layer(&vertices, &normals, &uvs, &colors, None);
    assert_eq!(surface.format, 0x8_0000_001f);
    assert_eq!(surface.vertex_count, 6);
    assert_eq!(surface.vertex_data, hex(PROBE_VERTEX_DATA));
    assert_eq!(
        surface.attribute_data,
        hex("007fff54000000000000c0bf197fd8540000803e000000bf337fb2540000003f0000003f4c7f8c540000403f0000c03f667f66540000803f000020407f7f3f540000a03f00006040")
    );
}

#[test]
fn the_road_layer_carries_custom0_as_four_floats() {
    let (vertices, normals, uvs, colors, custom) = probe_arrays();
    let surface = encode_layer(&vertices, &normals, &uvs, &colors, Some(&custom));
    assert_eq!(surface.format, 0x8_0000_e05f);
    assert_eq!(surface.vertex_data, hex(PROBE_VERTEX_DATA));
    assert_eq!(
        surface.attribute_data,
        hex("007fff54000000000000c0bf000000000000803f0000004000004040197fd8540000803e000000bf0000003f0000c03f0000204000006040337fb2540000003f0000003f0000803f0000004000004040000080404c7f8c540000403f0000c03f0000c03f000020400000604000009040667f66540000803f000020400000004000004040000080400000a0407f7f3f540000a03f000060400000204000006040000090400000b040")
    );
}

#[test]
fn axis_normals_negative_z_and_zero_encode_as_godot() {
    let normals = [
        (0.0, 1.0, 0.0),
        (0.0, -1.0, 0.0),
        (1.0, 0.0, 0.0),
        (-1.0, 0.0, 0.0),
        (0.0, 0.0, 1.0),
        (0.0, 0.0, -1.0),
        (0.6, 0.0, -0.8),
        (-0.3, -0.4, -0.866),
        (0.0, 0.0, 0.0),
        (0.0, 2.0, 0.0),
    ]
    .map(|(x, y, z)| Vector3::new(x, y, z));
    let vertices = [Vector3::ZERO; 10];
    let surface = encode_layer(&vertices, &normals, &[Vector2::ZERO; 10], &[Color::BLACK; 10], None);
    assert_eq!(
        surface.vertex_data[120..],
        hex("ff7fffff0000ffbf ff7f00000000ffbf ffffff7fff7fffbf 0000ff7fff7fffbf ff7fff7fff7f0080 ffffffffff7f0080 ffff23c982a952a4 b1208518d75bdea4 0000000000000080 ff7fffff0000ffbf")
    );
}

fn aabb_bits(vertices: &[Vector3]) -> Vec<u8> {
    let surface = encode_positions(vertices);
    assert_eq!(surface.format, 0x8_0000_0001);
    surface.aabb.iter().flat_map(|value| value.to_le_bytes()).collect()
}

#[test]
fn the_aabb_is_the_exact_bounds_with_the_smallest_extent() {
    let v = |x: f32, y: f32, z: f32| Vector3::new(x, y, z);
    assert_eq!(aabb_bits(&[v(1.5, 0.0, 0.0)]), hex("0000c03f0000000000000000acc52737acc52737acc52737"));
    assert_eq!(
        aabb_bits(&[v(1.5, 0.0, 0.0), v(6.5, -11.25, 3.125)]),
        hex("0000c03f000034c1000000000000a0400000344100004840")
    );
    assert_eq!(
        aabb_bits(&[v(0.1, 0.2, 0.3), v(-0.7, 0.25, 1e-7)]),
        hex("333333bfcdcc4c3e95bfd633cdcc4c3fcccc4c3d9799993e")
    );
    assert_eq!(
        aabb_bits(&[v(0.0, 0.0, 0.0), v(1e-6, 2e-5, -3e-6)]),
        hex("00000000000000009c5349b6acc52737acc5a737acc52737")
    );
}

#[test]
fn a_chunk_lists_its_non_empty_layers_and_the_lining_caster() {
    let (vertices, normals, uvs, colors, custom) = probe_arrays();
    let mut mesh = NetworkMeshData::new();
    mesh.curb_vertices = vertices.clone();
    mesh.curb_normals = normals.clone();
    mesh.curb_uvs = uvs.clone();
    mesh.curb_colors = colors.clone();
    mesh.road_vertices = vertices.clone();
    mesh.road_normals = normals.clone();
    mesh.road_uvs = uvs.clone();
    mesh.road_colors = colors.clone();
    mesh.road_crossings = custom;
    mesh.lining_vertices = vertices.clone();
    mesh.lining_normals = normals;
    mesh.lining_uvs = uvs;
    mesh.lining_colors = colors;
    let surfaces = RoadChunkSurfaces::encode(&mesh);
    let layers: Vec<usize> = surfaces.layers.iter().map(|(layer, _)| *layer).collect();
    assert_eq!(layers, [1, ROAD_LAYER, LINING_LAYER]);
    assert_eq!(surfaces.layers[1].1.format, 0x8_0000_e05f);
    let shadow = surfaces.lining_shadow.as_ref().unwrap();
    assert_eq!(shadow.vertex_data, hex(PROBE_VERTEX_DATA)[..72]);
    assert!(shadow.attribute_data.is_empty());
    let bytes: usize = surfaces.layers.iter().map(|(_, surface)| surface.bytes()).sum();
    assert_eq!(bytes + shadow.bytes(), 120 * 3 + 72 + 168 + 72 + 72);
}
