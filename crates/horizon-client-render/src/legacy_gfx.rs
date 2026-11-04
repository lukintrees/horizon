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
    entity::Kind,
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

/// Classic CPU geometry already posed relative to the camera. Projection uses
/// conventional desktop GL depth, not the modern renderer's reversed Z.
pub struct Geometry<'a> {
    pub entities: &'a [crate::mesher::Vertex],
    pub hand: &'a [crate::mesher::Vertex],
    pub hand_view_projection: [f32; 16],
    pub nether_lighting: bool,
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

    /// Convert all CPU GUI data first, so unsupported glint/portal materials do
    /// not clear the previous world image. This does not silently hide entities:
    /// callers have not yet switched the full game's gfx::Frame to this subset.
    pub fn render(&mut self, world: &SceneFrame<'_>, hud: &HudBatch) -> Result<(), String> {
        self.render_with_entities(world, hud, &[], false)
    }
    /// Classic camera-relative entity stream; special glint/portal
    /// materials remain explicit errors. Particles follow translucent terrain.
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
                hand_view_projection: world.view_projection,
                nether_lighting: nether,
            },
        )
    }
    pub fn render_geometry(
        &mut self,
        world: &SceneFrame<'_>,
        hud: &HudBatch,
        geometry: &Geometry<'_>,
    ) -> Result<(), String> {
        if geometry
            .entities
            .len()
            .checked_add(geometry.hand.len())
            .is_none_or(|n| n > self.limits.entity_vertices)
        {
            return Err("entity/hand input budget exceeded".into());
        }
        if !geometry.hand.is_empty() && geometry.hand_view_projection.iter().any(|v| !v.is_finite())
        {
            return Err("invalid hand projection".into());
        }
        let hand = entity_bridge::runs(
            geometry.hand,
            &self.lightmap,
            geometry.nether_lighting,
            self.limits.entity_vertices,
        )
        .map_err(diagnostic)?;
        let mut hand =
            paged_gui::split_entities(self.hud.layout(), &hand, self.limits.entity_vertices)
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
        let entity = entity_bridge::runs(
            geometry.entities,
            &self.lightmap,
            geometry.nether_lighting,
            self.limits.entity_vertices,
        )
        .map_err(diagnostic)?;
        let entity =
            paged_gui::split_entities(self.hud.layout(), &entity, self.limits.entity_vertices)
                .map_err(diagnostic)?;
        if entity
            .iter()
            .chain(&hand)
            .try_fold(0usize, |n, r| n.checked_add(r.geometry.vertices.len()))
            .is_none_or(|n| n > self.limits.entity_vertices)
        {
            return Err("entity/hand vertex budget exceeded".into());
        }
        let (mut body, particles): (Vec<_>, Vec<_>) = entity.into_iter().partition(
            |r| matches!(r.geometry.pass, horizon_render_api::Pass::Entity(m) if m.kind != Kind::Particle),
        );

        // Match the classic game's material ordering, retaining input order
        // within each kind (not sorting translucent quads by texture/page).
        body.sort_by_key(|r| match r.geometry.pass {
            horizon_render_api::Pass::Entity(m) => m.kind,
            _ => unreachable!(),
        });

        let runs = gui_bridge::runs(hud, self.size(), [1.; 2], self.limits.gui_vertices)
            .map_err(diagnostic)?;
        let runs = paged_gui::split(self.hud.layout(), &runs, self.limits.gui_vertices)
            .map_err(diagnostic)?;
        let changed = self
            .hud
            .patch(&hud.patches, self.limits.glyph_patch_bytes)?;
        self.cache.invalidate(&changed);
        self.collect_hud()?;
        self.world.collect_garbage().map_err(diagnostic)?;
        self.cache.collect_garbage(self.world.backend_mut())?;
        let remaining = self.world.backend_mut().remaining();
        if runs.iter().any(|run| {
            std::mem::size_of_val(run.geometry.vertices.as_slice())
                + std::mem::size_of_val(run.geometry.indices.as_slice())
                > remaining.mesh_bytes
        }) || body.iter().chain(&particles).chain(&hand).any(|run| {
            std::mem::size_of_val(run.geometry.vertices.as_slice())
                + std::mem::size_of_val(run.geometry.indices.as_slice())
                > remaining.mesh_bytes
        }) || ((!runs.is_empty()
            || !body.is_empty()
            || !particles.is_empty()
            || !hand.is_empty())
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
                    hand_view_projection: IDENTITY,
                    nether_lighting: false,
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
                        hand_view_projection: invalid,
                        nether_lighting: false
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
        let before = renderer.read_rgba([48, 32, 1, 1]).unwrap();
        assert!(
            renderer
                .render_geometry(
                    &frame,
                    &hud,
                    &Geometry {
                        entities: &[],
                        hand: &mask,
                        hand_view_projection: IDENTITY,
                        nether_lighting: false
                    }
                )
                .is_err()
        );
        assert_eq!(renderer.read_rgba([48, 32, 1, 1]).unwrap(), before);
    }
}
