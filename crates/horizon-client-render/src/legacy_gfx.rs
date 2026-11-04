//! Opt-in terrain/HUD coordinator using actual game types and the native GL2
//! renderer. This is not yet a replacement for every gfx::Renderer feature:
//! entities/hand/sky and special pack shaders must be ported before selecting it
//! from the normal Game startup. Unsupported GUI materials are explicit errors.
use crate::{
    gui_bridge,
    hud::{HudAtlas, HudBatch},
    mesher::SectionMesh,
    render_bridge,
};
use horizon_legacy_render::{Budget, Context, Renderer};
use horizon_render_api::{
    Image, RenderBackend, TextureId,
    atlas::AtlasLayout,
    gui,
    scene::{SceneFrame, SectionKey, WorldScene},
};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub gpu: Budget,
    pub sections: usize,
    pub section_vertices: usize,
    pub gui_vertices: usize,
    pub glyph_patch_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            gpu: Budget::default(),
            sections: 1024,
            section_vertices: 262144,
            gui_vertices: 262144,
            glyph_patch_bytes: 8 << 20,
        }
    }
}

/// Movable, thread-confined owner: scene -> backend -> native context. No leaked
/// contexts, lifetime transmutation or references to another field of this struct.
pub struct TerrainHudRenderer {
    world: WorldScene<Renderer<'static>>,
    hud: TextureId,
    white: TextureId,
    retired_hud: Vec<TextureId>,
    hud_size: [u32; 2],
    hud_uv_scale: [f32; 2],
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
        let hud_size = [hud.width, hud.height];
        let (texture, hud_uv_scale) = gui::upload_atlas(
            world.backend_mut(),
            hud_size,
            &hud.pixels(),
            limits.gpu.texture_bytes,
        )
        .map_err(diagnostic)?;
        let white = world.backend_mut().create_texture(Image {
            width: 1,
            height: 1,
            rgba: &[255; 4],
        })?;
        Ok(Self {
            world,
            hud: texture,
            white,
            retired_hud: Vec::new(),
            hud_size,
            hud_uv_scale,
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
        let runs = gui_bridge::runs(
            hud,
            self.size(),
            self.hud_uv_scale,
            self.limits.gui_vertices,
        )
        .map_err(diagnostic)?;
        gui_bridge::upload_patches(
            self.world.backend_mut(),
            self.hud,
            self.hud_size,
            hud,
            self.limits.glyph_patch_bytes,
        )
        .map_err(diagnostic)?;
        self.collect_hud()?;
        self.world.render(world).map_err(diagnostic)?;
        gui::render(self.world.backend_mut(), &runs, self.hud, self.white).map_err(diagnostic)
    }
    /// Stage the replacement atlas first. Allocation failure keeps the old atlas;
    /// a successful swap retires the old texture using the backend's owned IDs.
    pub fn reload_hud(&mut self, hud: &HudAtlas) -> Result<(), String> {
        self.collect_hud()?;
        let size = [hud.width, hud.height];
        let (texture, scale) = gui::upload_atlas(
            self.world.backend_mut(),
            size,
            &hud.pixels(),
            self.limits.gpu.texture_bytes,
        )
        .map_err(diagnostic)?;
        let previous = std::mem::replace(&mut self.hud, texture);
        self.hud_size = size;
        self.hud_uv_scale = scale;
        self.retired_hud.push(previous);
        Ok(())
    }
    fn collect_hud(&mut self) -> Result<(), String> {
        while let Some(&id) = self.retired_hud.last() {
            self.world.backend_mut().remove_texture(id)?;
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
        let _ = self.world.backend_mut().remove_texture(self.hud);
        let _ = self.world.backend_mut().remove_texture(self.white);
        for id in self.retired_hud.drain(..) {
            let _ = self.world.backend_mut().remove_texture(id);
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
}
