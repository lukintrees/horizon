//! Full PR 8: a real OpenGL 2.1/2.0 game-loop frame.
//!
//! This module is compiled only for `--no-default-features --features legacy-gl`.
//! It owns the glutin/winit window, loads GL 2.x entry points with glow, links
//! the six `gfx21` GLSL 110/120 programs and draws terrain, entities, sky, HUD
//! and line overlays from `FullFramePlan::reference()`. All FFI is confined to
//! this module; `gfx21` stays pure Rust and the default wgpu build is unchanged.

use std::collections::BTreeMap;
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
use horizon_client_core::lang;
use horizon_client_render::gfx21::{
    Blend, BlendMode, EntityCamera, EntityPose, Frame, FullFramePlan, GOLDEN_HEIGHT, GOLDEN_WIDTH,
    HudBatch, LegacySettings, LineBatch, MAX_DRAWS, OverlayRect, Pass, ShaderDialect,
    UNAVAILABLE_METHODS, all_programs, classify_strings, unavailable_methods, validate_all,
};
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
const WHITE: [u8; 4] = [255, 255, 255, 255];

pub type RendererError = Box<dyn Error>;

#[derive(Clone, Copy, Debug)]
struct Quad {
    rect: [f32; 4],
    color: [f32; 4],
    depth: f32,
    blend: Blend,
}

fn rgba(color: [u8; 4]) -> [f32; 4] {
    [
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
        f32::from(color[3]) / 255.0,
    ]
}

fn rect_f32(rect: &OverlayRect) -> [f32; 4] {
    [
        rect.x0 as f32,
        rect.y0 as f32,
        rect.x1 as f32,
        rect.y1 as f32,
    ]
}

/// Four corners in triangle order, with Y inverted for NDC or left in
/// top-down pixel space for the HUD program's `screen` uniform.
fn corners(rect: [f32; 4], depth: f32, ndc: bool) -> [[f32; 3]; 4] {
    let (x0, x1, y0, y1) = if ndc {
        (
            rect[0] / GOLDEN_WIDTH as f32 * 2.0 - 1.0,
            rect[2] / GOLDEN_WIDTH as f32 * 2.0 - 1.0,
            1.0 - rect[1] / GOLDEN_HEIGHT as f32 * 2.0,
            1.0 - rect[3] / GOLDEN_HEIGHT as f32 * 2.0,
        )
    } else {
        (rect[0], rect[2], rect[1], rect[3])
    };
    [
        [x0, y1, depth],
        [x1, y1, depth],
        [x1, y0, depth],
        [x0, y0, depth],
    ]
}

const UVS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
const ORDER: [usize; 6] = [0, 1, 2, 0, 2, 3];

fn smoke_vertices() -> Vec<f32> {
    vec![-0.95, -0.95, -0.85, -0.95, -0.90, -0.80]
}

fn sky_vertices() -> Vec<f32> {
    let mut out = Vec::new();
    for at in ORDER {
        let p = corners([0.0, 0.0, 80.0, 60.0], 1.0, true)[at];
        out.extend_from_slice(&[p[0], p[1], p[2]]);
    }
    out
}

fn terrain_vertices(plan: &FullFramePlan) -> Vec<f32> {
    let mut out = Vec::new();
    for (index, quad) in plan.terrain.raster_quads.iter().enumerate() {
        let c = corners(
            [
                quad.x0 as f32,
                quad.y0 as f32,
                quad.x1 as f32,
                quad.y1 as f32,
            ],
            0.10 + index as f32 * 0.01,
            true,
        );
        let color = rgba(quad.color);
        for at in ORDER {
            out.extend_from_slice(&c[at]);
            out.extend_from_slice(&UVS[at]);
            out.extend_from_slice(&color);
            out.extend_from_slice(&[0.0, 0.0]);
        }
    }
    out
}

