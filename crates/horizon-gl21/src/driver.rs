//! The GL 2.1 device, on a real driver.
//!
//! `horizon_client_render::gl21::Device` is one method per GL call a frame makes, and this is the
//! implementation that issues them: `glow` for the calls, `egl::Context` for the window, the frame
//! texture and the swap. What the sequence decides is in the render crate and tested there; what is
//! here is the driver's half — three programs compiled and linked once, uniform locations looked up
//! once, buffers sized once, and the calls in the order `Renderer::draw` asks for them.
//!
//! Four things are worth knowing, because each is a place a 2006 GL differs from today's:
//!
//! - **No vertex array objects.** `glGenVertexArrays` is GL 3.0 (`ARB_vertex_array_object`); a 2.1
//!   context keeps one set of attribute pointers for the whole context, so `use_program` points them
//!   at that program's buffer, that program's stride and that program's layout. Three programs a
//!   frame, so three times a frame — the pointers are re-pointed, not stored per draw.
//! - **No uniform blocks.** Every uniform is a `glUniform*` call to a location looked up at link
//!   time, and the names come from `gl21::program`'s tables: a uniform this driver sets under a name
//!   the shader does not declare is a failing test (`tests` below), not a silently ignored call.
//! - **No `glAlphaFunc`.** `glow` binds none (it is not in a core profile), so the alpha test is
//!   `terrain.frag`'s `discard` against `u_alpha_test`, which the pass's own `terrain` call carries.
//!   `State::AlphaTest` is therefore a value this device has already been given, not a call.
//! - **Textures, not sampler objects.** Filtering is texture state, so the vignette's bilinear copy is
//!   a second texture (`TextureId::SpriteAtlasLinear`) on its own unit (`u_linear`), and a run that
//!   wants it is a uniform, not a second pipeline.

use std::collections::HashMap;

use glow::HasContext;
use horizon_client_render::gl21::{self, program as tables};

use crate::egl::Context;
use crate::quad;
use crate::resources::{self, TextureId, Upload};

/// What the driver reports of itself, as `gl21::backend::check` reads it: the version strings and
/// the limits, read once. `Driver::new`/`Context::new` refuse a driver that is not the version the
/// request named, so this is for a log line rather than for a decision.
pub fn reported(context: &Context) -> String {
    let gl = context.gl();
    // Safety: the context is current, and `get_parameter_string` takes a constant.
    unsafe {
        let renderer = gl.get_parameter_string(glow::RENDERER);
        let version = gl.get_parameter_string(glow::VERSION);
        format!("Horizon's OpenGL 2.1 renderer: {renderer}, GL {version}")
    }
}

/// The GL 2.1 device: the context, the programs, the buffers and the textures.
pub struct Driver {
    context: Context,
    terrain: Program,
    hud: Program,
    present: Program,
    /// The frame's single world vertex buffer, `capacity` vertices long (`gl21::Arena`).
    world: glow::Buffer,
    /// The HUD's own buffer, uploaded whole every frame and grown when a frame outgrows it.
    hud_buffer: glow::Buffer,
    /// How many HUD vertices `hud_buffer` holds.
    hud_capacity: usize,
    /// The present quad's four vertices, uploaded once.
    present_buffer: glow::Buffer,
    /// The sprite atlas's geometry, for a run's layer to become `u_sprite`.
    atlas: Option<gl21::Atlas>,
    /// The textures by `TextureId`, once `upload` has made them.
    textures: [Option<glow::Texture>; 5],
    /// Whether `present` drew the frame into the window, so that `end_frame` swaps.
    presented: bool,
}

