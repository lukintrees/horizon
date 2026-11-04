//! Opt-in terrain/HUD coordinator using actual game types and the native GL2
//! renderer. This is not yet a replacement for every gfx::Renderer feature:
//! entities/hand/sky and special pack shaders must be ported before selecting it
//! from the normal Game startup. Unsupported GUI materials are explicit errors.
use crate::{
    gui_bridge,
    hud::{HudAtlas, HudBatch},
    mesher::SectionMesh,
    paged_hud::Backing,
    render_bridge,
};
use horizon_legacy_render::{Budget, Context, Renderer};
use horizon_render_api::{
    Image, RenderBackend, TextureId,
    atlas::AtlasLayout,
    paged_gui::{self, Cache, Layout},
    scene::{SceneFrame, SectionKey, WorldScene},
};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub gpu: Budget,
    pub sections: usize,
    pub section_vertices: usize,
    pub gui_vertices: usize,
    pub glyph_patch_bytes: usize,
    pub hud_page_size: u32,
    pub hud_resident_pages: usize,
    pub hud_backing_bytes: usize,
    pub hud_page_count: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            gpu: Budget::default(),
            sections: 1024,
            section_vertices: 262144,
            gui_vertices: 262144,
            glyph_patch_bytes: 8 << 20,
            hud_page_size: 512,
            hud_resident_pages: 8,
            hud_backing_bytes: 128 << 20,
            hud_page_count: 4096,
        }
    }
}

/// Movable, thread-confined owner: scene -> backend -> native context. No leaked
/// contexts, lifetime transmutation or references to another field of this struct.
pub struct TerrainHudRenderer {
    world: WorldScene<Renderer<'static>>,
    hud: Backing,
    cache: Cache,
    white: TextureId,
    retired_hud: Vec<Cache>,
    limits: Limits,
}
fn diagnostic(error: impl std::fmt::Debug) -> String {
    format!("legacy terrain/HUD: {error:?}")
}

impl TerrainHudRenderer {
    pub fn new(
        context: Context,
        layout: AtlasLayout,
        terrain_pages: &[&[u8]],
        lightmap: &[u8],
        hud: &HudAtlas,
        limits: Limits,
    ) -> Result<Self, String> {
        let backend = Renderer::owned(context, limits.gpu)?;
        let mut world = WorldScene::new(backend, layout, terrain_pages, lightmap, limits.sections)
            .map_err(diagnostic)?;
        let layout = Layout::new(
            [hud.width, hud.height],
            limits.hud_page_size,
            limits.hud_page_count,
        )
        .map_err(diagnostic)?;
        let backing = Backing::new(layout, &hud.pixels(), limits.hud_backing_bytes)?;
        let cache = Cache::new(layout, limits.hud_resident_pages).map_err(diagnostic)?;
        let white = world.backend_mut().create_texture(Image {
            width: 1,
            height: 1,
            rgba: &[255; 4],
        })?;
        Ok(Self {
            world,
            hud: backing,
            cache,
            white,
            retired_hud: Vec::new(),
            limits,
        })
    }

    pub fn upload_section(&mut self, key: SectionKey, mesh: &SectionMesh) -> Result<(), String> {
        render_bridge::upload_section(&mut self.world, key, mesh, self.limits.section_vertices)
            .map_err(diagnostic)
    }
    pub fn unload_section(&mut self, key: SectionKey) -> Result<bool, String> {
        self.world.unload_section(key).map_err(diagnostic)
    }
    pub fn update_lightmap(&mut self, rgba: &[u8]) -> Result<(), String> {
        self.world.update_lightmap(rgba).map_err(diagnostic)
    }
    pub fn resident_sections(&self) -> usize {
        self.world.section_count()
    }
    pub fn size(&mut self) -> [u32; 2] {
        self.world.backend_mut().context().size()
    }

