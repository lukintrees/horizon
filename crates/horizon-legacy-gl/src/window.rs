//! Native window context bootstrap for the experimental backend.
//!
//! These small wrappers accept an *existing* native window. They do not own it,
//! create an event loop, or make the game use this renderer. The window and its
//! display must outlive the context. Each context is confined to its thread.

use std::{ffi::c_void, marker::PhantomData, rc::Rc};

#[derive(Debug)]
pub enum Error {
    InvalidHandle,
    Native(&'static str),
    UnsupportedVisual,
}

#[cfg(target_os = "linux")]
mod native {
    use super::*;
    use std::ffi::{CStr, c_int, c_long, c_ulong};
    use std::ptr;

    #[repr(C)]
    struct WindowAttributes {
        x: c_int,
        y: c_int,
        width: c_int,
        height: c_int,
        border_width: c_int,
        depth: c_int,
        visual: *mut c_void,
        root: c_ulong,
        class: c_int,
        bit_gravity: c_int,
        win_gravity: c_int,
        backing_store: c_int,
        backing_planes: c_ulong,
        backing_pixel: c_ulong,
        save_under: c_int,
        colormap: c_ulong,
        map_installed: c_int,
        map_state: c_int,
        all_event_masks: c_long,
        your_event_mask: c_long,
        do_not_propagate_mask: c_long,
        override_redirect: c_int,
        screen: *mut c_void,
    }
    #[repr(C)]
    struct VisualInfo {
        visual: *mut c_void,
        visual_id: c_ulong,
        screen: c_int,
        depth: c_int,
        class: c_int,
        red_mask: c_ulong,
        green_mask: c_ulong,
        blue_mask: c_ulong,
        colormap_size: c_int,
        bits_per_rgb: c_int,
    }
    #[link(name = "X11")]
    unsafe extern "C" {
        fn XGetWindowAttributes(
            display: *mut c_void,
            window: c_ulong,
            attributes: *mut WindowAttributes,
        ) -> c_int;
        fn XVisualIDFromVisual(visual: *mut c_void) -> c_ulong;
        fn XGetVisualInfo(
            display: *mut c_void,
            mask: c_long,
            template: *mut VisualInfo,
            count: *mut c_int,
        ) -> *mut VisualInfo;
        fn XFree(data: *mut c_void) -> c_int;
    }
    #[link(name = "GL")]
    unsafe extern "C" {
        fn glXGetConfig(
            display: *mut c_void,
            visual: *mut VisualInfo,
            attr: c_int,
            value: *mut c_int,
        ) -> c_int;
        fn glXCreateContext(
            display: *mut c_void,
            visual: *mut VisualInfo,
            share: *mut c_void,
            direct: c_int,
        ) -> *mut c_void;
        fn glXDestroyContext(display: *mut c_void, context: *mut c_void);
        fn glXMakeCurrent(display: *mut c_void, drawable: c_ulong, context: *mut c_void) -> c_int;
        fn glXGetCurrentContext() -> *mut c_void;
        fn glXSwapBuffers(display: *mut c_void, drawable: c_ulong);
        fn glXGetProcAddressARB(name: *const u8) -> *const c_void;
    }

