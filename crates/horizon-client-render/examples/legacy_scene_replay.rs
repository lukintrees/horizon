//! Developer replay of actual captured Horizon terrain on OpenGL 2.0/2.1.
//! This is not the interactive game. It needs a `render-capture` directory and
//! the user's own prepared resources matching the capture; no assets are bundled.
//! `cargo run -p horizon-client-render --features legacy-scene,legacy-gl-window
//!  --example legacy_scene_replay -- CAPTURE_DIR [RESOURCE_ROOT]`
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod app {
    use horizon_client_render::{
        assets::{Assets, RenderAssets},
        gfx::capture::{self, AtlasCheck, Scene},
        hud::HudBatch,
        legacy::{
            animation::SpriteAnimator,
            atlas::Atlas,
            scene,
            world::{SectionKey, World},
        },
    };
    use horizon_legacy_gl::{Page, Renderer, window::WindowContext};
    use std::{
        ffi::{CStr, c_void},
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant},
    };
    #[cfg(target_os = "linux")]
    use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    use winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop},
        raw_window_handle::{HasWindowHandle, RawWindowHandle},
        window::{Window, WindowId},
    };

    struct State {
        renderer: Option<Renderer>,
        context: Option<WindowContext>,
        window: Arc<Window>,
        scene: Scene,
        world: World,
        animator: SpriteAnimator,
        started: Instant,
        next_frame: Instant,
        visible: Vec<SectionKey>,
    }
    impl Drop for State {
        fn drop(&mut self) {
            // SAFETY: state owns the window and context on the event-loop
            // thread; the renderer is deleted before its context and window.
            unsafe {
                if let Some(context) = &self.context
                    && context.make_current().is_ok()
                    && let Some(renderer) = self.renderer.take()
                {
                    renderer.destroy();
                }
            }
            self.context.take();
        }
    }

    unsafe fn context(window: &Window) -> Result<WindowContext, String> {
        let handle = window.window_handle().map_err(|e| e.to_string())?;
        #[cfg(target_os = "linux")]
        {
            let display = window.display_handle().map_err(|e| e.to_string())?;
            match (display.as_raw(), handle.as_raw()) {
                (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window)) => {
                    let display = display.display.ok_or("null Xlib display")?.as_ptr();
                    // SAFETY: caller retains the X11 window and display for the context lifetime.
                    unsafe {
                        WindowContext::new_xlib(display, window.window)
                            .map_err(|e| format!("{e:?}"))
                    }
                }
                _ => Err("legacy replay requires X11/Xlib, not Wayland".into()),
            }
        }
        #[cfg(target_os = "windows")]
        {
            match handle.as_raw() {
                // SAFETY: caller retains the Win32 window for the context lifetime.
                RawWindowHandle::Win32(window) => unsafe {
                    WindowContext::new_win32(window.hwnd.get() as *mut c_void)
                        .map_err(|e| format!("{e:?}"))
                },
                _ => Err("legacy replay requires a Win32 window".into()),
            }
        }
    }

    unsafe fn proc(context: &WindowContext, name: &CStr) -> Result<*const c_void, String> {
        // SAFETY: the caller made this context current on its owning thread.
        let address = unsafe { context.resolve(name) };
        if address.is_null() || address as usize <= 3 || address as usize == usize::MAX {
            Err(format!("missing {}", name.to_string_lossy()))
        } else {
            Ok(address)
        }
    }

    unsafe fn draw(state: &mut State) -> Result<(), String> {
        let context = state.context.as_ref().ok_or("missing GL context")?;
        let renderer = state.renderer.as_mut().ok_or("missing GL renderer")?;
        // SAFETY: the State owns a live context and window on this thread.
        unsafe {
            context.make_current().map_err(|e| format!("{e:?}"))?;
            let viewport = std::mem::transmute::<
                *const c_void,
                unsafe extern "system" fn(i32, i32, i32, i32),
            >(proc(context, c"glViewport")?);
            let clear_color = std::mem::transmute::<
                *const c_void,
                unsafe extern "system" fn(f32, f32, f32, f32),
            >(proc(context, c"glClearColor")?);
            let clear = std::mem::transmute::<*const c_void, unsafe extern "system" fn(u32)>(proc(
                context, c"glClear",
            )?);
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
            let fog = scene::frame_fog(&frame);
            viewport(0, 0, size.width as i32, size.height as i32);
            clear_color(fog.color[0], fog.color[1], fog.color[2], 1.0);
            clear(0x4000 | 0x0100);
            let draws = scene::plan(
                &mut state.world,
                &frame,
                [size.width, size.height],
                state.scene.renderer.min_y,
                &state.visible,
            )
            .map_err(|e| format!("world plan: {e:?}"))?;
            for draw in draws {
                if let Some(fog) = draw.fog {
                    renderer.draw_ordered_with_fog(draw.vertices, &draw.draws, &draw.transform, fog)
                } else {
                    renderer.draw_ordered(draw.vertices, &draw.draws, &draw.transform)
                }
                .map_err(|e| format!("world draw: {e:?}"))?;
            }
            context.swap_buffers().map_err(|e| format!("{e:?}"))?;
            Ok(())
        }
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
                // SAFETY: window, context and renderer are retained together in State.
                let context = unsafe { context(&window)? };
                // SAFETY: the context is current and remains on this event-loop thread.
                let mut renderer = unsafe { Renderer::new(|name| context.resolve(name)) }
                    .map_err(|e| format!("GL init: {e:?}"))?;
                let result = (|| -> Result<(World, Vec<SectionKey>), String> {
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
                    // SAFETY: the context remains current and page bytes are alive for the call.
                    unsafe { renderer.upload_pages(&pages) }
                        .map_err(|e| format!("atlas upload: {e:?}"))?;
                    let hud = HudBatch::new(scene.renderer.size);
                    let frame = scene.frame(&hud);
                    let lightmap =
                        scene::lightmap(&frame, scene.renderer.gamma, scene.renderer.fullbright);
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
                    Ok((world, visible))
                })();
                let (world, visible) = match result {
                    Ok(value) => value,
                    Err(error) => {
                        // SAFETY: still on the owning thread with the context current.
                        unsafe { renderer.destroy() };
                        return Err(error);
                    }
                };
                eprintln!(
                    "Loaded {} near sections. This replay omits the game loop, HUD, entities, sky and LOD.",
                    visible.len()
                );
                let animator = SpriteAnimator::new(&assets.animations, world.atlas());
                let started = Instant::now();
                window.request_redraw();
                Ok(State {
                    renderer: Some(renderer),
                    context: Some(context),
                    window,
                    scene,
                    world,
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
                        // SAFETY: state owns the context and the event loop stays on this thread.
                        if let Err(error) = unsafe { draw(state) } {
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
