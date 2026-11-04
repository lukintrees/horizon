//! Native desktop WGL bootstrap, using APIs available on Windows 7.
//! Bounded event pumping and resizable drawable ownership. Raw Win32 ABI
//! declarations avoid pulling modern window/runtime dependencies into the probe.
use crate::window::State;
use std::ffi::{CStr, c_void};
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

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

#[repr(C)]
struct CreateStruct {
    parameter: *mut c_void,
    instance: Handle,
    menu: Handle,
    parent: Handle,
    height: i32,
    width: i32,
    y: i32,
    x: i32,
    style: i32,
    name: *const u16,
    class: *const u16,
    ex_style: u32,
}
#[repr(C)]
#[derive(Default)]
struct Message {
    window: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    point: [i32; 2],
    // Reserved tail: allocation also covers SDK variants with lPrivate.
    reserved: u32,
}
static WINDOWS: AtomicU64 = AtomicU64::new(1);
unsafe extern "system" fn window_proc(
    window: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    // SAFETY: this procedure is installed only for our private registered class.
    // The stable Box<State> is passed at creation and outlives DestroyWindow.
    // No State queue borrow is held across any native call/reentrant callback.
    unsafe {
        if message == 0x0081 {
            // WM_NCCREATE
            let create = &*(lparam as *const CreateStruct);
            if create.parameter.is_null() {
                return 0;
            }
            set_window_pointer(window, 0, create.parameter as isize);
            if get_window_pointer(window, 0) != create.parameter as isize {
                return 0;
            }
        }
        let pointer = get_window_pointer(window, 0) as *const State;
        if !pointer.is_null() {
            let state = &*pointer;
            // WM_WINDOWPOSCHANGING's default caption constraint also clamps
            // programmatic sizes, independently of WM_GETMINMAXINFO. Only our
            // validated resize transaction bypasses it; interactive tracking
            // retains Windows' normal limits.
            if message == 0x0046 && state.programmatic_resize.get() {
                return 0;
            }
            let result = state.message(message, wparam, lparam);
            if matches!(message, 0x0201 | 0x0204 | 0x0207 | 0x020b) {
                SetCapture(window);
            }
            if ((matches!(message, 0x0202 | 0x0205 | 0x0208 | 0x020c) && state.buttons.get() == 0)
                || message == 8)
                && GetCapture() == window
            {
                ReleaseCapture();
            }
            if message == 0x0082 {
                set_window_pointer(window, 0, 0);
            } // WM_NCDESTROY
            if let Some(result) = result {
                return result;
            }
        }
        DefWindowProcW(window, message, wparam, lparam)
    }
}