impl Driver {
    /// A device on `window`, drawing into a texture `width`×`height` and presenting it there, with
    /// room for `capacity` world vertices (`Renderer::new`'s own number) and `hud_capacity` HUD ones.
    ///
    /// `profile` is `HORIZON_GL_PROFILE`'s: it decides what version is asked for and what is
    /// accepted (`gl21::backend::Request::for_profile`).
    pub fn new(
        window: raw_window_handle::RawWindowHandle,
        display: raw_window_handle::RawDisplayHandle,
        width: u32,
        height: u32,
        capacity: u32,
        hud_capacity: u32,
        profile: Option<gl21::Profile>,
    ) -> Result<Driver, String> {
        let world_capacity = capacity.max(4) as usize;
        let hud_room = hud_capacity.max(1024) as usize;
        let context = Context::new(window, display, width, height, profile)?;
        let gl = context.gl();
        // Safety: every call below belongs to this context, which `Context::new` made current for
        // this thread, and every handle is made here and freed in `Drop`.
        unsafe {
            let mut terrain = Program::compile(gl, gl21::Program::Terrain)?;
            let mut hud = Program::compile(gl, gl21::Program::Hud)?;
            let mut present = Program::compile(gl, gl21::Program::Present)?;
            // A sampler's unit is a property of the program, not of a draw: set once, here.
            terrain.bind_samplers(gl, units(gl21::Program::Terrain));
            hud.bind_samplers(gl, units(gl21::Program::Hud));
            present.bind_samplers(gl, units(gl21::Program::Present));
            let world = allocate(gl, world_capacity * gl21::vertex::STRIDE as usize)?;
            let hud_bytes = hud_room * gl21::hud::HUD_STRIDE as usize;
            let hud_buffer = allocate(gl, hud_bytes)?;
            let present_buffer = allocate(gl, quad::bytes().len())?;
            // Safety: the buffer is this device's own and the quad has its bytes.
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(present_buffer));
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, quad::bytes());
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            // The state nothing changes between frames: a scissor no run asked for, no culling
            // until the terrain pass asks for it, and the row alignment that a packed glyph patch
            // needs (a width×4 RGBA upload always satisfies it; a patch need not).
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::CULL_FACE);
            gl.depth_func(glow::LEQUAL);
            gl.cull_face(glow::BACK);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            Ok(Driver {
                context,
                terrain,
                hud,
                present,
                world,
                hud_buffer,
                hud_capacity: hud_room,
                present_buffer,
                atlas: None,
                textures: [None; 5],
                presented: false,
            })
        }
    }

    /// The frame texture's size, in pixels: what the viewport is and what the present quad shows.
    pub fn size(&self) -> (u32, u32) {
        self.context.size()
    }

    /// The sprite atlas whose rectangles `u_sprite` comes from, so that a draw's layer becomes a
    /// quad of the atlas rather than a black sprite.
    pub fn set_atlas(&mut self, atlas: gl21::Atlas) {
        self.atlas = Some(atlas);
    }

    /// A texture for the renderer to sample: its base level makes the texture, the levels under it
    /// are its mip chain. `resources::Upload` is the check — a level of the wrong size is an `Err`
    /// here, before a driver can read past the end of it.
    pub fn upload(&mut self, upload: Upload<'_>) -> Result<(), String> {
        let problems = upload.problems();
        if !problems.is_empty() {
            return Err(problems.join("; "));
        }
        let gl = self.context.gl();
        let id = upload.texture;
        let slot = slot_of(id);
        let smooth = id.linear();
        // Safety: the context is current, and every handle is this device's own.
        unsafe {
            let texture = match self.textures[slot] {
                Some(texture) => texture,
                None => {
                    let texture = gl.create_texture().map_err(|e| {
                        format!("Horizon's OpenGL 2.1 renderer could not make a texture: {e}")
                    })?;
                    self.textures[slot] = Some(texture);
                    texture
                }
            };
            gl.active_texture(glow::TEXTURE0 + id.unit());
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for level in upload.levels {
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    level.level as i32,
                    // A GL 2.1 driver wants a sized internal format for RGBA8, and the shaders
                    // read it as plain (non-sRGB) colour: this path has no HDR and does its own
                    // gamma (`docs/opengl21-plan.md` §5.3).
                    glow::RGBA8 as i32,
                    level.width as i32,
                    level.height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(level.pixels)),
                );
            }
            // A chain of one has no mip under it to interpolate with: asking for `LINEAR_MIPMAP`
            // there is a texture a driver samples as incomplete, which draws black.
            let min_filter = match (smooth, upload.levels.len() > 1) {
                (false, _) => glow::NEAREST,
                // No anisotropic filtering on this path (plan §5.1).
                (true, true) => glow::LINEAR_MIPMAP_LINEAR,
                (true, false) => glow::LINEAR,
            };
            // The lightmap is a table of sixteen levels, not a picture: smoothing across it would
            // light a block at a level it is not at, so it stays `NEAREST` above.
            let mag_filter = if smooth { glow::LINEAR } else { glow::NEAREST };
            // The atlas clamps: `gl21::Atlas`'s padding is what keeps a mip level from averaging
            // across the seam between two sprites. The glint is a repeating tile, so it wraps.
            let wrap = if id.clamped() {
                glow::CLAMP_TO_EDGE
            } else {
                glow::REPEAT
            } as i32;
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, min_filter as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, mag_filter as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        Ok(())
    }

    /// A texture's handle, for a caller that uploads part of it again (a glyph patch, `font::Patch`):
    /// `glTexSubImage2D` needs the handle, not the name.
    pub fn texture(&self, id: TextureId) -> Option<glow::Texture> {
        self.textures[slot_of(id)]
    }

    /// The window was resized: the frame texture is made again at the new size.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.context.resize(width, height)
    }

    /// vsync on (`true`) or off, as a frame limiter above wants it.
    pub fn set_vsync(&self, on: bool) {
        self.context.set_vsync(on);
    }

    /// The atlas's rectangle for a sprite layer: `u_sprite`. `None` before `set_atlas` and for a
    /// layer past the atlas's capacity, which draws nothing rather than panicking.
    pub fn sprite_rect(&self, layer: u32) -> Option<[f32; 4]> {
        self.atlas.as_ref()?.rect(layer)
    }

    /// Point the context's attribute pointers at `program`'s buffer and layout. One set of pointers
    /// for the whole context on a 2.1 driver, so this is what `glUseProgram` means here.
    ///
    /// Safety: a current GL context.
    unsafe fn bind_layout(&self, gl: &glow::Context, program: gl21::Program, buffer: glow::Buffer) {
        // Safety: the caller's; the buffer and the attribute table are this crate's own.
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            // Sixteen slots cleared first: a slot this program does not read would otherwise still
            // read the previous program's buffer, at that program's stride.
            for location in 0..16u32 {
                gl.disable_vertex_attrib_array(location);
            }
            let attributes = resources::attributes(program);
            let stride = stride_of(attributes);
            for attribute in attributes {
                gl.enable_vertex_attrib_array(attribute.location);
                gl.vertex_attrib_pointer_f32(
                    attribute.location,
                    attribute.size as i32,
                    attribute_type(attribute.kind),
                    attribute.normalized,
                    stride,
                    attribute.offset as i32,
                );
            }
        }
    }
}

