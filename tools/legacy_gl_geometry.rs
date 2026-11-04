// Each vertex has position (3), sprite UV (2), lightmap UV (2), color (4).
fn quad(output: &mut Vec<f32>, points: [[f32; 3]; 4], uv: [f32; 2], color: [f32; 4]) {
    for index in [0, 1, 2, 0, 2, 3] {
        output.extend_from_slice(&points[index]);
        output.extend_from_slice(&uv);
        output.extend_from_slice(&[0.5, 0.5]);
        output.extend_from_slice(&color);
    }
}
pub(crate) fn demo_mesh() -> Vec<f32> {
    let mut vertices = Vec::new();
    // A tiled ground plane and a little column: deliberately generated in Rust,
    // not Minecraft assets. Pixel-center UVs select each texel in the 2x1 atlas.
    for x in -4..4 {
        for z in -4..4 {
            let (x, z) = (x as f32, z as f32);
            quad(
                &mut vertices,
                [
                    [x, 0.0, z],
                    [x + 1.0, 0.0, z],
                    [x + 1.0, 0.0, z + 1.0],
                    [x, 0.0, z + 1.0],
                ],
                [0.25, 0.5],
                [1.0; 4],
            );
        }
    }
    for y in 0..3 {
        let y = y as f32;
        quad(
            &mut vertices,
            [
                [0.0, y, 0.0],
                [1.0, y, 0.0],
                [1.0, y + 1.0, 0.0],
                [0.0, y + 1.0, 0.0],
            ],
            [0.75, 0.5],
            [1.0; 4],
        );
        quad(
            &mut vertices,
            [
                [1.0, y, 0.0],
                [1.0, y, 1.0],
                [1.0, y + 1.0, 1.0],
                [1.0, y + 1.0, 0.0],
            ],
            [0.75, 0.5],
            [0.7, 0.7, 0.7, 1.0],
        );
    }
    quad(
        &mut vertices,
        [
            [0.0, 3.0, 0.0],
            [1.0, 3.0, 0.0],
            [1.0, 3.0, 1.0],
            [0.0, 3.0, 1.0],
        ],
        [0.25, 0.5],
        [1.0; 4],
    );
    vertices
}

/// Only desktop GL version strings are accepted, not OpenGL ES.
pub(crate) fn desktop_gl_version(version: &str) -> Option<(u32, u32)> {
    let token = version.split_whitespace().next()?;
    let mut numbers = token.split('.');
    Some((numbers.next()?.parse().ok()?, numbers.next()?.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_demo_contains_full_triangles_and_valid_attributes() {
        let mesh = demo_mesh();
        assert_eq!(mesh.len(), 71 * 6 * 11);
        for vertex in mesh.chunks_exact(11) {
            assert!(vertex.iter().all(|v| v.is_finite()));
            assert!((0.0..=1.0).contains(&vertex[3]));
            assert!((0.0..=1.0).contains(&vertex[4]));
            assert_eq!(&vertex[5..7], &[0.5, 0.5]);
            assert!(vertex[7..].iter().all(|v| (0.0..=1.0).contains(v)));
        }
    }

    #[test]
    fn quad_triangulation_preserves_winding_and_vertex_attributes() {
        let mut mesh = Vec::new();
        let corners = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        quad(&mut mesh, corners, [0.25, 0.5], [1.0, 0.5, 0.25, 1.0]);
        let positions: Vec<_> = mesh.chunks_exact(11).map(|v| [v[0], v[1], v[2]]).collect();
        assert_eq!(
            positions,
            [
                corners[0], corners[1], corners[2], corners[0], corners[2], corners[3]
            ]
        );
        assert!(
            mesh.chunks_exact(11)
                .all(|v| v[3..5] == [0.25, 0.5] && v[7..11] == [1.0, 0.5, 0.25, 1.0])
        );
    }
    #[test]
    fn parses_desktop_versions_without_mistaking_gles_for_gl() {
        assert_eq!(desktop_gl_version("2.1.8543 Release"), Some((2, 1)));
        assert_eq!(desktop_gl_version("4.6 Mesa"), Some((4, 6)));
        assert_eq!(desktop_gl_version("OpenGL ES 3.0"), None);
        assert_eq!(desktop_gl_version("unavailable"), None);
    }
}
