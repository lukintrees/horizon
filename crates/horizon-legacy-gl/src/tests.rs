use super::*;
use std::{cell::RefCell, ffi::CStr};

#[derive(Default)]
struct State {
    next: u32,
    shader_fails: bool,
    deleted_shaders: Vec<u32>,
    deleted_programs: Vec<u32>,
    deleted_buffers: Vec<u32>,
    deleted_textures: Vec<u32>,
    buffer_sizes: Vec<(u32, isize)>,
    draws: Vec<i32>,
    depth_masks: Vec<u8>,
    texture_updates: usize,
}
thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }
fn next() -> u32 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.next += 1;
        s.next
    })
}
unsafe extern "system" fn get_error() -> u32 {
    0
}
unsafe extern "system" fn get_integer(name: u32, out: *mut i32) {
    *out = match name {
        0x8872 => 4,
        0x0d33 => 256,
        0x8869 => 8,
        _ => 0,
    };
}
unsafe extern "system" fn create_shader(_: u32) -> u32 {
    next()
}
unsafe extern "system" fn shader_source(_: u32, _: i32, _: *const *const c_char, _: *const i32) {}
unsafe extern "system" fn compile_shader(_: u32) {}
unsafe extern "system" fn get_shader(_: u32, _: u32, out: *mut i32) {
    *out = STATE.with(|s| i32::from(!s.borrow().shader_fails));
}
unsafe extern "system" fn shader_log(_: u32, _: i32, len: *mut i32, data: *mut c_char) {
    let text = b"bad shader";
    ptr::copy_nonoverlapping(text.as_ptr(), data.cast(), text.len());
    *len = text.len() as i32;
}
unsafe extern "system" fn delete_shader(id: u32) {
    STATE.with(|s| s.borrow_mut().deleted_shaders.push(id));
}
unsafe extern "system" fn create_program() -> u32 {
    next()
}
unsafe extern "system" fn attach_shader(_: u32, _: u32) {}
unsafe extern "system" fn link_program(_: u32) {}
unsafe extern "system" fn get_program(_: u32, _: u32, out: *mut i32) {
    *out = 1;
}
unsafe extern "system" fn program_log(_: u32, _: i32, len: *mut i32, _: *mut c_char) {
    *len = 0;
}
unsafe extern "system" fn delete_program(id: u32) {
    STATE.with(|s| s.borrow_mut().deleted_programs.push(id));
}
unsafe extern "system" fn use_program(_: u32) {}
unsafe extern "system" fn uniform_location(_: u32, _: *const c_char) -> i32 {
    0
}
unsafe extern "system" fn attrib_location(_: u32, name: *const c_char) -> i32 {
    match CStr::from_ptr(name).to_bytes() {
        b"a_position" => 0,
        b"a_uv" => 1,
        b"a_light_uv" => 2,
        b"a_color" => 3,
        _ => -1,
    }
}
unsafe extern "system" fn uniform_1i(_: i32, _: i32) {}
unsafe extern "system" fn uniform_1f(_: i32, _: f32) {}
unsafe extern "system" fn uniform_3f(_: i32, _: f32, _: f32, _: f32) {}
unsafe extern "system" fn uniform_matrix(_: i32, _: i32, _: u8, _: *const f32) {}
unsafe extern "system" fn generate(_: i32, out: *mut u32) {
    *out = next();
}
unsafe extern "system" fn delete_buffers(_: i32, p: *const u32) {
    STATE.with(|s| s.borrow_mut().deleted_buffers.push(*p));
}
unsafe extern "system" fn delete_textures(_: i32, p: *const u32) {
    STATE.with(|s| s.borrow_mut().deleted_textures.push(*p));
}
unsafe extern "system" fn bind(_: u32, _: u32) {}
unsafe extern "system" fn buffer_data(target: u32, size: isize, _: *const c_void, _: u32) {
    STATE.with(|s| s.borrow_mut().buffer_sizes.push((target, size)));
}
unsafe extern "system" fn attrib_enable(_: u32) {}
unsafe extern "system" fn attrib_pointer(_: u32, _: i32, _: u32, _: u8, _: i32, _: *const c_void) {}
unsafe extern "system" fn active_texture(_: u32) {}
unsafe extern "system" fn parameter(_: u32, _: u32, _: i32) {}
unsafe extern "system" fn pixel_store(_: u32, _: i32) {}
unsafe extern "system" fn tex_image(
    _: u32,
    _: i32,
    _: i32,
    _: i32,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
    _: *const c_void,
) {
}
unsafe extern "system" fn tex_sub_image(
    _: u32,
    _: i32,
    _: i32,
    _: i32,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
    _: *const c_void,
) {
    STATE.with(|s| s.borrow_mut().texture_updates += 1);
}
unsafe extern "system" fn viewport(_: i32, _: i32, _: i32, _: i32) {}
unsafe extern "system" fn clear_color(_: f32, _: f32, _: f32, _: f32) {}
unsafe extern "system" fn unit(_: u32) {}
unsafe extern "system" fn depth_mask(value: u8) {
    STATE.with(|s| s.borrow_mut().depth_masks.push(value));
}
unsafe extern "system" fn blend_func(_: u32, _: u32) {}
unsafe extern "system" fn color_mask(_: u8, _: u8, _: u8, _: u8) {}
unsafe extern "system" fn draw_elements(_: u32, count: i32, _: u32, _: *const c_void) {
    STATE.with(|s| s.borrow_mut().draws.push(count));
}