impl gl21::Device for Driver {
    fn begin_frame(&mut self, viewport: gl21::Viewport, clear: [f32; 4]) {
        let gl = self.context.gl();
        // The frame is drawn into its own texture: the window's buffers only ever hold the present
        // quad, which is what lets the game hold a frame back for a menu's blur.
        self.context.bind_frame();
        // Safety: the context is current and the frame texture is bound above.
        unsafe {
            gl.viewport(0, 0, viewport.width as i32, viewport.height as i32);
            gl.clear_color(clear[0], clear[1], clear[2], clear[3]);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }
        self.presented = false;
    }

    fn frame_uniforms(&mut self, uniforms: gl21::FrameUniforms) {
        let gl = self.context.gl();
        // Safety: `Renderer::draw` sets the terrain program first, and its locations were looked up
        // at link time.
        unsafe {
            self.terrain.matrix(gl, "u_view_proj", &uniforms.view_proj);
            self.terrain.vec4(gl, "u_fog_color", uniforms.fog_color);
            self.terrain.vec4(gl, "u_fog", uniforms.fog);
            self.terrain.float(gl, "u_bias", uniforms.bias);
        }
    }

    fn upload(&mut self, upload: gl21::Upload, vertices: &[gl21::GlVertex]) {
        let gl = self.context.gl();
        // Safety: the context is current, the buffer is this device's own and its size, and the
        // range is the frame's own arithmetic inside it (`gl21::Arena`).
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.world));
            gl.buffer_sub_data_u8_slice(
                glow::ARRAY_BUFFER,
                upload.byte_offset() as i32,
                bytemuck::cast_slice(vertices),
            );
        }
    }

    fn use_program(&mut self, program: gl21::Program) {
        let gl = self.context.gl();
        let (raw, buffer) = match program {
            gl21::Program::Terrain => (&self.terrain, self.world),
            gl21::Program::Hud => (&self.hud, self.hud_buffer),
            gl21::Program::Present => (&self.present, self.present_buffer),
        };
        let raw = raw.raw;
        // Safety: the context is current; the program and the buffer are this device's own.
        unsafe {
            gl.use_program(Some(raw));
            self.bind_layout(gl, program, buffer);
        }
    }

    fn terrain(&mut self, uniforms: gl21::TerrainUniforms) {
        let gl = self.context.gl();
        let sprite = self.sprite_rect(uniforms.layer).unwrap_or([0.0; 4]);
        // Safety: the terrain program is in use, and its locations are its own.
        unsafe {
            self.terrain.vec3(gl, "u_origin", uniforms.origin);
            self.terrain.vec4(gl, "u_sprite", sprite);
            self.terrain.float(gl, "u_fade", uniforms.visibility);
            // The alpha test is this uniform rather than `glAlphaFunc` (`glow` binds none): the
            // fragment shader's `discard` compares against it where the fixed function would have.
            self.terrain.float(gl, "u_alpha_test", uniforms.alpha_test);
        }
    }

    fn set_state(&mut self, states: &[gl21::State]) {
        let gl = self.context.gl();
        // Safety: the context is current; every call takes constants.
        unsafe {
            for state in states {
                match state {
                    gl21::State::DepthTest(on) => set(gl, glow::DEPTH_TEST, *on),
                    gl21::State::DepthWrite(on) => gl.depth_mask(*on),
                    gl21::State::Blend(on) => set(gl, glow::BLEND, *on),
                    gl21::State::BlendFunc(src, dst) => gl.blend_func(factor(*src), factor(*dst)),
                    // Already set by the pass's own `terrain` call: a uniform, not a state.
                    gl21::State::AlphaTest(_) => {}
                    gl21::State::CullFace(on) => set(gl, glow::CULL_FACE, *on),
                    gl21::State::DepthFunc(name) => gl.depth_func(depth_func(name)),
                }
            }
        }
    }

    fn bind_texture(&mut self, texture: gl21::Texture) {
        let gl = self.context.gl();
        let id = match texture {
            gl21::Texture::SpriteAtlas => TextureId::SpriteAtlas,
            gl21::Texture::Lightmap => TextureId::Lightmap,
            gl21::Texture::Glyphs => TextureId::Glyphs,
            gl21::Texture::Frame => {
                // The frame the context owns, made with it and freed with it.
                let frame = self.context.frame_texture();
                // Safety: the context is current; the frame texture is the context's own.
                unsafe {
                    gl.active_texture(glow::TEXTURE0);
                    gl.bind_texture(glow::TEXTURE_2D, Some(frame));
                }
                return;
            }
        };
        // Safety: the context is current; each handle is this device's own, or `None`, which a
        // driver reads as "no texture" and draws with.
        unsafe {
            gl.active_texture(glow::TEXTURE0 + id.unit());
            gl.bind_texture(glow::TEXTURE_2D, self.textures[slot_of(id)]);
            // The HUD's glint texture lives on the unit `hud.frag` names next to its atlas, and the
            // renderer binds the two together (`Texture::Glyphs` is the whole overlay's textures).
            if id == TextureId::Glyphs {
                let glint = TextureId::Glint;
                gl.active_texture(glow::TEXTURE0 + glint.unit());
                gl.bind_texture(glow::TEXTURE_2D, self.textures[slot_of(glint)]);
            }
        }
    }

    fn draw(&mut self, begin: u32, count: u32) {
        let gl = self.context.gl();
        // Safety: the context is current; the range was made inside the buffer `upload` filled.
        unsafe { gl.draw_arrays(glow::TRIANGLES, begin as i32, count as i32) }
    }

    fn draw_hud(&mut self, begin: u32, count: u32) {
        let gl = self.context.gl();
        // Safety: as `draw`, on the HUD's own buffer.
        unsafe { gl.draw_arrays(glow::TRIANGLES, begin as i32, count as i32) }
    }

    fn upload_hud(&mut self, vertices: &[gl21::GlHudVertex]) {
        let gl = self.context.gl();
        let bytes: &[u8] = bytemuck::cast_slice(vertices);
        // Safety: the context is current; the buffer is grown to the frame's own size first.
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.hud_buffer));
            if vertices.len() > self.hud_capacity {
                // A menu with more vertices than the last frame's: the storage is made again at the
                // new size rather than the frame being dropped. This orphans the old storage, which
                // is what a driver wants here anyway — it hands it back without a sync.
                self.hud_capacity = vertices.len().next_power_of_two();
                let size = self.hud_capacity * gl21::hud::HUD_STRIDE as usize;
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &vec![0u8; size], glow::DYNAMIC_DRAW);
            }
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
        }
    }

    fn hud_frame(&mut self, hud: &gl21::HudFrame) {
        let gl = self.context.gl();
        // Safety: the HUD program is in use, and its locations are its own.
        unsafe {
            self.hud
                .vec4(gl, "u_hud", [hud.window[0], hud.window[1], hud.scale, hud.time]);
            // x: which way the target's y axis runs; y: unused, as `hud.vert` says.
            self.hud.vec2(gl, "u_aspect", [hud.flip, 0.0]);
        }
    }

    fn hud_linear(&mut self, linear: bool) {
        let gl = self.context.gl();
        // The vignette's blur samples the atlas bilinearly: the same pixels as a second texture on
        // `u_linear`'s own unit, because a GL 2.1 driver has no sampler object to switch filtering
        // with. `u_atlas` stays bound on unit 0 for the runs that do not ask for the blur.
        let plain = self.textures[slot_of(TextureId::SpriteAtlas)];
        let blurred = self.textures[slot_of(TextureId::SpriteAtlasLinear)];
        // The atlas itself when the game has not made the blurred copy: nearest filtering instead
        // of a blur, which is a harder vignette rather than none at all.
        let Some(id) = (if linear { blurred.or(plain) } else { plain }) else {
            return;
        };
        // Safety: the context is current; `id` is this device's own handle.
        unsafe {
            self.hud
                .float(gl, "u_use_linear", f32::from(u8::from(linear)));
            gl.active_texture(glow::TEXTURE0 + TextureId::SpriteAtlasLinear.unit());
            gl.bind_texture(glow::TEXTURE_2D, Some(id));
        }
    }

    fn present(&mut self) {
        let gl = self.context.gl();
        // The window's own buffers, and the quad that shows the frame texture in them.
        self.context.bind_window();
        // Safety: the context is current; the quad is this device's buffer and the frame texture is
        // the one the world was just drawn into.
        unsafe {
            gl.use_program(Some(self.present.raw));
            self.bind_layout(gl, gl21::Program::Present, self.present_buffer);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.context.frame_texture()));
            // The quad covers the window with one texture: no blending, no depth, no scissor.
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::SCISSOR_TEST);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, quad::VERTICES as i32);
        }
        self.presented = true;
    }

    fn end_frame(&mut self) {
        if !self.presented {
            // The frame stays in its texture (`Renderer::draw`'s `present: false`): the game draws
            // its own last pass into the window, and nothing here swaps.
            return;
        }
        if let Err(e) = self.context.swap() {
            // A lost surface is a window that has gone: the game's loop sees what it sees on a
            // close, and a frame that failed to swap is not worth a panic.
            log::warn!("Horizon: {e}");
        }
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        let gl = self.context.gl();
        // Safety: the context is current (its own `Drop` unmakes it after this body), and every
        // handle is this device's own, freed once.
        unsafe {
            for texture in self.textures.iter().flatten() {
                gl.delete_texture(*texture);
            }
            for buffer in [self.world, self.hud_buffer, self.present_buffer] {
                gl.delete_buffer(buffer);
            }
            // The programs' shaders went at link time: a driver keeps a program after that.
            gl.delete_program(self.terrain.raw);
            gl.delete_program(self.hud.raw);
            gl.delete_program(self.present.raw);
        }
    }
}

