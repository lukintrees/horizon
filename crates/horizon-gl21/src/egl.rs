//! The window's OpenGL 2.1 context, and the frame it presents.
//!
//! The one thing this crate exists for: `horizon-client-render` forbids `unsafe` code, and a GL
//! context is a pile of C calls with raw handles. So the context lives here — `khronos-egl` for the
//! display, the config, the context and the surface, `glow` for the entry points `gl21::Device`
//! needs — and everything else about the GL 2.1 path stays in the crate that can be tested without a
//! driver.
//!
//! **What is asked for, and why each line is there** (the X1650's driver answers all of it):
//!
//! - `EGL_OPENGL_API`, *not* OpenGL ES: the R500's OpenGL driver is `r300g`, and GLES 2.0 has neither
//!   the `#version 120` shaders nor the fixed-function state this path is written for.
//! - `EGL_CONTEXT_MAJOR_VERSION`/`MINOR` = what the request names when the display has
//!   `EGL_KHR_create_context`, else no version at all (EGL 1.4 without that extension gives the
//!   driver's newest context, which on the target machine *is* 2.1). `gl21::backend::open` then
//!   refuses a context that is not the version asked for, so a 4.6 driver pretending to be the card
//!   is a sentence, not a mystery.
//! - `EGL_OPENGL_PROFILE_MASK` only above GL 3.1: a profile mask is GL 3.2 and later, and sending it
//!   to a 2.1 driver is `EGL_BAD_ATTRIBUTE`.
//! - `EGL_DEPTH_SIZE` 24 and `EGL_STENCIL_SIZE` 8: the world's depth test needs the first, the HUD
//!   model's isolated depth pass the second.
//!
//! The display comes from `eglGetPlatformDisplay` when the client lists the platform extension
//! (`EGL_EXT_platform_wayland`, `EGL_EXT_platform_x11`, `EGL_EXT_platform_xcb`), else from
//! `eglGetDisplay(EGL_DEFAULT_DISPLAY)`: a 2006 driver's libEGL has neither extension, and the
//! default display is exactly what it is for.
//!
//! **Two framebuffers.** The renderer draws the world and the HUD into a texture and then presents
//! it (`Renderer::draw`'s `present`), so this module owns one: a colour texture plus a depth
//! renderbuffer, made again on a resize. The window's own framebuffer is where only the present quad
//! draws, which is what lets the game hold a frame back for a menu's blur.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::ptr;
use std::sync::OnceLock;

use glow::HasContext;
use horizon_client_render::gl21;
use khronos_egl as egl;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// The EGL instance, loaded once: `libEGL` and its symbols cost a `dlopen` each, and a second window
/// in one process would otherwise load it again.
type Egl = egl::DynamicInstance<egl::EGL1_4>;

/// The EGL platform constants `khronos-egl` does not name (they are `KHR`/`ANGLE` extensions).
const EGL_PLATFORM_X11_KHR: u32 = 0x31D5;
const EGL_PLATFORM_WAYLAND_KHR: u32 = 0x31D8;
const EGL_PLATFORM_XCB_EXT: u32 = 0x31DC;
const EGL_PLATFORM_XCB_SCREEN_EXT: usize = 0x31DE;
/// `EGL_ANGLE_platform_angle`: a Windows 7 machine whose card has no WGL driver left is exactly the
/// case ANGLE's D3D9 backend exists for, and it is a GL 2.1 context.
const EGL_PLATFORM_ANGLE_ANGLE: u32 = 0x3202;
const EGL_PLATFORM_ANGLE_TYPE_ANGLE: usize = 0x3203;
const EGL_PLATFORM_ANGLE_TYPE_D3D9_ANGLE: usize = 0x3204;

