// SPDX-License-Identifier: GPL-2.0-only

//! Rigid vehicle support over road, apron and pad boundaries.

use crate::nodes::sim::core::SimCore;
use godot::prelude::{Vector2, Vector3};
use rayon::prelude::*;

impl SimCore {
    /// Shared off-lane support solve for simulation snapshots and interpolated render poses.
    pub(crate) fn vehicle_ground_pose(
        &self,
        vehicle: u8,
        position: Vector2,
        heading: Vector3,
    ) -> (f32, [Vector3; 3]) {
        let support = self
            .vehicle_ground_support
            .get(vehicle as usize)
            .unwrap_or(&self.vehicle_ground_support[0]);
        vehicle_support_pose(support, position, heading, |point| {
            self.get_world_surface_height_internal(point)
        })
    }

    /// Corrects one render bucket in place. Input/index validation precedes every mutation.
    pub(crate) fn ground_vehicle_transforms(
        &self,
        vehicle: u8,
        transforms: &mut [f32],
        flags: &[u8],
    ) -> bool {
        if usize::from(vehicle) >= self.vehicle_ground_support.len()
            || transforms.len() != flags.len() * 12
            || !self.allocator.building_site_query_index_ready()
            || transforms.iter().any(|v| !v.is_finite())
        {
            return false;
        }
        self.solve_vehicle_transforms(vehicle, transforms, flags);
        true
    }

    /// Allocation-free parallel fill of validated, stable-order snapshot/render buckets.
    pub(crate) fn solve_vehicle_transforms(
        &self,
        vehicle: u8,
        transforms: &mut [f32],
        flags: &[u8],
    ) {
        transforms
            .par_chunks_exact_mut(12)
            .zip(flags.par_iter())
            .with_min_len(32)
            .for_each(|(transform, &flag)| {
                if flag == 0 {
                    return;
                }
                let point = Vector2::new(transform[3], transform[11]);
                let heading = Vector3::new(transform[2], 0.0, transform[10]);
                let (height, [x, y, z]) = self.vehicle_ground_pose(vehicle, point, heading);
                transform[0] = x.x;
                transform[1] = y.x;
                transform[2] = z.x;
                transform[4] = x.y;
                transform[5] = y.y;
                transform[6] = z.y;
                transform[8] = x.z;
                transform[9] = y.z;
                transform[10] = z.z;
                transform[7] = height;
            });
    }
}

/// Model-local support samples, prepared once when the renderer loads a vehicle mesh.
#[derive(Clone)]
pub(crate) struct VehicleGroundSupport {
    bounds: [f32; 4],
    contacts: Vec<Vector3>,
}

impl Default for VehicleGroundSupport {
    fn default() -> Self {
        // Matches the renderer's procedural fallback body. Loaded meshes replace this before use.
        Self::from_valid_bounds([-0.9, -2.1, 0.9, 2.1], &[])
    }
}

impl VehicleGroundSupport {
    /// Contact set retained for independent geometric assertions in local regressions.
    #[cfg(test)]
    pub(crate) fn contacts(&self) -> &[Vector3] {
        &self.contacts
    }

    /// Validates geometry metadata and prepares a bounded contact set, outside the agent loop.
    pub(crate) fn new(bounds: [f32; 4], mesh_contacts: &[Vector3]) -> Option<Self> {
        if bounds.iter().any(|v| !v.is_finite())
            || bounds[2] - bounds[0] <= 0.01
            || bounds[3] - bounds[1] <= 0.01
            || mesh_contacts.len() > 64
            || mesh_contacts.iter().any(|p| {
                !p.is_finite()
                    || p.y < -0.0001
                    || p.x < bounds[0] - 0.0001
                    || p.x > bounds[2] + 0.0001
                    || p.z < bounds[1] - 0.0001
                    || p.z > bounds[3] + 0.0001
            })
        {
            return None;
        }
        Some(Self::from_valid_bounds(bounds, mesh_contacts))
    }

    fn from_valid_bounds(bounds: [f32; 4], mesh_contacts: &[Vector3]) -> Self {
        let mut contacts = Vec::with_capacity(mesh_contacts.len() + 9);
        for z in [bounds[1], (bounds[1] + bounds[3]) * 0.5, bounds[3]] {
            for x in [bounds[0], (bounds[0] + bounds[2]) * 0.5, bounds[2]] {
                contacts.push(Vector3::new(x, 0.0, z));
            }
        }
        for &point in mesh_contacts {
            if !contacts.contains(&point) {
                contacts.push(point);
            }
        }
        Self { bounds, contacts }
    }
}

/// Aligns an existing horizontal model heading to a nonvertical, upward surface normal.
pub(crate) fn ground_vehicle_basis(heading: Vector3, normal: Vector3) -> [Vector3; 3] {
    let z = Vector3::new(
        heading.x,
        -(normal.x * heading.x + normal.z * heading.z) / normal.y,
        heading.z,
    )
    .normalized();
    let x = normal.cross(z).normalized();
    [x, z.cross(x).normalized(), z]
}

