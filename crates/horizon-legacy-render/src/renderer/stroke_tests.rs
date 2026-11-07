use super::*;
use horizon_render_api::{Fog, GameFog, IDENTITY, stroke};
fn draw(
    r: &mut Renderer<'_>,
    mesh: MeshId,
    white: TextureId,
    pass: Pass,
    clear: Clear,
    fog: Option<Fog>,
) -> Result<(), String> {
    r.render(&Frame {
        view_projection: IDENTITY,
        camera: [0.; 3],
        clear,
        fog,
        draws: &[Draw {
            mesh,
            texture: white,
            lightmap: white,
            origin: [0.; 3],
            pass,
            scissor: None,
        }],
    })
}
fn pixel(r: &mut Renderer<'_>, x: u32, y: u32) -> [u8; 3] {
    r.read_rgba([x, y, 1, 1]).unwrap()[..3].try_into().unwrap()
}
fn near(actual: [u8; 3], expected: [u8; 3]) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(&a, b)| a.abs_diff(b) <= 2),
        "{actual:?} != {expected:?}"
    );
}
#[test]
#[ignore = "requires Mesa OSMesa and explicit GL2 overrides"]
fn strokes_preserve_depth_fog_width_typed_layout_and_reclaim_every_mesh() {
    let mut r =
        Renderer::owned(Context::new("osmesa", [64; 2]).unwrap(), Budget::default()).unwrap();
    let white = r
        .create_texture(Image {
            width: 1,
            height: 1,
            rgba: &[255; 4],
        })
        .unwrap();
    let quad = |color, depth| {
        [[-1., -1.], [1., -1.], [-1., 1.], [1., 1.]].map(|[x, y]| Vertex {
            pos: [x, y, depth],
            uv: [0.; 2],
            light_uv: [0.; 2],
            color,
        })
    };
    let blue = r
        .create_mesh(&quad([0., 0., 1., 1.], 0.), &[0, 1, 2, 2, 1, 3])
        .unwrap();
    let green = r
        .create_mesh(&quad([0., 1., 0., 1.], -0.1), &[0, 1, 2, 2, 1, 3])
        .unwrap();
    let baseline = r.stats();
    let edge = [-0.8, 0., 0., 0.8, 0., 0.];
    let mut mesh = stroke::Mesh::new(
        &[edge],
        [0.; 3],
        IDENTITY,
        [64; 2],
        &[(4., [1., 0., 0., 0.5])],
    )
    .unwrap();
    for repeat in 0..64 {
        let id = r.create_stroke_mesh(&mesh.vertices, &mesh.indices).unwrap();
        assert_eq!(r.stats().mesh_bytes, baseline.mesh_bytes + 172);
        draw(
            &mut r,
            blue,
            white,
            Pass::Opaque,
            Clear::ColorDepth([0.; 4]),
            None,
        )
        .unwrap();
        // Equal depth is visible (LEQUAL), width is four framebuffer pixels.
        draw(&mut r, id, white, Pass::Stroke, Clear::Load, None).unwrap();
        for y in [30, 31, 32, 33] {
            near(pixel(&mut r, 32, y), [128, 0, 127]);
        }
        for y in [29, 34] {
            near(pixel(&mut r, 32, y), [0, 0, 255]);
        }
        // A stroke MUST NOT write depth: a subsequent farther-than-stroke but
        // nearer-than-blue ordinary quad still draws.
        for v in &mut mesh.vertices {
            v.clip[2] = -0.5 * v.clip[3];
        }
        r.remove_mesh(id).unwrap();
        let id = r.create_stroke_mesh(&mesh.vertices, &mesh.indices).unwrap();
        draw(&mut r, id, white, Pass::Stroke, Clear::Load, None).unwrap();
        draw(&mut r, green, white, Pass::Opaque, Clear::Load, None).unwrap();
        near(pixel(&mut r, 32, 32), [0, 255, 0]);
        // Ordinary material immediately after strokes must leave stroke mode.
        for v in &mut mesh.vertices {
            v.clip[2] = 0.;
        }
        draw(
            &mut r,
            id,
            white,
            Pass::Opaque,
            Clear::ColorDepth([1.; 4]),
            None,
        )
        .unwrap_err();
        near(pixel(&mut r, 32, 32), [0, 255, 0]); // type error precedes clear
        draw(
            &mut r,
            blue,
            white,
            Pass::Stroke,
            Clear::ColorDepth([1.; 4]),
            None,
        )
        .unwrap_err();
        r.remove_mesh(id).unwrap();
        assert_eq!(r.stats(), baseline, "iteration {repeat}");
    }
    // Occluded lines stay hidden and do not acquire the entity polygon bias.
    for v in &mut mesh.vertices {
        v.clip[2] = 0.5 * v.clip[3];
    }
    let id = r.create_stroke_mesh(&mesh.vertices, &mesh.indices).unwrap();
    draw(
        &mut r,
        blue,
        white,
        Pass::Opaque,
        Clear::ColorDepth([0.; 4]),
        None,
    )
    .unwrap();
    draw(&mut r, id, white, Pass::Stroke, Clear::Load, None).unwrap();
    near(pixel(&mut r, 32, 32), [0, 0, 255]);
    r.remove_mesh(id).unwrap();
    // Perspective interpolation uses the retained clip w, not affine NDC color.
    // Screen centre is halfway: distances interpolate to 1.6, NOT 2.5.
    for (i, v) in mesh.vertices.iter_mut().enumerate() {
        let w = if i < 2 { 1. } else { 4. };
        v.clip = [
            if i < 2 { -0.8 * w } else { 0.8 * w },
            if i % 2 == 0 {
                4. / 64. * w
            } else {
                -4. / 64. * w
            },
            0.,
            w,
        ];
        v.color = [1., 0., 0., 1.];
        v.distances = [if i < 2 { 1. } else { 4. }; 2];
    }
    let id = r.create_stroke_mesh(&mesh.vertices, &mesh.indices).unwrap();
    for (fog, expected) in [
        (
            Fog {
                start: 0.,
                end: 4.,
                color: [0., 1., 0.],
                game: None,
            },
            [152, 103, 0],
        ),
        (
            Fog {
                start: 0.,
                end: 4.,
                color: [0., 1., 0.],
                game: Some(GameFog {
                    start: 0.,
                    end: 4.,
                    alpha: 0.5,
                }),
            },
            [204, 51, 0],
        ),
    ] {
        draw(
            &mut r,
            id,
            white,
            Pass::Stroke,
            Clear::ColorDepth([0.; 4]),
            Some(fog),
        )
        .unwrap();
        near(pixel(&mut r, 32, 32), expected);
    }
    r.remove_mesh(id).unwrap();
    let before = r.stats();
    for bad in 0..4 {
        let mut vertices = mesh.vertices.clone();
        match bad {
            0 => vertices[0].clip[3] = 0.,
            1 => vertices[0].color[0] = f32::NAN,
            2 => vertices[0].distances[0] = -1.,
            _ => vertices[0].clip[0] = f32::INFINITY,
        }
        assert!(r.create_stroke_mesh(&vertices, &mesh.indices).is_err());
        assert_eq!(r.stats(), before);
    }
}