fn entity_vertices() -> Vec<f32> {
    let camera = EntityCamera::reference();
    let mut out = Vec::new();
    for (index, pose) in EntityPose::reference().iter().enumerate() {
        let screen = pose.screen_quad(&camera);
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for p in screen {
            min_x = min_x.min(p[0]);
            min_y = min_y.min(p[1]);
            max_x = max_x.max(p[0]);
            max_y = max_y.max(p[1]);
        }
        let c = corners(
            [min_x, min_y, max_x, max_y],
            0.05 + index as f32 * 0.01,
            true,
        );
        let color = rgba(pose.color);
        for at in ORDER {
            out.extend_from_slice(&c[at]);
            out.extend_from_slice(&UVS[at]);
            out.extend_from_slice(&color);
        }
    }
    out
}

fn hud_vertices() -> Vec<f32> {
    let mut out = Vec::new();
    for element in HudBatch::reference().elements {
        let c = corners(rect_f32(&element.rect), 0.0, false);
        let color = rgba(element.rect.color);
        for at in ORDER {
            out.extend_from_slice(&[c[at][0], c[at][1]]);
            out.extend_from_slice(&UVS[at]);
            out.extend_from_slice(&color);
        }
    }
    out
}

fn line_vertices() -> Vec<f32> {
    let mut out = Vec::new();
    for rect in LineBatch::reference().to_rects() {
        let c = corners(rect_f32(&rect), -0.9, true);
        let color = rgba(rect.color);
        for at in ORDER {
            out.extend_from_slice(&c[at]);
            out.extend_from_slice(&color);
        }
    }
    out
}

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
        .with_title(lang::get("horizon.legacyGl.windowTitle"))
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

fn upload(gl: &glow::Context, vertices: &[f32]) -> Result<glow::Buffer, String> {
    unsafe {
        let buffer = gl.create_buffer()?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(vertices),
            glow::STATIC_DRAW,
        );
        Ok(buffer)
    }
}

fn make_texture(gl: &glow::Context) -> Result<glow::Texture, String> {
    unsafe {
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            1,
            1,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&WHITE)),
        );
        Ok(texture)
    }
}

fn bind_attribs(
    gl: &glow::Context,
    program: glow::Program,
    specs: &[(&str, i32, i32, i32)],
) -> Vec<u32> {
    let mut enabled = Vec::new();
    unsafe {
        for (name, size, stride, offset) in specs {
            if let Some(index) = gl.get_attrib_location(program, name) {
                gl.enable_vertex_attrib_array(index);
                gl.vertex_attrib_pointer_f32(index, *size, glow::FLOAT, false, *stride, *offset);
                enabled.push(index);
            }
        }
    }
    enabled
}

fn disable_attribs(gl: &glow::Context, enabled: &[u32]) {
    unsafe {
        for index in enabled {
            gl.disable_vertex_attrib_array(*index);
        }
    }
}

fn set_blend(gl: &glow::Context, blend: Blend) {
    unsafe {
        match blend {
            Blend::Alpha => {
                gl.enable(glow::BLEND);
                gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                gl.depth_mask(false);
            }
            Blend::Opaque | Blend::Cutout => {
                gl.disable(glow::BLEND);
                gl.depth_mask(true);
            }
        }
    }
}

fn set_sampler(gl: &glow::Context, program: glow::Program, name: &str, unit: u32) {
    unsafe {
        let location = gl.get_uniform_location(program, name);
        gl.uniform_1_i32(location.as_ref(), unit as i32);
    }
}

struct App {
    template: ConfigTemplateBuilder,
    display_builder: DisplayBuilder,
    gl_context: Option<PossiblyCurrentContext>,
    gl_surface: Option<Surface<WindowSurface>>,
    gl: Option<glow::Context>,
    programs: Option<BTreeMap<Pass, glow::Program>>,
    vbos: Option<BTreeMap<Pass, glow::Buffer>>,
    textures: Option<Vec<glow::Texture>>,
    window: Option<Window>,
    width: u32,
    height: u32,
    settings: LegacySettings,
    dialect: Option<ShaderDialect>,
    frames_left: u32,
    frames_drawn: u32,
    error: Option<String>,
}

