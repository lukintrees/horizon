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

#[test]
#[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
fn static_menu_is_explicit_preserves_world_and_rejects_panorama_without_clearing() {
    use crate::{
        chunk::ColumnLayout,
        mesher::{SectionMesh, TerrainVertex},
    };
    use horizon_render_api::{IDENTITY, scene::SceneFrame};
    let atlas = HudAtlas::boot();
    let mut hud = HudBatch::new([64; 2]);
    hud.fill(&atlas, 0., 0., 16., 16., u32::MAX);
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
    let page = build_page(&layout, 0, &[&[255, 0, 0, 255]]).unwrap();
    let mut renderer = TerrainHudRenderer::new(
        Context::new("osmesa", [64; 2]).unwrap(),
        layout,
        &[&page],
        &[255; 1024],
        &atlas,
        Limits::default(),
    )
    .unwrap();
    let options = frame::Options::default();
    let visible = frame::VisibleSections {
        solid: &[[0; 3]],
        translucent: &[],
    };
    let clear = [0.25, 0.5, 0.75, 1.];
    {
        let mut facade = RendererFacade::legacy(&mut renderer, visible, options);
        facade
            .clear_sections(ColumnLayout::new(0, 16).unwrap())
            .unwrap();
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
                .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, u32::MAX, 0xf0f0)),
        );
        facade.upload_section([0; 3], &mesh).unwrap();
    }
    renderer
        .render(
            &SceneFrame {
                view_projection: IDENTITY,
                camera: [0.; 3],
                clear: [0.; 4],
                fog: None,
                solid: &[[0; 3]],
                translucent: &[],
            },
            &HudBatch::new([64; 2]),
        )
        .unwrap();
    assert_eq!(
        renderer.read_rgba([32, 32, 1, 1]).unwrap(),
        [255, 0, 0, 255]
    );
    {
        let mut facade = RendererFacade::legacy(&mut renderer, visible, options);
        assert_eq!(
            facade.capabilities().require(&[Feature::StaticMenu]),
            Ok(())
        );
        assert_eq!(
            facade
                .render_menu(&MenuFrame::static_color(&hud, clear))
                .unwrap(),
            FrameOutcome::Offscreen
        );
        assert!(facade.has_section([0; 3]));
        assert_eq!(facade.resident_sections(), 1);
    }
    let before = renderer.read_rgba([0, 0, 64, 64]).unwrap();
    for (value, expected) in before[(32 * 64 + 32) * 4..][..4]
        .iter()
        .zip([64u8, 128, 191, 255])
    {
        assert!(value.abs_diff(expected) <= 1);
    }
    assert!(before.as_chunks::<4>().0.contains(&[255; 4]));
    {
        let mut facade = RendererFacade::legacy(&mut renderer, visible, options);
        assert!(
            facade
                .render_menu(&MenuFrame::panorama(&hud, 10., 10.))
                .is_err()
        );
        assert!(
            facade
                .render_menu(&MenuFrame::static_color(&hud, [f32::NAN; 4]))
                .is_err()
        );
    }
    assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), before);
    // Bad GUI input also cannot erase the previously displayed menu.
    hud.vertices.push(hud.vertices[0]);
    assert!(
        RendererFacade::legacy(&mut renderer, visible, options)
            .render_menu(&MenuFrame::static_color(&hud, clear))
            .is_err()
    );
    assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), before);
    let empty = HudBatch::new([32; 2]);
    {
        let mut facade = RendererFacade::legacy(&mut renderer, visible, options);
        facade.resize([32; 2]).unwrap();
        facade
            .render_menu(&MenuFrame::static_color(&empty, clear))
            .unwrap();
        assert_eq!(facade.resident_sections(), 1);
        let old_layout = HudBatch::new([64; 2]);
        assert!(
            facade
                .render_menu(&MenuFrame::static_color(&old_layout, clear))
                .is_err()
        );
    }
    assert!(
        renderer
            .read_rgba([16, 16, 1, 1])
            .unwrap()
            .iter()
            .zip([64u8, 128, 191, 255])
            .all(|(a, b)| a.abs_diff(b) <= 1)
    );
    // Returning to a world must still draw the original resident VBO/texture.
    renderer
        .render(
            &SceneFrame {
                view_projection: IDENTITY,
                camera: [0.; 3],
                clear: [0.; 4],
                fog: None,
                solid: &[[0; 3]],
                translucent: &[],
            },
            &empty,
        )
        .unwrap();
    assert_eq!(
        renderer.read_rgba([16, 16, 1, 1]).unwrap(),
        [255, 0, 0, 255]
    );
}

