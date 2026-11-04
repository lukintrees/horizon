//! Opt-in terrain/HUD coordinator using actual game types and the native GL2
//! renderer. This is not yet a replacement for every gfx::Renderer feature:
//! complete material/sky/platform coverage is required before selecting it
//! from the normal Game startup. Unsupported GUI materials are explicit errors.
use crate::{
    entity_bridge, gui_bridge,
    hud::{HudAtlas, HudBatch},
    mesher::SectionMesh,
    paged_hud::Backing,
    render_bridge,
};
use horizon_legacy_render::{Budget, Context, Renderer};
use horizon_render_api::{
    Image, RenderBackend, TextureId,
    atlas::AtlasLayout,
    entity::{self, Kind},
    paged_gui::{self, Cache, Layout},
    scene::{Phase, SceneFrame, SectionKey, WorldScene},
};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub gpu: Budget,
    pub sections: usize,
    pub section_vertices: usize,
    pub gui_vertices: usize,
    pub entity_vertices: usize,
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
            entity_vertices: 262144,
            glyph_patch_bytes: 8 << 20,
            hud_page_size: 512,
            hud_resident_pages: 8,
            hud_backing_bytes: 128 << 20,
            hud_page_count: 4096,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GlintParameters {
    pub strength: f32,
    pub ticks: f32,
}
impl Default for GlintParameters {
    fn default() -> Self {
        Self {
            strength: 1.,
            ticks: 0.,
        }
    }
}
struct GlintTextures {
    item: TextureId,
    armor: TextureId,
    bytes: usize,
}

/// Classic CPU geometry already posed relative to the camera. Projection uses
/// conventional desktop GL depth, not the modern renderer's reversed Z.
pub struct Geometry<'a> {
    pub entities: &'a [crate::mesher::Vertex],
    pub hand: &'a [crate::mesher::Vertex],
    /// World-space name tags/text: after particles, before the first-person hand.
    pub see_through: &'a [crate::mesher::Vertex],
    pub hand_view_projection: [f32; 16],
    pub nether_lighting: bool,
    pub glint: GlintParameters,
}