impl App {
    fn new(settings: &LegacySettings) -> Self {
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
            programs: None,
            vbos: None,
            textures: None,
            window: None,
            width: 800,
            height: 600,
            settings: settings.clone(),
            dialect: None,
            frames_left: 0,
            frames_drawn: 0,
            error: None,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, message: impl Into<String>) {
        self.error = Some(message.into());
        event_loop.exit();
    }

    fn setup(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let (window, gl_config) = match self.display_builder.clone().build(
            event_loop,
            self.template.clone(),
            gl_config_picker,
        ) {
            Ok((Some(window), config)) => (window, config),
            Ok((None, _)) => return Err("DisplayBuilder returned no window".to_owned()),
            Err(err) => return Err(err.to_string()),
        };

        let context = create_gl_context(&window, &gl_config)?.treat_as_possibly_current();
        self.gl_context = Some(context);
        let surface_attributes = window
            .build_surface_attributes(Default::default())
            .map_err(|err| err.to_string())?;
        let gl_surface = unsafe {
            gl_config
                .display()
                .create_window_surface(&gl_config, &surface_attributes)
        }
        .map_err(|err| err.to_string())?;
        self.gl_context
            .as_ref()
            .expect("context was just stored")
            .make_current(&gl_surface)
            .map_err(|err| err.to_string())?;
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
        let extensions = unsafe { gl.get_parameter_string(glow::EXTENSIONS) };
        println!("GL_VERSION: {version}");
        println!("GLSL_VERSION: {glsl}");
        println!("GL_RENDERER: {renderer}");
        println!("GL_VENDOR: {vendor}");

        let mut limits = BTreeMap::new();
        for (name, parameter) in [
            ("GL_MAX_TEXTURE_SIZE", glow::MAX_TEXTURE_SIZE),
            ("GL_MAX_TEXTURE_IMAGE_UNITS", glow::MAX_TEXTURE_IMAGE_UNITS),
            (
                "GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS",
                glow::MAX_COMBINED_TEXTURE_IMAGE_UNITS,
            ),
            ("GL_MAX_VERTEX_ATTRIBS", glow::MAX_VERTEX_ATTRIBS),
            (
                "GL_MAX_VERTEX_UNIFORM_COMPONENTS",
                glow::MAX_VERTEX_UNIFORM_COMPONENTS,
            ),
            ("GL_MAX_VARYING_FLOATS", glow::MAX_VARYING_FLOATS),
            ("GL_MAX_DRAW_BUFFERS", glow::MAX_DRAW_BUFFERS),
            (
                "GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS",
                glow::MAX_VERTEX_TEXTURE_IMAGE_UNITS,
            ),
        ] {
            let value = unsafe { gl.get_parameter_i32(parameter) };
            limits.insert(name.to_owned(), value.max(0) as u64);
        }
        let context = classify_strings(&version, &glsl, &extensions, limits)
            .map_err(|err| lang::tr("horizon.legacyGl.contextError", &[&err]))?;
        if context.dialect == ShaderDialect::FixedFunction {
            return Err(lang::get("horizon.legacyGl.unavailable"));
        }
        println!(
            "legacy-gl context: OK (tier {}, {:?})",
            context.report.tier.name(),
            context.dialect
        );

        validate_all().map_err(|err| err.to_string())?;
        println!("shader validation: OK");
        let dialect = requested_dialect(&glsl);
        println!("DIALECT: {dialect:?}");
        let mut programs = BTreeMap::new();
        let mut linked = 0_usize;
        for (pass, name, vertex_source, fragment_source) in all_programs(dialect) {
            let program = unsafe { make_program(&gl, vertex_source, fragment_source) }
                .map_err(|err| format!("GLSL link failed for {name}: {err}"))?;
            programs.insert(pass, program);
            linked += 1;
        }
        println!("all programs linked: OK ({linked})");

        let plan = FullFramePlan::reference();
        plan.validate().map_err(|err| err.to_string())?;
        let draws = plan.draw_call_count() + 1;
        if draws > MAX_DRAWS {
            return Err(format!("draw budget: {draws} > {}", MAX_DRAWS));
        }
        let mut vbos = BTreeMap::new();
        for (pass, vertices) in [
            (Pass::Smoke, smoke_vertices()),
            (Pass::Terrain, terrain_vertices(&plan)),
            (Pass::Entity, entity_vertices()),
            (Pass::Sky, sky_vertices()),
            (Pass::Hud, hud_vertices()),
            (Pass::Line, line_vertices()),
        ] {
            vbos.insert(pass, upload(&gl, &vertices)?);
        }
        let textures = vec![make_texture(&gl)?, make_texture(&gl)?];
        let error = unsafe { gl.get_error() };
        if error != glow::NO_ERROR {
            return Err(lang::tr(
                "horizon.legacyGl.glError",
                &[&format!("0x{error:x}")],
            ));
        }

        let size = window.inner_size();
        self.width = size.width.max(1);
        self.height = size.height.max(1);
        self.gl = Some(gl);
        self.programs = Some(programs);
        self.vbos = Some(vbos);
        self.textures = Some(textures);
        self.dialect = Some(dialect);
        self.window = Some(window);
        println!(
            "legacy-gl startup: OK ({draws}/{max_draws} draws)",
            max_draws = MAX_DRAWS
        );
        Ok(())
    }

