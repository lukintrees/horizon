//! Compiles and links the GLSL 1.20 programs of the OpenGL 2 path.

use glow::HasContext;

/// Compiles both stages and links them, binding `attributes` to locations 0, 1, ... in order.
/// Needs a current context.
pub fn program(
    gl: &glow::Context,
    vertex: &str,
    fragment: &str,
    attributes: &[&str],
) -> Result<glow::Program, String> {
    let vertex_shader = stage(gl, glow::VERTEX_SHADER, vertex)?;
    let fragment_shader = match stage(gl, glow::FRAGMENT_SHADER, fragment) {
        Ok(shader) => shader,
        Err(error) => {
            // SAFETY: the context is current on this thread, as the callers require.
            unsafe { gl.delete_shader(vertex_shader) };
            return Err(error);
        }
    };
    // SAFETY: the context is current on this thread, as the callers require.
    unsafe {
        let program = gl.create_program()?;
        gl.attach_shader(program, vertex_shader);
        gl.attach_shader(program, fragment_shader);
        for (index, name) in attributes.iter().enumerate() {
            gl.bind_attrib_location(program, index as u32, name);
        }
        gl.link_program(program);
        gl.detach_shader(program, vertex_shader);
        gl.detach_shader(program, fragment_shader);
        gl.delete_shader(vertex_shader);
        gl.delete_shader(fragment_shader);
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(format!("a program does not link: {log}"));
        }
        Ok(program)
    }
}

fn stage(gl: &glow::Context, kind: u32, source: &str) -> Result<glow::Shader, String> {
    // SAFETY: the context is current on this thread, as `program` requires.
    unsafe {
        let shader = gl.create_shader(kind)?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            return Err(format!("a shader does not compile: {log}"));
        }
        Ok(shader)
    }
}
