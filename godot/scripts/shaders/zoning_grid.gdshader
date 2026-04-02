shader_type spatial;
render_mode unshaded, cull_disabled;

varying vec4 v_color;
varying float v_is_front;

void vertex() {
    v_color = COLOR;
    v_is_front = INSTANCE_CUSTOM.x; // 1.0 for frontage cells
}

void fragment() {
    float border = 0.05;
    vec2 uv = UV;
    
    // Grid Lines (Dark)
    bool is_border = uv.x < border || uv.x > 1.0 - border || uv.y < border || uv.y > 1.0 - border;
    
    // Frontage Arrow (pointing towards the road edge at uv.y=0.0)
    bool is_arrow = false;
    if (v_is_front > 0.5) {
        // Arrow head pointing to top edge (y=0.1 in UV)
        if (uv.y > 0.1 && uv.y < 0.4) {
            float dist_from_center = abs(uv.x - 0.5);
            float max_dist = (uv.y - 0.1) * 0.33 + 0.02; 
            if (dist_from_center < max_dist) {
                is_arrow = true;
            }
        }
        // Stem
        if (uv.x > 0.47 && uv.x < 0.53 && uv.y >= 0.4 && uv.y < 0.7) {
            is_arrow = true;
        }
    }
    
    if (is_border || is_arrow) {
        ALBEDO = vec3(0.05, 0.05, 0.1); // Dark navy/black
        ALPHA = v_color.a * 0.8;
    } else {
        ALBEDO = v_color.rgb;
        ALPHA = v_color.a * 0.15; // Highly transparent interior
    }
}