fn egl_instance() -> Result<&'static Egl, String> {
    static INSTANCE: OnceLock<Result<Egl, String>> = OnceLock::new();
    let loaded = INSTANCE.get_or_init(|| {
        // Safety: libEGL's own ABI, loaded the way wgpu-hal loads it
        // (`third_party/wgpu-hal/src/gles/egl.rs`). A library of the same name with a different ABI
        // would be undefined behaviour, and no binding can check that: it is the trust every GL
        // binding makes.
        unsafe { Egl::load_required() }
            .map_err(|e| format!("Horizon's OpenGL 2.1 renderer could not load libEGL: {e}"))
    });
    match loaded {
        Ok(instance) => Ok(instance),
        Err(e) => Err(e.clone()),
    }
}

/// The GL objects that make the frame texture: the colour texture, the depth buffer and the
/// framebuffer binding them.
struct Frame {
    texture: glow::Texture,
    depth: glow::Renderbuffer,
    framebuffer: glow::Framebuffer,
}

/// An OpenGL 2.1 context that draws into a texture and presents it into a window.
///
/// Dropping it gives the window surface back, destroys the context and unmakes it current. The GL
/// objects are given back first; the display is not terminated, because another window in this
/// process may share it.
pub struct Context {
    egl: &'static Egl,
    display: egl::Display,
    surface: egl::Surface,
    raw: egl::Context,
    #[cfg(all(unix, not(target_os = "macos")))]
    wayland_window: Option<*mut c_void>,
    /// `glow`, which is how the renderer's calls reach the driver.
    gl: glow::Context,
    /// Made by `make_frame`, at `size`.
    frame: Frame,
    size: (u32, u32),
}

// The handles are EGL's and GL's own. A context is used from the thread that made it current and
// never from two at once (`Renderer`'s `&mut` enforces that), and EGL's other calls are thread-safe
// for a display.
unsafe impl Send for Context {}
unsafe impl Sync for Context {}

