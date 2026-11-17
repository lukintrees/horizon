//! OSMesa is an alternative Linux test fixture, not a target GPU emulation.
use crate::egl::Library;
use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_void};
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;

type Handle = *mut c_void;
type Create = unsafe extern "C" fn(u32, i32, i32, i32, Handle) -> Handle;
type MakeCurrent = unsafe extern "C" fn(Handle, *mut c_void, u32, i32, i32) -> u8;
type Destroy = unsafe extern "C" fn(Handle);
type GetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;

struct Buffer {
    pixels: Vec<u8>,
    size: [u32; 2],
}
pub struct Context {
    _library: Library,
    handle: Handle,
    buffer: RefCell<Buffer>,
    destroy: Destroy,
    make_current: MakeCurrent,

    get_proc_address: GetProcAddress,
    _thread: PhantomData<Rc<()>>,
}
impl Context {
    pub fn new(size: [u32; 2]) -> Result<Self, String> {
        let name = std::env::var("HORIZON_OSMESA_LIBRARY").unwrap_or("libOSMesa.so.8".into());
        let name = CString::new(name).map_err(|e| e.to_string())?;
        let library = Library::open(&name)?;
        macro_rules! load {
            ($name:literal, $ty:ty) => {{
                let address = library.symbol($name)?;
                // SAFETY: fixed names and signatures from the OSMesa C ABI.
                unsafe { std::mem::transmute::<*const c_void, $ty>(address) }
            }};
        }
        let create = load!(c"OSMesaCreateContextExt", Create);
        let make_current = load!(c"OSMesaMakeCurrent", MakeCurrent);
        let mut this = Self {
            destroy: load!(c"OSMesaDestroyContext", Destroy),
            get_proc_address: load!(c"OSMesaGetProcAddress", GetProcAddress),
            make_current,
            _library: library,
            handle: ptr::null_mut(),
            buffer: RefCell::new(Buffer {
                size,
                pixels: vec![0; size[0] as usize * size[1] as usize * 4],
            }),
            _thread: PhantomData,
        };
        // SAFETY: RGBA format, 24 depth bits, no shared context; framebuffer owns
        // width*height*4 bytes and stays allocated until after context destruction.
        unsafe {
            this.handle = create(0x1908, 24, 0, 0, ptr::null_mut());
            if this.handle.is_null() {
                return Err("OSMesaCreateContextExt failed".into());
            }
            if make_current(
                this.handle,
                this.buffer.get_mut().pixels.as_mut_ptr().cast(),
                0x1401,
                size[0] as i32,
                size[1] as i32,
            ) == 0
            {
                return Err("OSMesaMakeCurrent failed".into());
            }
        }
        Ok(this)
    }
}
impl crate::ContextImpl for Context {
    fn make_current(&self) -> Result<(), String> {
        let mut buffer = self.buffer.borrow_mut();
        // SAFETY: owned storage is not exposed to Rust readers. Only the driver
        // accesses its payload, and resize finishes prior work before replacing it.
        if unsafe {
            (self.make_current)(
                self.handle,
                buffer.pixels.as_mut_ptr().cast(),
                0x1401,
                buffer.size[0] as i32,
                buffer.size[1] as i32,
            )
        } == 0
        {
            Err("OSMesaMakeCurrent failed".into())
        } else {
            Ok(())
        }
    }
    fn resize(&self, size: [u32; 2]) -> Result<(), String> {
        let mut pixels = Vec::new();
        let bytes = size[0] as usize * size[1] as usize * 4;
        pixels
            .try_reserve_exact(bytes)
            .map_err(|e| format!("OSMesa resize allocation: {e}"))?;
        pixels.resize(bytes, 0);
        // SAFETY: caller validated size and completed GL work on old storage.
        // Failed MakeCurrent does not establish the new buffer binding.
        if unsafe {
            (self.make_current)(
                self.handle,
                pixels.as_mut_ptr().cast(),
                0x1401,
                size[0] as i32,
                size[1] as i32,
            )
        } == 0
        {
            return Err("OSMesa resize MakeCurrent failed".into());
        }
        *self.buffer.borrow_mut() = Buffer { pixels, size };
        Ok(())
    }

    fn description(&self) -> &str {
        "OSMesa desktop OpenGL memory buffer"
    }
    fn address(&self, name: &CStr) -> Result<*const c_void, String> {
        // SAFETY: live library/context, valid NUL-terminated symbol name.
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
        if !self.handle.is_null() {
            // SAFETY: uniquely owned context; library and framebuffer still live.
            unsafe { (self.destroy)(self.handle) };
        }
    }
}
