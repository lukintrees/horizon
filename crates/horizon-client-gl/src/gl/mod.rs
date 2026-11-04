//! The GL 2.1 backend: programs, buffers, textures, targets, and the draw list executor.
//!
//! What it is not: an abstraction over graphics APIs. There is one API here, the one the
//! target has, and every module is written to its limits rather than to a common denominator.
//! Where a limit changes the design, the module says so at the top (no sampler objects, no
//! uniform blocks, no `glMapBufferRange` on 2.1, no instancing).

pub mod buffer;
pub mod program;
pub mod texture;

pub use buffer::{Attribute, AttributeType, Buffer, BufferUsage, VertexFormat};
pub use texture::{SpriteRect, Texture};
pub use program::{attribs, compose_source, Program, Purpose, Sources, UniformSlot};

use glow::HasContext;

use crate::caps::{Caps, Feature, Limits, Version};
use crate::error::{Error, Result};
use crate::loader::GlLibrary;

/// A live GL 2.1 context: the function table, what it reported, and the counters the log and
/// the tests read.
///
/// Created by [`Context::new`] from an already-current context (the platform layer makes it
/// current: WGL on Windows, GLX on Linux, in `horizon-client-render`, which owns the window).
/// Everything else in this crate takes `&Context` and never a window.
pub struct Context {
    gl: glow::Context,
    caps: Caps,
    /// Bytes uploaded since the renderer started: the 2.1 path uploads far more than the
    /// wgpu one (no persistent maps), so it is worth watching.
    uploaded: std::sync::atomic::AtomicU64,
    /// Programs created and destroyed, for the leak check at the end of a run.
    programs_created: std::sync::atomic::AtomicU32,
    programs_destroyed: std::sync::atomic::AtomicU32,
    library: GlLibrary,
    /// `HORIZON_GL21_TRACE`: log every pass and its draw count.
    trace: bool,
}

/// How to build a context: read from the environment, so a player's build and CI behave the
/// same way without a settings screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// `HORIZON_GL21_STRICT=1`: the capability table is the target's, whatever the driver
    /// says. Always on in the comparison runs (docs/gl21-x1650-plan.ru.md §4).
    pub strict: bool,
    /// `HORIZON_GL21_TRACE=1`: log passes and draws.
    pub trace: bool,
}

impl Options {
    pub fn from_env() -> Self {
        Self {
            strict: env_flag("HORIZON_GL21_STRICT"),
            trace: env_flag("HORIZON_GL21_TRACE"),
        }
    }
}

fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let v = v.trim();
            !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off"))
        }
        Err(_) => false,
    }
}

impl Context {
    /// Reads the driver's identity, extension list and limits from the current context and
    /// checks the renderer's minimum. Must be called with the context current on this thread.
    ///
    /// # Safety
    /// The caller must have made a GL context current on the calling thread and must keep
    /// `library` alive for longer than the returned context: glow copies the function
    /// pointers out of it and calls them on every frame.
    pub unsafe fn new(library: GlLibrary, options: Options) -> Result<Self> {
        let missing = library.missing_essentials();
        if !missing.is_empty() {
            return Err(Error::unsupported(format!(
                "this GPU driver lacks {} OpenGL function(s) the renderer needs ({}): \
                 install the latest driver for your graphics card",
                missing.len(),
                missing.join(", ")
            )));
        }
        let gl = unsafe { glow::Context::from_loader_function(|name| library.symbol(name)) };

        let version_raw = unsafe { gl.get_parameter_string(glow::VERSION) };
        let version = Version::parse(&version_raw);
        let glsl = unsafe { gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION) };
        let vendor = unsafe { gl.get_parameter_string(glow::VENDOR) };
        let renderer = unsafe { gl.get_parameter_string(glow::RENDERER) };
        let extensions: Vec<String> = unsafe { gl.get_parameter_string(glow::EXTENSIONS) }
            .split_whitespace()
            .map(str::to_string)
            .collect();

        // The limits the renderer plans against: the driver's own where it reports them, so
        // a machine better than the target is not held to the target's numbers outside
        // strict mode, and a machine worse than it fails here instead of mid-frame.
        let reported = Limits {
            max_texture_size: int(&gl, glow::MAX_TEXTURE_SIZE) as u32,
            max_3d_texture_size: int(&gl, glow::MAX_3D_TEXTURE_SIZE) as u32,
            max_cube_map_size: int(&gl, glow::MAX_CUBE_MAP_TEXTURE_SIZE) as u32,
            max_renderbuffer_size: int(&gl, glow::MAX_RENDERBUFFER_SIZE) as u32,
            max_color_attachments: int(&gl, glow::MAX_COLOR_ATTACHMENTS) as u32,
            max_texture_units: int(&gl, glow::MAX_COMBINED_TEXTURE_IMAGE_UNITS) as u32,
            max_vertex_attribs: int(&gl, glow::MAX_VERTEX_ATTRIBS) as u32,
            max_vertex_uniform_components: int(&gl, glow::MAX_VERTEX_UNIFORM_COMPONENTS) as u32,
            max_fragment_uniform_components: int(&gl, glow::MAX_FRAGMENT_UNIFORM_COMPONENTS)
                as u32,
            max_varying_floats: int(&gl, glow::MAX_VARYING_FLOATS) as u32,
            max_vertex_texture_units: int(&gl, glow::MAX_VERTEX_TEXTURE_IMAGE_UNITS) as u32,
            max_viewport_width: int(&gl, glow::MAX_VIEWPORT_DIMS) as u32,
            max_viewport_height: 4096,
            max_instructions: None,
        };