fn loader(name: &CStr) -> *const c_void {
    macro_rules! f {
        ($f:ident) => {
            $f as *const () as *const c_void
        };
    }
    match name.to_bytes() {
        b"glGetError" => f!(get_error),
        b"glGetIntegerv" => f!(get_integer),
        b"glCreateShader" => f!(create_shader),
        b"glShaderSource" => f!(shader_source),
        b"glCompileShader" => f!(compile_shader),
        b"glGetShaderiv" => f!(get_shader),
        b"glGetShaderInfoLog" => f!(shader_log),
        b"glDeleteShader" => f!(delete_shader),
        b"glCreateProgram" => f!(create_program),
        b"glAttachShader" => f!(attach_shader),
        b"glLinkProgram" => f!(link_program),
        b"glGetProgramiv" => f!(get_program),
        b"glGetProgramInfoLog" => f!(program_log),
        b"glDeleteProgram" => f!(delete_program),
        b"glUseProgram" => f!(use_program),
        b"glGetUniformLocation" => f!(uniform_location),
        b"glGetAttribLocation" => f!(attrib_location),
        b"glUniform1i" => f!(uniform_1i),
        b"glUniform1f" => f!(uniform_1f),
        b"glUniform3f" => f!(uniform_3f),
        b"glUniformMatrix4fv" => f!(uniform_matrix),
        b"glGenBuffers" | b"glGenTextures" => f!(generate),
        b"glDeleteBuffers" => f!(delete_buffers),
        b"glDeleteTextures" => f!(delete_textures),
        b"glBindBuffer" | b"glBindTexture" => f!(bind),
        b"glBufferData" => f!(buffer_data),
        b"glEnableVertexAttribArray" | b"glDisableVertexAttribArray" => f!(attrib_enable),
        b"glVertexAttribPointer" => f!(attrib_pointer),
        b"glActiveTexture" => f!(active_texture),
        b"glTexParameteri" => f!(parameter),
        b"glPixelStorei" => f!(pixel_store),
        b"glTexImage2D" => f!(tex_image),
        b"glTexSubImage2D" => f!(tex_sub_image),
        b"glViewport" => f!(viewport),
        b"glClearColor" => f!(clear_color),
        b"glClear" | b"glEnable" | b"glDisable" | b"glDepthFunc" | b"glBlendEquation" => f!(unit),
        b"glDepthMask" => f!(depth_mask),
        b"glBlendFunc" => f!(blend_func),
        b"glColorMask" => f!(color_mask),
        b"glDrawElements" => f!(draw_elements),
        _ => ptr::null(),
    }
}

fn reset() {
    STATE.with(|s| *s.borrow_mut() = State::default());
}
fn vertex() -> Vertex {
    Vertex {
        pos: [0.0; 3],
        uv: [0.0; 2],
        light_uv: [0.5; 2],
        color: [1.0; 4],
    }
}

#[test]
fn initialization_errors_are_reported_and_partial_resources_cleaned() {
    reset();
    let missing = unsafe {
        Renderer::new(|name| {
            if name.to_bytes() == b"glCreateShader" {
                std::ptr::dangling::<c_void>()
            } else {
                loader(name)
            }
        })
    };
    assert!(matches!(
        missing,
        Err(Error::MissingFunction("glCreateShader"))
    ));
    STATE.with(|s| s.borrow_mut().shader_fails = true);
    let failure = unsafe { Renderer::new(loader) };
    assert!(matches!(failure, Err(Error::Shader(ref s)) if s == "bad shader"));
    STATE.with(|s| assert_eq!(s.borrow().deleted_shaders, [1]));
}

