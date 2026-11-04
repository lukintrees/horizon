//! Owns an EGL desktop OpenGL pbuffer. Every call stays on the creating thread.
//! No EGL/GL headers or link-time Mesa installation are required.
use std::ffi::{CStr, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;

type Handle = *mut c_void;
type Initialize = unsafe extern "C" fn(Handle, *mut c_int, *mut c_int) -> u32;
type GetDisplay = unsafe extern "C" fn(Handle) -> Handle;
type BindApi = unsafe extern "C" fn(u32) -> u32;
type ChooseConfig =
    unsafe extern "C" fn(Handle, *const c_int, *mut Handle, c_int, *mut c_int) -> u32;
type CreateSurface = unsafe extern "C" fn(Handle, Handle, *const c_int) -> Handle;
type CreateContext = unsafe extern "C" fn(Handle, Handle, Handle, *const c_int) -> Handle;
type MakeCurrent = unsafe extern "C" fn(Handle, Handle, Handle, Handle) -> u32;
type Destroy = unsafe extern "C" fn(Handle, Handle) -> u32;
type Terminate = unsafe extern "C" fn(Handle) -> u32;
type GetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;
type GetError = unsafe extern "C" fn() -> u32;

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(name: *const c_char, flags: c_int) -> Handle;
    fn dlsym(library: Handle, name: *const c_char) -> *const c_void;
    fn dlclose(library: Handle) -> c_int;
    fn dlerror() -> *const c_char;
}

pub(super) struct Library(Handle);
impl Library {
    pub(super) fn open(name: &CStr) -> Result<Self, String> {
        // SAFETY: static NUL-terminated name; RTLD_NOW. No user pointers are retained.
        let handle = unsafe { dlopen(name.as_ptr(), 2) };
        if handle.is_null() {
            // SAFETY: dlerror returns a borrowed NUL-terminated message or null.
            let message = unsafe {
                let error = dlerror();
                if error.is_null() {
                    "unknown loader error".into()
                } else {
                    CStr::from_ptr(error).to_string_lossy().into_owned()
                }
            };
            Err(format!("cannot load {}: {message}", name.to_string_lossy()))
        } else {
            Ok(Self(handle))
        }
    }

    pub(super) fn symbol(&self, name: &CStr) -> Result<*const c_void, String> {
        // SAFETY: live library and NUL-terminated symbol name.
        let address = unsafe { dlsym(self.0, name.as_ptr()) };
        if address.is_null() {
            Err(format!(
                "missing native entry point {}",
                name.to_string_lossy()
            ))
        } else {
            Ok(address)
        }
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: this object uniquely owns one successful dlopen reference.
        unsafe { dlclose(self.0) };
    }
}

pub struct Context {
    // Library must outlive every function pointer and context teardown.
    _library: Library,
    display: Handle,
    surface: Handle,
    context: Handle,
    make_current: MakeCurrent,
    destroy_surface: Destroy,
    destroy_context: Destroy,
    terminate: Terminate,
    get_proc_address: GetProcAddress,
    // EGL currentness is thread-local: do not permit Send or Sync.
    _thread: PhantomData<Rc<()>>,
}