impl Context {
    /// A context for `window`, drawn into at `width`×`height` and presented into it.
    ///
    /// `profile` is `HORIZON_GL_PROFILE`'s, and decides what is asked for and what is accepted
    /// (`gl21::backend::Request::for_profile`). Every failure says what to do about it, as the rest
    /// of the GL 2.1 path does.
    pub fn new(
        window: RawWindowHandle,
        display_handle: RawDisplayHandle,
        width: u32,
        height: u32,
        profile: Option<gl21::Profile>,
    ) -> Result<Context, String> {
        let egl = egl_instance()?;
        let size = (width.max(1), height.max(1));
        let display = open_display(egl, display_handle)?;
        // Re-initialising an initialised display is legal and answers its version, which is what a
        // second window in one process does.
        let egl_version = egl.initialize(display).map_err(|e| {
            format!("Horizon's OpenGL 2.1 renderer could not initialise the EGL display: {e}")
        })?;
        let client = client_extensions(egl);
        let config = choose_config(egl, display)?;
        let request = gl21::backend::Request::for_profile(profile);
        let raw = create_context(egl, display, config, &client, request)?;

        // The window, as EGL takes it. An X11 window is its id passed by address, so the id lives
        // here, in `new`'s own frame, for the whole `create_window_surface` call: a struct returned
        // by value would be free to move, and the pointer with it.
        let mut x11_id: u64 = 0;
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut wayland_window: Option<*mut c_void> = None;
        let native = match window {
            RawWindowHandle::Xlib(handle) => {
                x11_id = handle.window;
                ptr::from_mut(&mut x11_id).cast::<c_void>()
            }
            RawWindowHandle::Xcb(handle) => {
                x11_id = u64::from(handle.window.get());
                ptr::from_mut(&mut x11_id).cast::<c_void>()
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            RawWindowHandle::Wayland(handle) => {
                // Safety: the surface is the window's own, and alive as long as the window is.
                let egl_window =
                    unsafe { wayland_egl_create(handle.surface.as_ptr().cast(), size.0, size.1) };
                if egl_window.is_null() {
                    return Err(
                        "Horizon's OpenGL 2.1 renderer could not make a Wayland EGL window: is \
                         libwayland-egl installed?"
                            .to_string(),
                    );
                }
                wayland_window = Some(egl_window);
                egl_window
            }
            RawWindowHandle::Win32(handle) => handle.hwnd.as_ptr(),
            RawWindowHandle::AppKit(handle) => handle.ns_view.as_ptr(),
            other => {
                return Err(format!(
                    "Horizon's OpenGL 2.1 renderer has no EGL window for a {other:?} handle"
                ));
            }
        };
        // Safety: `native` came from the caller's own window handle, of the same platform as
        // `display`, and the window outlives this context. No attributes: the surface's colour
        // buffer is the config's own.
        let surface =
            unsafe { egl.create_window_surface(display, config, native, None) }.map_err(|e| {
                format!(
                    "Horizon's OpenGL 2.1 renderer could not make a window surface: {e}. The \
                     driver may have no GL 2.1 config for this window."
                )
            })?;
        egl.make_current(display, Some(surface), Some(surface), Some(raw))
            .map_err(|e| {
                format!("Horizon's OpenGL 2.1 renderer could not make its context current: {e}")
            })?;

        let gl = load_glow(egl);
        // What the driver says it is, and whether that is the version the profile asked for: the
        // backend's own checklist, so a 1.4 driver gets the same sentences here as in a CI test.
        let (version, glsl) = version_strings(&gl);
        let reported = reported(&gl, &version, &glsl);
        let lines = gl21::backend::open(request, reported, &extension_list(&gl))?;
        for line in lines {
            log::info!("{line}");
        }
        log::info!(
            "Horizon: EGL {}.{}, {}, {}",
            egl_version.0,
            egl_version.1,
            if client.contains("EGL_KHR_create_context") {
                "a versioned context"
            } else {
                "the driver's own context version"
            },
            if client_extensions(egl).is_empty() {
                "no client extensions"
            } else {
                "extensions listed"
            }
        );
        let frame = make_frame(&gl, size)?;
        let context = Context {
            egl,
            display,
            surface,
            raw,
            #[cfg(all(unix, not(target_os = "macos")))]
            wayland_window,
            gl,
            frame,
            size,
        };
        // vsync on, as a window wants it: the frame limiter above shaves the rest.
        context.set_vsync(true);
        Ok(context)
    }

    /// The GL entry points, for the device that issues the calls.
    pub fn gl(&self) -> &glow::Context {
        &self.gl
    }

    /// The size of the frame texture and of the window, in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The frame texture: what the present program samples (`Texture::Frame`).
    pub fn frame_texture(&self) -> glow::Texture {
        self.frame.texture
    }

    /// `SwapBuffers`: the frame drawn into the window's back buffer is shown.
    pub fn swap(&self) -> Result<(), String> {
        self.egl
            .swap_buffers(self.display, self.surface)
            .map_err(|e| format!("Horizon: eglSwapBuffers failed: {e}"))
    }

    /// vsync: `true` waits for the display, `false` draws as fast as the card can.
    pub fn set_vsync(&self, on: bool) {
        if let Err(e) = self.egl.swap_interval(self.display, i32::from(on)) {
            // A driver that cannot change it keeps whatever the first call set: worth a line, not a
            // reason to fail a frame.
            log::warn!("Horizon: eglSwapInterval({}) failed: {e:?}", i32::from(on));
        }
    }

    /// Bind the frame texture for drawing and set the viewport to it: where the world's passes and
    /// the HUD are drawn.
    pub fn bind_frame(&self) {
        // Safety: the context is current and the frame's handles came from `make_frame`.
        unsafe {
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(self.frame.framebuffer));
            self.gl
                .viewport(0, 0, self.size.0 as i32, self.size.1 as i32);
        }
    }

