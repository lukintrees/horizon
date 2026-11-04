//! WGL context for Windows: a hidden window, a pixel format and an OpenGL 2.1 compatibility
//! context. This is how Catalyst exposes OpenGL on Windows; EGL is not offered there.
//!
//! Only the Win32 calls the probe needs are declared here, so the crate gains no dependency.
//! Everything is `extern "system"`: stdcall on 32-bit Windows (the Windows 7 target), the x64
//! convention on 64-bit.

use std::ffi::{c_void, CString};
use std::mem::size_of;
use std::ptr;

type Hwnd = *mut c_void;
type Hdc = *mut c_void;
type Hglrc = *mut c_void;
type Hinstance = *mut c_void;
type Hmodule = *mut c_void;
type Hmenu = *mut c_void;
type Hicon = *mut c_void;
type Hcursor = *mut c_void;
type Hbrush = *mut c_void;

type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;
type CreateContextAttribs = unsafe extern "system" fn(Hdc, Hglrc, *const i32) -> Hglrc;

const WS_POPUP: u32 = 0x8000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const PFD_DRAW_TO_WINDOW: u32 = 0x0000_0004;
const PFD_SUPPORT_OPENGL: u32 = 0x0000_0020;
const PFD_DOUBLEBUFFER: u32 = 0x0000_0001;
const PFD_TYPE_RGBA: u8 = 0;
const PFD_MAIN_PLANE: u8 = 0;

const WGL_CONTEXT_MAJOR_VERSION_ARB: i32 = 0x2091;
const WGL_CONTEXT_MINOR_VERSION_ARB: i32 = 0x2092;
const WGL_CONTEXT_PROFILE_MASK_ARB: i32 = 0x9126;
const WGL_CONTEXT_COMPATIBILITY_PROFILE_BIT_ARB: i32 = 0x0000_0002;

#[repr(C)]
struct WndClassExW {
    cb_size: u32,
    style: u32,
    lpfn_wnd_proc: WndProc,
    cb_cls_extra: i32,
    cb_wnd_extra: i32,
    h_instance: Hinstance,
    h_icon: Hicon,
    h_cursor: Hcursor,
    hbr_background: Hbrush,
    lpsz_menu_name: *const u16,
    lpsz_class_name: *const u16,
    h_icon_sm: Hicon,
}

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
extern "system" {
    fn RegisterClassExW(class: *const WndClassExW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: Hmenu,
        instance: Hinstance,
        param: *mut c_void,
    ) -> Hwnd;
    fn DestroyWindow(hwnd: Hwnd) -> i32;
    fn SetProcessDPIAware() -> i32;
    fn GetDC(hwnd: Hwnd) -> Hdc;
    fn ReleaseDC(hwnd: Hwnd, hdc: Hdc) -> i32;
    fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
}

#[link(name = "gdi32")]
extern "system" {
    fn ChoosePixelFormat(hdc: Hdc, pfd: *const PixelFormatDescriptor) -> i32;
    fn SetPixelFormat(hdc: Hdc, format: i32, pfd: *const PixelFormatDescriptor) -> i32;
    fn SwapBuffers(hdc: Hdc) -> i32;
}

#[link(name = "opengl32")]
extern "system" {
    fn wglCreateContext(hdc: Hdc) -> Hglrc;
    fn wglMakeCurrent(hdc: Hdc, glrc: Hglrc) -> i32;
    fn wglDeleteContext(glrc: Hglrc) -> i32;
    fn wglGetProcAddress(name: *const i8) -> *const c_void;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Hmodule;
    fn GetProcAddress(module: Hmodule, name: *const i8) -> *const c_void;
    fn GetLastError() -> u32;
}

unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// wglGetProcAddress returns 0, 1, 2, 3 or -1 for names it does not know.
fn is_real_proc(p: *const c_void) -> bool {
    !matches!(p as isize, 0 | 1 | 2 | 3 | -1)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error(what: &str) -> String {
    format!("{what} failed (Win32 error {})", unsafe { GetLastError() })
}

/// A current OpenGL context on a hidden window. Dropping it releases everything.
pub struct Context {
    hwnd: Hwnd,
    hdc: Hdc,
    glrc: Hglrc,
    /// Why the OpenGL 2.1 request was not honoured, when the legacy context is in use.
    pub fallback: Option<String>,
}

impl Context {
    /// Creates the window, the pixel format and a 2.1 compatibility context, and makes it current.
    /// Falls back to the legacy context when `wglCreateContextAttribsARB` refuses 2.1; the reason
    /// is kept in `fallback` so the report can say so.
    /// A hidden window: for the probe and the CI runs.
    pub fn new() -> Result<Self, String> {
        Self::create(None)
    }

    /// A visible window whose client area is `width` x `height`, with no border, so the default
    /// framebuffer is exactly that size. The process is made DPI-aware first; otherwise Windows
    /// scales the window and the default framebuffer no longer matches the pixels the probe
    /// expects.
    pub fn new_window(width: i32, height: i32) -> Result<Self, String> {
        Self::create(Some((width, height)))
    }

    fn create(window: Option<(i32, i32)>) -> Result<Self, String> {
        let (style, width, height) = match window {
            Some((width, height)) => (WS_POPUP | WS_VISIBLE, width, height),
            None => (WS_POPUP, 1, 1),
        };
        if window.is_some() {
            // Failure only means the call was not needed (already aware, or not supported).
            unsafe { SetProcessDPIAware() };
        }
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        let class_name = wide("HorizonGl2Probe");
        let class = WndClassExW {
            cb_size: size_of::<WndClassExW>() as u32,
            style: 0,
            lpfn_wnd_proc: wnd_proc,
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: instance,
            h_icon: ptr::null_mut(),
            h_cursor: ptr::null_mut(),
            hbr_background: ptr::null_mut(),
            lpsz_menu_name: ptr::null(),
            lpsz_class_name: class_name.as_ptr(),
            h_icon_sm: ptr::null_mut(),
        };
        // Registering twice fails with ERROR_CLASS_ALREADY_EXISTS, which is harmless here.
        unsafe { RegisterClassExW(&class) };

        let title = wide("gl2-probe");
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                style,
                0,
                0,
                width,
                height,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null_mut(),
            )
        };
        if hwnd.is_null() {
            return Err(last_error("CreateWindowExW"));
        }
        // From here on, `ctx` owns the window and Drop cleans up whatever was created.
        let hdc = unsafe { GetDC(hwnd) };
        let mut ctx = Context { hwnd, hdc, glrc: ptr::null_mut(), fallback: None };
        if hdc.is_null() {
            return Err(last_error("GetDC"));
        }

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
        let format = unsafe { ChoosePixelFormat(hdc, &pfd) };
        if format == 0 {
            return Err(last_error("ChoosePixelFormat"));
        }
        if unsafe { SetPixelFormat(hdc, format, &pfd) } == 0 {
            return Err(last_error("SetPixelFormat"));
        }

        // Step one: a legacy context, which is needed to fetch the WGL extension entry points.
        let legacy = unsafe { wglCreateContext(hdc) };
        if legacy.is_null() {
            return Err(last_error("wglCreateContext"));
        }
        ctx.glrc = legacy;
        if unsafe { wglMakeCurrent(hdc, legacy) } == 0 {
            return Err(last_error("wglMakeCurrent"));
        }

        // Step two: the 2.1 compatibility context, if the driver offers the extension.
        let create_name = CString::new("wglCreateContextAttribsARB").expect("no NUL in name");
        let create_ptr = unsafe { wglGetProcAddress(create_name.as_ptr()) };
        if !is_real_proc(create_ptr) {
            ctx.fallback = Some("wglCreateContextAttribsARB not exported; legacy context".into());
            return Ok(ctx);
        }
        // SAFETY: the pointer came from wglGetProcAddress for this exact entry point.
        let create: CreateContextAttribs = unsafe { std::mem::transmute(create_ptr) };
        let attributes = [
            WGL_CONTEXT_MAJOR_VERSION_ARB,
            2,
            WGL_CONTEXT_MINOR_VERSION_ARB,
            1,
            WGL_CONTEXT_PROFILE_MASK_ARB,
            WGL_CONTEXT_COMPATIBILITY_PROFILE_BIT_ARB,
            0,
        ];
        let versioned = unsafe { create(hdc, ptr::null_mut(), attributes.as_ptr()) };
        if versioned.is_null() {
            let code = unsafe { GetLastError() };
            ctx.fallback = Some(format!("wglCreateContextAttribsARB(2.1) failed (Win32 error {code}); legacy context"));
            return Ok(ctx);
        }
        // Make the 2.1 context the owned one before the legacy one goes, so Drop never frees
        // a handle twice.
        ctx.glrc = versioned;
        unsafe {
            wglMakeCurrent(hdc, ptr::null_mut());
            wglDeleteContext(legacy);
        }
        if unsafe { wglMakeCurrent(hdc, versioned) } == 0 {
            return Err(last_error("wglMakeCurrent (2.1)"));
        }
        Ok(ctx)
    }

    /// Presents the back buffer. False when the system refuses the swap.
    pub fn swap(&self) -> bool {
        unsafe { SwapBuffers(self.hdc) != 0 }
    }

    /// Looks up a GL function. Core and 1.1 functions are not returned by wglGetProcAddress,
    /// so they come from the opengl32.dll export table.
    pub fn proc_address(&self, name: &str) -> *const c_void {
        let Ok(cname) = CString::new(name) else {
            return ptr::null();
        };
        let p = unsafe { wglGetProcAddress(cname.as_ptr()) };
        if is_real_proc(p) {
            return p;
        }
        let library = wide("opengl32.dll");
        let module = unsafe { GetModuleHandleW(library.as_ptr()) };
        if module.is_null() {
            return ptr::null();
        }
        unsafe { GetProcAddress(module, cname.as_ptr()) }
    }

    pub fn glow(&self) -> glow::Context {
        // SAFETY: the context is current on this thread for as long as `self` lives.
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
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
        }
    }
}
