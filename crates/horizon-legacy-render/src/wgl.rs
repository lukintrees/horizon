//! Native desktop WGL bootstrap, using APIs available on Windows 7.
//! This is a short-lived window diagnostic, not a game event loop. Raw Win32 ABI
//! declarations avoid pulling modern window/runtime dependencies into the probe.
use std::ffi::{CStr, c_void};
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;

type Handle = *mut c_void;
type WindowProc = unsafe extern "system" fn(Handle, u32, usize, isize) -> isize;

#[repr(C)]
struct WindowClass {
    style: u32,
    procedure: Option<WindowProc>,
    class_extra: i32,
    window_extra: i32,
    instance: Handle,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu_name: *const u16,
    class_name: *const u16,
}

#[repr(C)]
#[derive(Default)]
struct PixelFormat {
    size: u16,
    version: u16,
    flags: u32,
    pixel_type: u8,
    color_bits: u8,
    channel_bits_and_shifts: [u8; 8],
    accum_bits: [u8; 5],
    depth_bits: u8,
    stencil_bits: u8,
    aux_buffers: u8,
    layer_type: u8,
    reserved: u8,
    layer_mask: u32,
    visible_mask: u32,
    damage_mask: u32,
}

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

// ABI layout guards, evaluated by cargo check for each Windows target as well.
const _: () = {
    assert!(size_of::<PixelFormat>() == 40);
    assert!(std::mem::offset_of!(PixelFormat, depth_bits) == 23);
    assert!(size_of::<WindowClass>() == if size_of::<usize>() == 4 { 40 } else { 72 });
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const std::ffi::c_char) -> *const c_void;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClassW(class: *const WindowClass) -> u16;
    fn UnregisterClassW(name: *const u16, instance: Handle) -> i32;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Handle,
        menu: Handle,
        instance: Handle,
        parameter: Handle,
    ) -> Handle;
    fn DefWindowProcW(window: Handle, message: u32, wparam: usize, lparam: isize) -> isize;
    fn ShowWindow(window: Handle, command: i32) -> i32;
    fn UpdateWindow(window: Handle) -> i32;
    fn DestroyWindow(window: Handle) -> i32;
    fn GetDC(window: Handle) -> Handle;
    fn ReleaseDC(window: Handle, dc: Handle) -> i32;
    fn AdjustWindowRect(rect: *mut Rect, style: u32, menu: i32) -> i32;
}
#[link(name = "gdi32")]
unsafe extern "system" {
    fn ChoosePixelFormat(dc: Handle, descriptor: *const PixelFormat) -> i32;
    fn DescribePixelFormat(
        dc: Handle,
        format: i32,
        bytes: u32,
        descriptor: *mut PixelFormat,
    ) -> i32;
    fn SetPixelFormat(dc: Handle, format: i32, descriptor: *const PixelFormat) -> i32;
    fn SwapBuffers(dc: Handle) -> i32;
}
#[link(name = "opengl32")]
unsafe extern "system" {
    fn wglCreateContext(dc: Handle) -> Handle;
    fn wglMakeCurrent(dc: Handle, context: Handle) -> i32;
    fn wglDeleteContext(context: Handle) -> i32;
    fn wglGetProcAddress(name: *const std::ffi::c_char) -> *const c_void;
}

