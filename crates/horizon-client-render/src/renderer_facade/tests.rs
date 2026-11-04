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
