//! Only GL 2.0 entry points. The context owns all temporary probe GL objects;
//! destroying it releases them, including on early-return/error paths.
use horizon_legacy_render::Context;
use std::ffi::{CStr, CString, c_char, c_void};
use std::fmt::Write;
use std::ptr;

macro_rules! entry_points {
    ($($field:ident: $name:literal ($($arg:ty),*) -> $ret:ty;)*) => {
        struct Gl<'a> {
            _context: &'a Context,
            $($field: unsafe extern "system" fn($($arg),*) -> $ret,)*
        }
        impl<'a> Gl<'a> {
            fn load(context: &'a Context) -> Result<Self, String> {
                Ok(Self {
                    _context: context,
                    $($field: {
                        let address = context.address($name)?;
                        // SAFETY: each fixed symbol has the declared desktop GL ABI.
                        unsafe { std::mem::transmute::<*const c_void, unsafe extern "system" fn($($arg),*) -> $ret>(address) }
                    },)*
                })
            }
        }
    }
}
entry_points! {
    get_string: c"glGetString" (u32) -> *const u8;
    get_integer: c"glGetIntegerv" (u32, *mut i32) -> ();
    get_error: c"glGetError" () -> u32;
    create_shader: c"glCreateShader" (u32) -> u32;
    shader_source: c"glShaderSource" (u32, i32, *const *const c_char, *const i32) -> ();
    compile_shader: c"glCompileShader" (u32) -> ();
    shader_integer: c"glGetShaderiv" (u32, u32, *mut i32) -> ();
    shader_log: c"glGetShaderInfoLog" (u32, i32, *mut i32, *mut c_char) -> ();
    delete_shader: c"glDeleteShader" (u32) -> ();
    create_program: c"glCreateProgram" () -> u32;
    attach_shader: c"glAttachShader" (u32, u32) -> ();
    bind_attribute: c"glBindAttribLocation" (u32, u32, *const c_char) -> ();
    link_program: c"glLinkProgram" (u32) -> ();
    program_integer: c"glGetProgramiv" (u32, u32, *mut i32) -> ();
    program_log: c"glGetProgramInfoLog" (u32, i32, *mut i32, *mut c_char) -> ();
    use_program: c"glUseProgram" (u32) -> ();
    uniform_location: c"glGetUniformLocation" (u32, *const c_char) -> i32;
    uniform1f: c"glUniform1f" (i32, f32) -> ();
    uniform4f: c"glUniform4f" (i32, f32, f32, f32, f32) -> ();
    gen_buffers: c"glGenBuffers" (i32, *mut u32) -> ();
    bind_buffer: c"glBindBuffer" (u32, u32) -> ();
    buffer_data: c"glBufferData" (u32, isize, *const c_void, u32) -> ();
    attribute_pointer: c"glVertexAttribPointer" (u32, i32, u32, u8, i32, *const c_void) -> ();
    enable_attribute: c"glEnableVertexAttribArray" (u32) -> ();
    viewport: c"glViewport" (i32, i32, i32, i32) -> ();
    clear_color: c"glClearColor" (f32, f32, f32, f32) -> ();
    clear: c"glClear" (u32) -> ();
    enable: c"glEnable" (u32) -> ();
    draw_elements: c"glDrawElements" (u32, i32, u32, *const c_void) -> ();
    read_pixels: c"glReadPixels" (i32, i32, i32, i32, u32, u32, *mut c_void) -> ();
}

pub struct Report {
    pub renderer: String,
    pub version: String,
    pub buffer_swap: bool,
    pub backend_checks: Vec<&'static str>,
    vendor: String,
    context: String,
    glsl: String,
    limits: Vec<(&'static str, i32)>,
}
impl Report {
    pub fn json(&self, shader_version: &str) -> String {
        let mut fields = vec![
            "\"status\":\"passed\"".into(),
            format!("\"buffer_swap\":{}", self.buffer_swap),
            format!("\"pointer_bits\":{}", usize::BITS),
            format!("\"target_os\":{}", quote(std::env::consts::OS)),
            format!("\"context\":{}", quote(&self.context)),
            "\"checks\":[\"shader_compile_link\",\"vbo_ebo_draw\",\"clear_readback\",\"triangle_readback\",\"depth_readback\",\"invalid_shader_rejected\",\"no_gl_error\"]".into(),
        ];
        fields.push(format!(
            "\"backend_checks\":[{}]",
            self.backend_checks
                .iter()
                .map(|s| quote(s))
                .collect::<Vec<_>>()
                .join(",")
        ));
        for (name, value) in [
            ("vendor", self.vendor.as_str()),
            ("renderer", self.renderer.as_str()),
            ("gl_version", self.version.as_str()),
            ("glsl_version", self.glsl.as_str()),
            ("shader_version", shader_version),
        ] {
            fields.push(format!("\"{name}\":{}", quote(value)));
        }
        for (name, value) in &self.limits {
            fields.push(format!("\"{name}\":{value}"));
        }
        format!("{{{}}}", fields.join(","))
    }
}
fn quote(value: &str) -> String {
    let mut output = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\x00'..='\x1f' => write!(output, "\\u{:04x}", c as u32).unwrap(),
            _ => output.push(c),
        }
    }
    output.push('"');
    output
}