    /// Bind the window's own framebuffer: where the present quad draws, and the only place drawing
    /// to the window happens.
    pub fn bind_window(&self) {
        // Safety: the context is current; there is no argument here to get wrong.
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            self.gl
                .viewport(0, 0, self.size.0 as i32, self.size.1 as i32);
        }
    }

    /// The window was resized (`winit`'s `WindowEvent::Resized`): the frame texture is made again at
    /// the new size. The window surface follows the window itself.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        let size = (width.max(1), height.max(1));
        if size == self.size {
            return Ok(());
        }
        frame_destroy(&self.gl, &self.frame);
        self.frame = make_frame(&self.gl, size)?;
        self.size = size;
        Ok(())
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        frame_destroy(&self.gl, &self.frame);
        // Safety: the context is current, and these are this `Context`'s own handles.
        unsafe {
            self.egl.make_current(self.display, None, None, None).ok();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Some(window) = self.wayland_window {
            // Safety: made by `wl_egl_window_create` for the surface destroyed right below.
            unsafe { wayland_egl_destroy(window) };
        }
        let _ = self.egl.destroy_surface(self.display, self.surface);
        let _ = self.egl.destroy_context(self.display, self.raw);
    }
}

/// The frame texture at `size`: colour, depth, and the framebuffer that binds them.
fn make_frame(gl: &glow::Context, size: (u32, u32)) -> Result<Frame, String> {
    let (width, height) = (size.0 as i32, size.1 as i32);
    // Safety: the context is current for every caller of this function, which is a `Context`'s own.
    unsafe {
        let texture = gl.create_texture().map_err(|e| {
            format!("Horizon's OpenGL 2.1 renderer could not make the frame texture: {e}")
        })?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        // RGBA8: the world and the HUD are drawn into this, and the present quad reads it 1:1.
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            width,
            height,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        // No mip chain: the present quad samples it at 1:1, and a chain is one more upload a frame.
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        let depth = gl.create_renderbuffer().map_err(|e| {
            format!("Horizon's OpenGL 2.1 renderer could not make the depth buffer: {e}")
        })?;
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
        gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, width, height);
        let framebuffer = gl.create_framebuffer().map_err(|e| {
            format!("Horizon's OpenGL 2.1 renderer could not make its framebuffer: {e}")
        })?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(depth),
        );
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        if status != glow::FRAMEBUFFER_COMPLETE {
            return Err(format!(
                "Horizon's OpenGL 2.1 renderer cannot draw into a {width}x{height} texture: the \
                 driver answered framebuffer status {status:#x}. A driver without \
                 GL_ARB_framebuffer_object cannot run this renderer."
            ));
        }
        Ok(Frame {
            texture,
            depth,
            framebuffer,
        })
    }
}

/// The frame texture's handles, given back to the driver.
fn frame_destroy(gl: &glow::Context, frame: &Frame) {
    // Safety: the context is current, and the handles are this frame's own, given back once.
    unsafe {
        gl.delete_framebuffer(frame.framebuffer);
        gl.delete_renderbuffer(frame.depth);
        gl.delete_texture(frame.texture);
    }
}

