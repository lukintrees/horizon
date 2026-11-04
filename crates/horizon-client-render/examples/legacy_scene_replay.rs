//! Developer replay of actual captured Horizon terrain on OpenGL 2.0/2.1.
//! This is not the interactive game. It needs a `render-capture` directory and
//! the user's own prepared resources matching the capture; no assets are bundled.
//! `cargo run -p horizon-client-render --features legacy-scene,legacy-gl-window
//!  --example legacy_scene_replay -- CAPTURE_DIR [RESOURCE_ROOT]`
#![forbid(unsafe_code)]
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod app {
    use horizon_client_render::{
        assets::{Assets, RenderAssets},
        gfx::capture::{self, AtlasCheck, Scene},
        hud::HudBatch,
        legacy::{
            animation::SpriteAnimator,
            atlas::Atlas,
            hud as legacy_hud, scene,
            world::{SectionKey, World},
        },
    };
    use horizon_legacy_gl::{Page, surface::Surface};
    use std::{
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant},
    };
    use winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop},
        window::{Window, WindowId},
    };

    struct State {
        surface: Surface,
        window: Arc<Window>,
        scene: Scene,
        world: World,
        gui: legacy_hud::PatchAtlas,
        animator: SpriteAnimator,
        started: Instant,
        next_frame: Instant,
        visible: Vec<SectionKey>,
    }
    fn draw(state: &mut State) -> Result<(), String> {
        let renderer = &mut state.surface;
        let size = state.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        if size.width > i32::MAX as u32 || size.height > i32::MAX as u32 {
            return Err("viewport is too large".into());
        }
        let hud = HudBatch::new([size.width, size.height]);
        let frame = state.scene.frame(&hud);
        let elapsed_ticks =
            (state.started.elapsed().as_millis() / 50).min(u128::from(u64::MAX)) as u64;
        state
            .animator
            .advance(
                state.world.atlas_mut(),
                frame.ticks.saturating_add(elapsed_ticks),
                None,
            )
            .map_err(|e| format!("sprite animation: {e}"))?;
        let dirty: Vec<_> = state.animator.dirty().collect();
        for (page, rect) in dirty {
            let source = &state.world.atlas().pages()[page];
            renderer
                .update_page_region(
                    page,
                    Page {
                        side: source.side,
                        pixels: &source.pixels,
                    },
                    rect,
                )
                .map_err(|e| format!("sprite upload: {e:?}"))?;
            state.animator.uploaded(page).map_err(|e| e.to_string())?;
        }
        let prepared = scene::prepare_frame(
            &mut state.world,
            &mut state.gui,
            &frame,
            [size.width, size.height],
            state.scene.renderer.min_y,
            &state.visible,
            state.scene.renderer.gamma,
            state.scene.renderer.fullbright,
        )
        .map_err(|e| format!("frame plan: {e:?}"))?;
        // Captures deliberately omit entities and HUD. The placeholder GUI
        // atlas is only for preparing their empty passes, never rendered.
        if !prepared.entities.vertices.is_empty() || !prepared.hud.vertices.is_empty() {
            return Err("capture unexpectedly contains entity or HUD geometry".into());
        }
        if !renderer
            .begin_frame([
                prepared.fog.color[0],
                prepared.fog.color[1],
                prepared.fog.color[2],
                1.0,
            ])
            .map_err(|e| format!("clear: {e}"))?
        {
            return Ok(());
        }
        for draw in prepared.terrain {
            renderer
                .draw_ordered(draw.vertices, &draw.draws, &draw.transform, draw.fog)
                .map_err(|e| format!("world draw: {e:?}"))?;
        }
        renderer.present().map_err(|e| format!("{e:?}"))?;
        Ok(())
    }

    struct App {
        capture: PathBuf,
        scene: Option<Scene>,
        assets: Option<RenderAssets>,
        state: Option<State>,
        error: Option<String>,
    }
    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.state.is_some() {
                return;
            }
            let result = (|| -> Result<State, String> {
                let scene = self.scene.take().ok_or("scene already consumed")?;
                let assets = self.assets.take().ok_or("assets already consumed")?;
                let window = Arc::new(
                    event_loop
                        .create_window(
                            Window::default_attributes()
                                .with_title("Horizon GL 2 terrain capture replay (not the game)")
                                .with_inner_size(winit::dpi::PhysicalSize::new(
                                    scene.renderer.size[0].max(1),
                                    scene.renderer.size[1].max(1),
                                )),
                        )
                        .map_err(|e| e.to_string())?,
                );
                let mut renderer =
                    Surface::new(window.clone()).map_err(|e| format!("GL init: {e}"))?;
                let result =
                    (|| -> Result<(World, Vec<SectionKey>, legacy_hud::PatchAtlas), String> {
                        let atlas = Atlas::new(
                            assets.sprite_size,
                            assets.texture_layers,
                            renderer.max_texture_size().min(2048),
                            &assets.pixels,
                        )
                        .map_err(|e| e.to_string())?;
                        let pages: Vec<_> = atlas
                            .pages()
                            .iter()
                            .map(|p| Page {
                                side: p.side,
                                pixels: &p.pixels,
                            })
                            .collect();
                        renderer
                            .upload_pages(&pages)
                            .map_err(|e| format!("atlas upload: {e:?}"))?;
                        let hud = HudBatch::new(scene.renderer.size);
                        let frame = scene.frame(&hud);
                        let lightmap = scene::lightmap(
                            &frame,
                            scene.renderer.gamma,
                            scene.renderer.fullbright,
                        );
                        let mut world = World::new(atlas, lightmap);
                        let mut visible = Vec::new();
                        let mut failure = None;
                        capture::read_meshes(&self.capture, "sections", |key, mesh| {
                            if failure.is_none() {
                                let key = (key[0], key[1], key[2]);
                                match world.upload(key, &mesh) {
                                    Ok(()) => visible.push(key),
                                    Err(e) => failure = Some(format!("section {key:?}: {e:?}")),
                                }
                            }
                        })
                        .map_err(|e| format!("read sections: {e}"))?;
                        if let Some(error) = failure {
                            return Err(error);
                        }
                        let gui =
                            legacy_hud::Atlas::new(1, 1, &[255; 4], renderer.max_texture_size())
                                .map_err(|e| format!("placeholder GUI atlas: {e:?}"))?;
                        Ok((world, visible, legacy_hud::PatchAtlas::new(gui)))
                    })();
                let (world, visible, gui) = result?;
                eprintln!(
                    "Loaded {} near sections. This replay omits the game loop, HUD, entities, sky and LOD.",
                    visible.len()
                );
                let animator = SpriteAnimator::new(&assets.animations, world.atlas());
                let started = Instant::now();
                window.request_redraw();
                Ok(State {
                    surface: renderer,
                    window,
                    scene,
                    world,
                    gui,
                    animator,
                    started,
                    next_frame: started + Duration::from_millis(50),
                    visible,
                })
            })();
            match result {
                Ok(state) => self.state = Some(state),
                Err(error) => {
                    self.error = Some(error);
                    event_loop.exit();
                }
            }
        }
        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => {
                    self.state.take();
                    event_loop.exit();
                }
                WindowEvent::Resized(_) => {
                    if let Some(state) = &self.state {
                        state.window.request_redraw();
                    }
                }
                WindowEvent::RedrawRequested => {
                    if let Some(state) = &mut self.state {
                        if let Err(error) = draw(state) {
                            self.error = Some(error);
                            self.state.take();
                            event_loop.exit();
                        }
                    }
                }
                _ => {}
            }
        }
        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            if let Some(state) = &mut self.state {
                let now = Instant::now();
                if now >= state.next_frame {
                    state.window.request_redraw();
                    state.next_frame = now + Duration::from_millis(50);
                }
                event_loop
                    .set_control_flow(winit::event_loop::ControlFlow::WaitUntil(state.next_frame));
            }
        }
        fn exiting(&mut self, _: &ActiveEventLoop) {
            self.state.take();
        }
    }
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let mut args = std::env::args_os().skip(1);
        let capture = PathBuf::from(
            args.next()
                .ok_or("usage: legacy_scene_replay CAPTURE_DIR [RESOURCE_ROOT]")?,
        );
        let root = args.next().map(PathBuf::from);
        if args.next().is_some() {
            return Err("too many arguments".into());
        }
        let scene = Scene::read(&capture)?;
        let root = root.unwrap_or_else(|| scene.settings.resources.clone());
        let assets = Assets::load(&root)?.reload(&root)?;
        if AtlasCheck::of(&assets) != scene.settings.atlas {
            return Err("resources do not match the capture's block sprite layers".into());
        }
        let mut app = App {
            capture,
            scene: Some(scene),
            assets: Some(assets),
            state: None,
            error: None,
        };
        EventLoop::new()?.run_app(&mut app)?;
        if let Some(error) = app.error {
            return Err(error.into());
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn main() {
    eprintln!("Legacy scene replay requires X11 or Windows");
}
