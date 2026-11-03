//! The few EGL 1.4 calls the probe needs, loaded from libEGL at run time.
//!
//! The context is desktop OpenGL on the surfaceless platform (`EGL_MESA_platform_surfaceless`):
//! no window, no X server, no surface. That is what CI runs on, and it is enough for an
//! offscreen framebuffer. A Windows (WGL) context comes later with the game's window code.

use std::ffi::{CString, c_char, c_void};
use std::fmt;
use std::ptr;

type EglDisplay = *mut c_void;
type EglConfig = *mut c_void;
type EglContext = *mut c_void;
type EglSurface = *mut c_void;
type EglInt = i32;
type EglBoolean = u32;

const EGL_TRUE: EglBoolean = 1;
const EGL_NONE: EglInt = 0x3038;
const EGL_OPENGL_API: u32 = 0x30A2;
const EGL_RENDERABLE_TYPE: EglInt = 0x3040;
const EGL_OPENGL_BIT: EglInt = 0x0008;
const EGL_SURFACE_TYPE: EglInt = 0x3033;
const EGL_PBUFFER_BIT: EglInt = 0x0001;
const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;
const EGL_CONTEXT_MAJOR_VERSION: EglInt = 0x3098;
const EGL_CONTEXT_MINOR_VERSION: EglInt = 0x30FB;
const EGL_CONTEXT_OPENGL_PROFILE_MASK: EglInt = 0x30FD;
const EGL_CONTEXT_OPENGL_COMPATIBILITY_PROFILE_BIT: EglInt = 0x0002;

type GetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;
type GetPlatformDisplay = unsafe extern "C" fn(u32, *mut c_void, *const EglInt) -> EglDisplay;
type Initialize = unsafe extern "C" fn(EglDisplay, *mut EglInt, *mut EglInt) -> EglBoolean;
type BindApi = unsafe extern "C" fn(u32) -> EglBoolean;
type ChooseConfig =
    unsafe extern "C" fn(EglDisplay, *const EglInt, *mut EglConfig, EglInt, *mut EglInt) -> EglBoolean;
type CreateContext = unsafe extern "C" fn(EglDisplay, EglConfig, EglContext, *const EglInt) -> EglContext;
type MakeCurrent = unsafe extern "C" fn(EglDisplay, EglSurface, EglSurface, EglContext) -> EglBoolean;
type DestroyContext = unsafe extern "C" fn(EglDisplay, EglContext) -> EglBoolean;
type Terminate = unsafe extern "C" fn(EglDisplay) -> EglBoolean;
type GetError = unsafe extern "C" fn() -> EglInt;

/// Why an EGL step failed. `code` is `eglGetError()` right after the call.
#[derive(Debug)]
pub enum EglError {
    Load(String),
    Call { what: &'static str, code: EglInt },
}

impl fmt::Display for EglError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EglError::Load(msg) => write!(f, "cannot load libEGL: {msg}"),
            EglError::Call { what, code } => write!(f, "{what} failed (EGL error 0x{code:04x})"),
        }
    }
}

impl std::error::Error for EglError {}

/// The loaded EGL entry points. Holds the library so the pointers stay valid.
struct Egl {
    _lib: libloading::Library,
    get_proc_address: GetProcAddress,
    get_platform_display: GetPlatformDisplay,
    initialize: Initialize,
    bind_api: BindApi,
    choose_config: ChooseConfig,
    create_context: CreateContext,
    make_current: MakeCurrent,
    destroy_context: DestroyContext,
    terminate: Terminate,
    get_error: GetError,
}

unsafe fn symbol<T: Copy>(lib: &libloading::Library, name: &str) -> Result<T, EglError> {
    let bytes = format!("{name}\0");
    // SAFETY: the caller names the EGL function and its real C signature.
    let symbol: libloading::Symbol<T> = unsafe { lib.get(bytes.as_bytes()) }
        .map_err(|e| EglError::Load(format!("{name}: {e}")))?;
    Ok(*symbol)
}

impl Egl {
    fn load() -> Result<Self, EglError> {
        // SAFETY: loading libEGL runs its initialisers, which is what any GL program does.
        let lib = unsafe { libloading::Library::new("libEGL.so.1") }
            .map_err(|e| EglError::Load(e.to_string()))?;
        unsafe {
            let get_proc_address: GetProcAddress = symbol(&lib, "eglGetProcAddress")?;
            // eglGetPlatformDisplay is EGL 1.5; the EXT version works on any EGL 1.4 driver.
            let platform_name = CString::new("eglGetPlatformDisplayEXT").expect("no NUL");
            let raw = get_proc_address(platform_name.as_ptr());
            if raw.is_null() {
                return Err(EglError::Load("eglGetPlatformDisplayEXT is missing".into()));
            }
            Ok(Egl {
                get_platform_display: std::mem::transmute::<*const c_void, GetPlatformDisplay>(raw),
                initialize: symbol(&lib, "eglInitialize")?,
                bind_api: symbol(&lib, "eglBindAPI")?,
                choose_config: symbol(&lib, "eglChooseConfig")?,
                create_context: symbol(&lib, "eglCreateContext")?,
                make_current: symbol(&lib, "eglMakeCurrent")?,
                destroy_context: symbol(&lib, "eglDestroyContext")?,
                terminate: symbol(&lib, "eglTerminate")?,
                get_error: symbol(&lib, "eglGetError")?,
                get_proc_address,
                _lib: lib,
            })
        }
    }