#[test]
#[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
fn native_screenshot_completes_once_before_present_and_survives_rejection_resize_and_handover() {
    use horizon_render_api::render_contract::ScreenshotRequest;
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "horizon-legacy-screenshot-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let path = |name: &str| scratch.0.join(name);
    let request = |name: &str, size, scaled| ScreenshotRequest {
        path: path(name),
        size,
        scaled,
    };
    let decode = |name: &str| {
        let mut reader = png::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(path(name)).unwrap(),
        ))
        .read_info()
        .unwrap();
        let mut data = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut data).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        data.truncate(info.buffer_size());
        ([info.width, info.height], data)
    };
    let atlas = HudAtlas::boot();
    let mut hud = HudBatch::new([64, 32]);
    // Top half of the centre square: survives the icon crop, tests GL row inversion.
    hud.fill(&atlas, 16., 0., 32., 16., u32::MAX);
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
        Context::new("osmesa", [64, 32]).unwrap(),
        layout,
        &[&page],
        &[255; 1024],
        &atlas,
        Limits {
            screenshot_bytes: 64 * 32 * 4 + 8 * 8 * 4,
            ..Limits::default()
        },
    )
    .unwrap();
    let visible = frame::VisibleSections::default();
    let options = frame::Options::default();
    let clear = [0.25, 0.5, 0.75, 0.];
    {
        let mut facade = RendererFacade::legacy(&mut renderer, visible, options);
        facade
            .request_screenshot(request("full.png", None, false))
            .unwrap();
        assert!(facade.screenshot_pending());
        assert!(!path("full.png").exists());
        assert!(
            facade
                .request_screenshot(request("duplicate.png", None, false))
                .is_err()
        );
        assert!(
            facade
                .render_menu(&MenuFrame::panorama(&hud, 0., 0.))
                .is_err()
        );
        assert!(facade.screenshot_pending());
        assert!(facade.take_screenshot_result().is_none());
        // Borrowed facade replacement does not drop state owned by the renderer.
    }
    {
        let mut facade = RendererFacade::legacy(&mut renderer, visible, options);
        assert_eq!(
            facade
                .render_menu(&MenuFrame::static_color(&hud, clear))
                .unwrap(),
            FrameOutcome::Offscreen
        );
        assert!(!facade.screenshot_pending());
        assert!(
            facade
                .request_screenshot(request("unread-result.png", None, false))
                .is_err()
        );
        assert_eq!(facade.take_screenshot_result(), Some(Ok([64, 32])));
        assert!(facade.take_screenshot_result().is_none());
        assert!(!path("duplicate.png").exists());
        assert!(!path("unread-result.png").exists());
    }
    let (size, image) = decode("full.png");
    assert_eq!(size, [64, 32]);
    assert_eq!(&image[24 * 4..][..4], &[255; 4]); // top row inside white HUD
    for (a, b) in image[((31 * 64 + 24) * 4)..][..4]
        .iter()
        .zip([64u8, 128, 191, 255])
    {
        assert!(a.abs_diff(b) <= 1); // bottom background, forced opaque
    }
    {
        let mut facade = RendererFacade::legacy(&mut renderer, visible, options);
        assert!(
            facade
                .request_screenshot(request("scaled.png", None, true))
                .is_err()
        );
        assert!(
            facade
                .request_screenshot(request("zero.png", Some([0, 1]), false))
                .is_err()
        );
        assert!(
            facade
                .request_screenshot(request("upscale.png", Some([33, 1]), false))
                .is_err()
        );
        assert!(
            facade
                .request_screenshot(request("budget.png", Some([9, 9]), false))
                .is_err()
        );
        facade
            .request_screenshot(request("icon.png", Some([8, 8]), true))
            .unwrap();
        let moved = facade.take_screenshot_request().unwrap();
        assert!(!facade.screenshot_pending());
        assert!(facade.take_screenshot_request().is_none());
        facade.request_screenshot(moved).unwrap();
        // Full world submission completes the same command, not only menu frames.
        facade.render(&frame::tests::source(&hud)).unwrap();
        assert_eq!(facade.take_screenshot_result(), Some(Ok([8, 8])));
        facade.restore_screenshot_result(Err("carried error".into()));
        assert_eq!(
            facade.take_screenshot_result(),
            Some(Err("carried error".into()))
        );
    }
    let (size, icon) = decode("icon.png");
    assert_eq!(size, [8, 8]);
    assert!(
        icon.as_chunks::<4>()
            .0
            .iter()
            .take(8 * 4)
            .all(|p| *p == [255; 4])
    );
    // Encoding failures do not convert a successfully rendered frame into failure.
    {
        let mut facade = RendererFacade::legacy(&mut renderer, visible, options);
        facade
            .request_screenshot(request("missing/parent.png", None, false))
            .unwrap();
        facade
            .render_menu(&MenuFrame::static_color(&hud, clear))
            .unwrap();
        assert!(facade.take_screenshot_result().unwrap().is_err());
        facade
            .request_screenshot(request("resized.png", None, false))
            .unwrap();
        facade.resize([128, 64]).unwrap();
        // Revalidate the source budget after a resize; no readback/file allocation.
        facade
            .render_menu(&MenuFrame::static_color(&HudBatch::new([128, 64]), clear))
            .unwrap();
        assert!(
            facade
                .take_screenshot_result()
                .unwrap()
                .unwrap_err()
                .contains("Budget")
        );
        assert!(!facade.screenshot_pending());
        assert!(!path("resized.png").exists());
        facade.resize([32, 16]).unwrap();
        facade
            .request_screenshot(request("after.png", None, false))
            .unwrap();
        facade
            .render_menu(&MenuFrame::static_color(&HudBatch::new([32, 16]), clear))
            .unwrap();
        assert_eq!(facade.take_screenshot_result(), Some(Ok([32, 16])));
    }
    assert_eq!(decode("after.png").0, [32, 16]);
    assert!(!path("scaled.png").exists());
}
