// SPDX-License-Identifier: GPL-2.0-only

//! Godot's own arithmetic for the few operations the vegetation product reproduces bit for bit
//! (`RENDER-32-C`): `hash()` of an `int` and of a `Vector2`, `Basis` from an axis and an angle,
//! `Basis * Basis`, `Transform3D * AABB` and `AABB.merge`, each in the operation order of the
//! engine (Godot 4.7, `real_t` single precision). Single precision where the engine stores
//! `real_t`, double where GDScript computes with `float`; no fused multiply-add anywhere, like
//! the engine's x86-64 build. Checked against vectors from Godot in the module's tests and by
//! the product parity goldens.

const HASH_MURMUR3_SEED: u32 = 0x07F0_7C65;

fn murmur3_one_32(input: u32, seed: u32) -> u32 {
    let k = input
        .wrapping_mul(0xcc9e_2d51)
        .rotate_left(15)
        .wrapping_mul(0x1b87_3593);
    (seed ^ k)
        .rotate_left(13)
        .wrapping_mul(5)
        .wrapping_add(0xe654_6b64)
}

// Godot hashes both zeros and every NaN alike.
fn murmur3_one_f32(value: f32, seed: u32) -> u32 {
    let bits = if value == 0.0 {
        0.0f32.to_bits()
    } else if value.is_nan() {
        f32::NAN.to_bits()
    } else {
        value.to_bits()
    };
    murmur3_one_32(bits, seed)
}

fn fmix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

/// GDScript `hash(Vector2(x, y))`.
pub(crate) fn hash_vector2(x: f32, y: f32) -> u32 {
    fmix32(murmur3_one_f32(y, murmur3_one_f32(x, HASH_MURMUR3_SEED)))
}

/// GDScript `hash(value)` of an `int` (Godot's `hash_one_uint64`).
pub(crate) fn hash_int(value: i64) -> u32 {
    let mut v = value as u64;
    v = (!v).wrapping_add(v << 18);
    v ^= v >> 31;
    v = v.wrapping_mul(21);
    v ^= v >> 11;
    v = v.wrapping_add(v << 6);
    v ^= v >> 22;
    v as u32
}

/// A `Basis` as Godot stores it: three rows.
pub(crate) type Basis = [[f32; 3]; 3];

/// `Basis(axis, angle)`: Godot's `Basis::set_axis_angle`.
pub(crate) fn basis_axis_angle(axis: [f32; 3], angle: f32) -> Basis {
    let [x, y, z] = axis;
    let (sq_x, sq_y, sq_z) = (x * x, y * y, z * z);
    let cosine = angle.cos();
    let sine = angle.sin();
    let t = 1.0 - cosine;
    let mut rows = [[0.0f32; 3]; 3];
    rows[0][0] = sq_x + cosine * (1.0 - sq_x);
    rows[1][1] = sq_y + cosine * (1.0 - sq_y);
    rows[2][2] = sq_z + cosine * (1.0 - sq_z);
    let (xyzt, zyxs) = (x * y * t, z * sine);
    rows[0][1] = xyzt - zyxs;
    rows[1][0] = xyzt + zyxs;
    let (xyzt, zyxs) = (x * z * t, y * sine);
    rows[0][2] = xyzt + zyxs;
    rows[2][0] = xyzt - zyxs;
    let (xyzt, zyxs) = (y * z * t, x * sine);
    rows[1][2] = xyzt - zyxs;
    rows[2][1] = xyzt + zyxs;
    rows
}

/// `a * b`: Godot's `Basis::operator*=`, each entry `b[0][j] a[i][0] + b[1][j] a[i][1] +
/// b[2][j] a[i][2]` summed left to right.
pub(crate) fn basis_mul(a: &Basis, b: &Basis) -> Basis {
    let mut rows = [[0.0f32; 3]; 3];
    for (out, row) in rows.iter_mut().zip(a) {
        for (j, entry) in out.iter_mut().enumerate() {
            *entry = b[0][j] * row[0] + b[1][j] * row[1] + b[2][j] * row[2];
        }
    }
    rows
}

/// `Basis(x_axis, y_axis, z_axis)`: the three columns.
pub(crate) fn basis_columns(x: [f32; 3], y: [f32; 3], z: [f32; 3]) -> Basis {
    [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]]
}

/// Squared length of a basis column, `(x * x + y * y) + z * z`.
pub(crate) fn column_length_squared(basis: &Basis, column: usize) -> f32 {
    let (x, y, z) = (basis[0][column], basis[1][column], basis[2][column]);
    x * x + y * y + z * z
}