/// The EGL display for a window's own platform, or the default one.
fn open_display(egl: &'static Egl, display: RawDisplayHandle) -> Result<egl::Display, String> {
    let client = client_extensions(egl);
    // `get_platform_display` is EGL 1.5's. Without it, `eglGetDisplay(0)` is the default display,
    // which is what a 2006 driver has and what a machine with a single platform needs anyway.
    if let Some(platform) = egl.upcast::<egl::EGL1_5>() {
        match display {
            RawDisplayHandle::Wayland(handle) if client.contains("EGL_EXT_platform_wayland") => {
                let attributes = [egl::ATTRIB_NONE];
                // Safety: the display the window's own handle came from, of the platform the
                // client's extension list above says is there.
                let native = handle.display.as_ptr().cast::<c_void>();
                let platform_id = EGL_PLATFORM_WAYLAND_KHR;
                // Safety: as in the X11 arm below, for the window system's own display pointer.
                let opened =
                    unsafe { platform.get_platform_display(platform_id, native, &attributes) };
                return opened.map_err(|e| {
                    format!("Horizon's OpenGL 2.1 renderer could not open the Wayland display: {e}")
                });
            }
            RawDisplayHandle::Xlib(handle) if client.contains("EGL_EXT_platform_x11") => {
                let attributes = [egl::ATTRIB_NONE];
                // Safety: as above, for the window system's own display pointer.
                let opened = unsafe {
                    platform.get_platform_display(
                        EGL_PLATFORM_X11_KHR,
                        display_pointer(handle.display),
                        &attributes,
                    )
                };
                return opened.map_err(|e| {
                    format!("Horizon's OpenGL 2.1 renderer could not open the X11 display: {e}")
                });
            }
            RawDisplayHandle::Xcb(handle) if client.contains("EGL_EXT_platform_xcb") => {
                let attributes = [
                    EGL_PLATFORM_XCB_SCREEN_EXT,
                    handle.screen as usize,
                    egl::ATTRIB_NONE,
                ];
                // Safety: as above, for the window system's own connection pointer.
                let opened = unsafe {
                    platform.get_platform_display(
                        EGL_PLATFORM_XCB_EXT,
                        display_pointer(handle.connection),
                        &attributes,
                    )
                };
                return opened.map_err(|e| {
                    format!("Horizon's OpenGL 2.1 renderer could not open the XCB display: {e}")
                });
            }
            RawDisplayHandle::Windows(_) if client.contains("EGL_ANGLE_platform_angle") => {
                // D3D9 through ANGLE is a GL 2.1 context, which is exactly what a Windows 7 machine
                // with an X1650 has left when the card's own WGL driver is gone.
                let attributes = [
                    EGL_PLATFORM_ANGLE_TYPE_ANGLE,
                    EGL_PLATFORM_ANGLE_TYPE_D3D9_ANGLE,
                    egl::ATTRIB_NONE,
                ];
                // Safety: `EGL_DEFAULT_DISPLAY` is the native display ANGLE reads on Windows.
                let opened = unsafe {
                    platform.get_platform_display(
                        EGL_PLATFORM_ANGLE_ANGLE,
                        egl::DEFAULT_DISPLAY,
                        &attributes,
                    )
                };
                return opened.map_err(|e| {
                    format!("Horizon's OpenGL 2.1 renderer could not open ANGLE's display: {e}")
                });
            }
            _ => {}
        }
    }
    // Safety: `EGL_DEFAULT_DISPLAY` is defined to be valid, and this is what every EGL 1.4 driver
    // answers.
    unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }
        .ok_or_else(|| "Horizon's OpenGL 2.1 renderer found no EGL display".to_string())
}

/// A window system connection as EGL's `NativeDisplayType`, or the default display.
fn display_pointer(connection: Option<ptr::NonNull<c_void>>) -> *mut c_void {
    connection.map_or(egl::DEFAULT_DISPLAY, ptr::NonNull::as_ptr)
}

