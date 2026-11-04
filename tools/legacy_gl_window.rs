//! Standalone Windows WGL / OpenGL 2 terrain prototype, NOT the game renderer.
//!
//! Build with a Windows 7-compatible 32-bit Rust toolchain and linker, e.g.
//! `rustc --edition=2021 --target i686-pc-windows-gnu tools/legacy_gl_window.rs`.
//! No wgpu, assets, network, or external Rust crates. Arrow keys move the camera.
//! The complete game has additional Windows 7 compatibility requirements.

#[cfg(any(windows, test))]
#[path = "legacy_gl_geometry.rs"]
mod geometry;

#[cfg(windows)]
#[path = "../crates/horizon-legacy-gl/src/lib.rs"]
mod legacy_renderer;

#[cfg(not(windows))]
fn main() {
    eprintln!("This WGL prototype runs on Windows only; it is not the game.");
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows::run() {
        eprintln!("OpenGL prototype failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows {
    use std::{
        ffi::{CStr, c_char, c_void},
        mem, ptr,
        sync::atomic::{AtomicBool, Ordering},
    };

    type Handle = isize;
    static CLOSE: AtomicBool = AtomicBool::new(false);

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    struct Message {
        window: Handle,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        point: Point,
        private: u32,
    }
    #[repr(C)]
    struct WindowClass {
        style: u32,
        procedure: Option<unsafe extern "system" fn(Handle, u32, usize, isize) -> isize>,
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
    struct PixelFormat {
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

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(class: *const WindowClass) -> u16;
        fn CreateWindowExW(
            ex: u32,
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
            param: *mut c_void,
        ) -> Handle;
        fn DefWindowProcW(window: Handle, message: u32, wparam: usize, lparam: isize) -> isize;
        fn DestroyWindow(window: Handle) -> i32;
        fn ShowWindow(window: Handle, command: i32) -> i32;
        fn GetDC(window: Handle) -> Handle;
        fn ReleaseDC(window: Handle, dc: Handle) -> i32;
        fn GetClientRect(window: Handle, rect: *mut Rect) -> i32;
        fn PeekMessageW(
            message: *mut Message,
            window: Handle,
            min: u32,
            max: u32,
            remove: u32,
        ) -> i32;
        fn TranslateMessage(message: *const Message) -> i32;
        fn DispatchMessageW(message: *const Message) -> isize;
        fn GetAsyncKeyState(key: i32) -> i16;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
        fn GetProcAddress(module: Handle, name: *const c_char) -> *const c_void;
        fn Sleep(milliseconds: u32);
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn ChoosePixelFormat(dc: Handle, format: *const PixelFormat) -> i32;
        fn DescribePixelFormat(dc: Handle, index: i32, size: u32, format: *mut PixelFormat) -> i32;
        fn SetPixelFormat(dc: Handle, index: i32, format: *const PixelFormat) -> i32;
        fn SwapBuffers(dc: Handle) -> i32;
    }
    #[link(name = "opengl32")]
    extern "system" {
        fn wglCreateContext(dc: Handle) -> Handle;
        fn wglDeleteContext(context: Handle) -> i32;
        fn wglMakeCurrent(dc: Handle, context: Handle) -> i32;
        fn wglGetProcAddress(name: *const c_char) -> *const c_void;
        fn glGetString(name: u32) -> *const u8;
        fn glGetError() -> u32;
        fn glReadPixels(
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            format: u32,
            kind: u32,
            pixels: *mut c_void,
        );
    }

    unsafe extern "system" fn window_proc(window: Handle, msg: u32, w: usize, l: isize) -> isize {
        if msg == 0x0010 {
            // WM_CLOSE: release GL objects before destroying the window.
            CLOSE.store(true, Ordering::Relaxed);
            return 0;
        }
        DefWindowProcW(window, msg, w, l)
    }
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    struct Context {
        window: Handle,
        dc: Handle,
        gl: Handle,
    }
    impl Drop for Context {
        fn drop(&mut self) {
            unsafe {
                if self.gl != 0 {
                    wglMakeCurrent(0, 0);
                    wglDeleteContext(self.gl);
                }
                if self.dc != 0 {
                    ReleaseDC(self.window, self.dc);
                }
                if self.window != 0 {
                    DestroyWindow(self.window);
                }
            }
        }
    }
    unsafe fn context() -> Result<Context, String> {
        let class_name = wide("HorizonLegacyGlPrototype");
        let title = wide("Horizon OpenGL 2 prototype - arrow keys move");
        let instance = GetModuleHandleW(ptr::null());
        let class = WindowClass {
            style: 0x0020,
            procedure: Some(window_proc),
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: 0,
            cursor: 0,
            background: 0,
            menu_name: ptr::null(),
            class_name: class_name.as_ptr(),
        }; // CS_OWNDC
        if RegisterClassW(&class) == 0 {
            return Err("RegisterClassW".into());
        }
        let mut result = Context {
            window: 0,
            dc: 0,
            gl: 0,
        };
        result.window = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            0x00cf0000,
            i32::MIN,
            i32::MIN,
            960,
            640,
            0,
            0,
            instance,
            ptr::null_mut(),
        );
        if result.window == 0 {
            return Err("CreateWindowExW".into());
        }
        result.dc = GetDC(result.window);
        if result.dc == 0 {
            return Err("GetDC".into());
        }
        let mut format: PixelFormat = mem::zeroed();
        format.size = mem::size_of::<PixelFormat>() as u16;
        format.version = 1;
        format.flags = 0x00000004 | 0x00000020 | 0x00000001; // window, OpenGL, double buffer
        format.color_bits = 24;
        format.alpha_bits = 8;
        format.depth_bits = 16;
        let index = ChoosePixelFormat(result.dc, &format);
        if index == 0 {
            return Err("no usable OpenGL pixel format".into());
        }
        let mut actual: PixelFormat = mem::zeroed();
        if DescribePixelFormat(
            result.dc,
            index,
            mem::size_of::<PixelFormat>() as u32,
            &mut actual,
        ) == 0
            || actual.flags & format.flags != format.flags
            || actual.pixel_type != 0
            || actual.depth_bits < 16
        {
            return Err(
                "pixel format lacks a double-buffered RGBA OpenGL surface with 16-bit depth".into(),
            );
        }
        if SetPixelFormat(result.dc, index, &actual) == 0 {
            return Err("SetPixelFormat failed".into());
        }
        result.gl = wglCreateContext(result.dc);
        if result.gl == 0 || wglMakeCurrent(result.dc, result.gl) == 0 {
            return Err("cannot create or activate WGL context".into());
        }
        ShowWindow(result.window, 5);
        Ok(result)
    }

    unsafe fn gl_string(token: u32) -> String {
        let value = glGetString(token);
        if value.is_null() {
            return "unavailable".into();
        }
        CStr::from_ptr(value.cast()).to_string_lossy().into_owned()
    }

    // WGL returns only extension entry points. Core 1.1 entry points are
    // exported from opengl32.dll instead; do not call WGL sentinel values.
    unsafe fn lookup(name: &CStr) -> *const c_void {
        let p = wglGetProcAddress(name.as_ptr());
        if p.is_null() || p as usize <= 3 || p as isize == -1 {
            let module = GetModuleHandleW(wide("opengl32.dll").as_ptr());
            if module == 0 {
                return ptr::null();
            }
            return GetProcAddress(module, name.as_ptr());
        }
        p
    }

    // Context was created before this guard, and therefore outlives its Drop.
    struct RendererGuard(Option<crate::legacy_renderer::Renderer>);
    impl Drop for RendererGuard {
        fn drop(&mut self) {
            if let Some(renderer) = self.0.take() {
                unsafe {
                    renderer.destroy();
                }
            }
        }
    }

    /// Checks that the first rendered back buffer contains both atlas colors.
    unsafe fn verify_pixels(width: i32, height: i32) -> Result<(), String> {
        let (w, h) = (width.min(1024), height.min(1024));
        let mut data = vec![0u8; w as usize * h as usize * 4];
        glReadPixels(
            (width - w) / 2,
            (height - h) / 2,
            w,
            h,
            0x1908,
            0x1401,
            data.as_mut_ptr().cast(),
        );
        let error = glGetError();
        if error != 0 {
            return Err(format!("glReadPixels failed: 0x{error:04x}"));
        }
        let mut green = false;
        let mut brown = false;
        for pixel in data.chunks_exact(4) {
            let (r, g, b) = (
                i32::from(pixel[0]),
                i32::from(pixel[1]),
                i32::from(pixel[2]),
            );
            green |= g > r + 25 && g > b + 25;
            brown |= r > g + 20 && g > b + 10;
        }
        if !green || !brown {
            return Err(format!(
                "expected green ground and brown column (green={green}, brown={brown})"
            ));
        }
        println!("PASS: OpenGL 2 scene rendered with shaders, VBO, texture and depth buffer");
        Ok(())
    }

    pub fn run() -> Result<(), String> {
        unsafe { run_gl() }
    }
    unsafe fn run_gl() -> Result<(), String> {
        let context = context()?;
        for (name, token) in [
            ("Vendor", 0x1f00),
            ("Renderer", 0x1f01),
            ("OpenGL", 0x1f02),
            ("GLSL", 0x8b8c),
        ] {
            println!("{name}: {}", gl_string(token));
        }
        let version = gl_string(0x1f02);
        if crate::geometry::desktop_gl_version(&version).map_or(true, |v| v < (2, 0)) {
            return Err(format!(
                "desktop OpenGL 2.0 or newer is required, got {version}"
            ));
        }
        let check_pixels = std::env::args().skip(1).any(|arg| arg == "--check");
        let mut guard = RendererGuard(Some(
            crate::legacy_renderer::Renderer::new(|name| lookup(name))
                .map_err(|e| format!("renderer init: {e:?}"))?,
        ));
        let renderer = guard.0.as_mut().expect("renderer guard owns renderer");
        let atlas = renderer
            .create_texture(
                2,
                1,
                &[70, 170, 60, 255, 135, 85, 45, 255],
                crate::legacy_renderer::TextureFilter::Nearest,
            )
            .map_err(|e| format!("atlas upload: {e:?}"))?;
        let lightmap = renderer
            .create_texture(
                1,
                1,
                &[255; 4],
                crate::legacy_renderer::TextureFilter::Linear,
            )
            .map_err(|e| format!("lightmap upload: {e:?}"))?;
        let values = crate::geometry::demo_mesh();
        let vertices: Vec<crate::legacy_renderer::Vertex> = values
            .chunks_exact(11)
            .map(|v| crate::legacy_renderer::Vertex {
                pos: [v[0], v[1], v[2]],
                uv: [v[3], v[4]],
                light_uv: [v[5], v[6]],
                color: [v[7], v[8], v[9], v[10]],
            })
            .collect();
        let indices: Vec<u16> = (0..vertices.len()).map(|n| n as u16).collect();
        let mesh = renderer
            .create_mesh(&vertices, &indices)
            .map_err(|e| format!("mesh upload: {e:?}"))?;
        let matrix: [f32; 16] = [
            0.12, -0.05, 0.02, 0.0, 0.0, 0.14, -0.02, 0.0, 0.05, 0.04, 0.02, 0.0, 0.0, -0.25, 0.0,
            1.0,
        ];
        println!("Rendering synthetic terrain; arrows move the camera. Close the window to exit.");
        let mut camera = [0.0f32, 0.0f32];
        while !CLOSE.load(Ordering::Relaxed) {
            let mut message: Message = mem::zeroed();
            while PeekMessageW(&mut message, 0, 0, 0, 1) != 0 {
                if message.message == 0x0012 {
                    CLOSE.store(true, Ordering::Relaxed);
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if GetAsyncKeyState(0x25) < 0 {
                camera[0] += 0.08;
            }
            if GetAsyncKeyState(0x27) < 0 {
                camera[0] -= 0.08;
            }
            if GetAsyncKeyState(0x26) < 0 {
                camera[1] += 0.08;
            }
            if GetAsyncKeyState(0x28) < 0 {
                camera[1] -= 0.08;
            }
            let mut rect: Rect = mem::zeroed();
            if GetClientRect(context.window, &mut rect) == 0 {
                return Err("GetClientRect".into());
            }
            let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
            if width > 0 && height > 0 {
                renderer
                    .begin_frame(width as u32, height as u32, [0.45, 0.67, 0.9, 1.0])
                    .map_err(|e| format!("begin frame: {e:?}"))?;
                renderer
                    .draw_mesh(
                        mesh,
                        &crate::legacy_renderer::DrawState {
                            atlas,
                            lightmap,
                            view_projection: matrix,
                            section_origin: [camera[0], 0.0, camera[1]],
                            pass: crate::legacy_renderer::Pass::Opaque,
                            alpha_cutoff: 0.5,
                        },
                    )
                    .map_err(|e| format!("draw: {e:?}"))?;
                if check_pixels {
                    verify_pixels(width, height)?;
                    return Ok(());
                }
                if SwapBuffers(context.dc) == 0 {
                    return Err("SwapBuffers".into());
                }
            }
            Sleep(16);
        }
        Ok(())
    }
}
