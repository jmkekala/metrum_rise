shader_type spatial;
render_mode unshaded, cull_disabled;

varying vec4 v_color;
varying float v_is_front;
varying float v_border_x;
varying float v_border_y;
varying float v_arrow_only; // 1.0 = draw only the arrow, no fill or border

void vertex() {
    v_color = COLOR;
    v_is_front  = INSTANCE_CUSTOM.x;
    v_border_x  = INSTANCE_CUSTOM.y > 0.0 ? INSTANCE_CUSTOM.y : 0.05;
    v_border_y  = INSTANCE_CUSTOM.z > 0.0 ? INSTANCE_CUSTOM.z : 0.05;
    v_arrow_only = INSTANCE_CUSTOM.w;
}

void fragment() {
    vec2 uv = UV;

    bool is_border = uv.x < v_border_x || uv.x > 1.0 - v_border_x
                  || uv.y < v_border_y || uv.y > 1.0 - v_border_y;

    bool is_arrow = false;
    if (v_is_front > 0.5) {
        if (uv.y > 0.1 && uv.y < 0.4) {
            float dist_from_center = abs(uv.x - 0.5);
            float max_dist = (uv.y - 0.1) * 0.33 + 0.02;
            if (dist_from_center < max_dist) is_arrow = true;
        }
        if (uv.x > 0.47 && uv.x < 0.53 && uv.y >= 0.4 && uv.y < 0.7)
            is_arrow = true;
    }

    if (v_arrow_only > 0.5) {
        // Arrow-only overlay: transparent everywhere except the arrow symbol itself.
        if (is_arrow) {
            ALBEDO = vec3(0.05, 0.05, 0.1);
            ALPHA  = v_color.a * 0.9;
        } else {
            ALPHA = 0.0;
        }
    } else if (is_border || is_arrow) {
        ALBEDO = vec3(0.05, 0.05, 0.1);
        ALPHA  = v_color.a * 0.8;
    } else {
        ALBEDO = v_color.rgb;
        ALPHA  = v_color.a * 0.15;
    }
}