impl Context {
    pub fn new(size: [u32; 2]) -> Result<Self, String> {
        let library = Library::open(c"libEGL.so.1")?;
        macro_rules! load {
            ($name:literal, $ty:ty) => {{
                let address = library.symbol($name)?;
                // SAFETY: these fixed names/signatures are the EGL C ABI.
                unsafe { std::mem::transmute::<*const c_void, $ty>(address) }
            }};
        }
        let get_display = load!(c"eglGetDisplay", GetDisplay);
        let initialize = load!(c"eglInitialize", Initialize);
        let bind_api = load!(c"eglBindAPI", BindApi);
        let choose_config = load!(c"eglChooseConfig", ChooseConfig);
        let create_surface = load!(c"eglCreatePbufferSurface", CreateSurface);
        let create_context = load!(c"eglCreateContext", CreateContext);
        let get_error = load!(c"eglGetError", GetError);
        let mut this = Self {
            make_current: load!(c"eglMakeCurrent", MakeCurrent),
            destroy_surface: load!(c"eglDestroySurface", Destroy),
            destroy_context: load!(c"eglDestroyContext", Destroy),
            terminate: load!(c"eglTerminate", Terminate),
            get_proc_address: load!(c"eglGetProcAddress", GetProcAddress),
            _library: library,
            display: ptr::null_mut(),
            surface: ptr::null_mut(),
            context: ptr::null_mut(),
            _thread: PhantomData,
        };
        // SAFETY: EGL handles originate in this library; all attribute arrays end in
        // EGL_NONE. Output pointers refer to initialized stack storage. Drop cleans
        // partially created resources in reverse order on every error path.
        unsafe {
            let display = get_display(ptr::null_mut());
            let (mut major, mut minor) = (0, 0);
            if display.is_null() || initialize(display, &mut major, &mut minor) == 0 {
                return Err(format!(
                    "eglInitialize failed: {:#x}; try EGL_PLATFORM=surfaceless",
                    get_error()
                ));
            }
            this.display = display;
            if bind_api(0x30A2) == 0 {
                // EGL_OPENGL_API, NOT OpenGL ES.
                return Err(format!(
                    "eglBindAPI desktop OpenGL failed: {:#x}",
                    get_error()
                ));
            }
            let attributes = [
                0x3033, 1, // SURFACE_TYPE, PBUFFER_BIT
                0x3040, 8, // RENDERABLE_TYPE, OPENGL_BIT
                0x3024, 8, 0x3023, 8, 0x3022, 8, // RGB sizes
                0x3025, 16,     // DEPTH_SIZE
                0x3038, // NONE
            ];
            let (mut config, mut count) = (ptr::null_mut(), 0);
            if choose_config(display, attributes.as_ptr(), &mut config, 1, &mut count) == 0
                || count != 1
            {
                return Err(format!("no desktop GL pbuffer config: {:#x}", get_error()));
            }
            let size = [0x3057, size[0] as i32, 0x3056, size[1] as i32, 0x3038]; // WIDTH, HEIGHT, NONE
            this.surface = create_surface(display, config, size.as_ptr());
            if this.surface.is_null() {
                return Err(format!(
                    "eglCreatePbufferSurface failed: {:#x}",
                    get_error()
                ));
            }
            this.context = create_context(display, config, ptr::null_mut(), [0x3038].as_ptr());
            if this.context.is_null()
                || (this.make_current)(display, this.surface, this.surface, this.context) == 0
            {
                return Err(format!(
                    "creating/current desktop GL context failed: {:#x}",
                    get_error()
                ));
            }
        }
        Ok(this)
    }
}

impl crate::ContextImpl for Context {
    fn make_current(&self) -> Result<(), String> {
        // SAFETY: owned EGL handles on the creating thread.
        if unsafe { (self.make_current)(self.display, self.surface, self.surface, self.context) }
            == 0
        {
            Err("eglMakeCurrent failed".into())
        } else {
            Ok(())
        }
    }

    fn description(&self) -> &str {
        "EGL desktop OpenGL pbuffer"
    }

    fn address(&self, name: &CStr) -> Result<*const c_void, String> {
        // SAFETY: live EGL library, current context, NUL-terminated name.
        let address = unsafe { (self.get_proc_address)(name.as_ptr()) };
        if address.is_null() {
            Err(format!("missing GL entry point {}", name.to_string_lossy()))
        } else {
            Ok(address)
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: all non-null handles were created by this library on this thread;
        // detach before destruction; _library is still alive during this method.
        unsafe {
            if !self.display.is_null() {
                (self.make_current)(
                    self.display,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                if !self.context.is_null() {
                    (self.destroy_context)(self.display, self.context);
                }
                if !self.surface.is_null() {
                    (self.destroy_surface)(self.display, self.surface);
                }
                (self.terminate)(self.display);
            }
        }
    }
}