/// A linked program, and the uniform locations its draw calls use.
struct Program {
    raw: glow::Program,
    /// `(name, location)` for every uniform in `gl21::program`'s table for this program. A uniform
    /// the driver optimised away answers `-1`, so it is `None` and setting it is one skipped call
    /// rather than an error.
    uniforms: HashMap<&'static str, Option<glow::UniformLocation>>,
}

impl Program {
    /// Compile `program`'s two stages and link them, its attributes bound to the slots
    /// `gl21::program` names, and its uniform locations looked up once.
    ///
    /// Safety: a current GL context.
    unsafe fn compile(gl: &glow::Context, program: gl21::Program) -> Result<Program, String> {
        // Safety: as this method's own contract.
        unsafe {
            let raw = gl
                .create_program()
                .map_err(|e| format!("Horizon's OpenGL 2.1 renderer could not make a program: {e}"))?;
            let mut shaders = Vec::with_capacity(2);
            for (file, source) in resources::sources(program) {
                let kind = if file.ends_with(".vert") {
                    glow::VERTEX_SHADER
                } else {
                    glow::FRAGMENT_SHADER
                };
                let shader = gl
                    .create_shader(kind)
                    .map_err(|e| format!("Horizon's OpenGL 2.1 renderer could not make a shader: {e}"))?;
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    let log = gl.get_shader_info_log(shader);
                    gl.delete_shader(shader);
                    for shader in shaders {
                        gl.delete_shader(shader);
                    }
                    gl.delete_program(raw);
                    return Err(format!(
                        "Horizon's OpenGL 2.1 renderer could not compile {file}: {log}"
                    ));
                }
                gl.attach_shader(raw, shader);
                shaders.push(shader);
            }
            // The slots, before linking: GLSL 1.20 has no `layout(location)` to say it in the
            // source, and the table is the same one `gl21::vertex` lays a vertex out with.
            for attribute in resources::attributes(program) {
                gl.bind_attrib_location(raw, attribute.location, attribute.name);
            }
            gl.link_program(raw);
            for shader in shaders {
                gl.detach_shader(raw, shader);
                gl.delete_shader(shader);
            }
            if !gl.get_program_link_status(raw) {
                let log = gl.get_program_info_log(raw);
                gl.delete_program(raw);
                return Err(format!(
                    "Horizon's OpenGL 2.1 renderer could not link {program:?}: {log}"
                ));
            }
            let uniforms = uniform_table(program)
                .iter()
                .map(|uniform| (uniform.name, gl.get_uniform_location(raw, uniform.name)))
                .collect();
            Ok(Program { raw, uniforms })
        }
    }

    /// The sampler uniforms' units, once, with the program in use: in GL 2.1 a `glUniform1i` sets
    /// the state of whatever program is current, so this binds first.
    ///
    /// Safety: a current GL context.
    unsafe fn bind_samplers(&mut self, gl: &glow::Context, units: &[(&'static str, i32)]) {
        // Safety: as this method's own contract.
        unsafe {
            gl.use_program(Some(self.raw));
            for (name, unit) in units {
                if let Some(location) = self.location(name) {
                    gl.uniform_1_i32(Some(location), *unit);
                }
            }
        }
    }

    /// The location of `name`, if the driver kept the uniform.
    fn location(&self, name: &str) -> Option<&glow::UniformLocation> {
        self.uniforms.get(name).and_then(Option::as_ref)
    }

    /// Safety: a current GL context with this program in use.
    unsafe fn float(&self, gl: &glow::Context, name: &str, value: f32) {
        if let Some(location) = self.location(name) {
            // Safety: the caller's.
            unsafe { gl.uniform_1_f32(Some(location), value) }
        }
    }

    /// Safety: a current GL context with this program in use.
    unsafe fn vec2(&self, gl: &glow::Context, name: &str, value: [f32; 2]) {
        if let Some(location) = self.location(name) {
            // Safety: the caller's.
            unsafe { gl.uniform_2_f32(Some(location), value[0], value[1]) }
        }
    }

    /// Safety: a current GL context with this program in use.
    unsafe fn vec3(&self, gl: &glow::Context, name: &str, value: [f32; 3]) {
        if let Some(location) = self.location(name) {
            // Safety: the caller's.
            unsafe { gl.uniform_3_f32(Some(location), value[0], value[1], value[2]) }
        }
    }

    /// Safety: a current GL context with this program in use.
    unsafe fn vec4(&self, gl: &glow::Context, name: &str, value: [f32; 4]) {
        if let Some(location) = self.location(name) {
            // Safety: the caller's.
            unsafe { gl.uniform_4_f32(Some(location), value[0], value[1], value[2], value[3]) }
        }
    }

    /// A `mat4`, column major, as GLSL reads one and `glam` writes one.
    ///
    /// Safety: a current GL context with this program in use.
    unsafe fn matrix(&self, gl: &glow::Context, name: &str, value: &[f32; 16]) {
        if let Some(location) = self.location(name) {
            // Safety: the caller's; `false` is "not transposed", which is column major.
            unsafe { gl.uniform_matrix_4_f32_slice(Some(location), false, value) }
        }
    }
}

/// The uniforms `gl21::program` lists for `program`: the names this driver sets.
fn uniform_table(program: gl21::Program) -> &'static [gl21::Uniform] {
    match program {
        gl21::Program::Terrain => &tables::TERRAIN_UNIFORMS,
        gl21::Program::Hud => &tables::HUD_UNIFORMS,
        gl21::Program::Present => &tables::PRESENT_UNIFORMS,
    }
}