// ABI layout guards, evaluated by cargo check for each Windows target as well.
const _: () = {
    assert!(size_of::<PixelFormat>() == 40);
    assert!(size_of::<CreateStruct>() == if size_of::<usize>() == 4 { 48 } else { 80 });
    assert!(std::mem::offset_of!(Message, wparam) == if size_of::<usize>() == 4 { 8 } else { 16 });
    assert!(size_of::<Message>() == if size_of::<usize>() == 4 { 32 } else { 48 });
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
    #[cfg_attr(target_pointer_width = "32", link_name = "SetWindowLongW")]
    #[cfg_attr(target_pointer_width = "64", link_name = "SetWindowLongPtrW")]
    fn set_window_pointer(window: Handle, index: i32, value: isize) -> isize;
    #[cfg_attr(target_pointer_width = "32", link_name = "GetWindowLongW")]
    #[cfg_attr(target_pointer_width = "64", link_name = "GetWindowLongPtrW")]
    fn get_window_pointer(window: Handle, index: i32) -> isize;
    fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
    fn SetWindowTextW(window: Handle, text: *const u16) -> i32;
    #[cfg(test)]
    fn PostMessageW(window: Handle, message: u32, wparam: usize, lparam: isize) -> i32;
    #[cfg(test)]
    fn IsWindow(window: Handle) -> i32;
    fn IsIconic(window: Handle) -> i32;
    fn GetClientRect(window: Handle, rect: *mut Rect) -> i32;
    fn SetWindowPos(
        window: Handle,
        after: Handle,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    fn PeekMessageW(message: *mut Message, window: Handle, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(message: *const Message) -> i32;
    fn DispatchMessageW(message: *const Message) -> isize;
    fn PostQuitMessage(code: i32);
    fn SetCapture(window: Handle) -> Handle;
    fn GetCapture() -> Handle;
    fn ReleaseCapture() -> i32;
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
    state: Box<State>,
    _thread: PhantomData<Rc<()>>,
}
impl Context {
    fn create_window(size: [u32; 2]) -> Result<Self, String> {
        let serial = WINDOWS
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| "window ID exhausted")?;
        let mut this = Self {
            instance: ptr::null_mut(),
            opengl: ptr::null_mut(),
            class_name: wide(&format!("HorizonLegacyGl-{}-{serial}", std::process::id())),
            registered: false,
            window: ptr::null_mut(),
            dc: ptr::null_mut(),
            context: ptr::null_mut(),
            state: Box::new(State::new(size)),
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
            let cursor = LoadCursorW(ptr::null_mut(), 32512usize as *const u16); // shared IDC_ARROW
            if cursor.is_null() {
                return Err(failure("LoadCursorW"));
            }
            let class = WindowClass {
                style: 0x0020, // CS_OWNDC: stable pixel format and GL DC
                procedure: Some(window_proc),
                class_extra: 0,
                window_extra: size_of::<usize>() as i32,
                instance: this.instance,
                icon: ptr::null_mut(),
                cursor,
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
                (&*this.state as *const State).cast_mut().cast(),
            );
            if this.window.is_null() {
                return Err(failure("CreateWindowExW"));
            }
            this.dc = GetDC(this.window);
            if this.dc.is_null() {
                return Err(failure("GetDC"));
            }
        }
        // Initial creation may impose a caption minimum. Apply the explicit
        // client request through the same bounded programmatic transaction.
        <Self as crate::ContextImpl>::resize(&this, size)?;
        Ok(this)
    }
    pub fn new(size: [u32; 2]) -> Result<Self, String> {
        let mut this = Self::create_window(size)?;
        // SAFETY: owned live window/DC; pixel format is selected exactly once,
        // then a legacy context is made current. Drop cleans every failure path.
        unsafe {
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
            // UpdateWindow may return zero when there is no update region.
            UpdateWindow(this.window);
        }
        this.refresh_size()?;
        Ok(this)
    }
    fn refresh_size(&self) -> Result<(), String> {
        // GetClientRect on a minimized window must not accidentally resume the
        // drawable using the last restored rectangle.
        if unsafe { IsIconic(self.window) } != 0 {
            if self.state.size.get() != [0; 2] {
                self.state.resized([0; 2]);
            }
            return Ok(());
        }
        let mut rect = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: owned live window and writable stack rectangle.
        if unsafe { GetClientRect(self.window, &mut rect) } == 0 {
            return Err(failure("GetClientRect"));
        }
        let size = [
            rect.right.saturating_sub(rect.left).max(0) as u32,
            rect.bottom.saturating_sub(rect.top).max(0) as u32,
        ];
        if size != self.state.size.get() {
            self.state.resized(size);
        }

        Ok(())
    }
}
impl crate::ContextImpl for Context {
    fn set_title(&self, title: &str) -> Result<(), String> {
        let title = wide(title);
        // SAFETY: live window and terminated UTF-16 string valid through the call.
        if unsafe { SetWindowTextW(self.window, title.as_ptr()) } == 0 {
            Err(failure("SetWindowTextW"))
        } else {
            Ok(())
        }
    }
    fn is_windowed(&self) -> bool {
        true
    }
    fn drawable_size(&self) -> Option<[u32; 2]> {
        Some(self.state.size.get())
    }
    fn resize(&self, size: [u32; 2]) -> Result<(), String> {
        // SAFETY: live owned window. Resizing does not implicitly unminimize it.
        if unsafe { IsIconic(self.window) } != 0 {
            return Err("cannot resize suspended window".into());
        }
        let mut rect = Rect {
            left: 0,
            top: 0,
            right: size[0] as i32,
            bottom: size[1] as i32,
        };
        // SAFETY: validated client size, live window, no move/z-order/focus changes.
        unsafe {
            if AdjustWindowRect(&mut rect, 0x00CF0000, 0) == 0 {
                return Err(failure("AdjustWindowRect"));
            }
            let previous = self.state.programmatic_resize.replace(true);
            let result = SetWindowPos(
                self.window,
                ptr::null_mut(),
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                0x0002 | 0x0004 | 0x0010,
            );
            self.state.programmatic_resize.set(previous);
            if result == 0 {
                return Err(failure("SetWindowPos"));
            }
        }
        self.refresh_size()
    }
    fn poll_events(&self) -> Result<crate::WindowEvents, String> {
        let mut message = Message::default();
        // SAFETY: thread-confined window, correctly laid out writable MSG storage.
        // Filter to our HWND: other windows on this thread retain their messages.
        unsafe {
            for _ in 0..256 {
                if PeekMessageW(&mut message, self.window, 0, 0, 1) == 0 {
                    break;
                }
                if message.message == 0x0012 {
                    // WM_QUIT is unfiltered by Win32
                    self.state.message(0x0010, 0, 0);
                    PostQuitMessage(message.wparam as i32);
                    break;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        self.refresh_size()?;
        // SAFETY: same writable MSG allocation, PM_NOREMOVE checks pending work.
        let pending = unsafe { PeekMessageW(&mut message, self.window, 0, 0, 0) } != 0;
        Ok(self.state.take(pending))
    }
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
        // thread; WM_CLOSE only requested closure and left the drawable alive.
        unsafe {
            if !self.context.is_null() {
                wglMakeCurrent(ptr::null_mut(), ptr::null_mut());
                wglDeleteContext(self.context);
            }
            if !self.dc.is_null() {
                ReleaseDC(self.window, self.dc);
            }
            if !self.window.is_null() {
                // Detach the callback pointer even if native destruction fails;
                // no later message may reference the soon-to-drop State box.
                set_window_pointer(self.window, 0, 0);
                DestroyWindow(self.window);
            }
            if self.registered {
                UnregisterClassW(self.class_name.as_ptr(), self.instance);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContextImpl, WindowEvent};
    #[test]
    fn native_window_pump_resize_unicode_and_close_do_not_need_a_gpu_icd() {
        // Hidden real HWNDs, not GL contexts: this executes on the modern x86
        // Windows runner even when no hardware OpenGL ICD is installed.
        let first = Context::create_window([64; 2]).unwrap();
        let second = Context::create_window([64; 2]).unwrap();
        first.poll_events().unwrap();
        second.poll_events().unwrap();
        first.set_title("Horizon — Ж 🌍").unwrap();
        for size in [[1; 2], [96, 48]] {
            first.resize(size).unwrap();
            assert_eq!(first.drawable_size(), Some(size));
        }
        // SAFETY: our live private HWND, bounded Win32 message payloads. The
        // window proc retains its Box<State> through close and until Drop.
        unsafe {
            for (message, wparam, lparam) in [
                (0x0100, 65, (0x1e << 16) | 1),
                (0x0102, 0x0416, 1),
                (0x0102, 0xd83c, 1),
                (0x0102, 0xdf0d, 1),
                (0x0010, 0, 0),
            ] {
                assert_ne!(PostMessageW(first.window, message, wparam, lparam), 0);
            }
        }
        let untouched = second.poll_events().unwrap();
        assert!(!untouched.close_requested);
        assert!(
            !untouched
                .events
                .iter()
                .any(|e| matches!(e, WindowEvent::Key { .. } | WindowEvent::Text { .. }))
        );
        let events = first.poll_events().unwrap();
        assert_eq!(events.lost_events, 0);
        assert!(events.close_requested);
        assert!(events.events.iter().any(|e| matches!(
            e,
            WindowEvent::Key {
                virtual_key: 65,
                scan_code: 0x1e,
                pressed: true,
                ..
            }
        )));
        for character in ['Ж', '🌍'] {
            assert!(
                events
                    .events
                    .iter()
                    .any(|e| matches!(e,WindowEvent::Text{character:c,..} if *c==character))
            );
        }
        // SAFETY: query only; WM_CLOSE must NOT destroy a drawable behind GL.
        assert_ne!(unsafe { IsWindow(first.window) }, 0);
        assert!(first.poll_events().unwrap().close_requested);
        let handle = first.window;
        drop(first);
        // SAFETY: liveness query only; owner teardown must finally close it.
        assert_eq!(unsafe { IsWindow(handle) }, 0);
    }
}