fn client_extensions(egl: &'static Egl) -> String {
    egl.query_string(None, egl::EXTENSIONS)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The framebuffer config the frame needs: RGBA8, 24 bits of depth, 8 of stencil, and a window
/// surface.
///
/// A config that cannot be found is the most common way a machine gets here and has no picture, so
/// the failure names what was wanted and what to do about it.
fn choose_config(egl: &'static Egl, display: egl::Display) -> Result<egl::Config, String> {
    let wanted = [
        egl::SURFACE_TYPE,
        egl::WINDOW_BIT,
        egl::RENDERABLE_TYPE,
        egl::OPENGL_BIT,
        egl::RED_SIZE,
        8,
        egl::GREEN_SIZE,
        8,
        egl::BLUE_SIZE,
        8,
        egl::ALPHA_SIZE,
        8,
        egl::DEPTH_SIZE,
        24,
        egl::STENCIL_SIZE,
        8,
        // A multisampled window would need the present quad resolved, and four samples on an R500 is
        // a quarter of the frame rate: the world's antialiasing is the renderer's own business.
        egl::SAMPLES,
        0,
        egl::NONE,
    ];
    if let Ok(Some(config)) = egl.choose_first_config(display, &wanted) {
        return Ok(config);
    }
    // A driver with no 24-bit depth at 8/8/8/8: the world still needs a depth buffer, so the colour
    // sizes and the sample count are what give way.
    let fallback = [
        egl::SURFACE_TYPE,
        egl::WINDOW_BIT,
        egl::RENDERABLE_TYPE,
        egl::OPENGL_BIT,
        egl::DEPTH_SIZE,
        16,
        egl::NONE,
    ];
    match egl.choose_first_config(display, &fallback) {
        Ok(Some(config)) => {
            log::warn!(
                "Horizon: this driver has no 8/8/8/8 EGL config with 24 bits of depth; drawing \
                 with its 16-bit depth one instead"
            );
            Ok(config)
        }
        _ => Err(
            "Horizon's OpenGL 2.1 renderer found no EGL config with an OpenGL window, 8-bit colour \
             and a depth buffer. Install the newest driver your card and operating system allow: \
             the renderer needs nothing a 2007 driver does not have."
                .to_string(),
        ),
    }
}

/// The OpenGL context the request names.
fn create_context(
    egl: &'static Egl,
    display: egl::Display,
    config: egl::Config,
    client: &str,
    request: gl21::backend::Request,
) -> Result<egl::Context, String> {
    egl.bind_api(egl::OPENGL_API).map_err(|e| {
        format!(
            "Horizon's OpenGL 2.1 renderer could not select desktop OpenGL (EGL says {e}): this \
             driver offers OpenGL ES only"
        )
    })?;
    let mut attributes = Vec::with_capacity(8);
    // Without `EGL_KHR_create_context` a version attribute is `EGL_BAD_ATTRIBUTE`: the driver gives
    // its newest context, and `open` checks which one that was.
    if client.contains("EGL_KHR_create_context") {
        attributes.push(egl::CONTEXT_MAJOR_VERSION);
        attributes.push(egl::Int::from(request.major));
        attributes.push(egl::CONTEXT_MINOR_VERSION);
        attributes.push(egl::Int::from(request.minor));
        // A profile mask as well, only from 3.2 on: a mask on a 2.1 request is a bad attribute.
        if (request.major, request.minor) >= (3, 2) {
            attributes.push(egl::CONTEXT_OPENGL_PROFILE_MASK);
            attributes.push(if request.compatibility {
                egl::CONTEXT_OPENGL_COMPATIBILITY_PROFILE_BIT
            } else {
                egl::CONTEXT_OPENGL_CORE_PROFILE_BIT
            });
        }
    }
    attributes.push(egl::NONE);
    egl.create_context(display, config, None, &attributes)
        .map_err(|e| {
            format!(
                "Horizon's OpenGL 2.1 renderer could not create an OpenGL {}.{} context (EGL says \
                 {e})",
                request.major, request.minor
            )
        })
}

#[cfg(all(unix, not(target_os = "macos")))]
/// A `wl_egl_window` for a Wayland surface, which EGL needs and cannot make itself.
///
/// `libwayland-egl` is loaded by name rather than linked: a machine without it (an X11-only
/// desktop) must still start, and only a Wayland window needs the two functions.
///
/// Safety: `surface` is a live `wl_surface`, and the returned window is destroyed with it.
unsafe fn wayland_egl_create(surface: *mut c_void, width: u32, height: u32) -> *mut c_void {
    let Some(create) = wayland_egl_symbol::<WaylandCreate>("wl_egl_window_create") else {
        return ptr::null_mut();
    };
    // Safety: `surface` is the caller's live `wl_surface`, and `create` is that library's own
    // function of the EGL window API's signature.
    unsafe { create(surface, width as i32, height as i32) }
}

#[cfg(all(unix, not(target_os = "macos")))]
/// The same window, given back with the surface it was made for.
///
/// Safety: `window` came from `wayland_egl_create`, and its surface is gone.
unsafe fn wayland_egl_destroy(window: *mut c_void) {
    let Some(destroy) = wayland_egl_symbol::<WaylandDestroy>("wl_egl_window_destroy") else {
        return;
    };
    // Safety: as above.
    unsafe { destroy(window) }
}

#[cfg(all(unix, not(target_os = "macos")))]
type WaylandCreate = unsafe extern "C" fn(*mut c_void, i32, i32) -> *mut c_void;
#[cfg(all(unix, not(target_os = "macos")))]
type WaylandDestroy = unsafe extern "C" fn(*mut c_void);

#[cfg(all(unix, not(target_os = "macos")))]
/// One function of `libwayland-egl`, loaded once by name.
fn wayland_egl_symbol<T: Copy>(name: &str) -> Option<T> {
    use std::ffi::CString;
    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flag: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    static LIBRARY: OnceLock<Option<usize>> = OnceLock::new();
    let handle = LIBRARY.get_or_init(|| {
        let names = ["libwayland-egl.so.1", "libwayland-egl.so"];
        names.iter().find_map(|name| {
            let name = CString::new(*name).ok()?;
            // Safety: `dlopen` on a name this call owns; a machine without the library gets null,
            // which becomes a sentence about installing it.
            let handle = unsafe { dlopen(name.as_ptr(), 1) };
            (!handle.is_null()).then_some(handle as usize)
        })
    });
    let handle = (*handle)? as *mut c_void;
    let symbol = CString::new(name).ok()?;
    // Safety: the library is the wayland-egl one, loaded above, and the symbol is one of its own
    // documented entry points; the handle is kept for the life of the process, so the pointer stays
    // valid.
    unsafe {
        let found = dlsym(handle, symbol.as_ptr());
        if found.is_null() {
            return None;
        }
        // `T` is one of the two function types above, both of the library's documented signatures.
        Some(std::mem::transmute_copy::<*mut c_void, T>(&found))
    }
}

/// `glGetString(GL_VERSION)` and `GL_SHADING_LANGUAGE_VERSION`.
///
/// Safety: the context is current, and both take a constant.
fn version_strings(gl: &glow::Context) -> (String, String) {
    unsafe {
        (
            gl.get_parameter_string(glow::VERSION),
            gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION),
        )
    }
}

