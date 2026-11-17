//! Owns an EGL desktop OpenGL pbuffer. Every call stays on the creating thread.
//! No EGL/GL headers or link-time Mesa installation are required.
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;
use std::sync::Mutex;

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

/// EGL hands out one display per native display: `eglGetDisplay(NULL)` returns the same
/// handle to every caller, and `eglTerminate` shuts down every context created on it. A
/// process may hold several contexts at once (the isolation check does), so count the
/// users and terminate only when the last one is gone; otherwise dropping a temporary
/// context leaves the remaining ones failing every call with `EGL_NOT_INITIALIZED`.
static DISPLAY_USERS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

fn display_guard() -> std::sync::MutexGuard<'static, Vec<(usize, usize)>> {
    // A panic while holding the count must not make the display permanently unusable.
    DISPLAY_USERS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// One share of an initialized `EGLDisplay`, released when the last holder drops it.
struct Display {
    handle: Handle,
    terminate: Terminate,
}

impl Display {
    fn acquire(
        handle: Handle,
        initialize: Initialize,
        terminate: Terminate,
    ) -> Result<Self, String> {
        let key = handle as usize;
        let mut users = display_guard();
        if let Some(user) = users.iter_mut().find(|user| user.0 == key) {
            user.1 += 1;
            return Ok(Self { handle, terminate });
        }
        let (mut major, mut minor) = (0, 0);
        // SAFETY: display came from eglGetDisplay; outputs are stack storage.
        if unsafe { initialize(handle, &mut major, &mut minor) } == 0 {
            return Err("eglInitialize failed".into());
        }
        users.push((key, 1));
        Ok(Self { handle, terminate })
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        let key = self.handle as usize;
        let mut users = display_guard();
        let Some(index) = users.iter().position(|user| user.0 == key) else {
            return;
        };
        users[index].1 -= 1;
        if users[index].1 > 0 {
            return;
        }
        users.remove(index);
        // SAFETY: no context is left on this display, and it was initialized above.
        unsafe { (self.terminate)(self.handle) };
    }
}