#[test]
fn upload_update_draw_and_destroy_obey_resource_and_pass_contracts() {
    reset();
    unsafe {
        let mut renderer = Renderer::new(loader).unwrap();
        assert_eq!(
            renderer.create_texture(3, 2, &[0; 24], TextureFilter::Nearest),
            Err(Error::InvalidTexture)
        );
        assert_eq!(
            renderer.create_texture(4, 4, &[0; 7], TextureFilter::Nearest),
            Err(Error::InvalidTexture)
        );
        let atlas = renderer
            .create_texture(4, 4, &[255; 64], TextureFilter::Nearest)
            .unwrap();
        let lightmap = renderer
            .create_texture(1, 1, &[255; 4], TextureFilter::Linear)
            .unwrap();
        assert_eq!(
            renderer.update_texture(atlas, 3, 0, 2, 1, &[0; 8]),
            Err(Error::InvalidTexture)
        );
        renderer.update_texture(atlas, 2, 1, 2, 1, &[0; 8]).unwrap();
        assert_eq!(
            renderer.begin_frame(0, 1, [0.0; 4]),
            Err(Error::InvalidViewport)
        );
        renderer.begin_frame(20, 20, [0.0, 0.0, 1.0, 1.0]).unwrap();
        let vertices = [vertex(); 4];
        let indices = [0u16, 1, 2, 0, 2, 3];
        let mut draw = Draw {
            vertices: &vertices,
            indices: &indices,
            state: DrawState {
                atlas,
                lightmap,
                view_projection: [1.0; 16],
                section_origin: [0.0; 3],
                pass: Pass::Opaque,
                alpha_cutoff: 0.5,
            },
        };
        renderer.draw(&draw).unwrap();
        draw.state.pass = Pass::Translucent;
        renderer.draw(&draw).unwrap();
        draw.indices = &[0, 1, 4];
        assert_eq!(renderer.draw(&draw), Err(Error::InvalidMesh));
        renderer.delete_texture(atlas).unwrap();
        assert_eq!(renderer.delete_texture(atlas), Err(Error::InvalidTexture));
        assert_eq!(
            renderer.update_texture(atlas, 0, 0, 1, 1, &[0; 4]),
            Err(Error::InvalidTexture)
        );
        renderer.destroy();
    }
    STATE.with(|state| {
        let s = state.borrow();
        assert_eq!(s.draws, [6, 6]);
        assert_eq!(
            s.buffer_sizes,
            [
                (ARRAY_BUFFER, 176),
                (ELEMENT_ARRAY_BUFFER, 12),
                (ARRAY_BUFFER, 176),
                (ELEMENT_ARRAY_BUFFER, 12)
            ]
        );
        assert_eq!(s.depth_masks, [1, 1, 0]);
        assert_eq!(s.texture_updates, 1);
        assert_eq!(s.deleted_textures.len(), 2);
        assert_eq!(s.deleted_buffers.len(), 2);
        assert_eq!(s.deleted_shaders.len(), 2);
        assert_eq!(s.deleted_programs.len(), 1);
    });
}

#[test]
fn handles_from_other_contexts_are_rejected() {
    reset();
    unsafe {
        let mut first = Renderer::new(loader).unwrap();
        let mut second = Renderer::new(loader).unwrap();
        let texture = first
            .create_texture(1, 1, &[0; 4], TextureFilter::Nearest)
            .unwrap();
        assert_eq!(
            second.update_texture(texture, 0, 0, 1, 1, &[0; 4]),
            Err(Error::InvalidTexture)
        );
        first.destroy();
        second.destroy();
    }
}

#[test]
fn vertex_layout_matches_gl_attribute_offsets() {
    assert_eq!(std::mem::size_of::<Vertex>(), 44);
    assert_eq!(std::mem::offset_of!(Vertex, pos), 0);
    assert_eq!(std::mem::offset_of!(Vertex, uv), 12);
    assert_eq!(std::mem::offset_of!(Vertex, light_uv), 20);
    assert_eq!(std::mem::offset_of!(Vertex, color), 28);
}

#[test]
fn resident_mesh_is_uploaded_once_and_handles_expire_on_delete() {
    reset();
    unsafe {
        let mut renderer = Renderer::new(loader).unwrap();
        let atlas = renderer
            .create_texture(1, 1, &[255; 4], TextureFilter::Nearest)
            .unwrap();
        let state = DrawState {
            atlas,
            lightmap: atlas,
            view_projection: [1.0; 16],
            section_origin: [0.0; 3],
            pass: Pass::Opaque,
            alpha_cutoff: 0.5,
        };
        let vertices = [vertex(); 4];
        let indices = [0, 1, 2, 0, 2, 3];
        assert_eq!(
            renderer.create_mesh(&vertices, &[0, 2, 4]),
            Err(Error::InvalidMesh)
        );
        let mesh = renderer.create_mesh(&vertices, &indices).unwrap();
        renderer.draw_mesh(mesh, &state).unwrap();
        renderer.draw_mesh(mesh, &state).unwrap();
        STATE.with(|s| {
            let s = s.borrow();
            assert_eq!(s.buffer_sizes.len(), 2);
            assert_eq!(s.draws, [6, 6]);
        });
        renderer.delete_mesh(mesh).unwrap();
        assert_eq!(renderer.draw_mesh(mesh, &state), Err(Error::InvalidMesh));
        assert_eq!(renderer.delete_mesh(mesh), Err(Error::InvalidMesh));
        let another = renderer.create_mesh(&vertices, &indices).unwrap();
        assert_ne!(mesh, another);
        renderer.destroy();
    }
    STATE.with(|s| assert_eq!(s.borrow().deleted_buffers.len(), 6));
}