impl Gl<'_> {
    fn string(&self, token: u32) -> Result<String, String> {
        // SAFETY: token is one of the GL2 string queries; context is current and
        // GL returns a borrowed NUL-terminated string, copied before context drop.
        unsafe {
            let value = (self.get_string)(token);
            if value.is_null() {
                return Err(format!("glGetString({token:#x}) returned null"));
            }
            Ok(CStr::from_ptr(value.cast()).to_string_lossy().into_owned())
        }
    }

    fn shader(&self, kind: u32, source: &str) -> Result<u32, String> {
        let source = CString::new(source).map_err(|e| e.to_string())?;
        // SAFETY: valid GL2 shader kind; source is a live NUL-terminated string.
        // Driver copies it. Query outputs and log storage are writable and sized.
        unsafe {
            let shader = (self.create_shader)(kind);
            if shader == 0 {
                return Err("glCreateShader failed".into());
            }
            (self.shader_source)(shader, 1, &source.as_ptr(), ptr::null());
            (self.compile_shader)(shader);
            let mut success = 0;
            (self.shader_integer)(shader, 0x8B81, &mut success); // COMPILE_STATUS
            if success == 0 {
                let mut length = 0;
                (self.shader_integer)(shader, 0x8B84, &mut length); // INFO_LOG_LENGTH
                let mut log = vec![0u8; length.max(1) as usize];
                (self.shader_log)(
                    shader,
                    log.len() as i32,
                    ptr::null_mut(),
                    log.as_mut_ptr().cast(),
                );
                (self.delete_shader)(shader);
                return Err(format!(
                    "shader compile failed: {}",
                    String::from_utf8_lossy(&log).trim_end_matches('\0')
                ));
            }
            Ok(shader)
        }
    }
}

