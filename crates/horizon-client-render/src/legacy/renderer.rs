//! Partial native GL frame renderer for real game data. This owns the GL
//! surface, CPU world, two texture atlases and their retryable updates. It is
//! not yet installed in the game loop and deliberately omits several passes.
use super::{
    animation::SpriteAnimator,
    atlas, hud, scene,
    world::{SectionKey, World},
};
use crate::{gfx::Frame, mesher::SectionMesh, sprites::SpriteAnimation};
use horizon_legacy_gl::{AtlasId, Page, surface::Surface};
use std::sync::Arc;
use winit::window::Window;

pub struct Textures<'a> {
    pub terrain_pixels: &'a [u8],
    pub terrain_side: u32,
    pub terrain_layers: u32,
    pub gui_pixels: &'a [u8],
    pub gui_width: u32,
    pub gui_height: u32,
    pub animations: &'a [SpriteAnimation],
}

/// Reports omissions explicitly: this is not yet a complete game frame.
#[derive(Debug, Default)]
pub struct FrameReport {
    pub presented: bool,
    pub terrain_draws: usize,
    pub entity_vertices: usize,
    pub static_entity_draws: usize,
    pub hand_vertices: usize,
    pub hud_vertices: usize,
    pub omitted_entity_overlays: Vec<(u32, usize)>,
    pub omitted_static_overlays: Vec<(u32, usize)>,
    pub omitted_hand_overlays: Vec<(u32, usize)>,
    pub omitted_hud_blends: Vec<(crate::hud::Blend, usize)>,
}

pub struct Renderer {
    surface: Surface,
    terrain_atlas: AtlasId,
    gui_atlas: AtlasId,
    world: World,
    gui: hud::PatchAtlas,
    animator: SpriteAnimator,
}