    fn draw(&mut self) -> Result<(), String> {
        let Some(gl) = self.gl.as_ref() else {
            return Ok(());
        };
        let Some(programs) = self.programs.as_ref() else {
            return Ok(());
        };
        let Some(vbos) = self.vbos.as_ref() else {
            return Ok(());
        };
        let Some(textures) = self.textures.as_ref() else {
            return Ok(());
        };
        let Some(surface) = self.gl_surface.as_ref() else {
            return Ok(());
        };
        let Some(context) = self.gl_context.as_ref() else {
            return Ok(());
        };
        let width = self.width as i32;
        let height = self.height as i32;
        let plan = FullFramePlan::reference();

        unsafe {
            gl.viewport(0, 0, width, height);
            let bottom = rgba([
                plan.sky.bottom[0],
                plan.sky.bottom[1],
                plan.sky.bottom[2],
                255,
            ]);
            gl.clear_color(bottom[0], bottom[1], bottom[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.disable(glow::CULL_FACE);
        }

        // Sky: one full-screen gradient quad, no depth writes.
        {
            let program = programs[&Pass::Sky];
            let vbo = vbos[&Pass::Sky];
            unsafe {
                gl.disable(glow::DEPTH_TEST);
                gl.disable(glow::BLEND);
                gl.depth_mask(false);
                gl.use_program(Some(program));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                let enabled = bind_attribs(gl, program, &[("position", 3, 12, 0)]);
                let top = rgba([plan.sky.top[0], plan.sky.top[1], plan.sky.top[2], 255]);
                let bottom = rgba([
                    plan.sky.bottom[0],
                    plan.sky.bottom[1],
                    plan.sky.bottom[2],
                    255,
                ]);
                let top_loc = gl.get_uniform_location(program, "sky_top");
                let bottom_loc = gl.get_uniform_location(program, "sky_bottom");
                gl.uniform_3_f32(top_loc.as_ref(), top[0], top[1], top[2]);
                gl.uniform_3_f32(bottom_loc.as_ref(), bottom[0], bottom[1], bottom[2]);
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
                disable_attribs(gl, &enabled);
            }
        }

        // Smoke: the minimal link/draw probe, still using all six programs.
        {
            let program = programs[&Pass::Smoke];
            let vbo = vbos[&Pass::Smoke];
            unsafe {
                gl.disable(glow::DEPTH_TEST);
                gl.disable(glow::BLEND);
                gl.depth_mask(false);
                gl.use_program(Some(program));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                let enabled = bind_attribs(gl, program, &[("position", 2, 8, 0)]);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
                disable_attribs(gl, &enabled);
            }
        }

        // Terrain: five section quads with the GL2 atlas/lightmap/fog uniforms.
        {
            let program = programs[&Pass::Terrain];
            let vbo = vbos[&Pass::Terrain];
            unsafe {
                gl.enable(glow::DEPTH_TEST);
                gl.use_program(Some(program));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                let enabled = bind_attribs(
                    gl,
                    program,
                    &[
                        ("position", 3, 44, 0),
                        ("uv", 2, 44, 12),
                        ("color", 4, 44, 20),
                        ("light_uv", 2, 44, 36),
                    ],
                );
                let view = gl.get_uniform_location(program, "view_proj");
                let origin = gl.get_uniform_location(program, "origin");
                let fog_color = gl.get_uniform_location(program, "fog_color");
                let fog_amount = gl.get_uniform_location(program, "fog_amount");
                gl.uniform_matrix_4_f32_slice(view.as_ref(), false, &IDENTITY);
                gl.uniform_3_f32(origin.as_ref(), 0.0, 0.0, 0.0);
                let fog = rgba([
                    plan.terrain.fog_color[0],
                    plan.terrain.fog_color[1],
                    plan.terrain.fog_color[2],
                    255,
                ]);
                gl.uniform_4_f32(fog_color.as_ref(), fog[0], fog[1], fog[2], 0.0);
                gl.uniform_1_f32(fog_amount.as_ref(), 0.0);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(textures[0]));
                gl.active_texture(glow::TEXTURE0 + 1);
                gl.bind_texture(glow::TEXTURE_2D, Some(textures[1]));
                set_sampler(gl, program, "atlas", 0);
                set_sampler(gl, program, "lightmap", 1);
                for (index, quad) in plan.terrain.raster_quads.iter().enumerate() {
                    let blend = match quad.blend {
                        Blend::Alpha => Blend::Alpha,
                        Blend::Opaque | Blend::Cutout => Blend::Opaque,
                    };
                    set_blend(gl, blend);
                    gl.draw_arrays(glow::TRIANGLES, (index * 6) as i32, 6);
                }
                disable_attribs(gl, &enabled);
            }
        }

        // Entities: two CPU-posed billboards.
        {
            let program = programs[&Pass::Entity];
            let vbo = vbos[&Pass::Entity];
            unsafe {
                gl.enable(glow::DEPTH_TEST);
                gl.disable(glow::BLEND);
                gl.depth_mask(false);
                gl.use_program(Some(program));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                let enabled = bind_attribs(
                    gl,
                    program,
                    &[
                        ("position", 3, 36, 0),
                        ("uv", 2, 36, 12),
                        ("color", 4, 36, 20),
                    ],
                );
                let view = gl.get_uniform_location(program, "model_view_proj");
                gl.uniform_matrix_4_f32_slice(view.as_ref(), false, &IDENTITY);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(textures[0]));
                set_sampler(gl, program, "atlas", 0);
                for index in 0..2 {
                    gl.draw_arrays(glow::TRIANGLES, (index * 6) as i32, 6);
                }
                disable_attribs(gl, &enabled);
            }
        }

        // HUD: four screen rectangles in the 80x60 reference space.
        {
            let program = programs[&Pass::Hud];
            let vbo = vbos[&Pass::Hud];
            unsafe {
                gl.disable(glow::DEPTH_TEST);
                gl.use_program(Some(program));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                let enabled = bind_attribs(
                    gl,
                    program,
                    &[
                        ("position", 2, 32, 0),
                        ("uv", 2, 32, 8),
                        ("color", 4, 32, 16),
                    ],
                );
                let screen = gl.get_uniform_location(program, "screen");
                gl.uniform_2_f32(screen.as_ref(), GOLDEN_WIDTH as f32, GOLDEN_HEIGHT as f32);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(textures[0]));
                set_sampler(gl, program, "atlas", 0);
                for (index, element) in HudBatch::reference().elements.iter().enumerate() {
                    let blend = if matches!(element.blend, BlendMode::Opaque | BlendMode::Cutout) {
                        Blend::Opaque
                    } else {
                        Blend::Alpha
                    };
                    set_blend(gl, blend);
                    gl.draw_arrays(glow::TRIANGLES, (index * 6) as i32, 6);
                }
                disable_attribs(gl, &enabled);
            }
        }