    /// An Xlib/GLX context for a pre-existing X11 window. Not Wayland.
    pub struct WindowContext {
        display: *mut c_void,
        window: c_ulong,
        context: *mut c_void,
        _thread: PhantomData<Rc<()>>,
    }
    impl WindowContext {
        /// Creates and makes current a double-buffered compatibility context.
        /// The window's visual must support RGBA, double buffering and depth.
        ///
        /// # Safety
        /// `display` and `window` must identify a live Xlib window on this thread,
        /// kept alive and unchanged until this object is dropped. The display
        /// must remain usable throughout. No other thread may use this context.
        pub unsafe fn new_xlib(display: *mut c_void, window: c_ulong) -> Result<Self, Error> {
            if display.is_null() || window == 0 {
                return Err(Error::InvalidHandle);
            }
            unsafe {
                let mut attributes: WindowAttributes = std::mem::zeroed();
                if XGetWindowAttributes(display, window, &mut attributes) == 0
                    || attributes.visual.is_null()
                {
                    return Err(Error::Native("XGetWindowAttributes"));
                }
                let mut template: VisualInfo = std::mem::zeroed();
                template.visual_id = XVisualIDFromVisual(attributes.visual);
                let mut count = 0;
                let visual = XGetVisualInfo(display, 1, &mut template, &mut count); // VisualIDMask
                if visual.is_null() || count == 0 {
                    if !visual.is_null() {
                        XFree(visual.cast());
                    }
                    return Err(Error::UnsupportedVisual);
                }
                let supports = [1, 4, 5, 12].into_iter().all(|attr| {
                    let mut value = 0;
                    glXGetConfig(display, visual, attr, &mut value) == 0
                        && value >= if attr == 12 { 16 } else { 1 }
                }); // GLX_USE_GL, RGBA, DOUBLEBUFFER, DEPTH_SIZE
                if !supports {
                    XFree(visual.cast());
                    return Err(Error::UnsupportedVisual);
                }
                let context = glXCreateContext(display, visual, ptr::null_mut(), 1);
                XFree(visual.cast());
                if context.is_null() {
                    return Err(Error::Native("glXCreateContext"));
                }
                if glXMakeCurrent(display, window, context) == 0 {
                    glXDestroyContext(display, context);
                    return Err(Error::Native("glXMakeCurrent"));
                }
                Ok(Self {
                    display,
                    window,
                    context,
                    _thread: PhantomData,
                })
            }
        }
        /// # Safety
        /// The native window and display must still be valid on this thread.
        pub unsafe fn make_current(&self) -> Result<(), Error> {
            if unsafe { glXMakeCurrent(self.display, self.window, self.context) } == 0 {
                Err(Error::Native("glXMakeCurrent"))
            } else {
                Ok(())
            }
        }
        /// Resolve an OpenGL symbol while this context is current.
        /// # Safety
        /// This context must be current on the calling thread.
        pub unsafe fn resolve(&self, name: &CStr) -> *const c_void {
            unsafe { glXGetProcAddressARB(name.as_ptr().cast()) }
        }
        /// # Safety
        /// This context must be current and its native window must be valid.
        pub unsafe fn swap_buffers(&self) -> Result<(), Error> {
            unsafe { glXSwapBuffers(self.display, self.window) };
            Ok(())
        }
    }
    impl Drop for WindowContext {
        fn drop(&mut self) {
            unsafe {
                if glXGetCurrentContext() == self.context {
                    glXMakeCurrent(self.display, 0, ptr::null_mut());
                }
                glXDestroyContext(self.display, self.context);
            }
        }
    }
}
#[cfg(target_os = "linux")]
pub use native::WindowContext;

#[cfg(target_os = "windows")]
mod native {
    use super::*;
    use std::{
        ffi::{CStr, c_char},
        ptr,
    };

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct PixelFormatDescriptor {
        size: u16,
        version: u16,
        flags: u32,
        pixel_type: u8,
        color_bits: u8,
        red_bits: u8,
        red_shift: u8,
        green_bits: u8,
        green_shift: u8,
        blue_bits: u8,
        blue_shift: u8,
        alpha_bits: u8,
        alpha_shift: u8,
        accum_bits: u8,
        accum_red_bits: u8,
        accum_green_bits: u8,
        accum_blue_bits: u8,
        accum_alpha_bits: u8,
        depth_bits: u8,
        stencil_bits: u8,
        aux_buffers: u8,
        layer_type: u8,
        reserved: u8,
        layer_mask: u32,
        visible_mask: u32,
        damage_mask: u32,
    }
    const DRAW_TO_WINDOW: u32 = 0x4;
    const SUPPORT_OPENGL: u32 = 0x20;
    const DOUBLEBUFFER: u32 = 0x1;
    const FLAGS: u32 = DRAW_TO_WINDOW | SUPPORT_OPENGL | DOUBLEBUFFER;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDC(hwnd: *mut c_void) -> *mut c_void;
        fn ReleaseDC(hwnd: *mut c_void, dc: *mut c_void) -> i32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn ChoosePixelFormat(dc: *mut c_void, descriptor: *const PixelFormatDescriptor) -> i32;
        fn DescribePixelFormat(
            dc: *mut c_void,
            format: i32,
            size: u32,
            descriptor: *mut PixelFormatDescriptor,
        ) -> i32;
        fn GetPixelFormat(dc: *mut c_void) -> i32;
        fn SetPixelFormat(
            dc: *mut c_void,
            format: i32,
            descriptor: *const PixelFormatDescriptor,
        ) -> i32;
        fn SwapBuffers(dc: *mut c_void) -> i32;
    }
    #[link(name = "opengl32")]
    unsafe extern "system" {
        fn wglCreateContext(dc: *mut c_void) -> *mut c_void;
        fn wglDeleteContext(context: *mut c_void) -> i32;
        fn wglMakeCurrent(dc: *mut c_void, context: *mut c_void) -> i32;
        fn wglGetCurrentContext() -> *mut c_void;
        fn wglGetProcAddress(name: *const c_char) -> *const c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *const c_void;
    }