pub struct Context {
    // Initialized first, released last: `Display::drop` calls into the library.
    display: Display,
    surface: Cell<Handle>,
    size: Cell<[u32; 2]>,
    config: Handle,
    create_surface: CreateSurface,
    retired: RefCell<Vec<Handle>>,
    context: Handle,
    make_current: MakeCurrent,
    get_error: GetError,
    destroy_surface: Destroy,
    destroy_context: Destroy,
    get_proc_address: GetProcAddress,
    // Declared last so it is dropped last: it must outlive every function pointer
    // above, including the one `Display::drop` calls.
    _library: Library,
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
        let terminate = load!(c"eglTerminate", Terminate);
        let bind_api = load!(c"eglBindAPI", BindApi);
        let choose_config = load!(c"eglChooseConfig", ChooseConfig);
        let create_surface = load!(c"eglCreatePbufferSurface", CreateSurface);
        let create_context = load!(c"eglCreateContext", CreateContext);
        let get_error = load!(c"eglGetError", GetError);
        // SAFETY: eglGetDisplay is a fixed EGL entry point taking a native display or null.
        let handle = unsafe { get_display(ptr::null_mut()) };
        if handle.is_null() {
            return Err("eglGetDisplay returned null".into());
        }
        let display = Display::acquire(handle, initialize, terminate).map_err(|error| {
            // SAFETY: eglGetError reads the calling thread's last EGL error.
            format!("{error}: {:#x}; try EGL_PLATFORM=surfaceless", unsafe {
                get_error()
            })
        })?;
        let mut this = Self {
            make_current: load!(c"eglMakeCurrent", MakeCurrent),
            get_error: get_error,
            destroy_surface: load!(c"eglDestroySurface", Destroy),
            destroy_context: load!(c"eglDestroyContext", Destroy),
            get_proc_address: load!(c"eglGetProcAddress", GetProcAddress),
            display,
            surface: Cell::new(ptr::null_mut()),
            size: Cell::new(size),
            config: ptr::null_mut(),
            create_surface,
            retired: RefCell::new(Vec::new()),
            context: ptr::null_mut(),
            _library: library,
            _thread: PhantomData,
        };
        // SAFETY: EGL handles originate in this library; all attribute arrays end in
        // EGL_NONE. Output pointers refer to initialized stack storage. Drop cleans
        // partially created resources in reverse order on every error path.
        unsafe {
            let display = this.display.handle;
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
                // Destination alpha, as the GLX and WGL windows have: without it the
                // framebuffer reports alpha 255 everywhere and a translucent overlay's
                // `glReadPixels` is opaque. Blending to alpha is part of the contract.
                0x3021, 8, // ALPHA_SIZE
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
            this.config = config;
            this.surface
                .set(create_surface(display, config, size.as_ptr()));
            if this.surface.get().is_null() {
                return Err(format!(
                    "eglCreatePbufferSurface failed: {:#x}",
                    get_error()
                ));
            }
            this.context = create_context(display, config, ptr::null_mut(), [0x3038].as_ptr());
            if this.context.is_null()
                || (this.make_current)(
                    display,
                    this.surface.get(),
                    this.surface.get(),
                    this.context,
                ) == 0
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
    fn drawable_size(&self) -> Option<[u32; 2]> {
        Some(self.size.get())
    }
    fn resize(&self, size: [u32; 2]) -> Result<(), String> {
        // Bound deferred surfaces to one failed retirement: do not stage another
        // allocation until the previous cleanup succeeds.
        loop {
            let Some(surface) = self.retired.borrow().last().copied() else {
                break;
            };
            // SAFETY: non-current owned surface on this initialized display.
            if unsafe { (self.destroy_surface)(self.display.handle, surface) } == 0 {
                return Err("eglDestroySurface retirement failed".into());
            }
            self.retired.borrow_mut().pop();
        }
        let attributes = [0x3057, size[0] as i32, 0x3056, size[1] as i32, 0x3038];
        // SAFETY: retained matching config, validated dimensions, terminated list.
        let surface =
            unsafe { (self.create_surface)(self.display.handle, self.config, attributes.as_ptr()) };
        if surface.is_null() {
            return Err("EGL resize surface allocation failed".into());
        }
        // SAFETY: live context/new surface from the same config/display.
        if unsafe { (self.make_current)(self.display.handle, surface, surface, self.context) } == 0
        {
            // A failed MakeCurrent leaves the old binding valid.
            if unsafe { (self.destroy_surface)(self.display.handle, surface) } == 0 {
                self.retired.borrow_mut().push(surface);
            }
            return Err("EGL resize MakeCurrent failed".into());
        }
        let old = self.surface.replace(surface);
        self.size.set(size);
        // Native cleanup failure is retained and blocks the next resize, but the
        // new drawable is live and its actual dimensions are already observable.
        if unsafe { (self.destroy_surface)(self.display.handle, old) } == 0 {
            self.retired.borrow_mut().push(old);
        }
        Ok(())
    }
    fn make_current(&self) -> Result<(), String> {
        // SAFETY: owned EGL handles on the creating thread.
        if unsafe {
            (self.make_current)(
                self.display.handle,
                self.surface.get(),
                self.surface.get(),
                self.context,
            )
        } == 0
        {
            // The code is the only diagnosis a machine without the probe has: keep it.
            // SAFETY: eglGetError is a thread-local read with no arguments.
            Err(format!("eglMakeCurrent failed: {:#x}", unsafe {
                (self.get_error)()
            }))
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
            if !self.display.handle.is_null() {
                (self.make_current)(
                    self.display.handle,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                if !self.context.is_null() {
                    (self.destroy_context)(self.display.handle, self.context);
                }
                if !self.surface.get().is_null() {
                    (self.destroy_surface)(self.display.handle, self.surface.get());
                }
                for surface in self.retired.get_mut().drain(..) {
                    (self.destroy_surface)(self.display.handle, surface);
                }
                // The display outlives this context only if another one still uses it;
                // `Display::drop` terminates it with the last user.
            }
        }
    }
}