/// What the driver reports, as `gl21::backend::check` reads it.
///
/// Safety: the context is current, and `glGetIntegerv` takes a constant.
fn reported<'a>(
    gl: &glow::Context,
    version: &'a str,
    glsl: &'a str,
) -> gl21::backend::Reported<'a> {
    let limit = |name: u32| unsafe { gl.get_parameter_i32(name).max(0) as u32 };
    gl21::backend::Reported {
        version,
        glsl_version: glsl,
        max_texture_size: limit(glow::MAX_TEXTURE_SIZE),
        max_vertex_attributes: limit(glow::MAX_VERTEX_ATTRIBS),
        max_texture_units: limit(glow::MAX_TEXTURE_IMAGE_UNITS),
    }
}

/// The extensions the checklist asks about, from `glow`'s own list (`GL_EXTENSIONS` split, or
/// `glGetStringi` on a 3.0 driver).
///
/// An empty list is not an error: `capabilities` reads the version for everything an extension would
/// have said, and a driver that lists none still has what its version made core.
fn extension_list(gl: &glow::Context) -> Vec<&'static str> {
    // Safety: the context is current, and `glow` caches the list it read when it was made.
    let listed = unsafe { gl.supported_extensions() };
    const WANTED: [&str; 5] = [
        "GL_ARB_framebuffer_object",
        "GL_ARB_draw_buffers",
        "GL_ARB_occlusion_query",
        "GL_ARB_texture_non_power_of_two",
        "GL_EXT_texture_lod_bias",
    ];
    WANTED
        .into_iter()
        .filter(|name| listed.contains(*name))
        .collect()
}

/// `glow::Context::from_loader_function`, with `eglGetProcAddress` and the GL library behind it.
fn load_glow(egl: &'static Egl) -> glow::Context {
    // Safety: every pointer comes from EGL or from the GL library, which is all a GL binding can
    // check; `glow` reads the version from the context that is current.
    unsafe {
        glow::Context::from_loader_function(|name| {
            if let Some(proc) = egl.get_proc_address(name) {
                return proc as *const c_void as *const _;
            }
            // GL 1.1's entry points are exported by the GL library itself, and a 2006 libEGL
            // answers `eglGetProcAddress` for none of them (`gl21::backend::require` says the same
            // thing to a caller with its own loader).
            if gl21::backend::core_1_1(name) {
                gl_proc(name).map_or(ptr::null(), |proc| proc as *const _)
            } else {
                ptr::null()
            }
        })
    }
}

