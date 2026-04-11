shader_type spatial;
render_mode unshaded, cull_disabled, depth_draw_never;

// RG8 texture: one little-endian u16 runtime profile id per zoning cell.
uniform sampler2D zone_texture : filter_nearest;
// 1-row RGBA8 style LUT indexed by runtime profile id.
uniform sampler2D profile_style_lut : filter_nearest;
// R8 texture: distance to nearest road in metres (raw byte = metres, clamped 255)
uniform sampler2D distance_texture : filter_nearest;
// R8 texture: occupied bool (0=free, 255=occupied)
uniform sampler2D occupied_texture : filter_nearest;
// R8 texture: no-build mask (255 = within ~32m of a no_building_spawn road, 0 = buildable)
uniform sampler2D no_build_texture : filter_nearest;

// 0.0 = passive tint, 1.0 = tool active (brighter + cell grid)
uniform float tool_active : hint_range(0.0, 1.0) = 0.0;

// Number of zone cells per axis (default 2000 for a 20km map at 10m cells)
uniform float grid_cells : hint_range(1.0, 4096.0) = 2000.0;
// Number of entries in the 1-row style LUT.
uniform float profile_style_lut_size : hint_range(1.0, 1024.0) = 1.0;

float decode_runtime_id(vec2 rg) {
    float lo = floor(rg.r * 255.0 + 0.5);
    float hi = floor(rg.g * 255.0 + 0.5);
    return lo + hi * 256.0;
}

vec4 profile_style(float runtime_id) {
    float lut_x = (runtime_id + 0.5) / max(profile_style_lut_size, 1.0);
    return texture(profile_style_lut, vec2(lut_x, 0.5));
}

void fragment() {
    float runtime_id = decode_runtime_id(texture(zone_texture, UV).rg);
    vec4 style       = profile_style(runtime_id);
    float dist_raw   = texture(distance_texture, UV).r * 255.0;
    float occ        = texture(occupied_texture, UV).r; // 0.0 or ~1.0
    float no_build   = texture(no_build_texture, UV).r; // 0.0 or ~1.0

    // Invisible when: no zone type, within 3m of road edge, or masked by a no-build road.
    float visible = step(0.5, runtime_id) * step(3.0, dist_raw) * (1.0 - step(0.5, no_build));

    // Distance-to-road alpha scale
    float dist_alpha = 1.0;
    dist_alpha = mix(dist_alpha, mix(1.0, 0.4, (dist_raw - 40.0) / 60.0),
                     step(40.0, dist_raw) * step(dist_raw, 100.0));
    dist_alpha = mix(dist_alpha, 0.2, step(100.0, dist_raw));

    // Occupied cell suppression (0.4x when occupied)
    float occ_factor = mix(1.0, 0.4, occ);

    // Base alpha: 0.20 passive, 0.38 when tool active
    float base_alpha = mix(0.20, 0.38, tool_active);
    float alpha = base_alpha * dist_alpha * occ_factor * visible;

    // Colour
    vec3 col = style.rgb;

    // Procedural 10m cell grid when tool is active and camera is close enough
    vec2 cell_uv = fract(UV * grid_cells);
    vec2 fw = fwidth(UV * grid_cells);
    float cell_px = 1.0 / max(fw.x, fw.y);
    float line_w = 0.04;
    float on_grid = step(1.0, float(
        cell_uv.x < line_w || cell_uv.y < line_w ||
        cell_uv.x > 1.0 - line_w || cell_uv.y > 1.0 - line_w));
    float show_grid = step(0.5, tool_active) * step(4.0, cell_px) * on_grid;
    col   = mix(col, col * 0.5, show_grid);
    alpha = min(alpha + show_grid * 0.12, 1.0);

    ALBEDO = col;
    ALPHA  = alpha;
}
