//! The game window on Windows: a winit window with a WGL context attached to it, and the frame loop.
//! Gate G1 runs in it: each redraw draws the frame, checks it, and presents it.

use glow::HasContext;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::frame;
use crate::report::Report;
use crate::version;

/// The client area asked for at start, in physical pixels. Every size works; the quad needs 160x160.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

pub struct Options {
    /// Frames to present before the run ends; 0 keeps the window open until it is closed.
    pub frames: u32,
    /// Where to write the JSON report, besides the one `run` returns.
    pub out: Option<String>,
}

/// Runs the window. Returns the exit status and the JSON report. The status is 0 when every required
/// check passed, 1 when a check failed, and 2 when there was no window, no OpenGL 2.1 context, or
/// the frame could not run.
pub fn run(options: Options) -> (i32, String) {
    let mut app = App { options, report: Report::default(), running: None };
    match EventLoop::new() {
        Ok(event_loop) => {
            if let Err(e) = event_loop.run_app(&mut app) {
                app.report.error.get_or_insert(format!("event loop: {e}"));
            }
        }
        Err(e) => app.report.error = Some(format!("event loop: {e}")),
    }
    app.finish()
}

struct App {
    options: Options,
    report: Report,
    running: Option<Running>,
}

/// What exists once the window and its context do. Fields drop in this order: the GL objects go
/// while the context is still alive, the context before the window it is attached to.
struct Running {
    renderer: frame::Renderer,
    gl: glow::Context,
    context: horizon_gl2::wgl::Context,
    window: Window,
}

impl App {
    fn finish(mut self) -> (i32, String) {
        if self.report.error.is_none() && self.report.frames == 0 {
            self.report.error = Some("the window closed before its first frame".into());
        }
        let status = if self.report.passed() {
            0
        } else if self.report.error.is_some() {
            2
        } else {
            1
        };
        let json = self.report.to_json();
        if let Some(path) = &self.options.out {
            let parent = std::path::Path::new(path).parent().unwrap_or(std::path::Path::new("."));
            let written = std::fs::create_dir_all(parent).and_then(|()| std::fs::write(path, &json));
            if let Err(e) = written {
                eprintln!("cannot write {path}: {e}");
            }
        }
        (status, json)
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(running) = self.running.as_mut() else { return };
        let size = running.window.inner_size();
        if size.width == 0 || size.height == 0 {
            // Minimised: there is nothing to draw into until the window has a size again.
            return;
        }
        match running.renderer.draw(&running.gl, size.width, size.height) {
            Ok(pixels) => {
                self.report.checks = frame::checks(&pixels);
                self.report.window = Some((size.width, size.height));
                if !running.context.swap() {
                    self.report.error = Some("SwapBuffers refused the swap".into());
                    event_loop.exit();
                    return;
                }
                self.report.frames += 1;
                if self.options.frames > 0 && self.report.frames >= self.options.frames {
                    event_loop.exit();
                }
            }
            Err(e) => {
                self.report.error = Some(e);
                event_loop.exit();
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() || self.report.error.is_some() {
            return;
        }
        match start(event_loop, &mut self.report) {
            Ok(running) => {
                running.window.request_redraw();
                self.running = Some(running);
            }
            Err(e) => {
                self.report.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.draw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
}

/// Creates the window, attaches the context and checks that the driver can run the frame.
fn start(event_loop: &ActiveEventLoop, report: &mut Report) -> Result<Running, String> {
    let attributes = Window::default_attributes()
        .with_title("Horizon (OpenGL 2.1)")
        .with_inner_size(PhysicalSize::new(WIDTH, HEIGHT));
    let window = event_loop
        .create_window(attributes)
        .map_err(|e| format!("cannot create the window: {e}"))?;
    let handle = window.window_handle().map_err(|e| format!("no window handle: {e}"))?;
    let hwnd = match handle.as_raw() {
        RawWindowHandle::Win32(win32) => win32.hwnd.get(),
        _ => return Err("the window is not a Win32 window".into()),
    };
    // SAFETY: `hwnd` is the live window winit made above, and `Running` drops the context before it.
    let context = unsafe { horizon_gl2::wgl::Context::attach(hwnd) }
        .map_err(|e| format!("no OpenGL context on the window: {e}"))?;
    let gl = context.glow();
    // SAFETY: the context made by `attach` is current on this thread.
    let (version_text, renderer_name) = unsafe {
        (gl.get_parameter_string(glow::VERSION), gl.get_parameter_string(glow::RENDERER))
    };
    report.gl_version = Some(version_text.clone());
    report.renderer = Some(renderer_name);
    if !version::runs_frame(&version_text) {
        let (major, minor) = version::FRAME_MINIMUM;
        return Err(format!(
            "the frame needs OpenGL {major}.{minor} (GLSL 1.20); this driver gives '{version_text}'"
        ));
    }
    let renderer = frame::Renderer::new(&gl)?;
    Ok(Running { renderer, gl, context, window })
}