    /// Convert all CPU GUI data first, so unsupported glint/portal materials do
    /// not clear the previous world image. This does not silently hide entities:
    /// callers have not yet switched the full game's gfx::Frame to this subset.
    pub fn render(&mut self, world: &SceneFrame<'_>, hud: &HudBatch) -> Result<(), String> {
        let runs = gui_bridge::runs(hud, self.size(), [1.; 2], self.limits.gui_vertices)
            .map_err(diagnostic)?;
        let runs = paged_gui::split(self.hud.layout(), &runs, self.limits.gui_vertices)
            .map_err(diagnostic)?;
        let changed = self
            .hud
            .patch(&hud.patches, self.limits.glyph_patch_bytes)?;
        self.cache.invalidate(&changed);
        self.collect_hud()?;
        self.world.render(world).map_err(diagnostic)?;
        let backing = &self.hud;
        self.cache
            .render(self.world.backend_mut(), &runs, self.white, |page| {
                backing.load(page)
            })
            .map_err(diagnostic)
    }
    /// Prepare a compressed replacement before swapping. GPU pages are uploaded
    /// lazily; old resident pages are retired before the next streamed frame.
    pub fn reload_hud(&mut self, hud: &HudAtlas) -> Result<(), String> {
        self.collect_hud()?;
        let layout = Layout::new(
            [hud.width, hud.height],
            self.limits.hud_page_size,
            self.limits.hud_page_count,
        )
        .map_err(diagnostic)?;
        let backing = Backing::new(layout, &hud.pixels(), self.limits.hud_backing_bytes)?;
        let cache = Cache::new(layout, self.limits.hud_resident_pages).map_err(diagnostic)?;
        self.retired_hud
            .push(std::mem::replace(&mut self.cache, cache));
        self.hud = backing;
        Ok(())
    }
    fn collect_hud(&mut self) -> Result<(), String> {
        while let Some(cache) = self.retired_hud.last_mut() {
            cache.release(self.world.backend_mut())?;
            self.retired_hud.pop();
        }
        Ok(())
    }
    pub fn present(&mut self) -> Result<bool, String> {
        self.world.backend_mut().context().present()
    }
    pub fn read_rgba(&mut self, rect: [u32; 4]) -> Result<Vec<u8>, String> {
        self.world.read_rgba(rect).map_err(diagnostic)
    }
}
impl Drop for TerrainHudRenderer {
    fn drop(&mut self) {
        let _ = self.cache.release(self.world.backend_mut());
        let _ = self.world.backend_mut().remove_texture(self.white);
        for mut cache in self.retired_hud.drain(..) {
            let _ = cache.release(self.world.backend_mut());
        }
        // WorldScene then deletes sections/pages/lightmap before its owned
        // backend/context are dropped, including any failed cleanup handles.
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::mesher::TerrainVertex;
    use horizon_render_api::{
        IDENTITY,
        atlas::{SpriteSize, build_page, pack_atlas},
    };
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_coordinator_renders_and_unloads_actual_game_geometry() {
        let atlas = HudAtlas::boot();
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
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[255; 16 * 16 * 4],
            &atlas,
            Limits::default(),
        )
        .unwrap();
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
                .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, u32::MAX, 0xf0f0)),
        );
        renderer.upload_section([0; 3], &mesh).unwrap();
        let mut hud = HudBatch::new([64; 2]);
        hud.fill(&atlas, 16., 16., 32., 32., u32::MAX);
        hud.invert_since(0);
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[[0; 3]],
            translucent: &[],
        };
        renderer.render(&frame, &hud).unwrap();
        assert_eq!(
            renderer.read_rgba([32, 32, 1, 1]).unwrap(),
            [191, 127, 63, 255]
        );
        assert_eq!(
            renderer.read_rgba([4, 4, 1, 1]).unwrap(),
            [64, 128, 192, 255]
        );
        renderer.reload_hud(&atlas).unwrap();
        renderer.render(&frame, &hud).unwrap();
        assert_eq!(
            renderer.read_rgba([32, 32, 1, 1]).unwrap(),
            [191, 127, 63, 255]
        );
        assert_eq!(renderer.resident_sections(), 1);
        assert!(renderer.unload_section([0; 3]).unwrap());
        assert_eq!(renderer.resident_sections(), 0);
        assert!(!renderer.present().unwrap()); // offscreen context, no fake window swap
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_large_hud_streams_one_gpu_page_and_patches_across_a_seam() {
        // Wider than a conventional GL2 driver's texture limit, but only 2 rows:
        // the fixture proves virtual addressing without a giant test allocation.
        let mut atlas = HudAtlas::boot();
        atlas.width = 16385;
        atlas.height = 2;
        atlas.pixels.clear();
        for _ in 0..2 {
            for x in 0..atlas.width {
                atlas.pixels.extend(if x < 510 {
                    [255, 0, 0, 128]
                } else {
                    [0, 0, 255, 128]
                });
            }
        }
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
        let limits = Limits {
            gpu: Budget {
                texture_bytes: (1 << 20) + 4096,
                ..Budget::default()
            },
            hud_resident_pages: 1,
            ..Limits::default()
        };
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[255; 16 * 16 * 4],
            &atlas,
            limits,
        )
        .unwrap();
        let mut batch = HudBatch::new([64; 2]);
        let (u0, u1) = (509. / atlas.width as f32, 511. / atlas.width as f32);
        batch.quad(
            [[0., 0.], [64., 0.], [64., 64.], [0., 64.]],
            [[u0, 0.25], [u1, 0.25], [u1, 0.25], [u0, 0.25]],
            u32::MAX,
        );
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[],
            translucent: &[],
        };
        renderer.render(&frame, &batch).unwrap();
        for x in 0..64 {
            let expected = if x < 32 {
                [128, 0, 0, 255]
            } else {
                [0, 0, 128, 255]
            };
            assert_eq!(renderer.read_rgba([x, 32, 1, 1]).unwrap(), expected);
        }
        assert_eq!(
            renderer.world.backend_mut().stats().texture_bytes,
            (1 << 20) + 1092
        );
        batch.patches.push(crate::font::Patch {
            x: 509,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![0, 255, 0, 255, 255, 255, 255, 255],
        });
        renderer.render(&frame, &batch).unwrap();
        assert_eq!(
            renderer.read_rgba([16, 32, 1, 1]).unwrap(),
            [0, 255, 0, 255]
        );
        assert_eq!(renderer.read_rgba([48, 32, 1, 1]).unwrap(), [255; 4]);
        batch.patches.push(crate::font::Patch {
            x: atlas.width,
            y: 0,
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        });
        assert!(renderer.render(&frame, &batch).is_err());
        assert_eq!(
            renderer.read_rgba([16, 32, 1, 1]).unwrap(),
            [0, 255, 0, 255]
        );
    }
}
