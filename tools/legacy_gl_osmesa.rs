//! Linux offscreen integration test for the actual legacy GL renderer.
//! Load an existing OSMesa library via OSMESA_LIBRARY; see the legacy GL plan.
use horizon_legacy_gl::{Draw, DrawState, Pass, Renderer, TextureFilter, Vertex};
use std::ffi::{c_char, c_int, c_void, CStr, CString};

#[link(name = "dl")]
extern "C" {
    fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

unsafe fn symbol(handle: *mut c_void, name: &str) -> *mut c_void {
    let name = CString::new(name).unwrap();
    let ptr = dlsym(handle, name.as_ptr());
    assert!(!ptr.is_null(), "missing symbol: {:?}", name);
    ptr
}

fn main() {
    unsafe { run() }
}

unsafe fn run() {
    let path = CString::new(std::env::var("OSMESA_LIBRARY").expect("set OSMESA_LIBRARY")).unwrap();
    let lib = dlopen(path.as_ptr(), 2);
    if lib.is_null() {
        panic!("dlopen failed: {}", CStr::from_ptr(dlerror()).to_string_lossy());
    }
    let create: unsafe extern "C" fn(u32, i32, i32, i32, *mut c_void) -> *mut c_void =
        std::mem::transmute(symbol(lib, "OSMesaCreateContextExt"));
    let make_current: unsafe extern "C" fn(*mut c_void, *mut c_void, u32, i32, i32) -> u8 =
        std::mem::transmute(symbol(lib, "OSMesaMakeCurrent"));
    let destroy: unsafe extern "C" fn(*mut c_void) = std::mem::transmute(symbol(lib, "OSMesaDestroyContext"));
    let get_proc: unsafe extern "C" fn(*const c_char) -> *const c_void =
        std::mem::transmute(symbol(lib, "OSMesaGetProcAddress"));
    let context = create(0x1908, 24, 0, 0, std::ptr::null_mut());
    assert!(!context.is_null());
    let mut framebuffer = vec![0u8; 32 * 32 * 4];
    assert_ne!(make_current(context, framebuffer.as_mut_ptr().cast(), 0x1401, 32, 32), 0);
    let lookup = |name: &CStr| get_proc(name.as_ptr());
    for name in [c"glGetString", c"glReadPixels"] {
        assert!(!lookup(name).is_null(), "missing GL function: {name:?}");
    }
    let get_string: unsafe extern "C" fn(u32) -> *const c_char = std::mem::transmute(lookup(c"glGetString"));
    for (label, token) in [("renderer", 0x1f01), ("version", 0x1f02), ("GLSL", 0x8b8c)] {
        let p = get_string(token);
        assert!(!p.is_null());
        let value = CStr::from_ptr(p).to_string_lossy();
        println!("{label}: {value}");
        let expected = match label {
            "renderer" => std::env::var("EXPECT_GL_RENDERER").ok(),
            "version" => std::env::var("EXPECT_GL_VERSION").ok(),
            _ => None,
        };
        if let Some(expected) = expected {
            assert!(value.starts_with(&expected), "unexpected {label}: {value}, expected prefix {expected}");
        }
    }
    let read: unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void) =
        std::mem::transmute(lookup(c"glReadPixels"));
    let pixel = || {
        let mut p = [0u8; 4];
        read(16, 16, 1, 1, 0x1908, 0x1401, p.as_mut_ptr().cast());
        p
    };
    let mut renderer = Renderer::new(lookup).expect("renderer and GLSL compile/link");
    let atlas = renderer.create_texture(1, 1, &[255, 0, 0, 255], TextureFilter::Nearest).unwrap();
    let lightmap = renderer.create_texture(1, 1, &[255; 4], TextureFilter::Linear).unwrap();
    let vertex = |x, y| Vertex { pos: [x, y, 0.0], uv: [0.5; 2], light_uv: [0.5; 2], color: [1.0; 4] };
    let vertices = [vertex(-1.0, -1.0), vertex(1.0, -1.0), vertex(1.0, 1.0), vertex(-1.0, 1.0)];
    let indices = [0, 1, 2, 0, 2, 3];
    let mesh = renderer.create_mesh(&vertices, &indices).unwrap();
    let mut state = DrawState { atlas, lightmap, view_projection: [1.,0.,0.,0., 0.,1.,0.,0., 0.,0.,1.,0., 0.,0.,0.,1.], section_origin: [0.; 3], pass: Pass::Opaque, alpha_cutoff: 0.5 };
    renderer.begin_frame(32, 32, [0., 0., 0., 1.]).unwrap();
    renderer.draw_mesh(mesh, &state).unwrap();
    assert_eq!(pixel(), [255, 0, 0, 255], "resident opaque draw");
    renderer.update_texture(atlas, 0, 0, 1, 1, &[0, 255, 0, 255]).unwrap();
    renderer.begin_frame(32, 32, [0., 0., 0., 1.]).unwrap();
    renderer.draw(&Draw { vertices: &vertices, indices: &indices, state }).unwrap();
    assert_eq!(pixel(), [0, 255, 0, 255], "stream draw and texture update");
    renderer.update_texture(lightmap, 0, 0, 1, 1, &[128, 255, 255, 255]).unwrap();
    renderer.update_texture(atlas, 0, 0, 1, 1, &[255, 0, 0, 255]).unwrap();
    renderer.begin_frame(32, 32, [0., 0., 0., 1.]).unwrap();
    renderer.draw_mesh(mesh, &state).unwrap();
    assert!((127..=129).contains(&pixel()[0]), "lightmap modulation: {:?}", pixel());
    renderer.update_texture(lightmap, 0, 0, 1, 1, &[255; 4]).unwrap();
    renderer.begin_frame(32, 32, [0., 0., 1., 1.]).unwrap();
    state.section_origin = [3., 0., 0.];
    renderer.draw_mesh(mesh, &state).unwrap();
    assert_eq!(pixel(), [0, 0, 255, 255], "section origin transform");
    state.section_origin = [0.; 3];
    renderer.update_texture(atlas, 0, 0, 1, 1, &[255, 0, 0, 0]).unwrap();
    renderer.begin_frame(32, 32, [0., 0., 1., 1.]).unwrap();
    state.pass = Pass::Cutout;
    renderer.draw_mesh(mesh, &state).unwrap();
    assert_eq!(pixel(), [0, 0, 255, 255], "cutout discard");
    renderer.update_texture(atlas, 0, 0, 1, 1, &[255, 0, 0, 128]).unwrap();
    renderer.begin_frame(32, 32, [0., 0., 1., 1.]).unwrap();
    state.pass = Pass::Translucent;
    renderer.draw_mesh(mesh, &state).unwrap();
    let blended = pixel();
    assert!((127..=129).contains(&blended[0]) && blended[1] == 0 && (126..=128).contains(&blended[2]), "translucent blend: {blended:?}");
    // A translucent surface must test depth but must not write it.
    renderer.begin_frame(32, 32, [0., 0., 1., 1.]).unwrap();
    state.section_origin = [0., 0., -0.5];
    renderer.draw_mesh(mesh, &state).unwrap();
    renderer.update_texture(atlas, 0, 0, 1, 1, &[0, 255, 0, 255]).unwrap();
    state.pass = Pass::Opaque;
    state.section_origin = [0., 0., 0.5];
    renderer.draw_mesh(mesh, &state).unwrap();
    assert_eq!(pixel(), [0, 255, 0, 255], "translucent depth writes must be disabled");
    renderer.update_texture(atlas, 0, 0, 1, 1, &[255, 0, 0, 255]).unwrap();
    state.section_origin = [0., 0., 0.8];
    renderer.draw_mesh(mesh, &state).unwrap();
    assert_eq!(pixel(), [0, 255, 0, 255], "opaque depth test");
    renderer.delete_mesh(mesh).unwrap();
    renderer.delete_texture(atlas).unwrap();
    renderer.delete_texture(lightmap).unwrap();
    renderer.destroy();
    destroy(context);
    println!("OSMesa renderer pixel checks passed");
}