    fn error(&self, what: &'static str) -> EglError {
        // SAFETY: eglGetError has no preconditions.
        EglError::Call { what, code: unsafe { (self.get_error)() } }
    }
}

/// A current desktop OpenGL context on a surfaceless EGL display.
pub struct Context {
    egl: Egl,
    display: EglDisplay,
    context: EglContext,
}

impl Context {
    /// Creates a compatibility-profile context, asking for OpenGL 2.1 first and for any
    /// version the driver gives when that is refused (Mesa's `MESA_GL_VERSION_OVERRIDE`
    /// can cap the version below 2.1 for the floor check).
    pub fn new_surfaceless() -> Result<Self, EglError> {
        let egl = Egl::load()?;
        // SAFETY: every pointer passed is either null (allowed by EGL here) or a live local.
        unsafe {
            let display = (egl.get_platform_display)(EGL_PLATFORM_SURFACELESS_MESA, ptr::null_mut(), ptr::null());
            if display.is_null() {
                return Err(egl.error("eglGetPlatformDisplayEXT"));
            }
            let (mut major, mut minor) = (0, 0);
            if (egl.initialize)(display, &mut major, &mut minor) != EGL_TRUE {
                return Err(egl.error("eglInitialize"));
            }
            if (egl.bind_api)(EGL_OPENGL_API) != EGL_TRUE {
                return Err(egl.error("eglBindAPI"));
            }
            // Without EGL_SURFACE_TYPE the default requires EGL_WINDOW_BIT, which the surfaceless
            // platform does not offer, so no config matches. A zero mask matches every config.
            let attribute_sets: [&[EglInt]; 3] = [
                &[EGL_SURFACE_TYPE, 0, EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT, EGL_NONE],
                &[EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT, EGL_NONE],
                &[EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT, EGL_NONE],
            ];
            let mut config: EglConfig = ptr::null_mut();
            let mut chosen = false;
            for attributes in attribute_sets {
                let mut count: EglInt = 0;
                if (egl.choose_config)(display, attributes.as_ptr(), &mut config, 1, &mut count) == EGL_TRUE
                    && count >= 1
                {
                    chosen = true;
                    break;
                }
            }
            if !chosen {
                return Err(egl.error("eglChooseConfig (no OpenGL config on this display)"));
            }
            let versioned = [
                EGL_CONTEXT_MAJOR_VERSION,
                2,
                EGL_CONTEXT_MINOR_VERSION,
                1,
                EGL_CONTEXT_OPENGL_PROFILE_MASK,
                EGL_CONTEXT_OPENGL_COMPATIBILITY_PROFILE_BIT,
                EGL_NONE,
            ];
            let mut context = (egl.create_context)(display, config, ptr::null_mut(), versioned.as_ptr());
            if context.is_null() {
                let unversioned = [EGL_NONE];
                context = (egl.create_context)(display, config, ptr::null_mut(), unversioned.as_ptr());
            }
            if context.is_null() {
                return Err(egl.error("eglCreateContext"));
            }
            if (egl.make_current)(display, ptr::null_mut(), ptr::null_mut(), context) != EGL_TRUE {
                let error = egl.error("eglMakeCurrent");
                (egl.destroy_context)(display, context);
                return Err(error);
            }
            Ok(Context { egl, display, context })
        }
    }

    /// The address of a GL function in the current context, or null.
    pub fn proc_address(&self, name: &str) -> *const c_void {
        let Ok(name) = CString::new(name) else {
            return ptr::null();
        };
        // SAFETY: the name is a valid NUL-terminated string for the call.
        unsafe { (self.egl.get_proc_address)(name.as_ptr()) }
    }

    /// A `glow` context whose functions come from this EGL context.
    pub fn glow(&self) -> glow::Context {
        // SAFETY: the context stays current for as long as `self` lives (the caller keeps `self`).
        unsafe { glow::Context::from_loader_function(|name| self.proc_address(name)) }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: the handles came from this display and context; they are released once.
        unsafe {
            (self.egl.make_current)(self.display, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
            (self.egl.destroy_context)(self.display, self.context);
            (self.egl.terminate)(self.display);
        }
    }
}