pub fn probe(context: &Context, shader_version: &str) -> Result<Report, String> {
    context.make_current()?;
    // Inspect the GL1-era strings first: GDI Generic/old contexts often have no
    // shader symbols at all. Fail with a driver diagnosis, not a vague load error.
    let address = context.address(c"glGetString")?;
    // SAFETY: fixed GL1 ABI, provided by the live current context's loader.
    let get_string = unsafe {
        std::mem::transmute::<*const c_void, unsafe extern "system" fn(u32) -> *const u8>(address)
    };
    // SAFETY: VERSION and RENDERER return borrowed NUL-terminated strings.
    let (version, renderer) = unsafe {
        let (version, renderer) = (get_string(0x1F02), get_string(0x1F01));
        if version.is_null() || renderer.is_null() {
            return Err("driver returned null GL identity".into());
        }
        (
            CStr::from_ptr(version.cast())
                .to_string_lossy()
                .into_owned(),
            CStr::from_ptr(renderer.cast())
                .to_string_lossy()
                .into_owned(),
        )
    };
    Context::qualify(&version, &renderer)?;
    let gl = Gl::load(context)?;
    let mut report = Report {
        vendor: gl.string(0x1F00)?,
        context: context.description().into(),
        renderer,
        version,
        buffer_swap: false,
        backend_checks: Vec::new(),
        glsl: gl.string(0x8B8C)?,
        limits: Vec::new(),
    };
    for (name, token, minimum) in [
        ("max_texture_size", 0x0D33, 64),
        ("max_texture_units", 0x8872, 2),
        ("max_vertex_attributes", 0x8869, 1),
        ("max_vertex_uniform_components", 0x8B4A, 16),
        ("max_fragment_uniform_components", 0x8B49, 4),
        ("max_varying_floats", 0x8B4B, 0),
        ("depth_bits", 0x0D56, 16),
    ] {
        let mut value = 0;
        // SAFETY: valid GL2 scalar integer query and writable single integer.
        unsafe { (gl.get_integer)(token, &mut value) };
        if value < minimum {
            return Err(format!("{name}={value}, probe needs at least {minimum}"));
        }
        report.limits.push((name, value));
    }
    let vertex = gl.shader(0x8B31, &format!(
        "#version {shader_version}\nattribute vec2 position; uniform float depth;\nvoid main() {{ gl_Position = vec4(position, depth, 1.0); }}"
    ))?;
    let fragment = gl.shader(0x8B30, &format!(
        "#version {shader_version}\nuniform vec4 color;\nvoid main() {{ gl_FragColor = color; }}"
    ))?;
    // SAFETY: all GL calls use the current context and declared GL2 ABI. Buffer
    // pointers/sizes match live arrays; attribute pointer and index offsets refer
    // to bound buffers. Readback is exactly one RGBA8 pixel (4 writable bytes).
    unsafe {
        let program = (gl.create_program)();
        (gl.attach_shader)(program, vertex);
        (gl.attach_shader)(program, fragment);
        (gl.bind_attribute)(program, 0, c"position".as_ptr());
        (gl.link_program)(program);
        let mut linked = 0;
        (gl.program_integer)(program, 0x8B82, &mut linked);
        if linked == 0 {
            let mut length = 0;
            (gl.program_integer)(program, 0x8B84, &mut length);
            let mut log = vec![0u8; length.max(1) as usize];
            (gl.program_log)(
                program,
                log.len() as i32,
                ptr::null_mut(),
                log.as_mut_ptr().cast(),
            );
            return Err(format!(
                "program link failed: {}",
                String::from_utf8_lossy(&log)
            ));
        }
        (gl.use_program)(program);
        let color = (gl.uniform_location)(program, c"color".as_ptr());
        let depth = (gl.uniform_location)(program, c"depth".as_ptr());
        if color < 0 || depth < 0 {
            return Err("required shader uniforms missing".into());
        }
        let vertices: [f32; 6] = [-0.75, -0.75, 0.75, -0.75, 0.0, 0.75];
        let indices: [u16; 3] = [0, 1, 2];
        let mut buffers = [0u32; 2];
        (gl.gen_buffers)(2, buffers.as_mut_ptr());
        (gl.bind_buffer)(0x8892, buffers[0]); // ARRAY_BUFFER
        (gl.buffer_data)(
            0x8892,
            size_of_val(&vertices) as isize,
            vertices.as_ptr().cast(),
            0x88E4,
        );
        (gl.bind_buffer)(0x8893, buffers[1]); // ELEMENT_ARRAY_BUFFER
        (gl.buffer_data)(
            0x8893,
            size_of_val(&indices) as isize,
            indices.as_ptr().cast(),
            0x88E4,
        );
        (gl.attribute_pointer)(0, 2, 0x1406, 0, 8, ptr::null()); // FLOAT
        (gl.enable_attribute)(0);
        (gl.viewport)(0, 0, 64, 64);
        (gl.clear_color)(0.125, 0.25, 0.5, 1.0);
        (gl.clear)(0x4000 | 0x0100); // COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT
        (gl.enable)(0x0B71); // DEPTH_TEST, default LESS
        (gl.uniform1f)(depth, -0.5);
        (gl.uniform4f)(color, 0.0, 1.0, 0.0, 1.0);
        (gl.draw_elements)(0x0004, 3, 0x1403, ptr::null()); // TRIANGLES, UNSIGNED_SHORT
        let read = |x, y| {
            let mut pixel = [0u8; 4];
            (gl.read_pixels)(x, y, 1, 1, 0x1908, 0x1401, pixel.as_mut_ptr().cast());
            pixel
        };
        check_rgb("clear outside triangle", read(1, 1), [32, 64, 128])?;
        check_rgb("triangle center", read(32, 32), [0, 255, 0])?;
        // A farther red triangle must not overwrite the nearer green triangle.
        (gl.uniform1f)(depth, 0.5);
        (gl.uniform4f)(color, 1.0, 0.0, 0.0, 1.0);
        (gl.draw_elements)(0x0004, 3, 0x1403, ptr::null());
        check_rgb("depth rejection", read(32, 32), [0, 255, 0])?;
    }
    if gl
        .shader(0x8B30, "#version 110\nthis is deliberately invalid;")
        .is_ok()
    {
        return Err("driver accepted deliberately invalid GLSL".into());
    }
    // SAFETY: current context; no parameters or memory accesses.
    let error = unsafe { (gl.get_error)() };
    if error != 0 {
        return Err(format!("GL error after probe: {error:#x}"));
    }
    Ok(report)
}

fn check_rgb(label: &str, actual: [u8; 4], expected: [u8; 3]) -> Result<(), String> {
    if actual[..3]
        .iter()
        .zip(expected)
        .any(|(&a, b)| a.abs_diff(b) > 2)
    {
        Err(format!(
            "{label}: expected RGB {expected:?}, read {actual:?}"
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readback_accepts_rounding_but_not_wrong_pixels() {
        assert!(check_rgb("pixel", [31, 65, 128, 0], [32, 64, 128]).is_ok());
        assert!(check_rgb("pixel", [0, 0, 0, 255], [0, 255, 0]).is_err());
    }
    #[test]
    fn json_escapes_driver_strings() {
        assert_eq!(
            quote("ATI\n\"GPU\"\\\t\0"),
            "\"ATI\\u000a\\\"GPU\\\"\\\\\\u0009\\u0000\""
        );
    }
}