        // Lines: crosshair/selection rectangles, no depth writes.
        {
            let program = programs[&Pass::Line];
            let vbo = vbos[&Pass::Line];
            unsafe {
                gl.disable(glow::DEPTH_TEST);
                gl.disable(glow::BLEND);
                gl.depth_mask(false);
                gl.use_program(Some(program));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                let enabled =
                    bind_attribs(gl, program, &[("position", 3, 28, 0), ("color", 4, 28, 12)]);
                let view = gl.get_uniform_location(program, "view_proj");
                gl.uniform_matrix_4_f32_slice(view.as_ref(), false, &IDENTITY);
                for index in 0..LineBatch::reference().to_rects().len() {
                    gl.draw_arrays(glow::TRIANGLES, (index * 6) as i32, 6);
                }
                disable_attribs(gl, &enabled);
            }
        }

        let error = unsafe { gl.get_error() };
        if error != glow::NO_ERROR {
            return Err(lang::tr(
                "horizon.legacyGl.glError",
                &[&format!("0x{error:x}")],
            ));
        }
        surface
            .swap_buffers(context)
            .map_err(|err| err.to_string())?;
        println!("legacy-gl frame: OK; GL_NO_ERROR");
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() || self.window.is_some() {
            return;
        }
        if let Err(err) = self.setup(event_loop) {
            self.fail(event_loop, err);
        } else if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.width = size.width.max(1);
                self.height = size.height.max(1);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(err) = self.draw() {
                    self.fail(event_loop, err);
                    return;
                }
                self.frames_drawn += 1;
                self.frames_left = self.frames_left.saturating_sub(1);
                if self.frames_left == 0 {
                    event_loop.exit();
                } else if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() || self.frames_left == 0 {
            event_loop.exit();
        } else if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

/// The real legacy renderer. `Frame` stays the zero-sized gfx21 marker; the
/// GL backend draws the deterministic full-frame reference plan.
pub struct Renderer {
    event_loop: EventLoop<()>,
    app: App,
}

pub type RenderFrame = Frame;

impl Renderer {
    pub fn new(settings: &LegacySettings) -> Result<Self, RendererError> {
        let event_loop = EventLoop::new().map_err(|err| io::Error::other(err.to_string()))?;
        Ok(Self {
            event_loop,
            app: App::new(settings),
        })
    }

