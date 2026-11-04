//! Standalone Windows WGL / OpenGL 2 terrain prototype, NOT the game renderer.
//!
//! Build with a Windows 7-compatible 32-bit Rust toolchain and linker, e.g.
//! `rustc --edition=2021 --target i686-pc-windows-gnu tools/legacy_gl_window.rs`.
//! No wgpu, assets, network, or external Rust crates. Arrow keys move the camera.
//! The complete game has additional Windows 7 compatibility requirements.

#[cfg(any(windows, test))]
#[path = "legacy_gl_geometry.rs"]
mod geometry;

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
        ffi::{CStr, CString, c_char, c_void},
        mem, ptr,
        sync::atomic::{AtomicBool, Ordering},
    };

    type Handle = isize;
    type GlUint = u32;
    type GlInt = i32;
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
        fn glViewport(x: i32, y: i32, width: i32, height: i32);
        fn glClearColor(r: f32, g: f32, b: f32, a: f32);
        fn glClear(mask: u32);
        fn glEnable(capability: u32);
        fn glDepthFunc(function: u32);
        fn glGenTextures(count: i32, textures: *mut u32);
        fn glBindTexture(target: u32, texture: u32);
        fn glTexParameteri(target: u32, name: u32, value: i32);
        fn glTexImage2D(
            target: u32,
            level: i32,
            internal: i32,
            width: i32,
            height: i32,
            border: i32,
            format: u32,
            kind: u32,
            pixels: *const c_void,
        );
        fn glDrawArrays(mode: u32, first: i32, count: i32);
        fn glGetIntegerv(name: u32, value: *mut i32);
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

    unsafe fn proc(name: &str) -> Result<*const c_void, String> {
        let name = CString::new(name).map_err(|_| "invalid GL function name")?;
        let value = wglGetProcAddress(name.as_ptr());
        if value.is_null() || (value as usize) <= 3 || value as isize == -1 {
            return Err(format!(
                "missing OpenGL function {}",
                name.to_string_lossy()
            ));
        }
        Ok(value)
    }
    macro_rules! load {
        ($name:expr, $ty:ty) => {
            mem::transmute::<*const c_void, $ty>(proc($name)?)
        };
    }
    type CreateShader = unsafe extern "system" fn(u32) -> GlUint;
    type ShaderSource =
        unsafe extern "system" fn(GlUint, GlInt, *const *const c_char, *const GlInt);
    type CompileShader = unsafe extern "system" fn(GlUint);
    type GetShaderiv = unsafe extern "system" fn(GlUint, u32, *mut GlInt);
    type GetShaderLog = unsafe extern "system" fn(GlUint, GlInt, *mut GlInt, *mut c_char);
    type CreateProgram = unsafe extern "system" fn() -> GlUint;
    type AttachShader = unsafe extern "system" fn(GlUint, GlUint);
    type LinkProgram = unsafe extern "system" fn(GlUint);
    type GetProgramiv = unsafe extern "system" fn(GlUint, u32, *mut GlInt);
    type GetProgramLog = unsafe extern "system" fn(GlUint, GlInt, *mut GlInt, *mut c_char);
    type UseProgram = unsafe extern "system" fn(GlUint);
    type GetLocation = unsafe extern "system" fn(GlUint, *const c_char) -> GlInt;
    type Uniform1i = unsafe extern "system" fn(GlInt, GlInt);
    type Uniform1f = unsafe extern "system" fn(GlInt, f32);
    type Uniform3f = unsafe extern "system" fn(GlInt, f32, f32, f32);
    type UniformMatrix4fv = unsafe extern "system" fn(GlInt, GlInt, u8, *const f32);
    type GenBuffers = unsafe extern "system" fn(GlInt, *mut GlUint);
    type BindBuffer = unsafe extern "system" fn(u32, GlUint);
    type BufferData = unsafe extern "system" fn(u32, isize, *const c_void, u32);
    type EnableAttrib = unsafe extern "system" fn(GlUint);
    type AttribPointer = unsafe extern "system" fn(GlUint, GlInt, u32, u8, GlInt, *const c_void);
    type ActiveTexture = unsafe extern "system" fn(u32);

    unsafe fn gl_string(token: u32) -> String {
        let value = glGetString(token);
        if value.is_null() {
            return "unavailable".into();
        }
        CStr::from_ptr(value.cast()).to_string_lossy().into_owned()
    }
    unsafe fn shader(kind: u32, source: &str) -> Result<u32, String> {
        let create: CreateShader = load!("glCreateShader", CreateShader);
        let source_fn: ShaderSource = load!("glShaderSource", ShaderSource);
        let compile: CompileShader = load!("glCompileShader", CompileShader);
        let get: GetShaderiv = load!("glGetShaderiv", GetShaderiv);
        let log: GetShaderLog = load!("glGetShaderInfoLog", GetShaderLog);
        let id = create(kind);
        if id == 0 {
            return Err("glCreateShader failed".into());
        }
        let text = CString::new(source).map_err(|_| "shader contains NUL")?;
        source_fn(id, 1, &text.as_ptr(), ptr::null());
        compile(id);
        let mut ok = 0;
        get(id, 0x8b81, &mut ok); // COMPILE_STATUS
        if ok == 0 {
            let mut message = vec![0u8; 4096];
            let mut len = 0;
            log(
                id,
                message.len() as i32,
                &mut len,
                message.as_mut_ptr().cast(),
            );
            return Err(format!(
                "shader compilation: {}",
                String::from_utf8_lossy(&message[..(len as usize).min(message.len())])
            ));
        }
        Ok(id)
    }
    unsafe fn program() -> Result<u32, String> {
        let vs = shader(
            0x8b31,
            include_str!("../crates/horizon-client-render/src/legacy_gl_terrain.vert"),
        )?;
        let fs = shader(
            0x8b30,
            include_str!("../crates/horizon-client-render/src/legacy_gl_terrain.frag"),
        )?;
        let create: CreateProgram = load!("glCreateProgram", CreateProgram);
        let attach: AttachShader = load!("glAttachShader", AttachShader);
        let link: LinkProgram = load!("glLinkProgram", LinkProgram);
        let get: GetProgramiv = load!("glGetProgramiv", GetProgramiv);
        let log: GetProgramLog = load!("glGetProgramInfoLog", GetProgramLog);
        let id = create();
        if id == 0 {
            return Err("glCreateProgram failed".into());
        }
        attach(id, vs);
        attach(id, fs);
        link(id);
        let mut ok = 0;
        get(id, 0x8b82, &mut ok); // LINK_STATUS
        if ok == 0 {
            let mut message = vec![0u8; 4096];
            let mut len = 0;
            log(
                id,
                message.len() as i32,
                &mut len,
                message.as_mut_ptr().cast(),
            );
            return Err(format!(
                "program link: {}",
                String::from_utf8_lossy(&message[..(len as usize).min(message.len())])
            ));
        }
        Ok(id)
    }
    unsafe fn location(program: u32, name: &str, get: GetLocation) -> Result<i32, String> {
        let text = CString::new(name).map_err(|_| "invalid variable name")?;
        let value = get(program, text.as_ptr());
        if value < 0 {
            return Err(format!("missing shader variable {name}"));
        }
        Ok(value)
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
        let mut texture_units = 0;
        glGetIntegerv(0x8872, &mut texture_units); // MAX_TEXTURE_IMAGE_UNITS
        if texture_units < 2 {
            return Err(format!(
                "at least two texture units required, got {texture_units}"
            ));
        }
        let check_pixels = std::env::args().skip(1).any(|arg| arg == "--check");
        let program = program()?;
        let use_program: UseProgram = load!("glUseProgram", UseProgram);
        let get_uniform: GetLocation = load!("glGetUniformLocation", GetLocation);
        let get_attrib: GetLocation = load!("glGetAttribLocation", GetLocation);
        let uniform1i: Uniform1i = load!("glUniform1i", Uniform1i);
        let uniform1f: Uniform1f = load!("glUniform1f", Uniform1f);
        let uniform3f: Uniform3f = load!("glUniform3f", Uniform3f);
        let uniform_matrix: UniformMatrix4fv = load!("glUniformMatrix4fv", UniformMatrix4fv);
        let generate_buffers: GenBuffers = load!("glGenBuffers", GenBuffers);
        let bind_buffer: BindBuffer = load!("glBindBuffer", BindBuffer);
        let buffer_data: BufferData = load!("glBufferData", BufferData);
        let enable_attrib: EnableAttrib = load!("glEnableVertexAttribArray", EnableAttrib);
        let attrib_pointer: AttribPointer = load!("glVertexAttribPointer", AttribPointer);
        let active_texture: ActiveTexture = load!("glActiveTexture", ActiveTexture);
        use_program(program);
        let vertices = crate::geometry::demo_mesh();
        let mut buffer = 0;
        generate_buffers(1, &mut buffer);
        bind_buffer(0x8892, buffer); // ARRAY_BUFFER
        buffer_data(
            0x8892,
            (vertices.len() * mem::size_of::<f32>()) as isize,
            vertices.as_ptr().cast(),
            0x88e4,
        );
        for (name, size, offset) in [
            ("a_position", 3, 0),
            ("a_uv", 2, 3),
            ("a_light_uv", 2, 5),
            ("a_color", 4, 7),
        ] {
            let index = location(program, name, get_attrib)? as u32;
            enable_attrib(index);
            attrib_pointer(index, size, 0x1406, 0, 44, (offset * 4) as *const c_void);
        }
        let matrix: [f32; 16] = [
            0.12, -0.05, 0.02, 0.0, 0.0, 0.14, -0.02, 0.0, 0.05, 0.04, 0.02, 0.0, 0.0, -0.25, 0.0,
            1.0,
        ];
        uniform_matrix(
            location(program, "u_view_projection", get_uniform)?,
            1,
            0,
            matrix.as_ptr(),
        );
        let origin = location(program, "u_section_origin", get_uniform)?;
        uniform1f(location(program, "u_alpha_cutoff", get_uniform)?, 0.5);
        let mut textures = [0u32; 2];
        glGenTextures(2, textures.as_mut_ptr());
        let atlas = [70u8, 170, 60, 255, 135, 85, 45, 255];
        let white = [255u8; 4];
        for (unit, name, width, pixels) in [
            (0, "u_atlas", 2, atlas.as_ptr()),
            (1, "u_lightmap", 1, white.as_ptr()),
        ] {
            active_texture(0x84c0 + unit);
            glBindTexture(0x0de1, textures[unit as usize]);
            glTexParameteri(0x0de1, 0x2801, 0x2600); // MIN_FILTER = NEAREST, no mipmaps
            glTexParameteri(0x0de1, 0x2800, 0x2600);
            glTexImage2D(
                0x0de1,
                0,
                0x1908,
                width,
                1,
                0,
                0x1908,
                0x1401,
                pixels.cast(),
            );
            uniform1i(location(program, name, get_uniform)?, unit as i32);
        }
        glEnable(0x0b71); // DEPTH_TEST
        glDepthFunc(0x0203); // LEQUAL
        let error = glGetError();
        if error != 0 {
            return Err(format!("OpenGL initialization error 0x{error:04x}"));
        }
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
                glViewport(0, 0, width, height);
                glClearColor(0.45, 0.67, 0.9, 1.0);
                glClear(0x00004000 | 0x00000100);
                uniform3f(origin, camera[0], 0.0, camera[1]);
                glDrawArrays(0x0004, 0, (vertices.len() / 11) as i32);
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