/// `dlsym`/`GetProcAddress` on the GL library, for the names `eglGetProcAddress` answered with null.
fn gl_proc(name: &str) -> Option<extern "C" fn()> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        unsafe extern "C" {
            fn dlopen(filename: *const c_char, flag: i32) -> *mut c_void;
            fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        }
        // `dlopen` on a library the process already has (libGL loads with the context) is a
        // refcount bump, done once per missing 1.1 name at startup. The handle is deliberately kept
        // for the life of the process: closing it would unmap the functions this very context uses.
        const RTLD_LAZY: i32 = 1;
        let symbol = CString::new(name).ok()?;
        for library in ["libGL.so.1", "libGL.so", "libOpenGL.so.0"] {
            let Ok(library) = CString::new(library) else {
                continue;
            };
            // Safety: `dlopen`/`dlsym` on the C strings built here; a library that is not there, or
            // a name it does not export, gives null, which is handled.
            unsafe {
                let handle = dlopen(library.as_ptr(), RTLD_LAZY);
                if handle.is_null() {
                    continue;
                }
                let found = dlsym(handle, symbol.as_ptr());
                if !found.is_null() {
                    // Safety: a symbol of that name from a GL library is a GL function.
                    return Some(std::mem::transmute::<*mut c_void, extern "C" fn()>(found));
                }
            }
        }
        return None;
    }
    #[cfg(windows)]
    {
        use std::ffi::CString;
        unsafe extern "system" {
            fn LoadLibraryA(name: *const c_char) -> *mut c_void;
            fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        }
        let module = CString::new("opengl32.dll").ok()?;
        let symbol = CString::new(name).ok()?;
        // Safety: both strings are this call's own; a module that is not found, or a name it does
        // not export, gives null, which is handled.
        unsafe {
            let library = LoadLibraryA(module.as_ptr());
            let found = GetProcAddress(library, symbol.as_ptr());
            if found.is_null() {
                None
            } else {
                Some(std::mem::transmute::<*mut c_void, extern "C" fn()>(found))
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = name;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_core_1_1_names_this_module_loads_around_are_the_backends_own() {
        // Two callers, one list: the backend says what a missing pointer means, this module knows
        // where else to look for it. A name in one and not the other is a null call at runtime.
        for name in ["glGetString", "glEnable", "glTexImage2D", "glDrawArrays"] {
            assert!(gl21::backend::core_1_1(name), "{name}");
        }
        assert!(!gl21::backend::core_1_1("glGenVertexArrays"));
    }

    /// A display that no window system named: `eglGetDisplay(EGL_DEFAULT_DISPLAY)` is what a 2006
    /// driver answers, and what a machine with one platform needs anyway.
    #[test]
    fn the_default_display_is_what_a_missing_connection_becomes() {
        assert_eq!(display_pointer(None), egl::DEFAULT_DISPLAY);
    }

    /// The profile's request decides the attributes: 2.1 asks for no profile mask (a mask on a 2.1
    /// context is `EGL_BAD_ATTRIBUTE`), 3.3 core asks for one.
    #[test]
    fn the_requested_version_decides_the_context_attributes() {
        let gl21 = gl21::backend::Request::for_profile(Some(gl21::Profile::X1650));
        assert_eq!((gl21.major, gl21.minor), (2, 1));
        assert!(gl21.compatibility);
        assert!(!gl21.accept_newer);
        // What a machine without the card asks for: the driver's own newest, which `open` accepts.
        let any = gl21::backend::Request::for_profile(None);
        assert!(any.accept_newer);
        assert!(any.accepts((4, 6)));
        assert!(!gl21.accepts((4, 6)));
    }
}