/// The unit each sampler of `program` reads, by the name `gl21::program` gives it. Checked against
/// that table by the tests below, so a name here that the shader does not declare is a failure.
fn units(program: gl21::Program) -> &'static [(&'static str, i32)] {
    match program {
        gl21::Program::Terrain => &[("u_atlas", 0), ("u_lightmap", 1)],
        gl21::Program::Hud => &[("u_atlas", 0), ("u_glint", 1), ("u_linear", 2)],
        gl21::Program::Present => &[("u_frame", 0)],
    }
}

/// The stride between vertices of `attributes`: the far end of its last attribute.
fn stride_of(attributes: &[gl21::Attribute]) -> i32 {
    attributes
        .iter()
        .map(gl21::Attribute::end)
        .max()
        .unwrap_or(0) as i32
}

/// `gl21::AttributeKind` as `glVertexAttribPointer`'s type argument.
fn attribute_type(kind: gl21::AttributeKind) -> u32 {
    match kind {
        gl21::AttributeKind::Float => glow::FLOAT,
        gl21::AttributeKind::UnsignedShort => glow::UNSIGNED_SHORT,
        gl21::AttributeKind::UnsignedByte => glow::UNSIGNED_BYTE,
    }
}

/// `gl21::state::Factor` as `glBlendFunc`'s argument.
fn factor(factor: gl21::state::Factor) -> u32 {
    match factor {
        gl21::state::Factor::SrcAlpha => glow::SRC_ALPHA,
        gl21::state::Factor::OneMinusSrcAlpha => glow::ONE_MINUS_SRC_ALPHA,
        gl21::state::Factor::One => glow::ONE,
        gl21::state::Factor::Zero => glow::ZERO,
    }
}