/// Movable, thread-confined owner: scene -> backend -> native context. No leaked
/// contexts, lifetime transmutation or references to another field of this struct.
pub struct TerrainHudRenderer {
    world: WorldScene<Renderer<'static>>,
    hud: Backing,
    cache: Cache,
    hud_capacity: usize,
    white: TextureId,
    lightmap: Vec<u8>,
    retired_hud: Vec<Cache>,
    glint_textures: Option<GlintTextures>,
    retired_glint: Vec<TextureId>,
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
        let white = world.backend_mut().create_texture(Image {
            width: 1,
            height: 1,
            rgba: &[255; 4],
        })?;
        let remaining = world.backend_mut().remaining();
        let hud_capacity = limits
            .hud_resident_pages
            .min(remaining.textures)
            .min(remaining.texture_bytes / layout.page_bytes());
        let cache = Cache::new(layout, hud_capacity).map_err(diagnostic)?;
        let backing = Backing::new(layout, &hud.pixels(), limits.hud_backing_bytes)?;
        Ok(Self {
            world,
            hud: backing,
            cache,
            hud_capacity,
            white,
            lightmap: lightmap.to_vec(),
            retired_hud: Vec::new(),
            glint_textures: None,
            retired_glint: Vec::new(),
            limits,
        })
    }

    /// Stage dedicated repeating glint images without changing atlas pixels.
    /// Existing glint remains selected on validation/allocation failure. The
    /// bounded page cache is emptied (framebuffer colour is not cleared).
    pub fn set_glint_textures(&mut self, item: Image<'_>, armor: Image<'_>) -> Result<(), String> {
        self.collect_glint()?;
        self.collect_hud()?;
        self.cache.release(self.world.backend_mut())?;
        let staged = (|| -> Result<_, String> {
            let bytes = item
                .rgba
                .len()
                .checked_add(armor.rgba.len())
                .ok_or("glint size overflow")?;
            let item = self.world.backend_mut().create_texture(item)?;
            self.retired_glint.push(item);
            let armor = self.world.backend_mut().create_texture(armor)?;
            self.retired_glint.push(armor);
            let remaining = self.world.backend_mut().remaining();
            let old_bytes = self.glint_textures.as_ref().map_or(0, |t| t.bytes);
            let old_count = if self.glint_textures.is_some() { 2 } else { 0 };
            let capacity = self
                .limits
                .hud_resident_pages
                .min(remaining.textures + old_count)
                .min((remaining.texture_bytes + old_bytes) / self.hud.layout().page_bytes());
            let cache = Cache::new(self.hud.layout(), capacity).map_err(diagnostic)?;
            Ok((GlintTextures { item, armor, bytes }, cache, capacity))
        })();
        let (textures, cache, capacity) = match staged {
            Ok(value) => value,
            Err(error) => {
                let _ = self.collect_glint();
                return Err(error);
            }
        };
        // Both staged handles now belong to the new active owner.
        self.retired_glint.clear();
        if let Some(old) = self.glint_textures.replace(textures) {
            self.retired_glint.extend([old.item, old.armor]);
        }
        self.cache = cache;
        self.hud_capacity = capacity;
        Ok(())
    }
    fn collect_glint(&mut self) -> Result<(), String> {
        while let Some(&id) = self.retired_glint.last() {
            self.world.backend_mut().remove_texture(id)?;
            self.retired_glint.pop();
        }
        Ok(())
    }

    pub fn upload_section(&mut self, key: SectionKey, mesh: &SectionMesh) -> Result<(), String> {
        render_bridge::upload_section(&mut self.world, key, mesh, self.limits.section_vertices)
            .map_err(diagnostic)
    }
    pub fn unload_section(&mut self, key: SectionKey) -> Result<bool, String> {
        self.world.unload_section(key).map_err(diagnostic)
    }
    pub fn update_lightmap(&mut self, rgba: &[u8]) -> Result<(), String> {
        self.world.update_lightmap(rgba).map_err(diagnostic)?;
        self.lightmap.copy_from_slice(rgba);
        Ok(())
    }
    pub fn resident_sections(&self) -> usize {
        self.world.section_count()
    }
    pub fn size(&mut self) -> [u32; 2] {
        self.world.backend_mut().context().size()
    }

    /// Convert all CPU GUI data first, so missing glint assets/unsupported portal materials do
    /// not clear the previous world image. This does not silently hide entities:
    /// callers have not yet switched the full game's gfx::Frame to this subset.
    pub fn render(&mut self, world: &SceneFrame<'_>, hud: &HudBatch) -> Result<(), String> {
        self.render_with_entities(world, hud, &[], false)
    }
    /// Classic camera-relative entity stream; missing glint assets and unsupported
    /// special materials remain explicit errors. Particles follow translucent terrain.
    pub fn render_with_entities(
        &mut self,
        world: &SceneFrame<'_>,
        hud: &HudBatch,
        entities: &[crate::mesher::Vertex],
        nether: bool,
    ) -> Result<(), String> {
        self.render_geometry(
            world,
            hud,
            &Geometry {
                entities,
                hand: &[],
                see_through: &[],
                hand_view_projection: world.view_projection,
                nether_lighting: nether,
                glint: GlintParameters::default(),
            },
        )
    }
    pub fn render_geometry(
        &mut self,
        world: &SceneFrame<'_>,
        hud: &HudBatch,
        geometry: &Geometry<'_>,
    ) -> Result<(), String> {
        let strength = entity::Strength::new(geometry.glint.strength).map_err(diagnostic)?;
        if !geometry.glint.ticks.is_finite() {
            return Err("invalid glint clock".into());
        }
        let options = entity::Options {
            nether: geometry.nether_lighting,
            glint: self.glint_textures.as_ref().map(|t| entity::GlintState {
                item: t.item,
                armor: t.armor,
                strength,
                ticks: geometry.glint.ticks,
            }),
        };
        if geometry
            .entities
            .len()
            .checked_add(geometry.hand.len())
            .and_then(|n| n.checked_add(geometry.see_through.len()))
            .is_none_or(|n| n > self.limits.entity_vertices)
        {
            return Err("entity/hand input budget exceeded".into());
        }
        if !geometry.hand.is_empty() && geometry.hand_view_projection.iter().any(|v| !v.is_finite())
        {
            return Err("invalid hand projection".into());
        }
        let mut hand = entity_bridge::prepare_with(
            geometry.hand,
            &self.lightmap,
            options,
            self.hud.layout(),
            self.limits.entity_vertices,
            entity_bridge::View::World,
        )
        .map_err(diagnostic)?;
        if hand
            .iter()
            .any(|r| matches!(r.geometry.pass, horizon_render_api::Pass::Entity(m) if matches!(m.kind, Kind::Particle | Kind::Shadow | Kind::WaterMask | Kind::Text)))
        {
            return Err("world-only material in hand view".into());
        }
        // The modern hand uses its ordinary entity pipeline even for item-sheet
        // quads; its near-plane bias is not the world item pipeline's default.
        for run in &mut hand {
            if let horizon_render_api::Pass::Entity(ref mut material) = run.geometry.pass {
                material.kind = Kind::Body;
            }
        }
        let remaining = self.limits.entity_vertices
            - hand
                .iter()
                .map(|r| r.geometry.vertices.len())
                .sum::<usize>();
        let see_through = entity_bridge::prepare_with(
            geometry.see_through,
            &self.lightmap,
            options,
            self.hud.layout(),
            remaining,
            entity_bridge::View::SeeThrough,
        )
        .map_err(diagnostic)?;
        let remaining = remaining
            - see_through
                .iter()
                .map(|r| r.geometry.vertices.len())
                .sum::<usize>();
        let entity = entity_bridge::prepare_with(
            geometry.entities,
            &self.lightmap,
            options,
            self.hud.layout(),
            remaining,
            entity_bridge::View::World,
        )
        .map_err(diagnostic)?;
        let (mut body, particles): (Vec<_>, Vec<_>) = entity.into_iter().partition(
            |r| matches!(r.geometry.pass, horizon_render_api::Pass::Entity(m) if m.kind != Kind::Particle),
        );

        // Match the classic game's material ordering, retaining input order
        // within each kind (not sorting translucent quads by texture/page).
        body.sort_by_key(|r| match r.geometry.pass {
            horizon_render_api::Pass::Entity(m) => m.kind,
            _ => unreachable!(),
        });

        let runs = {
            let source = gui_bridge::glint_runs(
                hud,
                self.size(),
                self.limits.gui_vertices,
                self.glint_textures.as_ref().map(|t| t.item),
            )
            .map_err(diagnostic)?;
            // Drop unpaged scratch before patch staging and GPU submission.
            paged_gui::split_gui(self.hud.layout(), &source, self.limits.gui_vertices)
                .map_err(diagnostic)?
        };
        let changed = self
            .hud
            .patch(&hud.patches, self.limits.glyph_patch_bytes)?;
        self.cache.invalidate(&changed);
        self.collect_hud()?;
        self.world.collect_garbage().map_err(diagnostic)?;
        self.cache.collect_garbage(self.world.backend_mut())?;
        self.collect_glint()?;
        let remaining = self.world.backend_mut().remaining();
        if runs.iter().any(|run| {
            std::mem::size_of_val(run.geometry.vertices.as_slice())
                + std::mem::size_of_val(run.geometry.indices.as_slice())
                > remaining.mesh_bytes
        }) || body
            .iter()
            .chain(&particles)
            .chain(&hand)
            .chain(&see_through)
            .any(|run| {
                std::mem::size_of_val(run.geometry.vertices.as_slice())
                    + std::mem::size_of_val(run.geometry.indices.as_slice())
                    > remaining.mesh_bytes
            })
            || ((!runs.is_empty()
                || !body.is_empty()
                || !particles.is_empty()
                || !hand.is_empty()
                || !see_through.is_empty())
                && remaining.meshes == 0)
        {
            return Err("no transient mesh capacity for legacy frame".into());
        }
        self.world
            .render_phase(world, Phase::Solid)
            .map_err(diagnostic)?;
        let view = paged_gui::WorldView {
            view_projection: world.view_projection,
            camera: world.camera,
            fog: world.fog,
            lightmap: self.world.lightmap(),
        };
        let backing = &self.hud;
        self.cache
            .render_world(self.world.backend_mut(), &body, &view, |page| {
                backing.load(page)
            })
            .map_err(diagnostic)?;
        self.world
            .render_phase(world, Phase::Translucent)
            .map_err(diagnostic)?;
        self.cache
            .render_world(self.world.backend_mut(), &particles, &view, |page| {
                backing.load(page)
            })
            .map_err(diagnostic)?;
        self.cache
            .render_world(self.world.backend_mut(), &see_through, &view, |page| {
                backing.load(page)
            })
            .map_err(diagnostic)?;
        if !hand.is_empty() {
            use horizon_render_api::{Clear, Frame};
            self.world.backend_mut().render(&Frame {
                view_projection: geometry.hand_view_projection,
                camera: world.camera,
                clear: Clear::Depth,
                fog: None,
                draws: &[],
            })?;
            let hand_view = paged_gui::WorldView {
                view_projection: geometry.hand_view_projection,
                camera: world.camera,
                fog: None,
                lightmap: view.lightmap,
            };
            self.cache
                .render_world(self.world.backend_mut(), &hand, &hand_view, |page| {
                    backing.load(page)
                })
                .map_err(diagnostic)?;
        }
        self.cache
            .render_gui(self.world.backend_mut(), &runs, self.white, |page| {
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
        let cache = Cache::new(layout, self.hud_capacity).map_err(diagnostic)?;
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
    pub fn set_title(&mut self, title: &str) -> Result<(), String> {
        self.world.backend_mut().context().set_title(title)
    }
    /// Pump native window events before computing view/HUD input for the frame.
    /// Zero drawable dimensions mean minimized: suspend rendering until restored.
    pub fn poll_events(&mut self) -> Result<horizon_legacy_render::WindowEvents, String> {
        self.world.backend_mut().context().poll_events()
    }
    /// Resize in-place: resident terrain, atlas pages and foil images survive.
    /// The caller must rebuild projection and HudBatch for the new dimensions.
    pub fn resize(&mut self, size: [u32; 2]) -> Result<(), String> {
        self.world.backend_mut().context().resize(size)
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
        if let Some(textures) = self.glint_textures.take() {
            self.retired_glint.extend([textures.item, textures.armor]);
        }
        let _ = self.collect_glint();
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
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_transient_mesh_preflight_does_not_erase_the_previous_frame() {
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
        let limits = Limits {
            gpu: Budget {
                mesh_bytes: 4 * 44 + 6 * 2,
                ..Budget::default()
            },
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
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
                .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, u32::MAX, 0xf0f0)),
        );
        renderer.upload_section([0; 3], &mesh).unwrap();
        let mut frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[[0; 3]],
            translucent: &[],
        };
        let mut hud = HudBatch::new([64; 2]);
        renderer.render(&frame, &hud).unwrap();
        frame.solid = &[];
        frame.clear = [1., 0., 0., 1.];
        hud.fill(&atlas, 0., 0., 64., 64., u32::MAX);
        assert!(renderer.render(&frame, &hud).is_err());
        assert_eq!(
            renderer.read_rgba([32, 32, 1, 1]).unwrap(),
            [64, 128, 192, 255]
        );
        let entity = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|pos| {
            crate::mesher::Vertex {
                pos,
                uv: [0.; 2],
                layer: 0,
                color: u32::MAX,
                light: 0xf0f0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            }
        });
        assert!(
            renderer
                .render_with_entities(&frame, &HudBatch::new([64; 2]), &entity, false)
                .is_err()
        );
        assert_eq!(
            renderer.read_rgba([32, 32, 1, 1]).unwrap(),
            [64, 128, 192, 255]
        );
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_entities_terrain_particles_hand_and_hud_share_one_page_cache() {
        use crate::mesher::Vertex;
        let mut atlas = HudAtlas::boot();
        atlas.width = 6;
        atlas.height = 2;
        atlas.pixels = [
            255, 0, 0, 128, 255, 0, 0, 128, 0, 255, 0, 128, 0, 255, 0, 128, 0, 0, 255, 128, 0, 0,
            255, 128,
        ]
        .repeat(2);
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
        let limits = Limits {
            hud_page_size: 4,
            gpu: Budget {
                texture_bytes: 1156,
                ..Budget::default()
            },
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
        let positions = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]];
        let mut terrain = SectionMesh::default();
        terrain
            .translucent
            .extend(positions.map(|p| TerrainVertex::new(p, [0.5; 2], 0, 0xffff0000, 0xf0f0)));
        renderer.upload_section([0; 3], &terrain).unwrap();
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[],
            translucent: &[[0; 3]],
        };
        let quad = |u, depth, layer| {
            positions.map(|p| Vertex {
                pos: [p[0], p[1], depth],
                uv: [u, 0.5],
                layer,
                color: u32::MAX,
                light: 0xf0f0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            })
        };
        let entities = [quad(0.1, 0.2, 32), quad(0.5, 0.1, 15)].concat();
        let mut hud = HudBatch::new([64; 2]);
        hud.quad(
            [[0., 0.], [8., 0.], [8., 8.], [0., 8.]],
            [[0.9, 0.5]; 4],
            u32::MAX,
        );
        renderer
            .render_with_entities(&frame, &hud, &entities, false)
            .unwrap();
        let near = |actual: &[u8], expected: [u8; 3]| {
            for i in 0..3 {
                assert!(
                    (i16::from(actual[i]) - i16::from(expected[i])).abs() <= 2,
                    "{actual:?} != {expected:?}"
                );
            }
        };
        near(&renderer.read_rgba([32, 32, 1, 1]).unwrap(), [32, 128, 64]);
        near(&renderer.read_rgba([4, 60, 1, 1]).unwrap(), [16, 64, 160]);
        assert_eq!(renderer.world.backend_mut().stats().texture_bytes, 1156);
        assert_eq!(renderer.world.backend_mut().stats().mesh_bytes, 188);
        // A farther hand must draw on top using its independent projection/depth
        // view, but the HUD still follows it. The hand's red texel is half alpha.
        let hand = quad(0.1, 0.9, 0);
        renderer
            .render_geometry(
                &frame,
                &hud,
                &Geometry {
                    entities: &entities,
                    hand: &hand,
                    see_through: &[],
                    hand_view_projection: IDENTITY,
                    nether_lighting: false,
                    glint: GlintParameters::default(),
                },
            )
            .unwrap();
        near(&renderer.read_rgba([32, 32, 1, 1]).unwrap(), [144, 64, 32]);
        near(&renderer.read_rgba([4, 60, 1, 1]).unwrap(), [72, 32, 144]);
        let before = renderer.read_rgba([32, 32, 1, 1]).unwrap();
        assert!(
            renderer
                .render_with_entities(&frame, &hud, &quad(0.1, 0., 7), false)
                .is_err()
        );
        assert_eq!(renderer.read_rgba([32, 32, 1, 1]).unwrap(), before);
        let mut invalid = IDENTITY;
        invalid[0] = f32::NAN;
        assert!(
            renderer
                .render_geometry(
                    &frame,
                    &hud,
                    &Geometry {
                        entities: &[],
                        hand: &hand,
                        see_through: &[],
                        hand_view_projection: invalid,
                        nether_lighting: false,
                        glint: GlintParameters::default(),
                    }
                )
                .is_err()
        );
        assert_eq!(renderer.read_rgba([32, 32, 1, 1]).unwrap(), before);
        renderer.update_lightmap(&[0; 1024]).unwrap();
        renderer
            .render_with_entities(&frame, &HudBatch::new([64; 2]), &quad(0.1, -0.5, 0), false)
            .unwrap();
        near(&renderer.read_rgba([32, 32, 1, 1]).unwrap(), [0; 3]);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_shadow_item_mask_text_and_water_obey_classic_submission_order() {
        use crate::mesher::Vertex;
        let mut atlas = HudAtlas::boot();
        atlas.width = 3;
        atlas.height = 1;
        atlas.pixels = vec![255, 0, 0, 128, 0, 0, 0, 16, 0, 0, 0, 0];
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }; 2],
            4,
            1,
            2,
        )
        .unwrap();
        let sprites: [&[u8]; 2] = [&[255; 4], &[255, 255, 255, 128]];
        let pages: Vec<_> = (0..layout.page_count())
            .map(|p| build_page(&layout, p, &sprites).unwrap())
            .collect();
        let refs: Vec<_> = pages.iter().map(Vec::as_slice).collect();
        let limits = Limits {
            hud_page_size: 4,
            gpu: Budget {
                texture_bytes: 1220,
                ..Budget::default()
            },
            ..Limits::default()
        };
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &refs,
            &[255; 1024],
            &atlas,
            limits,
        )
        .unwrap();
        let positions = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]];
        let mut terrain = SectionMesh::default();
        terrain
            .solid
            .extend(positions.map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, 0xff808080, 0xf0f0)));
        terrain.translucent.extend(
            positions
                .map(|p| TerrainVertex::new([p[0], p[1], -0.3], [0.5; 2], 1, 0xffff0000, 0xf0f0)),
        );
        renderer.upload_section([0; 3], &terrain).unwrap();
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[[0; 3]],
            translucent: &[[0; 3]],
        };
        let entity = |layer, z, u| {
            positions.map(|p| Vertex {
                pos: [p[0], p[1], z],
                uv: [u, 0.5],
                layer,
                color: u32::MAX,
                light: 0xf0f0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            })
        };
        let mut mask = entity(13, -0.5, f32::NAN);
        for v in &mut mask {
            v.pos[0] = v.pos[0] * 0.5 + 0.5;
            v.color = 0;
        }
        let item = entity(32, -0.2, 0.5 / 3.);
        let shadow = entity(14, 0., 1.5 / 3.);
        // Deliberately opposite to submission order: mask first, item before
        // shadow. Sorting by texture or preserving these phase tags verbatim
        // would hide the item or miss its faint shadow beneath the water.
        let vertices = [mask, item, shadow].concat();
        let hud = HudBatch::new([64; 2]);
        renderer
            .render_with_entities(&frame, &hud, &vertices, false)
            .unwrap();
        let near = |actual: &[u8], expected: [u8; 3]| {
            for i in 0..3 {
                assert!(
                    (i16::from(actual[i]) - i16::from(expected[i])).abs() <= 2,
                    "{actual:?} != {expected:?}"
                );
            }
        };
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [94, 30, 158]);
        near(&renderer.read_rgba([48, 32, 1, 1]).unwrap(), [188, 60, 60]);
        assert_eq!(renderer.world.backend_mut().stats().texture_bytes, 1220);
        assert_eq!(renderer.world.backend_mut().stats().mesh_bytes, 376);
        let text = entity(35, 0.0000005, 0.5 / 3.);
        renderer
            .render_with_entities(&frame, &hud, &text, false)
            .unwrap();
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [96, 32, 160]);
        let mut back = text;
        back.reverse();
        renderer
            .render_with_entities(&frame, &hud, &back, false)
            .unwrap();
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [64, 64, 192]);
        let names = entity(0, 0.8, 0.5 / 3.);
        renderer
            .render_geometry(
                &frame,
                &hud,
                &Geometry {
                    entities: &[],
                    hand: &[],
                    see_through: &names,
                    hand_view_projection: IDENTITY,
                    nether_lighting: false,
                    glint: GlintParameters::default(),
                },
            )
            .unwrap();
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [160, 32, 96]);
        let held = entity(32, 0.9, 0.5 / 3.);
        renderer
            .render_geometry(
                &frame,
                &hud,
                &Geometry {
                    entities: &[],
                    hand: &held,
                    see_through: &names,
                    hand_view_projection: IDENTITY,
                    nether_lighting: false,
                    glint: GlintParameters::default(),
                },
            )
            .unwrap();
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [208, 16, 48]);
        let stable = renderer.read_rgba([16, 32, 1, 1]).unwrap();
        renderer.limits.entity_vertices = 10; // eight input vertices expand to twelve
        assert!(
            renderer
                .render_geometry(
                    &frame,
                    &hud,
                    &Geometry {
                        entities: &[],
                        hand: &held,
                        see_through: &names,
                        hand_view_projection: IDENTITY,
                        nether_lighting: false,
                        glint: GlintParameters::default(),
                    }
                )
                .is_err()
        );
        assert_eq!(renderer.read_rgba([16, 32, 1, 1]).unwrap(), stable);
        renderer.limits.entity_vertices = 12;
        renderer
            .render_geometry(
                &frame,
                &hud,
                &Geometry {
                    entities: &[],
                    hand: &held,
                    see_through: &names,
                    hand_view_projection: IDENTITY,
                    nether_lighting: false,
                    glint: GlintParameters::default(),
                },
            )
            .unwrap();
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [208, 16, 48]);
        renderer.limits.entity_vertices = limits.entity_vertices;
        let before = renderer.read_rgba([48, 32, 1, 1]).unwrap();
        assert!(
            renderer
                .render_geometry(
                    &frame,
                    &hud,
                    &Geometry {
                        entities: &[],
                        hand: &mask,
                        see_through: &[],
                        hand_view_projection: IDENTITY,
                        nether_lighting: false,
                        glint: GlintParameters::default(),
                    }
                )
                .is_err()
        );
        assert_eq!(renderer.read_rgba([48, 32, 1, 1]).unwrap(), before);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_glint_hand_reload_rollback_and_exact_page_budget() {
        use crate::mesher::Vertex;
        let mut atlas = HudAtlas::boot();
        atlas.width = 3;
        atlas.height = 1;
        atlas.pixels = [64, 128, 192, 255].repeat(3);
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
            &[128, 255, 64, 255].repeat(256),
            &atlas,
            Limits {
                gpu: Budget {
                    texture_bytes: 1164,
                    mesh_bytes: 516,
                },
                hud_page_size: 4,
                ..Limits::default()
            },
        )
        .unwrap();
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[],
            translucent: &[],
        };
        let hud = HudBatch::new([64; 2]);
        let quad = |layer| {
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|pos| Vertex {
                pos,
                uv: [0.5 / 3., 0.5],
                layer,
                color: u32::MAX,
                light: 0xf0f0,
                normal: [0; 4],
                glint_uv: [-0.25, 1.25],
            })
        };
        let sample = |r: &mut TerrainHudRenderer, expected: [u8; 3]| {
            let actual = r.read_rgba([32, 32, 1, 1]).unwrap();
            for i in 0..3 {
                assert!(
                    (i16::from(actual[i]) - i16::from(expected[i])).abs() <= 2,
                    "{actual:?} != {expected:?}"
                );
            }
        };
        renderer
            .render_with_entities(&frame, &hud, &quad(0), false)
            .unwrap();
        sample(&mut renderer, [32, 128, 48]);
        let before = renderer.read_rgba([32, 32, 1, 1]).unwrap();
        assert!(
            renderer
                .render_with_entities(&frame, &hud, &quad(7), false)
                .is_err()
        );
        assert_eq!(renderer.read_rgba([32, 32, 1, 1]).unwrap(), before);
        let image = |rgba| Image {
            width: 1,
            height: 1,
            rgba,
        };
        renderer
            .set_glint_textures(image(&[128, 64, 0, 0]), image(&[0, 128, 128, 0]))
            .unwrap();
        let draw = |renderer: &mut TerrainHudRenderer, layer, hand, strength| {
            let vertices = quad(layer);
            renderer.render_geometry(
                &frame,
                &hud,
                &Geometry {
                    entities: if hand { &[] } else { &vertices },
                    hand: if hand { &vertices } else { &[] },
                    see_through: &[],
                    hand_view_projection: IDENTITY,
                    nether_lighting: false,
                    glint: GlintParameters {
                        strength,
                        ticks: 27500.,
                    },
                },
            )
        };
        for hand in [false, true] {
            for (layer, color) in [
                (7, [48, 132, 48]),
                (8, [77, 93, 34]),
                (33, [48, 132, 48]),
                (36, [32, 144, 64]),
            ] {
                draw(&mut renderer, layer, hand, 0.5).unwrap();
                sample(&mut renderer, color);
            }
        }
        draw(&mut renderer, 7, false, 0.).unwrap();
        sample(&mut renderer, [32, 128, 48]);
        let stable = renderer.read_rgba([32, 32, 1, 1]).unwrap();
        for strength in [f32::NAN, -0.1, 1.1] {
            assert!(draw(&mut renderer, 7, false, strength).is_err());
            assert_eq!(renderer.read_rgba([32, 32, 1, 1]).unwrap(), stable);
        }
        // A half-transparent base becomes opaque with full glint strength.
        let mut patched = HudBatch::new([64; 2]);
        patched.patches.push(crate::font::Patch {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            rgba: vec![64, 128, 192, 128],
        });
        renderer
            .render_with_entities(&frame, &patched, &quad(33), false)
            .unwrap();
        sample(&mut renderer, [96, 144, 48]);
        let stable = renderer.read_rgba([32, 32, 1, 1]).unwrap();
        // Stage first image, reject malformed second; active images/frame survive.
        assert!(
            renderer
                .set_glint_textures(
                    image(&[255; 4]),
                    Image {
                        width: 2,
                        height: 2,
                        rgba: &[255; 4]
                    }
                )
                .is_err()
        );
        assert_eq!(renderer.read_rgba([32, 32, 1, 1]).unwrap(), stable);
        draw(&mut renderer, 33, true, 1.).unwrap();
        sample(&mut renderer, [96, 144, 48]);
        // Both images fit, but leave no room for even one atlas page. Roll back
        // BOTH staged handles, retaining the old assets and original cache quota.
        assert!(
            renderer
                .set_glint_textures(
                    Image {
                        width: 2,
                        height: 2,
                        rgba: &[255; 16]
                    },
                    image(&[255; 4])
                )
                .is_err()
        );
        draw(&mut renderer, 7, false, 1.).unwrap();
        sample(&mut renderer, [96, 144, 48]);
        for _ in 0..4 {
            renderer
                .set_glint_textures(image(&[0, 128, 0, 0]), image(&[0, 0, 128, 0]))
                .unwrap();
        }
        // Retired assets must be collected BEFORE the next page preflight/upload:
        // only 64 bytes remain in this deliberately exact texture budget.
        draw(&mut renderer, 33, true, 1.).unwrap();
        sample(&mut renderer, [32, 192, 48]);
        draw(&mut renderer, 36, false, 1.).unwrap();
        sample(&mut renderer, [32, 128, 112]);
        let stats = renderer.world.backend_mut().stats();
        assert_eq!(stats.texture_bytes, 1164);
        assert_eq!(stats.textures, 6);
        assert_eq!(stats.mesh_bytes, 0);
        assert_eq!(stats.meshes, 0);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_gui_foil_masks_pages_patches_and_reloads_with_compact_mesh_budget() {
        let mut atlas = HudAtlas::boot();
        atlas.width = 6;
        atlas.height = 2;
        atlas.pixels = [0, 16, 128, 255, 255, 255]
            .into_iter()
            .flat_map(|a| [255, 255, 255, a])
            .collect::<Vec<_>>()
            .repeat(2);
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
            &[0; 1024],
            &atlas,
            Limits {
                gpu: Budget {
                    texture_bytes: 1164,
                    mesh_bytes: 324,
                },
                hud_page_size: 4,
                gui_vertices: 256,
                ..Limits::default()
            },
        )
        .unwrap();
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: Some(horizon_render_api::Fog {
                color: [0., 0., 1.],
                start: 0.,
                end: 0.0001,
            }),
            solid: &[],
            translucent: &[],
        };
        let p = [[2., 2.], [62., 2.], [62., 62.], [2., 62.]];
        let mut hud = HudBatch::new([64; 2]);
        hud.glint_alpha = 0.5;
        hud.quad(p, [[4.5 / 6., 0.5]; 4], 0xff302010);
        renderer.render(&frame, &hud).unwrap();
        let stable = renderer.read_rgba([27, 32, 1, 1]).unwrap();
        hud.quad_glint(
            p,
            [[2., 0.], [3., 0.], [3., 1.], [2., 1.]],
            [[0.25, 0.5]; 4],
            0,
        );
        hud.blend_since(6, crate::hud::Blend::Additive);
        assert!(renderer.render(&frame, &hud).is_err());
        assert_eq!(renderer.read_rgba([27, 32, 1, 1]).unwrap(), stable);
        let image = |rgba| Image {
            width: 1,
            height: 1,
            rgba,
        };
        renderer
            .set_glint_textures(image(&[128, 64, 0, 0]), image(&[0; 4]))
            .unwrap();
        let near = |r: &mut TerrainHudRenderer, x, expected: [u8; 3]| {
            let actual = r.read_rgba([x, 32, 1, 1]).unwrap();
            for i in 0..3 {
                assert!(
                    (i16::from(actual[i]) - i16::from(expected[i])).abs() <= 2,
                    "{actual:?} != {expected:?}"
                );
            }
        };
        renderer.render(&frame, &hud).unwrap();
        near(&mut renderer, 7, [16, 32, 48]);
        for x in [27, 37, 57] {
            near(&mut renderer, x, [32, 36, 48]);
        }
        hud.patches.push(crate::font::Patch {
            x: 2,
            y: 0,
            width: 1,
            height: 2,
            rgba: [255, 255, 255, 0].repeat(2),
        });
        renderer.render(&frame, &hud).unwrap();
        near(&mut renderer, 27, [16, 32, 48]);
        near(&mut renderer, 37, [32, 36, 48]);
        hud.patches.clear();
        let stable = renderer.read_rgba([37, 32, 1, 1]).unwrap();
        renderer.reload_hud(&atlas).unwrap();
        renderer
            .set_glint_textures(image(&[0, 128, 0, 0]), image(&[0; 4]))
            .unwrap();
        assert_eq!(renderer.read_rgba([37, 32, 1, 1]).unwrap(), stable);
        renderer.render(&frame, &hud).unwrap();
        near(&mut renderer, 27, [16, 48, 48]);
        let stable = renderer.read_rgba([27, 32, 1, 1]).unwrap();
        for bad in [f32::NAN, -0.1, 1.1] {
            hud.glint_alpha = bad;
            assert!(renderer.render(&frame, &hud).is_err());
            assert_eq!(renderer.read_rgba([27, 32, 1, 1]).unwrap(), stable);
        }
        hud.glint_alpha = 0.5;
        renderer.limits.gui_vertices = 11;
        assert!(renderer.render(&frame, &hud).is_err());
        renderer.limits.gui_vertices = 12;
        assert!(renderer.render(&frame, &hud).is_err());
        assert_eq!(renderer.read_rgba([27, 32, 1, 1]).unwrap(), stable);
        renderer.limits.gui_vertices = 256;
        hud.vertices[7].glint_uv = [f32::NAN; 2];
        assert!(renderer.render(&frame, &hud).is_err());
        hud.vertices[7].glint_uv = [0.25, 0.5];
        hud.vertices[7].uv[0] = 0.5;
        assert!(renderer.render(&frame, &hud).is_err());
        hud.vertices[7].uv[0] = 3.;
        for blend in [crate::hud::Blend::EndPortal, crate::hud::Blend::CitGlint(0)] {
            hud.blends = vec![(6..12, blend)];
            assert!(renderer.render(&frame, &hud).is_err());
            assert_eq!(renderer.read_rgba([27, 32, 1, 1]).unwrap(), stable);
        }
        hud.blends = vec![(6..12, crate::hud::Blend::Additive)];
        renderer.render(&frame, &hud).unwrap();
        let stats = renderer.world.backend_mut().stats();
        assert_eq!(stats.texture_bytes, 1164);
        assert_eq!(stats.textures, 6);
        assert_eq!(stats.mesh_bytes, 0);
        assert_eq!(stats.meshes, 0);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_resize_retains_world_pages_glint_and_rebuilds_hud_at_new_dimensions() {
        let mut atlas = HudAtlas::boot();
        atlas.width = 3;
        atlas.height = 1;
        atlas.pixels = [255; 12].to_vec();
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
            &[255; 1024],
            &atlas,
            Limits {
                gpu: Budget {
                    texture_bytes: 1164,
                    mesh_bytes: 512,
                },
                hud_page_size: 4,
                ..Limits::default()
            },
        )
        .unwrap();
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
                .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, u32::MAX, 0xf0f0)),
        );
        renderer.upload_section([0; 3], &mesh).unwrap();
        renderer
            .set_glint_textures(
                Image {
                    width: 1,
                    height: 1,
                    rgba: &[0, 128, 0, 0],
                },
                Image {
                    width: 1,
                    height: 1,
                    rgba: &[0; 4],
                },
            )
            .unwrap();
        let item_id = renderer.glint_textures.as_ref().unwrap().item;
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[[0; 3]],
            translucent: &[],
        };
        for size in [[64; 2], [96, 48], [40, 96], [128, 32], [64; 2]] {
            renderer.resize(size).unwrap();
            assert_eq!(renderer.size(), size);
            let events = renderer.poll_events().unwrap();
            assert!(events.events.is_empty());
            assert!(!events.close_requested);
            let mut hud = HudBatch::new(size);
            hud.glint_alpha = 1.;
            let p = [
                [2., 2.],
                [size[0] as f32 / 2., 2.],
                [size[0] as f32 / 2., size[1] as f32 / 2.],
                [2., size[1] as f32 / 2.],
            ];
            hud.quad(p, [[0.5 / 3., 0.5]; 4], 0xff302010);
            hud.quad_glint(p, [[2. + 0.5 / 3., 0.5]; 4], [[0.; 2]; 4], 0);
            hud.blend_since(6, crate::hud::Blend::Additive);
            renderer.render(&frame, &hud).unwrap();
            let inside = renderer.read_rgba([8, size[1] - 8, 1, 1]).unwrap();
            for (a, b) in inside[..3].iter().zip([16u8, 96, 48]) {
                assert!(a.abs_diff(b) <= 2, "{inside:?}");
            }
            assert_eq!(
                renderer.read_rgba([size[0] - 2, 2, 1, 1]).unwrap(),
                [64, 128, 192, 255]
            );
            assert!(!renderer.present().unwrap());
            assert_eq!(renderer.resident_sections(), 1);
            assert_eq!(renderer.glint_textures.as_ref().unwrap().item, item_id);
            let stats = renderer.world.backend_mut().stats();
            assert_eq!(stats.texture_bytes, 1164);
            assert_eq!(stats.mesh_bytes, 188);
            assert!(renderer.resize([0, 1]).is_err());
            assert_eq!(renderer.size(), size);
            assert_eq!(
                renderer.read_rgba([size[0] - 2, 2, 1, 1]).unwrap(),
                [64, 128, 192, 255]
            );
        }
        assert!(renderer.unload_section([0; 3]).unwrap());
        assert_eq!(renderer.resident_sections(), 0);
        // Scene unload retires resources; collect at the documented boundary.
        renderer.world.collect_garbage().unwrap();
        assert_eq!(renderer.world.backend_mut().stats().mesh_bytes, 0);
    }
}