    pub fn run(&mut self, frames: u32) -> Result<(), RendererError> {
        self.app.frames_left = frames.max(1);
        self.app.frames_drawn = 0;
        self.event_loop
            .run_app(&mut self.app)
            .map_err(|err| io::Error::other(err.to_string()))?;
        if let Some(message) = &self.app.error {
            return Err(io::Error::other(message.clone()).into());
        }
        let drawn = self.app.frames_drawn;
        let methods = unavailable_methods().len();
        println!("{}", lang::get("horizon.legacyGl.startup"));
        println!("{}", lang::tr("horizon.legacyGl.frameDrawn", &[&drawn]));
        println!(
            "{}",
            lang::tr("horizon.legacyGl.unavailableMethods", &[&methods])
        );
        println!(
            "legacy-gl frame: OK ({drawn} frames, {draws}/{max_draws} draws, {methods_count} unavailable methods, GL_NO_ERROR)",
            draws = FullFramePlan::reference().draw_call_count() + 1,
            max_draws = MAX_DRAWS,
            methods_count = UNAVAILABLE_METHODS.len()
        );
        Ok(())
    }

    pub fn render(&mut self, _frame: &RenderFrame) -> Result<bool, RendererError> {
        self.run(1)?;
        Ok(false)
    }

    pub fn settings(&self) -> &LegacySettings {
        &self.app.settings
    }

    pub fn dialect(&self) -> Option<ShaderDialect> {
        self.app.dialect
    }
}