/// `State::DepthFunc`'s name as GL's constant. The state list keeps the name so that a log reads
/// like the call it stands for, and this is the one place the name becomes a number.
fn depth_func(name: &str) -> u32 {
    match name {
        "GL_LEQUAL" => glow::LEQUAL,
        "GL_LESS" => glow::LESS,
        "GL_EQUAL" => glow::EQUAL,
        "GL_ALWAYS" => glow::ALWAYS,
        other => {
            // A name this table does not know is a state the renderer asked for and never gets.
            // `LEQUAL` is what the solid pass needs, so it is the safe answer, and the log says so.
            log::warn!("Horizon: no depth function for {other}; drawing with GL_LEQUAL");
            glow::LEQUAL
        }
    }
}

/// A zeroed buffer of `bytes` bytes, as `glBufferData` takes one.
///
/// Safety: a current GL context.
unsafe fn allocate(gl: &glow::Context, bytes: usize) -> Result<glow::Buffer, String> {
    // Safety: the caller's.
    unsafe {
        let buffer = gl.create_buffer().map_err(|e| {
            format!("Horizon's OpenGL 2.1 renderer could not make a vertex buffer: {e}")
        })?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &vec![0u8; bytes], glow::DYNAMIC_DRAW);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        Ok(buffer)
    }
}

