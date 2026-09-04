//! Canonical generated-contact geometry helpers.

mod edges;
mod overlay;
mod point_location;

pub(super) use edges::{
    generated_contact_edges_from_overlay_intersection,
    generated_contact_edges_from_overlay_shape_intersection,
    generated_contact_edges_from_source_edges_inside_shape_key_intersection,
    generated_contact_edges_inside_contour, generated_contact_points_from_contour_intersections,
    generated_directed_edge_segments_inside_shape_keys,
    generated_shape_boundary_segments_on_source_edge,
};
pub(super) use overlay::{
    GeneratedOverlayShapeKeys, generated_contour_overlay_shapes, generated_overlay_contour,
    generated_overlay_shape_keys_directed_edges, generated_overlay_shapes_keys,
};
pub(super) use point_location::{
    generated_contour_boundary_contains_key, generated_contour_contains_key,
};
