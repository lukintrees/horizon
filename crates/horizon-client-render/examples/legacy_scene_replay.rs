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
            renderer::{Renderer, Textures},
            world::SectionKey,
        },
    };
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
        renderer: Renderer,
        window: Arc<Window>,
        scene: Scene,
        started: Instant,
        next_frame: Instant,
        visible: Vec<SectionKey>,
    }
    fn draw(state: &mut State) -> Result<(), String> {
        let size = state.window.inner_size();
        let hud = HudBatch::new([size.width, size.height]);
        let mut frame = state.scene.frame(&hud);
        let elapsed_ticks =
            (state.started.elapsed().as_millis() / 50).min(u128::from(u64::MAX)) as u64;
        frame.ticks = frame.ticks.saturating_add(elapsed_ticks);
        let report = state.renderer.render(
            &frame,
            &state.visible,
            state.scene.renderer.min_y,
            state.scene.renderer.gamma,
            state.scene.renderer.fullbright,
            true,
        )?;
        if report.entity_vertices != 0
            || report.hud_vertices != 0
            || !report.omitted_entity_overlays.is_empty()
            || !report.omitted_hud_blends.is_empty()
        {
            return Err("capture unexpectedly contains entity or HUD geometry".into());
        }
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
                let mut renderer = Renderer::new(
                    window.clone(),
                    Textures {
                        terrain_pixels: &assets.pixels,
                        terrain_side: assets.sprite_size,
                        terrain_layers: assets.texture_layers,
                        gui_pixels: &[255; 4],
                        gui_width: 1,
                        gui_height: 1,
                        animations: &assets.animations,
                    },
                )?;
                let mut visible = Vec::new();
                let mut failure = None;
                capture::read_meshes(&self.capture, "sections", |key, mesh| {
                    if failure.is_none() {
                        let key = (key[0], key[1], key[2]);
                        match renderer.upload(key, &mesh) {
                            Ok(()) => visible.push(key),
                            Err(e) => failure = Some(format!("section {key:?}: {e}")),
                        }
                    }
                })
                .map_err(|e| format!("read sections: {e}"))?;
                if let Some(error) = failure {
                    return Err(error);
                }
                eprintln!(
                    "Loaded {} near sections. This replay omits the game loop, HUD, entities, sky and LOD.",
                    visible.len()
                );
                let started = Instant::now();
                window.request_redraw();
                Ok(State {
                    renderer,
                    window,
                    scene,
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
                    if let Some(state) = &mut self.state
                        && let Err(error) = draw(state)
                    {
                        self.error = Some(error);
                        self.state.take();
                        event_loop.exit();
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