fn failure(operation: &str) -> String {
    format!("{operation} failed: {}", std::io::Error::last_os_error())
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub struct Context {
    instance: Handle,
    opengl: Handle, // borrowed from the loader, not a LoadLibrary reference
    class_name: Vec<u16>,
    registered: bool,
    window: Handle,
    dc: Handle,
    context: Handle,
    _thread: PhantomData<Rc<()>>,
}
impl Context {
    pub fn new(size: [u32; 2]) -> Result<Self, String> {
        let mut this = Self {
            instance: ptr::null_mut(),
            opengl: ptr::null_mut(),
            class_name: wide(&format!("HorizonLegacyGlProbe-{}", std::process::id())),
            registered: false,
            window: ptr::null_mut(),
            dc: ptr::null_mut(),
            context: ptr::null_mut(),
            _thread: PhantomData,
        };
        // SAFETY: fixed Win32 ABI; NUL-terminated UTF-16 arrays stay alive through
        // calls/registration. Outputs point to live repr(C) structs. Handles are
        // checked before use, thread-confined, and cleaned by Drop on all exits.
        unsafe {
            this.instance = GetModuleHandleW(ptr::null());
            this.opengl = GetModuleHandleW(wide("opengl32.dll").as_ptr());
            if this.instance.is_null() || this.opengl.is_null() {
                return Err(failure("GetModuleHandleW"));
            }
            let class = WindowClass {
                style: 0x0020, // CS_OWNDC: stable pixel format and GL DC
                procedure: Some(DefWindowProcW),
                class_extra: 0,
                window_extra: 0,
                instance: this.instance,
                icon: ptr::null_mut(),
                cursor: ptr::null_mut(),
                background: ptr::null_mut(),
                menu_name: ptr::null(),
                class_name: this.class_name.as_ptr(),
            };
            if RegisterClassW(&class) == 0 {
                return Err(failure("RegisterClassW"));
            }
            this.registered = true;
            let style = 0x00CF0000; // WS_OVERLAPPEDWINDOW; intentionally not WS_VISIBLE
            let mut rect = Rect {
                left: 0,
                top: 0,
                right: size[0] as i32,
                bottom: size[1] as i32,
            };
            if AdjustWindowRect(&mut rect, style, 0) == 0 {
                return Err(failure("AdjustWindowRect"));
            }
            this.window = CreateWindowExW(
                0,
                this.class_name.as_ptr(),
                this.class_name.as_ptr(),
                style,
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                ptr::null_mut(),
                ptr::null_mut(),
                this.instance,
                ptr::null_mut(),
            );
            if this.window.is_null() {
                return Err(failure("CreateWindowExW"));
            }
            this.dc = GetDC(this.window);
            if this.dc.is_null() {
                return Err(failure("GetDC"));
            }
            let requested = PixelFormat {
                size: size_of::<PixelFormat>() as u16,
                version: 1,
                flags: 0x0004 | 0x0020 | 0x0001, // DRAW_TO_WINDOW | SUPPORT_OPENGL | DOUBLEBUFFER
                color_bits: 24,
                depth_bits: 24,
                ..PixelFormat::default() // TYPE_RGBA, MAIN_PLANE
            };
            let format = ChoosePixelFormat(this.dc, &requested);
            if format == 0 {
                return Err(failure("ChoosePixelFormat"));
            }
            let mut actual = PixelFormat::default();
            if DescribePixelFormat(
                this.dc,
                format,
                size_of::<PixelFormat>() as u32,
                &mut actual,
            ) == 0
            {
                return Err(failure("DescribePixelFormat"));
            }
            if actual.flags & 0x0040 != 0 {
                // PFD_GENERIC_FORMAT: no full hardware ICD
                return Err("generic GDI pixel format selected; install a matching 32-bit vendor OpenGL ICD for the x86 build".into());
            }
            if actual.flags & requested.flags != requested.flags
                || actual.pixel_type != 0
                || actual.depth_bits < 16
            {
                return Err(
                    "driver has no suitable double-buffered RGBA/depth pixel format".into(),
                );
            }
            if SetPixelFormat(this.dc, format, &actual) == 0 {
                return Err(failure("SetPixelFormat"));
            }
            this.context = wglCreateContext(this.dc); // no GL3 context/profile extension required
            if this.context.is_null() {
                return Err(failure("wglCreateContext"));
            }
            if wglMakeCurrent(this.dc, this.context) == 0 {
                return Err(failure("wglMakeCurrent"));
            }
            // A hidden/fully obscured window has no guaranteed readable pixels
            // under legacy pixel-ownership rules. Show it before draw/readback.
            ShowWindow(this.window, 4); // SW_SHOWNOACTIVATE, previous visibility is not an error
            if UpdateWindow(this.window) == 0 {
                return Err(failure("UpdateWindow"));
            }
        }
        Ok(this)
    }
}
impl crate::ContextImpl for Context {
    fn make_current(&self) -> Result<(), String> {
        // SAFETY: live owned DC/context on the creating thread.
        if unsafe { wglMakeCurrent(self.dc, self.context) } == 0 {
            Err(failure("wglMakeCurrent"))
        } else {
            Ok(())
        }
    }

    fn description(&self) -> &str {
        "WGL desktop OpenGL window"
    }
    fn address(&self, name: &CStr) -> Result<*const c_void, String> {
        // SAFETY: current context on this thread; borrowed module stays loaded
        // through the executable's opengl32 import. Do not cast WGL sentinels to fn.
        let address = unsafe {
            crate::wgl_address::resolve(wglGetProcAddress(name.as_ptr()), || {
                GetProcAddress(self.opengl, name.as_ptr())
            })
        };
        address.ok_or_else(|| {
            format!(
                "missing GL entry point {} (WGL and opengl32.dll)",
                name.to_string_lossy()
            )
        })
    }
    fn present(&self) -> Result<bool, String> {
        // SAFETY: live, double-buffered window DC on the creating thread. Probe
        // readbacks happened before swapping, while the default back buffer exists.
        if unsafe { SwapBuffers(self.dc) } == 0 {
            Err(failure("SwapBuffers"))
        } else {
            Ok(true)
        }
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: reverse teardown of uniquely owned handles, on their creating
        // thread; no window message loop can destroy this window earlier.
        unsafe {
            if !self.context.is_null() {
                wglMakeCurrent(ptr::null_mut(), ptr::null_mut());
                wglDeleteContext(self.context);
            }
            if !self.dc.is_null() {
                ReleaseDC(self.window, self.dc);
            }
            if !self.window.is_null() {
                DestroyWindow(self.window);
            }
            if self.registered {
                UnregisterClassW(self.class_name.as_ptr(), self.instance);
            }
        }
    }
}
