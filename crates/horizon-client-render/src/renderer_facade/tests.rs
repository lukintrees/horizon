use super::*;
use crate::{
    hud::{HudAtlas, HudBatch},
    legacy_gfx::{Limits, TerrainHudRenderer, frame},
};
use horizon_legacy_render::Context;
use horizon_render_api::atlas::{SpriteSize, build_page, pack_atlas};

#[test]
#[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
fn real_facade_preserves_offscreen_outcome_pixels_and_explicit_rejection() {
    let atlas = HudAtlas::boot();
    let mut hud = HudBatch::new([64; 2]);
    hud.fill(&atlas, 0., 0., 64., 64., u32::MAX);
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
    let page = build_page(&layout, 0, &[&[255; 4]]).unwrap();
    let mut renderer = TerrainHudRenderer::new(
        Context::new("osmesa", [64; 2]).unwrap(),
        layout,
        &[&page],
        &[255; 1024],
        &atlas,
        Limits::default(),
    )
    .unwrap();
    let mut source = frame::tests::source(&hud);
    let options = frame::Options {
        gamma: 0.,
        fullbright: true,
    };
    {
        let mut facade =
            RendererFacade::legacy(&mut renderer, frame::VisibleSections::default(), options);
        assert_eq!(facade.backend(), Backend::LegacyGl);
        assert_eq!(
            facade.capabilities().require(&[Feature::Panorama]),
            Err(Feature::Panorama)
        );
        assert_eq!(facade.capabilities().require(&[]), Ok(()));
        assert_eq!(facade.render(&source).unwrap(), FrameOutcome::Offscreen);
        assert!(facade.resize([0, 64]).is_err());
    }
    let before = renderer.read_rgba([0, 0, 64, 64]).unwrap();
    assert!(before.as_chunks::<4>().0.contains(&[255; 4]));
    source.title = true;
    assert!(
        RendererFacade::legacy(&mut renderer, frame::VisibleSections::default(), options)
            .render(&source)
            .is_err()
    );
    assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), before);
    RendererFacade::legacy(&mut renderer, frame::VisibleSections::default(), options)
        .resize([32; 2])
        .unwrap();
    assert_eq!(renderer.size(), [32; 2]);
}

#[test]
#[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
fn game_section_lifecycle_uses_dimension_minimum_and_reclaims_old_columns() {
    use crate::{
        chunk::ColumnLayout,
        mesher::{SectionMesh, TerrainVertex},
    };
    use horizon_render_api::{IDENTITY, scene::SceneFrame};
    let atlas = HudAtlas::boot();
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
    let page = build_page(&layout, 0, &[&[64, 128, 192, 255]]).unwrap();
    let limits = Limits {
        sections: 3,
        ..Limits::default()
    };
    let mut renderer = TerrainHudRenderer::new(
        Context::new("osmesa", [64; 2]).unwrap(),
        layout,
        &[&page],
        &[255; 1024],
        &atlas,
        limits,
    )
    .unwrap();
    let mut mesh = SectionMesh::default();
    mesh.solid.extend(
        [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
            .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, u32::MAX, 0xf0f0)),
    );
    let options = frame::Options::default();
    let visible = frame::VisibleSections::default();
    let dimension = ColumnLayout::new(-64, 384).unwrap();
    {
        let mut facade = RendererFacade::legacy(&mut renderer, visible, options);
        // Never silently interpret a Game-relative Y as an absolute section.
        assert!(facade.upload_section([0; 3], &mesh).is_err());
        assert_eq!(facade.resident_sections(), 0);
        facade.clear_sections(dimension).unwrap();
        assert!(facade.upload_section([0, -1, 0], &mesh).is_err());
        assert!(facade.upload_section([0, 24, 0], &mesh).is_err());
        facade.upload_section([0; 3], &mesh).unwrap();
        facade.upload_section([0, 1, 0], &mesh).unwrap();
        facade.upload_section([-2, 0, 3], &mesh).unwrap();
        assert!(facade.has_section([0; 3]));
        assert_eq!(facade.resident_sections(), 3);
    }
    // Independent projection: relative section 0 lies at block Y=-64, not 0.
    // A zero-origin bug leaves this center pixel black instead of terrain blue.
    let scene = SceneFrame {
        view_projection: IDENTITY,
        camera: [0., -64., 0.],
        clear: [0., 0., 0., 1.],
        fog: None,
        solid: &[[0; 3]],
        translucent: &[],
    };
    renderer.render(&scene, &hud).unwrap();
    assert_eq!(
        renderer.read_rgba([32, 32, 1, 1]).unwrap(),
        [64, 128, 192, 255]
    );
    {
        let mut facade = RendererFacade::legacy(&mut renderer, visible, options);
        facade.remove_columns([(0, 0), (0, 0), (99, 99)]).unwrap();
        assert!(!facade.has_section([0; 3]));
        assert!(!facade.has_section([0, 1, 0]));
        assert!(facade.has_section([-2, 0, 3]));
        assert_eq!(facade.resident_sections(), 1);
    }
    let stable = renderer.read_rgba([0, 0, 64, 64]).unwrap();
    // A removed visible key is rejected before the previous image is cleared.
    assert!(renderer.render(&scene, &hud).is_err());
    assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), stable);
    for minimum in [-64, -2032, 0, 128, -64] {
        let mut facade = RendererFacade::legacy(&mut renderer, visible, options);
        facade
            .clear_sections(ColumnLayout::new(minimum, 16).unwrap())
            .unwrap();
        assert_eq!(facade.resident_sections(), 0);
        // Repeated replacement/reset reuses the section budget and collects old meshes.
        facade.upload_section([0; 3], &mesh).unwrap();
        facade.upload_section([0; 3], &mesh).unwrap();
        assert_eq!(facade.resident_sections(), 1);
    }
    assert_eq!(renderer.size(), [64; 2]);
}
