use super::*;
use crate::{
    hud::{HudAtlas, HudBatch},
    legacy_gfx::{Limits, TerrainHudRenderer, frame},
};
use horizon_legacy_render::Context;
use horizon_render_api::{
    atlas::{SpriteSize, build_page, pack_atlas},
    paged_gui::Layout,
};
fn quad(layer: u32, light: u32) -> [Source; 4] {
    [[-2., -2., 4.], [-2., 2., 4.], [2., 2., 4.], [2., -2., 4.]].map(|pos| Source {
        pos,
        uv: [-1.25, 2.75],
        layer,
        color: u32::MAX,
        light,
        normal: [0; 4],
        glint_uv: [f32::NAN; 2],
    })
}
#[test]
fn actual_vertices_split_stably_preserve_wrapping_stages_and_bound_extraction() {
    let pixels: Vec<_> = (0..32).flat_map(|i| [i, 0, 0, 255]).collect();
    let backing = Backing::new(Layout::new([8, 4], 4, 12).unwrap(), &pixels, 4096).unwrap();
    let mut vertices = Vec::new();
    for (layer, stage) in [
        (0, 0),
        (11, 0),
        (15, 0),
        (12, crate::breaking::stage_light([0.5, 0.5])),
        (11, 0),
    ] {
        vertices.extend(quad(layer, stage));
    }
    let extent = Some([f32::NAN, f32::NAN, 0.25, 0.5]); // xy is not the stage origin
    let p = Prepared::new(&vertices, extent, &backing, false, 3, 96, 64).unwrap();
    assert_eq!(
        p.ordinary.iter().map(|v| v.layer).collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 15, 15, 15, 15]
    );
    assert_eq!(p.runs.len(), 3);
    assert_eq!(p.sprites.len(), 2);
    assert_eq!(p.largest_texture(), 16);
    assert!(!p.runs[0].translucent && p.runs[1].translucent && !p.runs[2].translucent);
    assert_eq!(p.runs[0].vertices[0].base.uv, [-1.25, 2.75]);
    assert_eq!(p.runs[0].vertices[0].light_color, [1.; 4]);
    assert_eq!(p.sprites[1].pixels[..4], [20, 0, 0, 255]);
    assert!(Prepared::new(&vertices, extent, &backing, false, 3, 95, 64).is_err());
    assert!(Prepared::new(&vertices, extent, &backing, false, 2, 96, 64).is_err());
    assert!(Prepared::new(&vertices, None, &backing, false, 3, 96, 64).is_err());
    let ordinary = quad(0, 0);
    assert!(matches!(
        Prepared::new(&ordinary, None, &backing, false, 0, 0, 0)
            .unwrap()
            .ordinary,
        Cow::Borrowed(_)
    ));
}
#[test]
fn malformed_mixed_unbounded_and_out_of_atlas_requests_fail_explicitly() {
    let backing = Backing::new(Layout::new([8, 4], 4, 12).unwrap(), &[0; 128], 4096).unwrap();
    for bad in 0..8 {
        let mut q = quad(11, 0);
        let mut extent = [0., 0., 0.25, 0.5];
        match bad {
            0 => q[1].layer = 0,
            1 => q[2].light = 1,
            2 => q[1].pos[0] = f32::NAN,
            3 => q[1].uv[1] = f32::INFINITY,
            4 => {
                for v in &mut q {
                    v.layer |= 1 << 31;
                }
            }
            5 => {
                for v in &mut q {
                    v.light = u32::MAX;
                }
            }
            6 => extent[2] = f32::NAN,
            _ => extent[3] = 2.,
        }
        assert!(
            Prepared::new(&q, Some(extent), &backing, false, 1, 80, 64).is_err(),
            "{bad}"
        );
    }
    assert!(Prepared::new(&quad(11, 0)[..3], None, &backing, false, 1, 80, 64).is_err());
    assert!(
        Prepared::new(
            &quad(11, 0),
            Some([0., 0., 0.25, 0.5]),
            &backing,
            false,
            1,
            80,
            1
        )
        .is_err()
    );
}
#[test]
#[ignore = "requires OSMesa; actual Frame decals with test assets, not a Game launch"]
fn actual_frame_cracks_multiply_cutout_order_and_reclaim_without_clearing_on_failure() {
    let mut atlas = HudAtlas::boot();
    atlas.width = 4;
    atlas.height = 2;
    atlas.pixels = (0..8)
        .flat_map(|i| {
            if i % 4 < 2 {
                [64, 128, 192, 255]
            } else {
                [255, 0, 0, 0]
            }
        })
        .collect();
    let hud = HudBatch::new([64; 2]);
    let layout = pack_atlas(
        &[SpriteSize {
            width: 1,
            height: 1,
        }],
        4,
        1,
        1,
    )
    .unwrap();
    let page = build_page(&layout, 0, &[&[255, 255, 255, 128]]).unwrap();
    let mut r = TerrainHudRenderer::new(
        Context::new("osmesa", [64; 2]).unwrap(),
        layout,
        &[&page],
        &[0; 1024],
        &atlas,
        Limits::default(),
    )
    .unwrap();
    let mut source = frame::tests::source(&hud);
    source.camera = glam::DVec3::ZERO;
    source.pitch = 0.;
    source.crumbling_sprite = [0., 0., 0.5, 1.];
    source.fog = Some(crate::fog::FogData {
        color: [128. / 255., 102. / 255., 76. / 255., 1.],
        environmental_start: 100.,
        environmental_end: 200.,
        render_distance_start: 100.,
        render_distance_end: 200.,
        sky_end: 200.,
        cloud_end: 200.,
    });
    let baseline = r.world.backend_mut().stats();
    let cycles: Vec<_> = (0..32)
        .map(|cycle| {
            quad(
                if cycle % 2 == 0 { 11 } else { 12 },
                if cycle % 3 == 0 {
                    crate::breaking::stage_light([0.5, 0.])
                } else {
                    0
                },
            )
        })
        .collect();
    for (cycle, q) in cycles.iter().enumerate() {
        source.entities = q;
        r.render_game_frame(
            &source,
            frame::VisibleSections::default(),
            frame::Options::default(),
        )
        .unwrap();
        let p = r.read_rgba([32, 32, 1, 1]).unwrap();
        let expected = if cycle % 3 == 0 {
            [128, 102, 76]
        } else {
            [64, 102, 114]
        };
        assert!(
            p[..3]
                .iter()
                .zip(expected)
                .all(|(&a, b)| a.abs_diff(b) <= 2),
            "{cycle}: {p:?}"
        );
        assert_eq!(r.world.backend_mut().stats(), baseline);
        source.entities = &[];
    }
    let mut water = crate::mesher::SectionMesh::default();
    water.translucent.extend(
        [[-2., -2.], [2., -2.], [2., 2.], [-2., 2.]].map(|[x, y]| {
            crate::mesher::TerrainVertex::new([x, y, 3.], [0.5; 2], 0, 0xff0000ff, 0)
        }),
    );
    let keys = [[0; 3]];
    r.upload_section(keys[0], &water).unwrap();
    let visible = frame::VisibleSections {
        solid: &[],
        translucent: &keys,
    };
    let options = frame::Options {
        fullbright: true,
        ..frame::Options::default()
    };
    let before_quad = quad(11, 0);
    source.entities = &before_quad;
    r.render_game_frame(&source, visible, options).unwrap();
    let before = r.read_rgba([32, 32, 1, 1]).unwrap();
    let after_quad = quad(12, 0);
    source.entities = &after_quad;
    r.render_game_frame(&source, visible, options).unwrap();
    assert_ne!(r.read_rgba([32, 32, 1, 1]).unwrap(), before);
    r.unload_section(keys[0]).unwrap();
    r.world.collect_garbage().unwrap();
    let mut solid = crate::mesher::SectionMesh::default();
    solid
        .solid
        .extend([[-2., -2.], [-2., 2.], [2., 2.], [2., -2.]].map(|[x, y]| {
            crate::mesher::TerrainVertex::new([x, y, 4.], [0.5; 2], 0x8000, 0xff808080, 0)
        }));
    r.upload_section(keys[0], &solid).unwrap();
    let visible = frame::VisibleSections {
        solid: &keys,
        translucent: &[],
    };
    source.entities = &before_quad;
    r.render_game_frame(&source, visible, options).unwrap();
    let coplanar = r.read_rgba([32, 32, 1, 1]).unwrap();
    assert!(
        coplanar[..3]
            .iter()
            .zip([64, 128, 193])
            .all(|(&a, b)| a.abs_diff(b) <= 2),
        "coplanar: {coplanar:?}"
    );
    r.unload_section(keys[0]).unwrap();
    r.world.collect_garbage().unwrap();
    let before = r.read_rgba([0, 0, 64, 64]).unwrap();
    r.world
        .backend_mut()
        .set_budget(horizon_legacy_render::Budget {
            texture_bytes: baseline.texture_bytes + 16,
            mesh_bytes: 347,
        })
        .unwrap();
    assert!(
        r.render_game_frame(&source, frame::VisibleSections::default(), options)
            .is_err()
    );
    assert_eq!(r.read_rgba([0, 0, 64, 64]).unwrap(), before);
    assert_eq!(r.world.backend_mut().stats(), baseline);
}
