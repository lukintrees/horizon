//! WGL for the OpenGL 2 path: a legacy compatibility context on the game's own window. Catalyst
//! offers OpenGL 2.1 on Windows this way, with no EGL. Only the Win32 calls are declared here, so the
//! crate gains no dependency. The calls are `extern "system"`: stdcall on 32-bit Windows (the Windows 7
//! target), the x64 convention on 64-bit.

use std::ffi::{CString, c_void};
use std::mem::size_of;
use std::ptr;

type Hwnd = *mut c_void;
type Hdc = *mut c_void;
type Hglrc = *mut c_void;
type Hmodule = *mut c_void;

const PFD_DRAW_TO_WINDOW: u32 = 0x0000_0004;
const PFD_SUPPORT_OPENGL: u32 = 0x0000_0020;
const PFD_DOUBLEBUFFER: u32 = 0x0000_0001;
const PFD_TYPE_RGBA: u8 = 0;
const PFD_MAIN_PLANE: u8 = 0;

/// PIXELFORMATDESCRIPTOR, the ANSI layout: 40 bytes, checked below.
#[repr(C)]
#[derive(Default)]
struct PixelFormatDescriptor {
    n_size: u16,
    n_version: u16,
    dw_flags: u32,
    i_pixel_type: u8,
    c_color_bits: u8,
    c_red_bits: u8,
    c_red_shift: u8,
    c_green_bits: u8,
    c_green_shift: u8,
    c_blue_bits: u8,
    c_blue_shift: u8,
    c_alpha_bits: u8,
    c_alpha_shift: u8,
    c_accum_bits: u8,
    c_accum_red_bits: u8,
    c_accum_green_bits: u8,
    c_accum_blue_bits: u8,
    c_accum_alpha_bits: u8,
    c_depth_bits: u8,
    c_stencil_bits: u8,
    c_aux_buffers: u8,
    i_layer_type: u8,
    b_reserved: u8,
    dw_layer_mask: u32,
    dw_visible_mask: u32,
    dw_damage_mask: u32,
}

const _: () = assert!(size_of::<PixelFormatDescriptor>() == 40);

#[link(name = "user32")]
unsafe extern "system" {
    fn GetDC(hwnd: Hwnd) -> Hdc;
    fn ReleaseDC(hwnd: Hwnd, hdc: Hdc) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn ChoosePixelFormat(hdc: Hdc, pfd: *const PixelFormatDescriptor) -> i32;
    fn SetPixelFormat(hdc: Hdc, format: i32, pfd: *const PixelFormatDescriptor) -> i32;
    fn GetPixelFormat(hdc: Hdc) -> i32;
    fn SwapBuffers(hdc: Hdc) -> i32;
}

#[link(name = "opengl32")]
unsafe extern "system" {
    fn wglCreateContext(hdc: Hdc) -> Hglrc;
    fn wglMakeCurrent(hdc: Hdc, glrc: Hglrc) -> i32;
    fn wglDeleteContext(glrc: Hglrc) -> i32;
    fn wglGetProcAddress(name: *const i8) -> *const c_void;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Hmodule;
    fn GetProcAddress(module: Hmodule, name: *const i8) -> *const c_void;
    fn GetLastError() -> u32;
}

/// wglGetProcAddress returns 0, 1, 2, 3 or -1 for names it does not know.
fn is_real_proc(p: *const c_void) -> bool {
    !matches!(p as isize, 0 | 1 | 2 | 3 | -1)
}

fn last_error(what: &str) -> String {
    format!("{what} failed (Win32 error {})", unsafe { GetLastError() })
}

/// A legacy OpenGL context on a window, current on the thread that made it. Dropping it releases
/// the context and the device context; the window itself is the game's and stays.
pub struct Context {
    hwnd: Hwnd,
    hdc: Hdc,
    glrc: Hglrc,
}

impl Context {
    /// Makes a context on `hwnd` and makes it current. A window keeps the pixel format it was first
    /// given, so one that already has a format (the context made before, say) keeps it.
    ///
    /// # Safety
    /// `hwnd` must be a live `HWND` on this thread, and it must outlive the returned context.
    pub unsafe fn attach(hwnd: isize) -> Result<Self, String> {
        let hwnd = hwnd as Hwnd;
        // SAFETY: the caller guarantees the window is live.
        let hdc = unsafe { GetDC(hwnd) };
        if hdc.is_null() {
            return Err(last_error("GetDC"));
        }
        let mut context = Context {
            hwnd,
            hdc,
            glrc: ptr::null_mut(),
        };
        // SAFETY: `hdc` is the window's device context.
        if unsafe { GetPixelFormat(hdc) } == 0 {
            let pfd = PixelFormatDescriptor {
                n_size: size_of::<PixelFormatDescriptor>() as u16,
                n_version: 1,
                dw_flags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                i_pixel_type: PFD_TYPE_RGBA,
                c_color_bits: 32,
                c_depth_bits: 24,
                i_layer_type: PFD_MAIN_PLANE,
                ..Default::default()
            };
            // SAFETY: `pfd` lives across both calls.
            let format = unsafe { ChoosePixelFormat(hdc, &pfd) };
            if format == 0 {
                return Err(last_error("ChoosePixelFormat"));
            }
            if unsafe { SetPixelFormat(hdc, format, &pfd) } == 0 {
                return Err(last_error("SetPixelFormat"));
            }
        }
        // SAFETY: the device context has a pixel format.
        let glrc = unsafe { wglCreateContext(hdc) };
        if glrc.is_null() {
            return Err(last_error("wglCreateContext"));
        }
        context.glrc = glrc;
        if unsafe { wglMakeCurrent(hdc, glrc) } == 0 {
            return Err(last_error("wglMakeCurrent"));
        }
        Ok(context)
    }

    /// Makes this context current on the calling thread. False when Windows refuses.
    pub fn make_current(&self) -> bool {
        unsafe { wglMakeCurrent(self.hdc, self.glrc) != 0 }
    }

    /// Presents the back buffer. False when the system refuses the swap.
    pub fn swap(&self) -> bool {
        unsafe { SwapBuffers(self.hdc) != 0 }
    }

    /// Looks up a GL function. Core and 1.1 functions are not returned by wglGetProcAddress, so they
    /// come from the opengl32.dll export table.
    pub fn proc_address(&self, name: &str) -> *const c_void {
        let Ok(cname) = CString::new(name) else {
            return ptr::null();
        };
        let p = unsafe { wglGetProcAddress(cname.as_ptr()) };
        if is_real_proc(p) {
            return p;
        }
        let library: Vec<u16> = "opengl32.dll"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let module = unsafe { GetModuleHandleW(library.as_ptr()) };
        if module.is_null() {
            return ptr::null();
        }
        unsafe { GetProcAddress(module, cname.as_ptr()) }
    }

    /// The GL functions of this context. Valid while the context is current on this thread.
    pub fn glow(&self) -> glow::Context {
        // SAFETY: the loader only looks names up; the context is current when GL is called.
        unsafe { glow::Context::from_loader_function(|name| self.proc_address(name)) }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            if !self.glrc.is_null() {
                wglMakeCurrent(ptr::null_mut(), ptr::null_mut());
                wglDeleteContext(self.glrc);
            }
            if !self.hdc.is_null() {
                ReleaseDC(self.hwnd, self.hdc);
            }
        }
    }
}