impl Renderer {
    /// All pixels are copied into paged CPU atlases and uploaded to the GL
    /// surface. On failure, the owning surface releases resources on drop.
    pub fn new(window: Arc<Window>, textures: Textures<'_>) -> Result<Self, String> {
        let mut surface = Surface::new(window).map_err(|e| format!("GL surface: {e}"))?;
        let limit = surface.max_texture_size().min(2048);
        let terrain = atlas::Atlas::new(
            textures.terrain_side,
            textures.terrain_layers,
            limit,
            textures.terrain_pixels,
        )
        .map_err(|e| format!("terrain atlas: {e}"))?;
        let gui = hud::Atlas::new(
            textures.gui_width,
            textures.gui_height,
            textures.gui_pixels,
            limit,
        )
        .map_err(|e| format!("GUI atlas: {e:?}"))?;
        let terrain_atlas = surface.active_atlas();
        let pages: Vec<_> = terrain
            .pages()
            .iter()
            .map(|p| Page {
                side: p.side,
                pixels: &p.pixels,
            })
            .collect();
        surface
            .upload_pages(&pages)
            .map_err(|e| format!("terrain upload: {e}"))?;
        let gui_atlas = surface
            .create_atlas()
            .map_err(|e| format!("GUI atlas: {e}"))?;
        surface
            .select_atlas(gui_atlas)
            .map_err(|e| format!("GUI atlas select: {e}"))?;
        let pages: Vec<_> = gui
            .pages()
            .iter()
            .map(|p| Page {
                side: p.side,
                pixels: &p.pixels,
            })
            .collect();
        surface
            .upload_pages(&pages)
            .map_err(|e| format!("GUI upload: {e}"))?;
        let animator = SpriteAnimator::new(textures.animations, &terrain);
        Ok(Self {
            surface,
            terrain_atlas,
            gui_atlas,
            world: World::new(terrain, [255; 1024]),
            gui: hud::PatchAtlas::new(gui),
            animator,
        })
    }
    pub fn upload(&mut self, key: SectionKey, mesh: &SectionMesh) -> Result<(), String> {
        self.world
            .upload(key, mesh)
            .map_err(|e| format!("section {key:?}: {e:?}"))
    }
    pub fn clear_sections(&mut self) {
        self.world.clear();
    }
    pub fn remove_columns(&mut self, columns: impl IntoIterator<Item = (i32, i32)>) {
        self.world.remove_columns(columns);
    }
    pub fn section_keys(&self) -> impl Iterator<Item = SectionKey> + '_ {
        self.world.section_keys()
    }

    /// Draws the supported subset in solid/entity/translucent/hand/HUD order.
    /// The caller must supply visible resident sections, current world min_y,
    /// gamma/fullbright and animation settings. Dirty uploads are acknowledged
    /// only after success and survive an error for the next call. Unsupported
    /// HUD blends and entity overlays are reported rather than silently drawn.
    /// Sky, LOD, particles and other passes are not implemented.
    pub fn render(
        &mut self,
        frame: &Frame<'_>,
        visible: &[SectionKey],
        min_y: i32,
        gamma: f32,
        fullbright: bool,
        animated: bool,
    ) -> Result<FrameReport, String> {
        let size = self.surface.window().inner_size();
        let viewport = [size.width, size.height];
        if viewport.contains(&0) {
            self.gui
                .apply(&frame.hud.patches)
                .map_err(|e| format!("GUI patch: {e:?}"))?;
            return Ok(FrameReport::default());
        }
        if animated {
            self.animator
                .advance(self.world.atlas_mut(), frame.ticks, None)
                .map_err(|e| format!("sprite animation: {e}"))?;
        }
        self.surface
            .select_atlas(self.terrain_atlas)
            .map_err(|e| e.to_string())?;
        let dirty: Vec<_> = self.animator.dirty().collect();
        for (page, rect) in dirty {
            let source = &self.world.atlas().pages()[page];
            self.surface
                .update_page_region(
                    page,
                    Page {
                        side: source.side,
                        pixels: &source.pixels,
                    },
                    rect,
                )
                .map_err(|e| format!("sprite upload: {e}"))?;
            self.animator.uploaded(page).map_err(|e| e.to_string())?;
        }
        let plan = scene::prepare_supported_frame(
            &mut self.world,
            &mut self.gui,
            frame,
            viewport,
            min_y,
            visible,
            gamma,
            fullbright,
        )
        .map_err(|e| format!("frame plan: {e:?}"))?;
        self.surface
            .select_atlas(self.gui_atlas)
            .map_err(|e| e.to_string())?;
        let dirty: Vec<_> = self.gui.dirty().collect();
        for change in dirty {
            let source = &self.gui.atlas().pages()[change.page];
            self.surface
                .update_page_region(
                    change.page,
                    Page {
                        side: source.side,
                        pixels: &source.pixels,
                    },
                    change.rect,
                )
                .map_err(|e| format!("GUI upload: {e}"))?;
            self.gui
                .uploaded(change.page)
                .map_err(|e| format!("GUI upload acknowledgement: {e:?}"))?;
        }
        let color = plan.fog.color;
        if !self
            .surface
            .begin_frame([color[0], color[1], color[2], 1.0])
            .map_err(|e| format!("clear: {e}"))?
        {
            return Ok(FrameReport::default());
        }
        let (solid, translucent) = plan.terrain.split_at(plan.solid_count);
        self.surface
            .select_atlas(self.terrain_atlas)
            .map_err(|e| e.to_string())?;
        for draw in solid {
            self.surface
                .draw_ordered(draw.vertices, &draw.draws, &draw.transform, draw.fog)
                .map_err(|e| format!("solid terrain: {e}"))?;
        }
        self.surface
            .select_atlas(self.gui_atlas)
            .map_err(|e| e.to_string())?;
        for draw in &plan.static_entities {
            self.surface
                .draw_ordered(
                    &draw.mesh.vertices,
                    &draw.mesh.draws,
                    &draw.transform,
                    Some(draw.fog),
                )
                .map_err(|e| format!("static block entity: {e}"))?;
        }
        self.surface
            .draw_ordered(
                &plan.entities.vertices,
                &plan.entities.draws,
                &plan.view_projection,
                Some(plan.fog),
            )
            .map_err(|e| format!("entities: {e}"))?;
        self.surface
            .select_atlas(self.terrain_atlas)
            .map_err(|e| e.to_string())?;
        for draw in translucent {
            self.surface
                .draw_ordered(draw.vertices, &draw.draws, &draw.transform, draw.fog)
                .map_err(|e| format!("translucent terrain: {e}"))?;
        }
        self.surface
            .select_atlas(self.gui_atlas)
            .map_err(|e| e.to_string())?;
        if let Some(transform) = plan.hand_view_projection {
            self.surface
                .clear_depth()
                .map_err(|e| format!("hand depth clear: {e}"))?;
            self.surface
                .draw_ordered(
                    &plan.hand.vertices,
                    &plan.hand.draws,
                    &transform,
                    Some(plan.fog),
                )
                .map_err(|e| format!("hand: {e}"))?;
        }
        self.surface
            .draw_ordered(
                &plan.hud.vertices,
                &plan.hud.draws(),
                &glam::Mat4::IDENTITY.to_cols_array(),
                None,
            )
            .map_err(|e| format!("HUD: {e}"))?;
        self.surface
            .present()
            .map_err(|e| format!("present: {e}"))?;
        Ok(FrameReport {
            presented: true,
            terrain_draws: plan.terrain.len(),
            entity_vertices: plan.entities.vertices.len(),
            static_entity_draws: plan.static_entities.len(),
            hand_vertices: plan.hand.vertices.len(),
            hud_vertices: plan.hud.vertices.len(),
            omitted_entity_overlays: plan.omitted_entity_overlays,
            omitted_static_overlays: plan.omitted_static_overlays,
            omitted_hand_overlays: plan.omitted_hand_overlays,
            omitted_hud_blends: plan.omitted_hud_blends,
        })
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::{
        font::Patch, hud::HudBatch, legacy::scene::tests::test_frame, mesher::TerrainVertex,
    };
    use winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop},
        platform::x11::EventLoopBuilderExtX11,
        window::WindowId,
    };

    #[derive(Default)]
    struct Smoke {
        completed: bool,
    }
    impl ApplicationHandler for Smoke {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_inner_size(winit::dpi::PhysicalSize::new(64, 64)),
                    )
                    .unwrap(),
            );
            let mut renderer = Renderer::new(
                window,
                Textures {
                    terrain_pixels: &[255; 4],
                    terrain_side: 1,
                    terrain_layers: 1,
                    gui_pixels: &[255; 4],
                    gui_width: 1,
                    gui_height: 1,
                    animations: &[],
                },
            )
            .unwrap();
            let quad = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
                .map(|p| TerrainVertex::new([p[0], p[1], 0.], p, 0, u32::MAX, 0));
            let mut mesh = SectionMesh::default();
            mesh.solid.extend(quad);
            mesh.translucent.extend(quad);
            renderer.upload((0, 0, 0), &mesh).unwrap();
            let mut hud = HudBatch::new([64, 64]);
            hud.quad(
                [[0., 0.], [32., 0.], [32., 32.], [0., 32.]],
                [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
                u32::MAX,
            );
            hud.patches.push(Patch {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
            });
            let hand = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]].map(|uv| crate::mesher::Vertex {
                pos: [uv[0], uv[1], -1.],
                uv,
                layer: 0,
                color: u32::MAX,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            });
            let mut frame = test_frame(&hud);
            frame.hand = &hand;
            let snapshots = [crate::block_entity_render::StaticSectionSnapshot {
                section: [0, 0, 0],
                origin: [0, 0, 0],
                revision: 1,
                entities: 1,
                vertices: hand.to_vec().into(),
            }];
            let visible_static = [[0, 0, 0]];
            frame.static_block_entities = &snapshots;
            frame.visible_static_block_entity_sections = &visible_static;
            let report = renderer
                .render(&frame, &[(0, 0, 0)], 0, 0.5, false, true)
                .unwrap();
            assert!(report.presented);
            assert_eq!(report.terrain_draws, 2);
            assert_ne!(report.hand_vertices, 0);
            assert_eq!(report.static_entity_draws, 1);
            assert!(report.omitted_static_overlays.is_empty());
            assert!(report.omitted_hand_overlays.is_empty());
            assert_eq!(report.hud_vertices, 6);
            assert!(report.omitted_hud_blends.is_empty());
            assert!(report.omitted_entity_overlays.is_empty());
            renderer.remove_columns([(0, 0)]);
            assert_eq!(renderer.section_keys().count(), 0);
            renderer.clear_sections();
            drop(renderer);
            self.completed = true;
            event_loop.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    #[test]
    #[ignore = "requires an X11 display with OpenGL 2.0 or newer"]
    fn native_frame_smoke() {
        let mut builder = EventLoop::builder();
        builder.with_x11().with_any_thread(true);
        let event_loop = builder.build().unwrap();
        let mut app = Smoke::default();
        event_loop.run_app(&mut app).unwrap();
        assert!(app.completed);
    }
}
