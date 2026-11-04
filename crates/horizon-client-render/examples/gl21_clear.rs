//! PR 3 stage 2: a real OpenGL 2.1/2.0 compatibility context and one visible
//! triangle, followed by PR 4's full GLSL 120/110 pass program set.
//!
//! Built only with `--features legacy-gl`; the default wgpu build is unchanged.
//! Under CI it runs in Xvfb with Mesa version overrides. This is a standalone
//! example crate, so the library's `#![forbid(unsafe_code)]` does not apply to
//! the FFI loader and GL calls below.

use std::error::Error;
use std::ffi::{CString, c_void};
use std::io;

use glow::HasContext;
use glutin::config::{Api, Config, ConfigTemplateBuilder};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, NotCurrentContext, PossiblyCurrentContext, Version,
};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::prelude::*;
use glutin::surface::{Surface, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use horizon_client_render::gfx21::{Pass, ShaderDialect, program};
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

const VERTICES: [f32; 6] = [-0.6, -0.4, 0.6, -0.4, 0.0, 0.6];

fn requested_gl_version() -> Version {
    let text = std::env::var("MESA_GL_VERSION_OVERRIDE").unwrap_or_else(|_| "2.1".to_owned());
    let mut parts = text.split('.');
    match (
        parts.next().and_then(|s| s.parse::<u8>().ok()),
        parts.next().and_then(|s| s.parse::<u8>().ok()),
    ) {
        (Some(major), Some(minor)) => Version::new(major, minor),
        _ => Version::new(2, 1),
    }
}

fn dialect_from_glsl(text: &str) -> ShaderDialect {
    if text.contains("1.10") {
        ShaderDialect::Glsl110
    } else {
        ShaderDialect::Glsl120
    }
}

fn requested_dialect(glsl: &str) -> ShaderDialect {
    match std::env::var("HORIZON_GLSL_DIALECT").as_deref() {
        Ok("110") | Ok("1.10") | Ok("Glsl110") => ShaderDialect::Glsl110,
        Ok("120") | Ok("1.20") | Ok("Glsl120") => ShaderDialect::Glsl120,
        _ => dialect_from_glsl(glsl),
    }
}

fn gl_config_picker(mut configs: Box<dyn Iterator<Item = Config> + '_>) -> Config {
    configs.next().expect("no OpenGL 2.x config available")
}

fn window_attributes() -> WindowAttributes {
    WindowAttributes::default()
        .with_title("horizon legacy GL 2.1 clear")
        .with_inner_size(LogicalSize::new(800.0, 600.0))
}

fn create_gl_context(window: &Window, gl_config: &Config) -> Result<NotCurrentContext, String> {
    let raw_window_handle = window.window_handle().ok().map(|handle| handle.as_raw());
    let attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(requested_gl_version())))
        .build(raw_window_handle);
    let gl_display = gl_config.display();
    unsafe { gl_display.create_context(gl_config, &attributes) }.map_err(|err| err.to_string())
}

/// # Safety
/// A current GL 2.x compatibility context must be active and `gl` must have
/// been loaded from it; all glow calls are FFI.
unsafe fn make_program(
    gl: &glow::Context,
    vertex_source: &str,
    fragment_source: &str,
) -> Result<glow::Program, String> {
    unsafe {
        let vertex = gl.create_shader(glow::VERTEX_SHADER)?;
        gl.shader_source(vertex, vertex_source);
        gl.compile_shader(vertex);
        if !gl.get_shader_compile_status(vertex) {
            return Err(gl.get_shader_info_log(vertex));
        }
        let fragment = gl.create_shader(glow::FRAGMENT_SHADER)?;
        gl.shader_source(fragment, fragment_source);
        gl.compile_shader(fragment);
        if !gl.get_shader_compile_status(fragment) {
            return Err(gl.get_shader_info_log(fragment));
        }
        let program = gl.create_program()?;
        gl.attach_shader(program, vertex);
        gl.attach_shader(program, fragment);
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(gl.get_program_info_log(program));
        }
        gl.delete_shader(vertex);
        gl.delete_shader(fragment);
        Ok(program)
    }
}

struct App {
    template: ConfigTemplateBuilder,
    display_builder: DisplayBuilder,
    gl_context: Option<PossiblyCurrentContext>,
    gl_surface: Option<Surface<WindowSurface>>,
    gl: Option<glow::Context>,
    program: Option<glow::Program>,
    vbo: Option<glow::Buffer>,
    attrib: Option<u32>,
    window: Option<Window>,
    drawn: bool,
    error: Option<String>,
}