/// Godot's `AABB`: a position and a size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Aabb {
    pub(crate) position: [f32; 3],
    pub(crate) size: [f32; 3],
}

// Godot's MIN and MAX macros: the second operand wins a tie, which decides signed zeros.
fn min_f32(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

fn max_f32(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// `max(a, b)` in GDScript's double precision (`maxf`).
pub(crate) fn max_f64(a: f64, b: f64) -> f64 {
    if a > b { a } else { b }
}

/// Godot's `Vector3::min`, lane by lane.
pub(crate) fn min3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        min_f32(a[0], b[0]),
        min_f32(a[1], b[1]),
        min_f32(a[2], b[2]),
    ]
}

/// Godot's `Vector3::max`, lane by lane.
pub(crate) fn max3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        max_f32(a[0], b[0]),
        max_f32(a[1], b[1]),
        max_f32(a[2], b[2]),
    ]
}

/// `a - b`, lane by lane.
pub(crate) fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `Vector3::length`: `sqrt((x * x + y * y) + z * z)` in single precision.
pub(crate) fn length3(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

impl Aabb {
    /// `AABB(position, size)`.
    pub(crate) fn new(position: [f32; 3], size: [f32; 3]) -> Self {
        Self { position, size }
    }

    /// Whether the size is exactly zero, Godot's `size == Vector3.ZERO`.
    pub(crate) fn is_zero_size(&self) -> bool {
        self.size == [0.0; 3]
    }

    /// `AABB::merge`: the end of each box is recomputed as `size + position` first, so merging a
    /// box with itself can move its end by a rounding step, as in the engine.
    pub(crate) fn merge(&self, other: &Aabb) -> Aabb {
        let end_1 = add3(self.size, self.position);
        let end_2 = add3(other.size, other.position);
        let low = min3(self.position, other.position);
        let high = max3(end_1, end_2);
        Aabb::new(low, sub3(high, low))
    }

    /// `AABB::grow`: every side moves out by `by`.
    pub(crate) fn grow(&self, by: f32) -> Aabb {
        let p = self.position;
        let s = self.size;
        Aabb::new(
            [p[0] - by, p[1] - by, p[2] - by],
            [s[0] + 2.0 * by, s[1] + 2.0 * by, s[2] + 2.0 * by],
        )
    }

    /// `AABB::get_center`: `position + size * 0.5`.
    pub(crate) fn center(&self) -> [f32; 3] {
        add3(
            self.position,
            [self.size[0] * 0.5, self.size[1] * 0.5, self.size[2] * 0.5],
        )
    }

    /// The eight `AABB::get_endpoint` corners.
    pub(crate) fn corners(&self) -> [[f32; 3]; 8] {
        let p = self.position;
        let e = add3(self.position, self.size);
        std::array::from_fn(|i| {
            [
                if i & 4 != 0 { e[0] } else { p[0] },
                if i & 2 != 0 { e[1] } else { p[1] },
                if i & 1 != 0 { e[2] } else { p[2] },
            ]
        })
    }

    /// `Transform3D::xform(AABB)` for a transform in Godot's MultiMesh row layout (three rows of
    /// basis then origin): the classic per-axis min and max of `basis[i][j] * bound[j]`.
    pub(crate) fn transformed(&self, rows: &[f32]) -> Aabb {
        let low = self.position;
        let high = add3(self.position, self.size);
        let mut tmin = [0.0f32; 3];
        let mut tmax = [0.0f32; 3];
        for i in 0..3 {
            let origin = rows[i * 4 + 3];
            tmin[i] = origin;
            tmax[i] = origin;
            for j in 0..3 {
                let entry = rows[i * 4 + j];
                let e = entry * low[j];
                let f = entry * high[j];
                if e < f {
                    tmin[i] += e;
                    tmax[i] += f;
                } else {
                    tmin[i] += f;
                    tmax[i] += e;
                }
            }
        }
        Aabb::new(tmin, sub3(tmax, tmin))
    }

    /// The six lanes as the bridge packs them: position, then size.
    pub(crate) fn lanes(&self) -> [f32; 6] {
        let (p, s) = (self.position, self.size);
        [p[0], p[1], p[2], s[0], s[1], s[2]]
    }
}

/// `a + b`, lane by lane.
pub(crate) fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// GDScript `lerpf`: `from + (to - from) * weight`.
pub(crate) fn lerp_f64(from: f64, to: f64, weight: f64) -> f64 {
    from + (to - from) * weight
}

/// GDScript `smoothstep` for `from < to`: the clamped cubic.
pub(crate) fn smoothstep_f64(from: f64, to: f64, value: f64) -> f64 {
    let s = ((value - from) / (to - from)).clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}
