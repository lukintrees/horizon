//! Developer-only window/context smoke test. This is not the game renderer.
//! Run on Linux/X11 or Windows with `--features legacy-gl-window`; `--smoke`
//! checks a framebuffer pixel and exits after one frame without player data.
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod app {
    use horizon_legacy_gl::{Batch, Page, Pass, Renderer, Vertex, window::WindowContext};
    use std::{
        ffi::{CStr, c_char, c_void},
        sync::Arc,
    };
    use winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop},
        raw_window_handle::{HasWindowHandle, RawWindowHandle},
        window::{Window, WindowId},
    };

    #[cfg(target_os = "linux")]
    use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

    struct State {
        // Explicit teardown below ensures GL resources and context die before the window.
        renderer: Option<Renderer>,
        context: Option<WindowContext>,
        window: Arc<Window>,
    }
    impl Drop for State {
        fn drop(&mut self) {
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

    unsafe fn init_context(window: &Window) -> Result<WindowContext, String> {
        let handle = window.window_handle().map_err(|e| e.to_string())?;
        #[cfg(target_os = "linux")]
        {
            let display = window.display_handle().map_err(|e| e.to_string())?;
            match (display.as_raw(), handle.as_raw()) {
                (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window)) => {
                    let display = display.display.ok_or("Xlib display is null")?.as_ptr();
                    unsafe {
                        WindowContext::new_xlib(display, window.window)
                            .map_err(|e| format!("{e:?}"))
                    }
                }
                _ => Err("this developer test requires X11/Xlib, not Wayland".into()),
            }
        }
        #[cfg(target_os = "windows")]
        {
            match handle.as_raw() {
                RawWindowHandle::Win32(window) => unsafe {
                    WindowContext::new_win32(window.hwnd.get() as *mut c_void)
                        .map_err(|e| format!("{e:?}"))
                },
                _ => Err("this developer test requires a Win32 window".into()),
            }
        }
    }

    unsafe fn gl_proc(context: &WindowContext, name: &CStr) -> Result<*const c_void, String> {
        let address = unsafe { context.resolve(name) };
        if address.is_null() || address as usize <= 3 || address as usize == usize::MAX {
            Err(format!("missing {}", name.to_string_lossy()))
        } else {
            Ok(address)
        }
    }

    unsafe fn draw(state: &mut State, smoke: bool) -> Result<(), String> {
        let context = state.context.as_ref().ok_or("no GL context")?;
        let renderer = state.renderer.as_mut().ok_or("no renderer")?;
        unsafe {
            context.make_current().map_err(|e| format!("{e:?}"))?;
            let viewport = std::mem::transmute::<
                *const c_void,
                unsafe extern "system" fn(i32, i32, i32, i32),
            >(gl_proc(context, c"glViewport")?);
            let clear_color = std::mem::transmute::<
                *const c_void,
                unsafe extern "system" fn(f32, f32, f32, f32),
            >(gl_proc(context, c"glClearColor")?);
            let clear = std::mem::transmute::<*const c_void, unsafe extern "system" fn(u32)>(
                gl_proc(context, c"glClear")?,
            );
            let read = std::mem::transmute::<
                *const c_void,
                unsafe extern "system" fn(i32, i32, i32, i32, u32, u32, *mut c_void),
            >(gl_proc(context, c"glReadPixels")?);
            let size = state.window.inner_size();
            if size.width == 0 || size.height == 0 {
                return Ok(());
            }
            viewport(0, 0, size.width as i32, size.height as i32);
            clear_color(0., 0., 0., 1.);
            clear(0x4000 | 0x0100);
            let vertex = |position| Vertex {
                position,
                sprite_uv: [0.5, 0.5],
                atlas_rect: [0., 0., 1., 1.],
                color: [1.; 4],
            };
            let vertices = [
                vertex([-1., -1., 0.]),
                vertex([1., -1., 0.]),
                vertex([0., 1., 0.]),
            ];
            let identity = [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ];
            renderer
                .draw(
                    &vertices,
                    &[Batch {
                        page: 0,
                        first: 0,
                        count: 3,
                    }],
                    &identity,
                    Pass::Solid { alpha_cutoff: 0.1 },
                )
                .map_err(|e| format!("{e:?}"))?;
            if smoke {
                let get_string = std::mem::transmute::<
                    *const c_void,
                    unsafe extern "system" fn(u32) -> *const c_char,
                >(gl_proc(context, c"glGetString")?);
                let get_error = std::mem::transmute::<
                    *const c_void,
                    unsafe extern "system" fn() -> u32,
                >(gl_proc(context, c"glGetError")?);
                let string = |id| {
                    let value = get_string(id);
                    if value.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr(value).to_string_lossy().into_owned()
                    }
                };
                let renderer_name = string(0x1f01);
                let version = string(0x1f02);
                if std::env::var("LIBGL_ALWAYS_SOFTWARE").as_deref() == Ok("1")
                    && !renderer_name.to_lowercase().contains("llvmpipe")
                {
                    return Err(format!("expected llvmpipe, got {renderer_name}"));
                }
                if let Ok(expected) = std::env::var("MESA_GL_VERSION_OVERRIDE")
                    && !version.starts_with(&format!("{expected} "))
                {
                    return Err(format!("expected GL {expected}, got {version}"));
                }
                let mut pixel = [0u8; 4];
                read(
                    (size.width / 2) as i32,
                    (size.height / 2) as i32,
                    1,
                    1,
                    0x1908,
                    0x1401,
                    pixel.as_mut_ptr().cast(),
                );
                if pixel[..3]
                    .iter()
                    .zip([200u8, 40, 20])
                    .any(|(actual, expected)| actual.abs_diff(expected) > 2)
                {
                    return Err(format!("unexpected center pixel: {pixel:?}"));
                }
                let error = get_error();
                if error != 0 {
                    return Err(format!("GL error after readback: {error:#x}"));
                }
                println!(
                    "PASS: {renderer_name}, GL {version}, native window, framebuffer {pixel:?}"
                );
            }
            context.swap_buffers().map_err(|e| format!("{e:?}"))?;
            Ok(())
        }
    }

    struct App {
        state: Option<State>,
        error: Option<String>,
        smoke: bool,
    }
    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.state.is_some() {
                return;
            }
            let result = (|| -> Result<State, String> {
                let window = Arc::new(
                    event_loop
                        .create_window(
                            Window::default_attributes()
                                .with_title("Horizon legacy GL developer test")
                                .with_inner_size(winit::dpi::LogicalSize::new(320.0, 240.0)),
                        )
                        .map_err(|e| e.to_string())?,
                );
                // SAFETY: window is kept alive by State, and both the event loop and
                // the context are confined to this thread.
                let context = unsafe { init_context(&window)? };
                let mut renderer = unsafe { Renderer::new(|name| context.resolve(name)) }
                    .map_err(|e| format!("{e:?}"))?;
                let texels = [200, 40, 20, 255].repeat(4);
                if let Err(error) = unsafe {
                    renderer.upload_pages(&[Page {
                        side: 2,
                        pixels: &texels,
                    }])
                } {
                    unsafe { renderer.destroy() };
                    return Err(format!("{error:?}"));
                }
                window.request_redraw();
                Ok(State {
                    renderer: Some(renderer),
                    context: Some(context),
                    window,
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
        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            _id: WindowId,
            event: WindowEvent,
        ) {
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
                        if let Err(error) = unsafe { draw(state, self.smoke) } {
                            self.error = Some(error);
                        }
                        if self.smoke || self.error.is_some() {
                            self.state.take();
                            event_loop.exit();
                        }
                    }
                }
                _ => {}
            }
        }
        fn exiting(&mut self, _: &ActiveEventLoop) {
            self.state.take();
        }
    }
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let event_loop = EventLoop::new()?;
        let mut app = App {
            state: None,
            error: None,
            smoke: std::env::args().any(|arg| arg == "--smoke"),
        };
        event_loop.run_app(&mut app)?;
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
    eprintln!("Legacy GL window test is implemented for X11 and Windows only");
}