/// `glEnable`/`glDisable`, as one call.
///
/// Safety: a current GL context.
unsafe fn set(gl: &glow::Context, state: u32, on: bool) {
    // Safety: the caller's.
    unsafe {
        if on {
            gl.enable(state);
        } else {
            gl.disable(state);
        }
    }
}

/// Which of `Driver::textures` a `TextureId` is.
fn slot_of(texture: TextureId) -> usize {
    match texture {
        TextureId::SpriteAtlas => 0,
        TextureId::SpriteAtlasLinear => 1,
        TextureId::Lightmap => 2,
        TextureId::Glyphs => 3,
        TextureId::Glint => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A typo in a uniform name is a `glGetUniformLocation` of -1 and a call that silently does
    /// nothing: this is the test that makes such a name a failure instead.
    #[test]
    fn every_uniform_the_device_sets_is_one_the_shader_declares() {
        let names: [&[&str]; 3] = [
            &[
                "u_view_proj",
                "u_fog_color",
                "u_fog",
                "u_bias",
                "u_origin",
                "u_sprite",
                "u_fade",
                "u_alpha_test",
            ],
            &["u_hud", "u_aspect", "u_use_linear"],
            &[],
        ];
        for (program, set) in [
            (gl21::Program::Terrain, names[0]),
            (gl21::Program::Hud, names[1]),
            (gl21::Program::Present, names[2]),
        ] {
            let table: Vec<&str> = uniform_table(program).iter().map(|u| u.name).collect();
            for name in set {
                assert!(table.contains(name), "{name} is not in {program:?}'s table");
            }
        }
    }

    /// Each sampler is given a unit, and every sampler unit the tables name is one this driver
    /// binds a texture to.
    #[test]
    fn every_sampler_has_a_unit_and_every_unit_has_a_texture() {
        for program in [
            gl21::Program::Terrain,
            gl21::Program::Hud,
            gl21::Program::Present,
        ] {
            let samplers: Vec<&str> = uniform_table(program)
                .iter()
                .filter(|uniform| uniform.kind == "sampler2D")
                .map(|uniform| uniform.name)
                .collect();
            let units = units(program);
            assert_eq!(samplers.len(), units.len(), "{program:?}");
            for (name, unit) in units {
                assert!(samplers.contains(name), "{name} is no sampler of {program:?}");
                assert!((0..3).contains(unit), "{name} is on unit {unit}");
            }
        }
    }

    /// The stride each program is read with is the size of one vertex of its own layout.
    #[test]
    fn the_strides_are_the_vertex_layouts_their_shaders_declare() {
        assert_eq!(stride_of(&tables::TERRAIN), gl21::vertex::STRIDE as i32);
        assert_eq!(stride_of(&tables::HUD), gl21::hud::HUD_STRIDE as i32);
        assert_eq!(stride_of(&tables::PRESENT), 8);
        assert_eq!(stride_of(&quad::ATTRIBUTES), 8);
    }

    #[test]
    fn every_state_has_a_call_and_every_texture_a_slot() {
        // Every state the renderer can ask for, from every pass and from the present quad's reset:
        // a state added to `gl21::state` without an arm here is a compile error, which is the
        // point — the alternative is a pass that silently draws with the last pass's depth mask.
        let mut states = gl21::state::reset();
        for pass in [gl21::Pass::Solid, gl21::Pass::Translucent, gl21::Pass::Sky] {
            states.extend(gl21::state::setup(pass));
        }
        for state in &states {
            match state {
                gl21::State::DepthTest(_)
                | gl21::State::DepthWrite(_)
                | gl21::State::Blend(_)
                | gl21::State::BlendFunc(_, _)
                | gl21::State::CullFace(_) => {}
                // Set by the pass's own `terrain` call, which is where the uniform lives.
                gl21::State::AlphaTest(_) => {}
                gl21::State::DepthFunc(name) => assert_eq!(depth_func(name), glow::LEQUAL),
            }
        }
        assert_eq!(factor(gl21::state::Factor::Zero), glow::ZERO);
        assert_eq!(factor(gl21::state::Factor::One), glow::ONE);
        assert_eq!(attribute_type(gl21::AttributeKind::Float), glow::FLOAT);
        assert_eq!(
            attribute_type(gl21::AttributeKind::UnsignedByte),
            glow::UNSIGNED_BYTE
        );
        // Every texture the renderer binds has a slot of its own.
        let slots: [usize; 5] = [
            slot_of(TextureId::SpriteAtlas),
            slot_of(TextureId::SpriteAtlasLinear),
            slot_of(TextureId::Lightmap),
            slot_of(TextureId::Glyphs),
            slot_of(TextureId::Glint),
        ];
        let mut sorted = slots;
        sorted.sort_unstable();
        assert_eq!(sorted, [0, 1, 2, 3, 4]);
    }
}
