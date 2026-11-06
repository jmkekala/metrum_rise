shader_type spatial;
render_mode cull_disabled, depth_draw_opaque, specular_disabled;

#define ZONING_SITE_SURFACE
#include "res://scripts/shaders/paving_surface.gdshaderinc"