/// Fits cross/longitudinal grades over the whole footprint, then solves the minimum height
/// satisfying every contact at its final rotated XZ. No allocations or query-time mesh work.
pub(crate) fn vehicle_support_pose(
    support: &VehicleGroundSupport,
    position: Vector2,
    heading: Vector3,
    mut height: impl FnMut(Vector2) -> f32,
) -> (f32, [Vector3; 3]) {
    let heading = Vector3::new(heading.x, 0.0, heading.z);
    let z = if heading.length_squared() > 1e-6 {
        heading.normalized()
    } else {
        Vector3::BACK
    };
    let x = Vector3::UP.cross(z);
    let xz = |offset: Vector3| position + Vector2::new(offset.x, offset.z);
    let [left, front, right, rear] = support.bounds;
    let fl = height(xz(x * left + z * front));
    let fr = height(xz(x * right + z * front));
    let rl = height(xz(x * left + z * rear));
    let rr = height(xz(x * right + z * rear));
    // Least-squares plane through the four rectangular support samples. Unlike a centre-face
    // normal, these secants transition over the vehicle's length and width at a grade break.
    let cross_grade = ((fr - fl) + (rr - rl)) / (2.0 * (right - left));
    let long_grade = ((rl - fl) + (rr - fr)) / (2.0 * (rear - front));
    let normal = (Vector3::UP - x * cross_grade - z * long_grade).normalized();
    let basis = ground_vehicle_basis(z, normal);
    let mut support_y = f32::NEG_INFINITY;
    for point in &support.contacts {
        let offset = basis[0] * point.x + basis[1] * point.y + basis[2] * point.z;
        support_y = support_y.max(height(xz(offset)) - offset.y);
    }
    (support_y + 0.02, basis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footprint_plane_fit_preserves_yaw_and_grade_for_asymmetric_models() {
        let support =
            VehicleGroundSupport::new([-1.0, -3.0, 1.2, 2.0], &[Vector3::new(0.7, 0.0, -2.8)])
                .unwrap();
        for sx in [-0.3, 0.0, 0.3] {
            for sz in [-0.4, 0.0, 0.4] {
                for yaw in [0.0_f32, 0.5, 1.5, 3.2, 4.7] {
                    let heading = Vector3::new(yaw.sin(), 0.0, yaw.cos());
                    let (y, basis) = vehicle_support_pose(&support, Vector2::ZERO, heading, |p| {
                        sx * p.x + sz * p.y
                    });
                    assert!(basis[1].distance_to(Vector3::new(-sx, 1.0, -sz).normalized()) < 1e-5);
                    assert!(
                        Vector2::new(basis[2].x, basis[2].z)
                            .normalized()
                            .distance_to(Vector2::new(heading.x, heading.z))
                            < 1e-5
                    );
                    for p in &support.contacts {
                        let p = basis[0] * p.x + basis[1] * p.y + basis[2] * p.z;
                        assert!((y + p.y - sx * p.x - sz * p.z - 0.02).abs() < 1e-5);
                    }
                }
            }
        }
        assert!(
            VehicleGroundSupport::new([-1.0, -2.0, 1.0, 2.0], &[Vector3::new(3.0, 0.0, 0.0)])
                .is_none()
        );
        assert!(VehicleGroundSupport::new([0.0; 4], &[]).is_none());
    }

    #[test]
    fn mirrored_road_apron_pad_transitions_keep_all_contacts_supported() {
        let support = VehicleGroundSupport::default();
        for side in [-1.0, 1.0] {
            for direction in [-1.0, 1.0] {
                for rise in [-1.0, 1.0] {
                    for i in -100..200 {
                        let position = Vector2::new(0.0, side * i as f32 * 0.05);
                        let ground = |p: Vector2| {
                            let t = (side * p.y / 2.0).clamp(0.0, 1.0);
                            0.2 * p.x * (1.0 - t) + rise * t
                        };
                        let (y, basis) = vehicle_support_pose(
                            &support,
                            position,
                            Vector3::new(0.0, 0.0, side * direction),
                            ground,
                        );
                        let mut closest = f32::INFINITY;
                        for point in &support.contacts {
                            let offset =
                                basis[0] * point.x + basis[1] * point.y + basis[2] * point.z;
                            let clearance =
                                y + offset.y - ground(position + Vector2::new(offset.x, offset.z));
                            assert!(
                                clearance >= 0.0199,
                                "side={side} direction={direction} i={i} clearance={clearance}"
                            );
                            closest = closest.min(clearance);
                        }
                        assert!((closest - 0.02).abs() < 0.0001);
                        assert!(basis[0].cross(basis[1]).distance_to(basis[2]) < 1e-5);
                    }
                }
            }
        }
    }
}