impl App {
    fn new() -> Self {
        Self {
            template: ConfigTemplateBuilder::new()
                .with_api(Api::OPENGL)
                .with_alpha_size(8)
                .with_depth_size(24)
                .with_stencil_size(8),
            display_builder: DisplayBuilder::new()
                .with_window_attributes(Some(window_attributes())),
            gl_context: None,
            gl_surface: None,
            gl: None,
            program: None,
            vbo: None,
            attrib: None,
            window: None,
            drawn: false,
            error: None,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, message: impl Into<String>) {
        self.error = Some(message.into());
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() || self.window.is_some() {
            return;
        }
        let (window, gl_config) = match self.display_builder.clone().build(
            event_loop,
            self.template.clone(),
            gl_config_picker,
        ) {
            Ok((Some(window), config)) => (window, config),
            Ok((None, _)) => return self.fail(event_loop, "DisplayBuilder returned no window"),
            Err(err) => return self.fail(event_loop, err.to_string()),
        };

        let context = match create_gl_context(&window, &gl_config) {
            Ok(context) => context.treat_as_possibly_current(),
            Err(err) => return self.fail(event_loop, err),
        };
        self.gl_context = Some(context);

        let surface_attributes = match window.build_surface_attributes(Default::default()) {
            Ok(attributes) => attributes,
            Err(err) => return self.fail(event_loop, err.to_string()),
        };
        let gl_surface = match unsafe {
            gl_config
                .display()
                .create_window_surface(&gl_config, &surface_attributes)
        } {
            Ok(surface) => surface,
            Err(err) => return self.fail(event_loop, err.to_string()),
        };
        if let Err(err) = self
            .gl_context
            .as_ref()
            .expect("context was just stored")
            .make_current(&gl_surface)
        {
            return self.fail(event_loop, err.to_string());
        }
        self.gl_surface = Some(gl_surface);

        let gl_display = gl_config.display();
        let gl = unsafe {
            glow::Context::from_loader_function(|symbol| {
                let symbol = CString::new(symbol).unwrap();
                gl_display
                    .get_proc_address(symbol.as_c_str())
                    .cast::<c_void>()
            })
        };

        let version = unsafe { gl.get_parameter_string(glow::VERSION) };
        let glsl = unsafe { gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION) };
        let renderer = unsafe { gl.get_parameter_string(glow::RENDERER) };
        let vendor = unsafe { gl.get_parameter_string(glow::VENDOR) };
        println!("GL_VERSION: {version}");
        println!("GLSL_VERSION: {glsl}");
        println!("GL_RENDERER: {renderer}");
        println!("GL_VENDOR: {vendor}");

        if let Err(err) = program::validate_all() {
            return self.fail(event_loop, format!("shader validation failed: {err}"));
        }
        println!("shader validation: OK");

        let dialect = requested_dialect(&glsl);
        println!("DIALECT: {dialect:?}");
        let mut smoke: Option<glow::Program> = None;
        let mut linked = 0_usize;
        for (pass, name, vertex_source, fragment_source) in program::all_programs(dialect) {
            let built = match unsafe { make_program(&gl, vertex_source, fragment_source) } {
                Ok(built) => built,
                Err(err) => {
                    return self.fail(event_loop, format!("GLSL link failed for {name}: {err}"));
                }
            };
            if pass == Pass::Smoke {
                smoke = Some(built);
            } else {
                unsafe { gl.delete_program(built) };
            }
            linked += 1;
        }
        println!("all programs linked: OK ({linked})");
        let program = match smoke {
            Some(program) => program,
            None => {
                return self.fail(
                    event_loop,
                    "smoke program missing from the full program set",
                );
            }
        };
        let vbo = match unsafe { gl.create_buffer() } {
            Ok(buffer) => buffer,
            Err(err) => return self.fail(event_loop, err),
        };
        let attrib = match unsafe { gl.get_attrib_location(program, "position") } {
            Some(index) => index,
            None => return self.fail(event_loop, "missing `position` attribute"),
        };
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&VERTICES),
                glow::STATIC_DRAW,
            );
            let _ = gl.get_error();
        }

        self.program = Some(program);
        self.vbo = Some(vbo);
        self.attrib = Some(attrib);
        self.gl = Some(gl);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if let WindowEvent::CloseRequested = event {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() {
            event_loop.exit();
            return;
        }
        if self.drawn {
            return;
        }
        let Some(gl) = self.gl.as_ref() else {
            return;
        };
        let Some(program) = self.program else {
            return;
        };
        let Some(vbo) = self.vbo else {
            return;
        };
        let Some(attrib) = self.attrib else {
            return;
        };
        let Some(surface) = self.gl_surface.as_ref() else {
            return;
        };
        let Some(context) = self.gl_context.as_ref() else {
            return;
        };
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();

        let mut draw_error: Option<String> = None;
        unsafe {
            gl.viewport(0, 0, size.width as i32, size.height as i32);
            gl.clear_color(0.05, 0.08, 0.12, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.use_program(Some(program));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.vertex_attrib_pointer_f32(attrib, 2, glow::FLOAT, false, 8, 0);
            gl.enable_vertex_attrib_array(attrib);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            let error = gl.get_error();
            if error != glow::NO_ERROR {
                draw_error = Some(format!("glGetError: 0x{error:x}"));
            }
        }
        if let Err(err) = surface.swap_buffers(context) {
            draw_error.get_or_insert(err.to_string());
        }
        if let Some(err) = draw_error {
            self.error = Some(err);
        } else {
            println!("gl21_clear: OK; GL_NO_ERROR");
        }
        self.drawn = true;
        event_loop.exit();
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    let mut app = App::new();
    event_loop.run_app(&mut app)?;
    if let Some(message) = app.error {
        return Err(io::Error::other(message).into());
    }
    Ok(())
}
