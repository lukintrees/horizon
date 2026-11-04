//! Standalone Win32/WGL probe. This does not load game data or start the game.
//! It is useful for reporting the target driver's actual GL strings and limits.
#[cfg(target_os = "windows")]
mod windows {
    use horizon_legacy_gl::{Batch, Page, Pass, Renderer, Vertex, window::WindowContext};
    use std::{
        ffi::{CStr, c_char, c_void},
        ptr,
    };
    #[link(name = "user32")]
    unsafe extern "system" {
        fn CreateWindowExW(
            ex_style: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: *mut c_void,
            menu: *mut c_void,
            instance: *mut c_void,
            param: *mut c_void,
        ) -> *mut c_void;
        fn DestroyWindow(hwnd: *mut c_void) -> i32;
    }
    struct Window(*mut c_void);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    pub fn run() -> Result<(), String> {
        // A hidden built-in STATIC window needs no custom WndProc or message loop.
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let title: Vec<u16> = "Horizon WGL probe\0".encode_utf16().collect();
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                64,
                64,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if hwnd.is_null() {
            return Err("CreateWindowExW failed".into());
        }
        let window = Window(hwnd);
        // SAFETY: the window stays live and private to this thread until context drop.
        let context =
            unsafe { WindowContext::new_win32(window.0) }.map_err(|e| format!("{e:?}"))?;
        unsafe {
            let address = context.resolve(c"glGetString");
            if address.is_null() {
                return Err("cannot resolve glGetString".into());
            }
            let get_string = std::mem::transmute::<
                *const c_void,
                unsafe extern "system" fn(u32) -> *const c_char,
            >(address);
            let mut gl_version = String::new();
            for (name, key) in [
                ("vendor", 0x1f00),
                ("renderer", 0x1f01),
                ("GL", 0x1f02),
                ("GLSL", 0x8b8c),
            ] {
                let text = get_string(key);
                let value = if text.is_null() {
                    "(unavailable)".into()
                } else {
                    CStr::from_ptr(text).to_string_lossy().into_owned()
                };
                if name == "GL" {
                    gl_version.clone_from(&value);
                }
                println!("{name}: {value}");
                if std::env::var_os("GITHUB_ACTIONS").is_some() {
                    println!("::notice::{name}: {value}");
                }
            }
            // A GL 1.1 software driver cannot report the GLSL string and
            // generates INVALID_ENUM; don't let this stale error affect the renderer.
            let address = context.resolve(c"glGetError");
            if address.is_null() {
                return Err("cannot resolve glGetError".into());
            }
            let get_error =
                std::mem::transmute::<*const c_void, unsafe extern "system" fn() -> u32>(address);
            let _ = get_error();
            match Renderer::new(|name| context.resolve(name)) {
                Ok(mut renderer) => {
                    println!(
                        "backend: initialized, max texture size {}",
                        renderer.max_texture_size()
                    );
                    // Check real output when the host actually exposes GL 2+.
                    let result = (|| -> Result<(), String> {
                        let pixels = [200, 40, 20, 255].repeat(4);
                        renderer
                            .upload_pages(&[Page {
                                side: 2,
                                pixels: &pixels,
                            }])
                            .map_err(|e| format!("{e:?}"))?;
                        macro_rules! function {
                            ($name:expr, $ty:ty) => {{
                                let address = context.resolve($name);
                                if address.is_null() {
                                    return Err(format!("missing {}", $name.to_string_lossy()));
                                }
                                std::mem::transmute::<*const c_void, $ty>(address)
                            }};
                        }
                        let viewport =
                            function!(c"glViewport", unsafe extern "system" fn(i32, i32, i32, i32));
                        let clear_color = function!(
                            c"glClearColor",
                            unsafe extern "system" fn(f32, f32, f32, f32)
                        );
                        let clear = function!(c"glClear", unsafe extern "system" fn(u32));
                        let read = function!(
                            c"glReadPixels",
                            unsafe extern "system" fn(i32, i32, i32, i32, u32, u32, *mut c_void)
                        );
                        viewport(0, 0, 64, 64);
                        clear_color(0., 0., 0., 1.);
                        clear(0x4000 | 0x0100);
                        let vertex = |position| Vertex {
                            position,
                            sprite_uv: [0.5, 0.5],
                            atlas_rect: [0., 0., 1., 1.],
                            color: [1.; 4],
                        };
                        let vertices = [
                            vertex([-1., -1., 0.]),
                            vertex([1., -1., 0.]),
                            vertex([0., 1., 0.]),
                        ];
                        let matrix = [
                            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
                        ];
                        renderer
                            .draw(
                                &vertices,
                                &[Batch {
                                    page: 0,
                                    first: 0,
                                    count: 3,
                                }],
                                &matrix,
                                Pass::Solid { alpha_cutoff: 0.1 },
                            )
                            .map_err(|e| format!("{e:?}"))?;
                        let mut pixel = [0u8; 4];
                        read(32, 32, 1, 1, 0x1908, 0x1401, pixel.as_mut_ptr().cast());
                        if pixel[..3]
                            .iter()
                            .zip([200u8, 40, 20])
                            .any(|(actual, expected)| actual.abs_diff(expected) > 2)
                        {
                            return Err(format!("unexpected GL framebuffer pixel: {pixel:?}"));
                        }
                        if get_error() != 0 {
                            return Err("GL readback failed".into());
                        }
                        println!("PASS: GL renderer framebuffer {pixel:?}");
                        Ok(())
                    })();
                    renderer.destroy();
                    result?;
                }
                Err(error) => {
                    println!("backend: unavailable ({error:?})");
                    if gl_version
                        .split('.')
                        .next()
                        .and_then(|n| n.parse::<u32>().ok())
                        .is_some_and(|major| major >= 2)
                    {
                        return Err(format!("GL 2+ backend initialization failed: {error:?}"));
                    }
                    println!("SKIP: GL drawing needs OpenGL 2.0 and GLSL 1.10");
                }
            }
            context.swap_buffers().map_err(|e| format!("{e:?}"))?;
        }
        drop(context);
        drop(window);
        println!("PASS: Win32/WGL context lifecycle and core function lookup");
        Ok(())
    }
}
#[cfg(target_os = "windows")]
fn main() -> Result<(), String> {
    windows::run()
}
#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("This probe requires Windows.");
}
