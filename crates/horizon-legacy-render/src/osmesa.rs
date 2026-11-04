//! OSMesa is an alternative Linux test fixture, not a target GPU emulation.
use crate::egl::Library;
use std::ffi::{CStr, CString, c_char, c_void};
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;

type Handle = *mut c_void;
type Create = unsafe extern "C" fn(u32, i32, i32, i32, Handle) -> Handle;
type MakeCurrent = unsafe extern "C" fn(Handle, *mut c_void, u32, i32, i32) -> u8;
type Destroy = unsafe extern "C" fn(Handle);
type GetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;

pub struct Context {
    _library: Library,
    handle: Handle,
    _pixels: Vec<u8>,
    destroy: Destroy,
    make_current: MakeCurrent,
    size: [u32; 2],
    pixels: *mut c_void,
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
            size,
            pixels: ptr::null_mut(),
            _library: library,
            handle: ptr::null_mut(),
            _pixels: vec![0; size[0] as usize * size[1] as usize * 4],
            _thread: PhantomData,
        };
        this.pixels = this._pixels.as_mut_ptr().cast();
        // SAFETY: RGBA format, 24 depth bits, no shared context; framebuffer owns
        // width*height*4 bytes and stays allocated until after context destruction.
        unsafe {
            this.handle = create(0x1908, 24, 0, 0, ptr::null_mut());
            if this.handle.is_null() {
                return Err("OSMesaCreateContextExt failed".into());
            }
            if make_current(
                this.handle,
                this._pixels.as_mut_ptr().cast(),
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
        // SAFETY: framebuffer allocation never moves/resizes and is not exposed;
        // only the driver accesses these bytes until context destruction.
        if unsafe {
            (self.make_current)(
                self.handle,
                self.pixels,
                0x1401,
                self.size[0] as i32,
                self.size[1] as i32,
            )
        } == 0
        {
            Err("OSMesaMakeCurrent failed".into())
        } else {
            Ok(())
        }
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
