//! Column-major 4x4 matrices in OpenGL's layout (`m[column * 4 + row]`), for the terrain's camera.
//! Frustum culling works on the same matrices: the six planes are read from the view-projection.

pub type Mat4 = [f32; 16];
pub type Vec3 = [f32; 3];
/// Plane as `[nx, ny, nz, d]`: a point `p` is inside when `dot(n, p) + d >= 0`.
pub type Plane = [f32; 4];

pub fn identity() -> Mat4 {
    let mut m = [0.0; 16];
    m[0] = 1.0;
    m[5] = 1.0;
    m[10] = 1.0;
    m[15] = 1.0;
    m
}

/// `a * b`: applies `b` first.
pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
        }
    }
    out
}

/// The OpenGL perspective projection (`gluPerspective`), with the vertical field of view in degrees.
pub fn perspective(fovy_degrees: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fovy_degrees.to_radians() / 2.0).tan();
    let mut m = [0.0; 16];
    m[0] = f / aspect;
    m[5] = f;
    m[10] = (far + near) / (near - far);
    m[11] = -1.0;
    m[14] = 2.0 * far * near / (near - far);
    m
}

/// The view matrix looking from `eye` at `target` (`gluLookAt`).
pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    let f = normalize(sub(target, eye));
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    [
        s[0], u[0], -f[0], 0.0,
        s[1], u[1], -f[1], 0.0,
        s[2], u[2], -f[2], 0.0,
        -dot(s, eye), -dot(u, eye), dot(f, eye), 1.0,
    ]
}

/// `m * v` for a homogeneous point.
pub fn transform(m: &Mat4, v: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for (row, slot) in out.iter_mut().enumerate() {
        *slot = (0..4).map(|col| m[col * 4 + row] * v[col]).sum();
    }
    out
}

/// The six planes of a view-projection matrix (Gribb and Hartmann), with normals pointing inwards.
pub fn frustum_planes(vp: &Mat4) -> [Plane; 6] {
    let row = |i: usize| [vp[i], vp[4 + i], vp[8 + i], vp[12 + i]];
    let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
    let add = |a: [f32; 4], b: [f32; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
    let sub4 = |a: [f32; 4], b: [f32; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
    [add(r3, r0), sub4(r3, r0), add(r3, r1), sub4(r3, r1), add(r3, r2), sub4(r3, r2)]
}

/// Whether the box `[min, max]` touches the frustum. Conservative: a box outside the frustum's
/// corners but inside a plane's cone is still drawn, never the other way round.
pub fn aabb_visible(planes: &[Plane; 6], min: Vec3, max: Vec3) -> bool {
    planes.iter().all(|p| {
        // The corner furthest along the plane's normal: if even that one is outside, the box is.
        let x = if p[0] >= 0.0 { max[0] } else { min[0] };
        let y = if p[1] >= 0.0 { max[1] } else { min[1] };
        let z = if p[2] >= 0.0 { max[2] } else { min[2] };
        p[0] * x + p[1] * y + p[2] * z + p[3] >= 0.0
    })
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(v: Vec3) -> Vec3 {
    let len = dot(v, v).sqrt();
    [v[0] / len, v[1] / len, v[2] / len]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn look_at_puts_the_target_straight_ahead() {
        let view = look_at([0.0, 0.0, 5.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let p = transform(&view, [0.0, 0.0, 0.0, 1.0]);
        assert!(close(p[0], 0.0) && close(p[1], 0.0) && close(p[2], -5.0), "{p:?}");
    }

    #[test]
    fn a_point_straight_ahead_projects_to_the_centre_of_the_screen() {
        let vp = mul(&perspective(60.0, 4.0 / 3.0, 0.5, 100.0), &look_at([0.0; 3], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]));
        let clip = transform(&vp, [0.0, 0.0, -10.0, 1.0]);
        assert!(close(clip[0], 0.0) && close(clip[1], 0.0), "{clip:?}");
        assert!(clip[3] > 0.0, "in front of the camera");
    }

    #[test]
    fn the_projection_spans_the_field_of_view() {
        // At depth 10 the top edge of a 90-degree view is at y = 10.
        let proj = perspective(90.0, 1.0, 0.5, 100.0);
        let top = transform(&proj, [0.0, 10.0, -10.0, 1.0]);
        assert!(close(top[1] / top[3], 1.0), "{top:?}");
    }

    #[test]
    fn frustum_keeps_boxes_ahead_and_drops_boxes_behind() {
        let vp = mul(&perspective(60.0, 1.0, 0.5, 100.0), &look_at([0.0; 3], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]));
        let planes = frustum_planes(&vp);
        assert!(aabb_visible(&planes, [-1.0, -1.0, -11.0], [1.0, 1.0, -9.0]), "a box ahead");
        assert!(!aabb_visible(&planes, [-1.0, -1.0, 9.0], [1.0, 1.0, 11.0]), "a box behind the eye");
        assert!(!aabb_visible(&planes, [50.0, -1.0, -11.0], [52.0, 1.0, -9.0]), "a box far to the side");
    }
}