        let mut caps = Caps::detect(version, &glsl, &vendor, &renderer, extensions, reported);
        if options.strict {
            caps = caps.strict_over();
        }
        caps.meets_minimum()?;
        log::info!("Horizon GL 2.1: {}", caps.summary());

        Ok(Self {
            gl,
            caps,
            uploaded: std::sync::atomic::AtomicU64::new(0),
            programs_created: std::sync::atomic::AtomicU32::new(0),
            programs_destroyed: std::sync::atomic::AtomicU32::new(0),
            library,
            trace: options.trace,
        })
    }

    pub fn gl(&self) -> &glow::Context {
        &self.gl
    }

    pub fn caps(&self) -> &Caps {
        &self.caps
    }

    pub fn library(&self) -> &GlLibrary {
        &self.library
    }

    pub fn trace(&self) -> bool {
        self.trace
    }

    pub(crate) fn count_upload(&self, bytes: usize) {
        self.uploaded
            .fetch_add(bytes as u64, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn uploaded_bytes(&self) -> u64 {
        self.uploaded.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Programs created minus destroyed, for the leak check at shutdown: a renderer that
    /// recreated its programs on every resource reload must end a run at zero.
    pub fn live_programs(&self) -> u32 {
        let created = self.programs_created.load(std::sync::atomic::Ordering::Relaxed);
        let destroyed = self
            .programs_destroyed
            .load(std::sync::atomic::Ordering::Relaxed);
        created.saturating_sub(destroyed)
    }

    pub(crate) fn note_program(&self, _program: &Program) {
        self.programs_created
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn note_program_destroyed(&self) {
        self.programs_destroyed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Asks the driver for an error after a call that has no return value. `where_` names the
    /// call for the log; `GL_NO_ERROR` is the common case and costs one round trip.
    pub fn check(&self, where_: &'static str) -> Result<()> {
        let code = unsafe { self.gl.get_error() };
        if code == glow::NO_ERROR {
            return Ok(());
        }
        Err(Error::gl(where_, gl_error_name(code)))
    }

    /// Whether a feature may be used here. Every pass asks this rather than a version.
    pub fn has(&self, feature: Feature) -> bool {
        self.caps.has(feature)
    }
}

fn int(gl: &glow::Context, parameter: u32) -> i32 {
    unsafe { gl.get_parameter_i32(parameter) }
}

/// The driver's error codes as words a player's log can carry.
pub fn gl_error_name(code: u32) -> String {
    let name = match code {
        glow::INVALID_ENUM => "GL_INVALID_ENUM",
        glow::INVALID_VALUE => "GL_INVALID_VALUE",
        glow::INVALID_OPERATION => "GL_INVALID_OPERATION",
        glow::INVALID_FRAMEBUFFER_OPERATION => "GL_INVALID_FRAMEBUFFER_OPERATION",
        glow::OUT_OF_MEMORY => "GL_OUT_OF_MEMORY",
        glow::STACK_OVERFLOW => "GL_STACK_OVERFLOW",
        glow::STACK_UNDERFLOW => "GL_STACK_UNDERFLOW",
        glow::CONTEXT_LOST => "GL_CONTEXT_LOST",
        _ => return format!("GL error 0x{code:04X}"),
    };
    name.to_string()
}

/// A one-line description of what this machine offers, before any context exists: can the GL
/// library be opened, and does it have the entry points every frame needs. The CLI prints
/// it; no per-frame code calls it. Cheap enough (one `dlopen`, a few `dlsym` calls) that it
/// is not cached.
pub fn probe() -> std::result::Result<String, String> {
    match GlLibrary::open() {
        Ok(lib) => {
            let missing = lib.missing_essentials();
            if missing.is_empty() {
                Ok(format!(
                    "{} opened, all {} required entry points present",
                    lib.name(),
                    crate::loader::ESSENTIAL.len()
                ))
            } else {
                Err(format!(
                    "{} opened but lacks {} entry point(s): {}",
                    lib.name(),
                    missing.len(),
                    missing.join(", ")
                ))
            }
        }
        Err(e) => Err(e.message),
    }
}
