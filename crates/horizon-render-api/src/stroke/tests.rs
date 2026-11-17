use super::*;
use crate::IDENTITY;
const WHITE: [(f32, [f32; 4]); 1] = [(2.5, [1.; 4])];
const PERSPECTIVE: [f32; 16] = [
    1.,
    0.,
    0.,
    0.,
    0.,
    1.,
    0.,
    0.,
    0.,
    0.,
    -11. / 9.,
    -1.,
    0.,
    0.,
    -20. / 9.,
    0.,
];
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.0001, "{a} != {b}");
}
#[test]
fn constant_pixel_width_survives_aspect_direction_and_endpoint_order() {
    for size in [[64, 32], [1920, 1080], [320, 640]] {
        for edge in [
            [-0.5, 0., 0., 0.5, 0., 0.],
            [0., -0.5, 0., 0., 0.5, 0.],
            [-0.5, -0.5, 0., 0.5, 0.5, 0.],
            [0.5, 0.5, 0., -0.5, -0.5, 0.],
        ] {
            let mesh = Mesh::new(&[edge], [0.; 3], IDENTITY, size, &WHITE).unwrap();
            assert_eq!(mesh.vertices.len(), 4);
            assert_eq!(mesh.indices, [0, 1, 2, 2, 1, 3]);
            let [a, b] = [mesh.vertices[0].clip, mesh.vertices[1].clip];
            close(
                ((a[0] / a[3] - b[0] / b[3]) * size[0] as f32 / 2.)
                    .hypot((a[1] / a[3] - b[1] / b[3]) * size[1] as f32 / 2.),
                2.5,
            );
            assert!(mesh.vertices.iter().all(Vertex::valid));
            assert_eq!(mesh.bytes(), 172);
        }
    }
}
#[test]
fn camera_subtraction_precedes_f32_and_layers_preserve_whole_shape_order() {
    let edges = [
        [-0.25, 0., 0., 0.25, 0., 0.],
        [-0.25, 0.5, 0., 0.25, 0.5, 0.],
    ];
    let layers = [(7., [0., 0., 0., 1.]), WHITE[0]];
    let expected = Mesh::new(&edges, [0.; 3], IDENTITY, [64; 2], &layers).unwrap();
    let camera = [1e12, -1e12, 1e12];
    let absolute = edges.map(|e| std::array::from_fn(|i| e[i] + camera[i % 3]));
    let actual = Mesh::new(&absolute, camera, IDENTITY, [64; 2], &layers).unwrap();
    assert_eq!(actual.vertices, expected.vertices);
    assert_eq!(actual.indices, expected.indices);
    assert!(actual.vertices[..8].iter().all(|v| v.color == layers[0].1));
    assert!(actual.vertices[8..].iter().all(|v| v.color == layers[1].1));
}
#[test]
fn near_far_eye_and_side_clipping_are_finite_and_keep_homogeneous_w_and_fog() {
    for edge in [
        [-0.4, 0., -0.5, 0.4, 0., -5.],  // crosses near
        [-0.4, 0., 1., 0.4, 0., -5.],    // crosses eye and near
        [-0.4, 0., -2., 0.4, 0., -20.],  // crosses far
        [-1e30, 0., -2., 1e30, 0., -2.], // crosses both side planes
    ] {
        let mesh = Mesh::new(&[edge], [0.; 3], PERSPECTIVE, [64; 2], &WHITE).unwrap();
        assert_eq!(mesh.vertices.len(), 4, "{edge:?}");
        for v in &mesh.vertices {
            assert!(v.valid());
            assert!(v.clip[2].abs() <= v.clip[3] + 0.00001);
            assert!(v.clip[3] >= 0.99999);
        }
    }
    let edge = [-0.4, 0., -0.5, 0.4, 0., -5.];
    let mesh = Mesh::new(&[edge], [0.; 3], PERSPECTIVE, [64; 2], &WHITE).unwrap();
    let [a, b] = [mesh.vertices[0], mesh.vertices[2]];
    close(a.clip[2], -a.clip[3]);
    assert!(b.clip[3] > a.clip[3] * 4.);
    close(b.distances[0], 0.4f32.hypot(5.));
    close(b.distances[1], 0.4f32.hypot(5.)); // original distance, not layered/projected distance
    for edge in [
        [0., 0., 1., 1., 0., 2.],
        [0., 0., -20., 1., 0., -30.],
        [5., 0., -2., 6., 0., -2.],
    ] {
        assert!(
            Mesh::new(&[edge], [0.; 3], PERSPECTIVE, [64; 2], &WHITE)
                .unwrap()
                .indices
                .is_empty()
        );
    }
}
#[test]
fn viewport_margin_keeps_partially_visible_strokes_and_degenerate_edges_are_empty() {
    let scale = (1. - 1. / 4096.) * (1. - 1. / 256.);
    let mesh = Mesh::new(
        &[[1.01 / scale, -0.5, 0., 1.01 / scale, 0.5, 0.]],
        [0.; 3],
        IDENTITY,
        [64; 2],
        &WHITE,
    )
    .unwrap();
    assert_eq!(mesh.vertices.len(), 4);
    assert!(mesh.vertices.iter().any(|v| v.clip[0] < v.clip[3]));
    assert!(
        Mesh::new(&[[0.; 6]], [0.; 3], IDENTITY, [64; 2], &WHITE)
            .unwrap()
            .indices
            .is_empty()
    );
}
#[test]
fn bounded_two_layer_shape_and_all_invalid_inputs_are_explicit() {
    let edge = [-0.5, 0., 0., 0.5, 0., 0.];
    let mesh = Mesh::new(
        &[edge; MAX_EDGES],
        [0.; 3],
        IDENTITY,
        [64; 2],
        &[WHITE[0]; 2],
    )
    .unwrap();
    assert_eq!(mesh.vertices.len(), MAX_EDGES * 8);
    assert_eq!(mesh.bytes(), 88064);
    assert!(
        mesh.indices
            .iter()
            .all(|&v| usize::from(v) < mesh.vertices.len())
    );
    for (edges, camera, matrix, size, layers) in [
        (
            vec![edge; MAX_EDGES + 1],
            [0.; 3],
            IDENTITY,
            [64; 2],
            WHITE.to_vec(),
        ),
        (
            vec![[f64::NAN; 6]],
            [0.; 3],
            IDENTITY,
            [64; 2],
            WHITE.to_vec(),
        ),
        (
            vec![[f64::MAX; 6]],
            [0.; 3],
            IDENTITY,
            [64; 2],
            WHITE.to_vec(),
        ),
        (
            vec![edge],
            [f64::INFINITY; 3],
            IDENTITY,
            [64; 2],
            WHITE.to_vec(),
        ),
        (vec![edge], [0.; 3], [f32::MAX; 16], [64; 2], WHITE.to_vec()),
        (vec![edge], [0.; 3], [f32::NAN; 16], [64; 2], WHITE.to_vec()),
        (vec![edge], [0.; 3], IDENTITY, [0, 64], WHITE.to_vec()),
        (vec![edge], [0.; 3], IDENTITY, [64; 2], vec![]),
        (vec![edge], [0.; 3], IDENTITY, [64; 2], vec![WHITE[0]; 3]),
        (vec![edge], [0.; 3], IDENTITY, [64; 2], vec![(0., [1.; 4])]),
        (vec![edge], [0.; 3], IDENTITY, [64; 2], vec![(65., [1.; 4])]),
        (
            vec![edge],
            [0.; 3],
            IDENTITY,
            [64; 2],
            vec![(f32::NAN, [1.; 4])],
        ),
        (vec![edge], [0.; 3], IDENTITY, [64; 2], vec![(2., [-1.; 4])]),
    ] {
        assert!(Mesh::new(&edges, camera, matrix, size, &layers).is_err());
    }
}
