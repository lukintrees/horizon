use super::*;

#[test]
#[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
fn staged_asset_commit_resets_glyphs_only_on_success_and_draws_unicode_patches() {
    use crate::{
        font::Font,
        legacy_gfx::{ResourceLimits, test_assets},
    };
    use horizon_render_api::{
        render_contract::ResourceGeneration, resource_contract::TerrainAtlasLimits,
    };
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = Scratch(std::env::temp_dir().join(
        format!("horizon-native-asset-font-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()),
    ));
    std::fs::create_dir_all(root.0.join("assets/minecraft/font")).unwrap();
    std::fs::create_dir_all(root.0.join("assets/minecraft/textures/font")).unwrap();
    std::fs::write(root.0.join("assets/minecraft/font/default.json"),
        r#"{"providers":[{"type":"bitmap","file":"minecraft:font/synthetic.png","ascent":7,"chars":["Ж"]}]}"#).unwrap();
    let mut encoder = png::Encoder::new(
        std::fs::File::create(root.0.join("assets/minecraft/textures/font/synthetic.png")).unwrap(),
        8,
        8,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&[255; 8 * 8 * 4]).unwrap();
    writer.finish().unwrap();
    let mut atlas = HudAtlas::boot();
    atlas.font = Font::load(
        &root.0,
        &Default::default(),
        &[],
        (atlas.width, atlas.height),
        (600, 400),
        atlas.sprites["horizon/white"],
    );
    atlas.pack_pixels(); // exercise the bounded decompression path
    let assets = test_assets();
    let preparation = ResourceLimits {
        terrain: TerrainAtlasLimits {
            page_size: 4,
            max_pages: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(atlas.font.glyph('Ж').rect.is_some());
    let epoch = atlas.font.epoch();
    assert!(
        TerrainHudRenderer::stage_assets(
            Context::new("osmesa", [64; 2]).unwrap(),
            &assets,
            &atlas,
            &[255; 1024],
            Limits::default(),
            ResourceLimits {
                base_bytes: 1,
                ..preparation
            }
        )
        .is_err()
    );
    assert_eq!(atlas.font.epoch(), epoch);
    assert_eq!(atlas.font.take_patches().len(), 1); // failure did not consume patches
    // Successful staging can also be discarded, without invalidating old UVs.
    let staged = TerrainHudRenderer::stage_assets(
        Context::new("osmesa", [64; 2]).unwrap(),
        &assets,
        &atlas,
        &[255; 1024],
        Limits::default(),
        preparation,
    )
    .unwrap();
    assert_eq!(atlas.font.epoch(), epoch);
    drop(staged);
    assert_eq!(atlas.font.epoch(), epoch);
    // A changed font epoch must reject commit, leaving the new cache untouched.
    let staged = TerrainHudRenderer::stage_assets(
        Context::new("osmesa", [64; 2]).unwrap(),
        &assets,
        &atlas,
        &[255; 1024],
        Limits::default(),
        preparation,
    )
    .unwrap();
    atlas.font.reset_cache();
    atlas.font.glyph('Ж');
    let changed_epoch = atlas.font.epoch();
    assert!(staged.commit().is_err());
    assert_eq!(atlas.font.epoch(), changed_epoch);
    // Leave this pending patch in place: commit must discard it, not replay it
    // over the next renderer's freshly allocated glyph region.
    let staged = TerrainHudRenderer::stage_assets(
        Context::new("osmesa", [64; 2]).unwrap(),
        &assets,
        &atlas,
        &[255; 1024],
        Limits::default(),
        preparation,
    )
    .unwrap();
    assert_eq!(atlas.font.epoch(), changed_epoch);
    let mut renderer = staged.commit().unwrap();
    assert_ne!(atlas.font.epoch(), changed_epoch);
    assert!(atlas.font.take_patches().is_empty());
    assert_eq!(
        renderer.resource_generation(),
        Some(ResourceGeneration(assets.generation))
    );
    let mut hud = HudBatch::new([64; 2]);
    hud.text(&atlas, "Ж", 8., 8., u32::MAX, false);
    assert!(!hud.vertices.is_empty());
    assert_eq!(hud.patches.len(), 1);
    RendererFacade::legacy(
        &mut renderer,
        frame::VisibleSections::default(),
        frame::Options::default(),
    )
    .render_menu(&MenuFrame::static_color(&hud, [0., 0., 0., 1.]))
    .unwrap();
    let image = renderer.read_rgba([0, 0, 64, 64]).unwrap();
    assert!(image.as_chunks::<4>().0.contains(&[255; 4]));
    assert_eq!(renderer.resident_sections(), 0);
}