    /// A WGL compatibility context attached to an existing Win32 window.
    pub struct WindowContext {
        hwnd: *mut c_void,
        dc: *mut c_void,
        context: *mut c_void,
        _thread: PhantomData<Rc<()>>,
    }
    impl WindowContext {
        /// Selects a double-buffered RGBA pixel format and makes a context current.
        /// Win32 allows a pixel format to be set only once per window. An already
        /// selected compatible format is reused; it cannot be undone on failure.
        ///
        /// # Safety
        /// `hwnd` must identify a live window on this thread. Keep it alive and
        /// unchanged until this object is dropped. No other thread may use the
        /// context or the window's device context during this object's lifetime.
        pub unsafe fn new_win32(hwnd: *mut c_void) -> Result<Self, Error> {
            if hwnd.is_null() {
                return Err(Error::InvalidHandle);
            }
            unsafe {
                let dc = GetDC(hwnd);
                if dc.is_null() {
                    return Err(Error::Native("GetDC"));
                }
                let result = (|| {
                    let mut desired: PixelFormatDescriptor = std::mem::zeroed();
                    desired.size = std::mem::size_of::<PixelFormatDescriptor>() as u16;
                    desired.version = 1;
                    desired.flags = FLAGS;
                    desired.color_bits = 24;
                    desired.depth_bits = 16;
                    let existing = GetPixelFormat(dc);
                    let format = if existing != 0 {
                        existing
                    } else {
                        ChoosePixelFormat(dc, &desired)
                    };
                    if format == 0 {
                        return Err(Error::Native("ChoosePixelFormat"));
                    }
                    let mut actual: PixelFormatDescriptor = std::mem::zeroed();
                    if DescribePixelFormat(
                        dc,
                        format,
                        std::mem::size_of::<PixelFormatDescriptor>() as u32,
                        &mut actual,
                    ) == 0
                    {
                        return Err(Error::Native("DescribePixelFormat"));
                    }
                    if actual.flags & FLAGS != FLAGS
                        || actual.pixel_type != 0
                        || actual.depth_bits < 16
                    {
                        return Err(Error::UnsupportedVisual);
                    }
                    if existing == 0 && SetPixelFormat(dc, format, &actual) == 0 {
                        return Err(Error::Native("SetPixelFormat"));
                    }
                    let context = wglCreateContext(dc);
                    if context.is_null() {
                        return Err(Error::Native("wglCreateContext"));
                    }
                    if wglMakeCurrent(dc, context) == 0 {
                        wglDeleteContext(context);
                        return Err(Error::Native("wglMakeCurrent"));
                    }
                    Ok(Self {
                        hwnd,
                        dc,
                        context,
                        _thread: PhantomData,
                    })
                })();
                if result.is_err() {
                    ReleaseDC(hwnd, dc);
                }
                result
            }
        }
        /// # Safety
        /// The native window and its device context must still be valid.
        pub unsafe fn make_current(&self) -> Result<(), Error> {
            if unsafe { wglMakeCurrent(self.dc, self.context) } == 0 {
                Err(Error::Native("wglMakeCurrent"))
            } else {
                Ok(())
            }
        }
        /// Resolve WGL extension symbols, falling back to opengl32.dll for core symbols.
        /// # Safety
        /// This context must be current on the calling thread.
        pub unsafe fn resolve(&self, name: &CStr) -> *const c_void {
            unsafe {
                let address = wglGetProcAddress(name.as_ptr());
                if address.is_null() || (address as usize) <= 3 || address as usize == usize::MAX {
                    let module = GetModuleHandleA(c"opengl32.dll".as_ptr());
                    if module.is_null() {
                        ptr::null()
                    } else {
                        GetProcAddress(module, name.as_ptr())
                    }
                } else {
                    address
                }
            }
        }
        /// # Safety
        /// This context must be current and its window must still be valid.
        pub unsafe fn swap_buffers(&self) -> Result<(), Error> {
            if unsafe { SwapBuffers(self.dc) } == 0 {
                Err(Error::Native("SwapBuffers"))
            } else {
                Ok(())
            }
        }
    }
    impl Drop for WindowContext {
        fn drop(&mut self) {
            unsafe {
                if wglGetCurrentContext() == self.context {
                    wglMakeCurrent(ptr::null_mut(), ptr::null_mut());
                }
                wglDeleteContext(self.context);
                ReleaseDC(self.hwnd, self.dc);
            }
        }
    }
}
#[cfg(target_os = "windows")]
pub use native::WindowContext;
