//! CPU-expanded constant-pixel strokes. Clip coordinates retain w so fog
//! distances interpolate perspectively, just like the game's line shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub clip: [f32; 4],
    pub color: [f32; 4],
    /// Spherical and cylindrical distances BEFORE the line layering transform.
    pub distances: [f32; 2],
}
impl Vertex {
    pub fn valid(&self) -> bool {
        self.clip.iter().all(|v| v.is_finite())
            && self.clip[3] > 0.
            && self
                .color
                .iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            && self.distances.iter().all(|v| v.is_finite() && *v >= 0.)
    }
}
/// Same source-edge ceiling as the modern block outline; never truncate a shape.
pub const MAX_EDGES: usize = 256;
/// Debug gizmos (chunk borders, hitboxes) are many more edges than one outline,
/// and each quad is four 16-bit indices, so this stays under `u16::MAX`.
pub const MAX_LINES: usize = 8192;
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
}
impl Mesh {
    pub fn bytes(&self) -> usize {
        size_of_val(self.vertices.as_slice()) + size_of_val(self.indices.as_slice())
    }
    /// Column-major conventional GL projection of camera-relative positions.
    /// Each layer is (pixel width, RGBA), drawn in order, not per-edge order.
    pub fn new(
        edges: &[[f64; 6]],
        camera: [f64; 3],
        matrix: [f32; 16],
        size: [u32; 2],
        layers: &[(f32, [f32; 4])],
    ) -> Result<Self, &'static str> {
        if edges.len() > MAX_EDGES
            || layers.len() > 2
            || (!edges.is_empty() && layers.is_empty())
            || size.contains(&0)
            || camera.iter().any(|v| !v.is_finite())
            || matrix.iter().any(|v| !v.is_finite())
            || layers.iter().any(|(w, c)| {
                !w.is_finite()
                    || !(0. ..=64.).contains(w)
                    || *w == 0.
                    || c.iter().any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
            })
        {
            return Err("invalid outline parameters/budget");
        }
        let mut result = Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        };
        result.reserve(edges.len() * layers.len())?;
        for &(width, color) in layers {
            for edge in edges {
                result.push(edge, camera, matrix, size, color, width)?;
            }
        }
        Ok(result)
    }
    /// Per-edge colour and width, in submission order: the debug gizmos'
    /// lines (`GizmoFeatureRenderer.buildLines`), not the outline's two
    /// uniform layers.
    pub fn lines(
        segments: &[([f64; 6], u32, f32)],
        camera: [f64; 3],
        matrix: [f32; 16],
        size: [u32; 2],
    ) -> Result<Self, &'static str> {
        if segments.len() > MAX_LINES
            || size.contains(&0)
            || camera.iter().any(|v| !v.is_finite())
            || matrix.iter().any(|v| !v.is_finite())
        {
            return Err("invalid gizmo line parameters/budget");
        }
        let mut result = Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        };
        result.reserve(segments.len())?;
        for &(edge, argb, width) in segments {
            // `ARGB` in a little-endian u32 reads as B, G, R, A.
            let [b, g, r, a] = argb.to_le_bytes().map(|c| f32::from(c) / 255.);
            if !width.is_finite() || !(0. ..=64.).contains(&width) || width == 0. {
                return Err("invalid gizmo line width");
            }
            result.push(&edge, camera, matrix, size, [r, g, b, a], width)?;
        }
        Ok(result)
    }
    fn reserve(&mut self, edges: usize) -> Result<(), &'static str> {
        self.vertices
            .try_reserve_exact(edges * 4)
            .map_err(|_| "outline allocation failed")?;
        self.indices
            .try_reserve_exact(edges * 6)
            .map_err(|_| "outline allocation failed")
    }
    /// Append one edge's quad. Edges the viewport cannot see, or that project
    /// to nothing on screen, contribute no vertices.
    fn push(
        &mut self,
        edge: &[f64; 6],
        camera: [f64; 3],
        matrix: [f32; 16],
        size: [u32; 2],
        color: [f32; 4],
        width: f32,
    ) -> Result<(), &'static str> {
        let points: [[f32; 3]; 2] = std::array::from_fn(|end| {
            std::array::from_fn(|axis| (edge[end * 3 + axis] - camera[axis]) as f32)
        });
        if points.iter().flatten().any(|v| !v.is_finite()) {
            return Err("non-finite outline endpoint/relative coordinate");
        }
        let distances = points.map(|p| {
            [
                p[0].hypot(p[1]).hypot(p[2]),
                p[0].hypot(p[2]).max(p[1].abs()),
            ]
        });
        // rendertype_lines: camera-space layering, not a constant NDC bias.
        let clip = points.map(|p| {
            let p = p.map(|v| (v * (1. - 1. / 4096.)) * (1. - 1. / 256.));
            std::array::from_fn(|row| {
                matrix[row] * p[0]
                    + matrix[4 + row] * p[1]
                    + matrix[8 + row] * p[2]
                    + matrix[12 + row]
            })
        });
        if clip
            .iter()
            .flatten()
            .chain(distances.iter().flatten())
            .any(|v| !v.is_finite())
        {
            return Err("outline projection/distance overflow");
        }
        let Some((clip, distances)) = clipped(clip, distances, size, width) else {
            return Ok(());
        };
        let [a, b] = clip;
        let dx = (b[0] / b[3] - a[0] / a[3]) * size[0] as f32;
        let dy = (b[1] / b[3] - a[1] / a[3]) * size[1] as f32;
        let length = dx.hypot(dy);
        // A zero-length screen projection has no stroke, not a NaN quad.
        if length <= 1e-12 {
            return Ok(());
        }
        let offset = [
            -dy / length * width / size[0] as f32,
            dx / length * width / size[1] as f32,
        ];
        let first = self.vertices.len() as u16;
        for end in 0..2 {
            for sign in [1., -1.] {
                let mut p = clip[end];
                for axis in 0..2 {
                    p[axis] += sign * offset[axis] * p[3];
                }
                let vertex = Vertex {
                    clip: p,
                    color,
                    distances: distances[end],
                };
                if !vertex.valid() {
                    return Err("invalid expanded outline vertex");
                }
                self.vertices.push(vertex);
            }
        }
        self.indices
            .extend([first, first + 1, first + 2, first + 2, first + 1, first + 3]);
        Ok(())
    }
}
type Clipped = ([[f32; 4]; 2], [[f32; 2]; 2]);
// Homogeneous segment clipping BEFORE division. Side planes include a stroke
// margin: a centreline just outside the viewport can still cover visible pixels.
fn clipped(
    mut p: [[f32; 4]; 2],
    mut d: [[f32; 2]; 2],
    size: [u32; 2],
    width: f32,
) -> Option<Clipped> {
    // Clip sequentially and snap the intersected component to its plane.
    // An interval-only t loses the whole visible segment for very long edges
    // (both ends round to t=0.5), even when all input floats are finite.
    let mut points = p.map(|p| p.map(f64::from));
    let mut distances = d.map(|d| d.map(f64::from));
    for axis in 0..3 {
        let margin = if axis < 2 {
            1. + f64::from(width) / f64::from(size[axis])
        } else {
            1.
        };
        for sign in [-1., 1.] {
            let [a, b] = points.map(|p| p[3] * margin + sign * p[axis]);
            if a < 0. && b < 0. {
                return None;
            }
            if (a < 0.) != (b < 0.) {
                let end = usize::from(b < 0.);
                let t = a / (a - b);
                let mut intersection: [f64; 4] =
                    std::array::from_fn(|i| points[0][i] * (1. - t) + points[1][i] * t);
                intersection[axis] = -sign * margin * intersection[3];
                distances[end] =
                    std::array::from_fn(|i| distances[0][i] * (1. - t) + distances[1][i] * t);
                points[end] = intersection;
            }
        }
    }
    p = points.map(|p| p.map(|v| v as f32));
    d = distances.map(|d| d.map(|v| v as f32));
    if p.iter().any(|p| p[3] <= 0.) {
        None
    } else {
        Some((p, d))
    }
}

#[cfg(test)]
mod tests;
