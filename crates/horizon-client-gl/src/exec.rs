//! The draw-list executor: a recorded frame becomes GL 2.1 calls, in order, once.
//!
//! This is the module that makes the port real rather than described. Everything above it is
//! either data (`draw.rs`), a budget (`budget.rs`) or a wrapper around one GL object; this is
//! the loop that binds a framebuffer, clears, binds a program, sets a uniform, binds a buffer
//! and calls `glDrawElements` 800 to 1500 times a frame on the target's hardware.
//!
//! Three rules, all of them consequences of the target rather than taste:
//!
//! 1. **State goes through [`StateCache`], never straight to the driver.** A changed state can
//!    flush the pipeline on a 2006 driver, so the executor asks the cache and pays only for
//!    real changes.
//! 2. **Uniforms are set by location, resolved at link time.** `glGetUniformLocation` is a
//!    string lookup in the driver, slow enough on the target that vanilla's own
//!    fixed-function path beats it. [`Program`] resolves every name once and
//!    [`crate::draw::UniformId`] is then an index; a uniform the linker dropped is skipped,
//!    which is how an optional feature costs nothing.
//! 3. **A requirement the device cannot meet stops the frame before its first draw**, naming
//!    the feature, instead of drawing half a frame and letting the player see the half.
//!
//! The executor does not own the GL objects: buffers, programs and textures live in a
//! [`Resources`] table keyed by the ids the draw list carries. A resource reload replaces the
//! object behind an id, and every recorded frame stays valid: that is what lets a texture pack
//! change at runtime on the target without re-recording anything.

use glow::HasContext;

use crate::caps::Feature;
use crate::draw::{self, Clear, Draw, Geometry, UniformId, Value, Viewport};
use crate::error::{Error, Result};
use crate::gl::buffer::{Buffer, VertexFormat};
use crate::gl::program::Program;
use crate::gl::state::{StateCache, StateKey};
use crate::gl::texture::{Sampling, Texture};
use crate::gl::Context;
use crate::multi_draw::{MultiDraw, MultiDrawElements};

/// Why the executor refused something.
///
/// Every variant is a bug in the renderer rather than in the player's machine, except
/// `Missing`, which is the machine: that is the distinction the log needs, because one is
/// fixed by a patch and the other by a driver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecuteError {
    /// A pass needed a feature the context does not have.
    Missing { pass: &'static str, feature: Feature },
    /// A draw named a buffer, program or index buffer the table does not hold.
    NoSuchResource {
        draw: &'static str,
        kind: &'static str,
        id: u32,
    },
    /// A draw's vertex or index slice lies outside its buffer, or its index count is not a
    /// multiple of three. A driver would read past the end and draw garbage; this refuses.
    BadGeometry { draw: &'static str, detail: String },
    /// A batched run was refused before it reached the driver: its counts and offsets do not
    /// agree, so `primcount` would make the driver read past one of the arrays.
    BadRun { detail: String },
}

impl std::fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecuteError::Missing { pass, feature } => write!(
                f,
                "pass `{pass}` needs {} ({}), which this GPU does not have",
                crate::error::feature_name(*feature),
                feature.source()
            ),
            ExecuteError::NoSuchResource { draw, kind, id } => write!(
                f,
                "draw `{draw}` uses {kind} {id}, which is not in the resource table"
            ),
            ExecuteError::BadGeometry { draw, detail } => write!(f, "draw `{draw}`: {detail}"),
            ExecuteError::BadRun { detail } => write!(f, "draw run refused: {detail}"),
        }
    }
}

impl std::error::Error for ExecuteError {}

/// Everything the executor can draw from, keyed by the ids the draw list carries.
///
/// The ids are the renderer's own (`u32`), not GL names, so a reload replaces the GL object
/// behind an id and a recorded frame stays valid. The tables are short (a dozen programs, two
/// atlases, a few buffers), so a linear scan beats a hash map and an insert allocates nothing
/// after the first frame.
#[derive(Default)]
pub struct Resources {
    vertex_buffers: Vec<(u32, Buffer)>,
    index_buffers: Vec<(u32, Buffer)>,
    programs: Vec<(u32, Program)>,
    textures: Vec<(u32, Texture)>,
    targets: Vec<(u32, crate::gl::target::Target)>,
}

impl Resources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_vertex_buffer(&mut self, id: u32, buffer: Buffer) {
        replace(&mut self.vertex_buffers, id, buffer);
    }

    pub fn insert_index_buffer(&mut self, id: u32, buffer: Buffer) {
        replace(&mut self.index_buffers, id, buffer);
    }

    pub fn insert_program(&mut self, id: u32, program: Program) {
        replace(&mut self.programs, id, program);
    }

    pub fn insert_texture(&mut self, id: u32, texture: Texture) {
        replace(&mut self.textures, id, texture);
    }

    pub fn insert_target(&mut self, id: u32, target: crate::gl::target::Target) {
        replace(&mut self.targets, id, target);
    }

    pub fn vertex_buffer(&self, id: u32) -> Option<&Buffer> {
        find(&self.vertex_buffers, id)
    }

    pub fn index_buffer(&self, id: u32) -> Option<&Buffer> {
        find(&self.index_buffers, id)
    }

    pub fn program(&self, id: u32) -> Option<&Program> {
        find(&self.programs, id)
    }

    pub fn texture(&self, id: u32) -> Option<&Texture> {
        find(&self.textures, id)
    }

    pub fn target(&self, id: u32) -> Option<&crate::gl::target::Target> {
        find(&self.targets, id)
    }

    pub fn vertex_buffer_count(&self) -> usize {
        self.vertex_buffers.len()
    }

    pub fn program_count(&self) -> usize {
        self.programs.len()
    }

    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    pub fn target_count(&self) -> usize {
        self.targets.len()
    }

    /// Drops every GL object, in the order that is safe on this driver: framebuffers first
    /// (they reference textures), then programs and buffers. Called at shutdown and before a
    /// context is rebuilt; a drop after the context is gone cannot call the driver at all.
    pub fn destroy_all(&mut self, ctx: &Context) {
        for (_, mut target) in self.targets.drain(..) {
            target.destroy(ctx);
        }
        for (_, mut texture) in self.textures.drain(..) {
            texture.destroy(ctx);
        }
        for (_, mut program) in self.programs.drain(..) {
            program.destroy(ctx);
        }
        for (_, mut buffer) in self.vertex_buffers.drain(..) {
            buffer.destroy(ctx);
        }
        for (_, mut buffer) in self.index_buffers.drain(..) {
            buffer.destroy(ctx);
        }
    }
}

fn find<T>(table: &[(u32, T)], id: u32) -> Option<&T> {
    table.iter().find(|(k, _)| *k == id).map(|(_, v)| v)
}

fn replace<T>(table: &mut Vec<(u32, T)>, id: u32, value: T) {
    match table.iter_mut().find(|(k, _)| *k == id) {
        Some(slot) => slot.1 = value,
        None => table.push((id, value)),
    }
}

/// What one frame cost, measured rather than estimated: the numbers the plan's §11 budget is
/// stated in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub draw_calls: u32,
    pub triangles: u64,
    pub state_changes: u32,
    pub program_binds: u32,
    pub buffer_binds: u32,
    pub uniform_sets: u32,
    pub passes: u32,
    /// Draws refused and skipped, so a frame that half-drew is visible in the log rather than
    /// as a missing wall.
    pub skipped: u32,
}

/// Executes draw lists against a context: the GL 2.1 backend of `draw.rs`.
pub struct Executor {
    state: StateCache,
    format: VertexFormat,
    /// Per-frame statistics, reset by [`Executor::begin_frame`].
    pub stats: FrameStats,
    /// The last completed frame's statistics, for the log and the benchmark.
    pub last_frame: FrameStats,
    /// When set, a refused draw is an error; otherwise it is counted in `skipped` and the frame
    /// continues. A shader reload mid-session should not close the game, but CI should fail.
    pub strict: bool,
    /// The program bound by the last draw, so a run of sections from one pipeline pays one
    /// `glUseProgram`.
    bound_program: Option<u32>,
    /// `GL_EXT_multi_draw_arrays`'s entry points, if the renderer loaded them. Every draw in
    /// this path is still one section at a time: batching a pass's sections into one call
    /// needs the pass's runs collected before they are executed, which is the next step
    /// (docs/gl21-x1650-plan.ru.md §11). What is here is the loader and the fallback, so the
    /// step that uses it does not have to invent either.
    multi_draw: MultiDraw,
}

impl Executor {
    pub fn new(format: VertexFormat) -> Self {
        Self {
            state: StateCache::new(),
            format,
            stats: FrameStats::default(),
            last_frame: FrameStats::default(),
            strict: false,
            bound_program: None,
            multi_draw: MultiDraw::none(),
        }
    }

    /// The same executor with a context's multi-draw entry points.
    ///
    /// Not loaded in [`Executor::new`] because loading touches the driver's function table and
    /// this constructor is what the tests call without a context at all.
    pub fn with_multi_draw(mut self, multi_draw: MultiDraw) -> Self {
        self.multi_draw = multi_draw;
        self
    }

    /// The extension's entry points, for the pass that batches draws.
    pub fn multi_draw(&self) -> &MultiDraw {
        &self.multi_draw
    }

    /// Draws a pass's indexed runs with as few calls as the driver allows.
    ///
    /// The caller binds the vertex and index buffers and the pipeline once (this is what makes
    /// a run a run), then hands every contiguous slice that shares them. Without
    /// `GL_EXT_multi_draw_arrays` this is the same loop the executor's per-draw path makes,
    /// which is why a driver without the extension is slower and not wrong.
    ///
    /// # Safety
    ///
    /// Every offset in `run` must lie inside the bound index buffer and every count must be a
    /// multiple of three; `index_problem` is the check a caller makes before it gets here.
    pub unsafe fn draw_run(
        &mut self,
        ctx: &Context,
        run: &MultiDrawElements,
    ) -> std::result::Result<u32, ExecuteError> {
        if run.is_empty() {
            return Ok(0);
        }
        let calls = unsafe { self.multi_draw.draw_elements(draw::PRIMITIVE_TRIANGLES_GL, run) }
            .map_err(|e| ExecuteError::BadRun {
                detail: e.to_string(),
            })?;
        let gl = ctx.gl();
        if calls > 1 {
            // The fallback: one `glDrawElements` per entry, and each one is a call the
            // statistics must see.
            for (count, offset) in run.counts.iter().zip(&run.offsets) {
                unsafe {
                    gl.draw_elements(
                        draw::PRIMITIVE_TRIANGLES_GL,
                        *count,
                        glow::UNSIGNED_SHORT,
                        *offset as i32,
                    );
                }
                self.stats.draw_calls += 1;
            }
        } else {
            self.stats.draw_calls += 1;
        }
        self.stats.triangles += run.indices() / 3;
        Ok(calls)
    }

    /// Clears the per-frame counters. The state cache forgets too, because a pass may bind a
    /// buffer or change a mask behind its back.
    pub fn begin_frame(&mut self) {
        self.last_frame = self.stats;
        self.stats.state_changes = self.state.changes_frame as u32;
        self.stats = FrameStats {
            state_changes: 0,
            ..FrameStats::default()
        };
        self.state.begin_frame();
        self.bound_program = None;
    }

    pub fn state_changes(&self) -> u64 {
        self.state.changes_frame
    }

    /// Runs every pass of `list`, in order. Returns the first refusal that stopped the frame,
    /// with everything drawn before it on screen: the game shows the sentence and keeps
    /// running, and the log says which pass and which feature.
    pub fn execute(
        &mut self,
        ctx: &Context,
        list: &draw::DrawList,
        resources: &Resources,
    ) -> Result<()> {
        let caps = ctx.caps();
        // One check for the whole frame, before the first pass: a missing feature must not
        // produce a world with no water in it.
        if let Some(feature) = list.unmet(caps).first().copied() {
            return Err(map(ExecuteError::Missing {
                pass: "frame",
                feature,
            }));
        }

        for pass in list.passes() {
            self.stats.passes += 1;
            for requirement in pass.needs.iter() {
                for feature in requirement.features() {
                    if !caps.has(feature) {
                        return Err(map(ExecuteError::Missing {
                            pass: pass.label,
                            feature,
                        }));
                    }
                }
            }
            if let Err(e) =
                self.begin_pass(ctx, pass.target, pass.viewport, pass.clear, resources)
            {
                if self.strict {
                    return Err(map(e));
                }
                self.stats.skipped += 1;
                if ctx.trace() {
                    eprintln!("horizon-gl21: skipped pass `{}`: {e}", pass.label);
                }
                continue;
            }
            for draw in &pass.draws {
                if let Err(e) = self.draw(ctx, draw, resources) {
                    if self.strict {
                        return Err(map(e));
                    }
                    self.stats.skipped += 1;
                    if ctx.trace() {
                        eprintln!("horizon-gl21: skipped draw `{}`: {e}", draw.label);
                    }
                }
            }
        }

        // Leave the window bound at the end of the frame: the next frame's first pass assumes
        // the default framebuffer is current.
        crate::gl::target::Target::unbind(ctx);
        self.state.reset(ctx)?;
        self.stats.state_changes = self.state.changes_frame as u32;
        self.bound_program = None;
        Ok(())
    }

    fn begin_pass(
        &mut self,
        ctx: &Context,
        target: draw::Target,
        viewport: Viewport,
        clear: Clear,
        resources: &Resources,
    ) -> std::result::Result<(), ExecuteError> {
        let gl = ctx.gl();
        match target {
            draw::Target::Screen => {
                crate::gl::target::Target::unbind(ctx);
                // The draw list's viewport origin is top-left, as the game's screens think;
                // GL's is bottom-left. This subtraction is the only place that differs.
                let (_, height) = ctx.drawable_size();
                let y = height.saturating_sub(viewport.y + viewport.height);
                unsafe {
                    gl.viewport(
                        viewport.x as i32,
                        y as i32,
                        viewport.width as i32,
                        viewport.height as i32,
                    );
                }
            }
            draw::Target::Texture(id) => {
                let Some(target) = resources.target(id) else {
                    return Err(ExecuteError::NoSuchResource {
                        draw: "pass",
                        kind: "render target",
                        id,
                    });
                };
                target.bind(ctx);
            }
        }

        if clear.color.is_some() || clear.depth.is_some() || clear.stencil.is_some() {
            // `glClear` of depth does nothing with the depth mask off, which is the state a
            // depth-only pass leaves behind; the same for colour.
            self.state
                .prepare_clear(ctx, clear.color.is_some(), clear.depth.is_some());
            let mut mask = 0;
            if let Some(color) = clear.color {
                unsafe {
                    gl.clear_color(color[0], color[1], color[2], color[3]);
                }
                mask |= glow::COLOR_BUFFER_BIT;
            }
            if let Some(depth) = clear.depth {
                unsafe {
                    gl.clear_depth_f32(depth);
                }
                mask |= glow::DEPTH_BUFFER_BIT;
            }
            if let Some(stencil) = clear.stencil {
                unsafe {
                    gl.clear_stencil(i32::from(stencil));
                }
                mask |= glow::STENCIL_BUFFER_BIT;
            }
            unsafe { gl.clear(mask) };
        }
        Ok(())
    }

    /// One draw: the hot path. Everything that can be skipped is skipped, and the order is
    /// chosen so a refusal happens before any state is changed.
    fn draw(
        &mut self,
        ctx: &Context,
        draw: &Draw,
        resources: &Resources,
    ) -> std::result::Result<(), ExecuteError> {
        let gl = ctx.gl();
        let Some(program) = resources.program(draw.pipeline) else {
            return Err(ExecuteError::NoSuchResource {
                draw: draw.label,
                kind: "program",
                id: draw.pipeline,
            });
        };
        let Some(vertices) = resources.vertex_buffer(draw.geometry.buffer) else {
            return Err(ExecuteError::NoSuchResource {
                draw: draw.label,
                kind: "vertex buffer",
                id: draw.geometry.buffer,
            });
        };
        if draw.geometry.index_count % 3 != 0 {
            return Err(ExecuteError::BadGeometry {
                draw: draw.label,
                detail: format!(
                    "{} indices is not a whole number of triangles",
                    draw.geometry.index_count
                ),
            });
        }
        if let Some(detail) = geometry_problem(draw.geometry, vertices.size()) {
            return Err(ExecuteError::BadGeometry {
                draw: draw.label,
                detail,
            });
        }

        // 1. State first: a wrong-state draw on the target is a wrong image, not a slow one.
        let key = StateKey::from_draw(draw.state);
        self.state.apply(ctx, key).map_err(|e| ExecuteError::BadGeometry {
            draw: draw.label,
            detail: e.to_string(),
        })?;

        // 2. The program, then its uniforms: a uniform belongs to whichever program is current.
        if self.bound_program != Some(draw.pipeline) {
            unsafe {
                match program.raw_program() {
                    Some(raw) => gl.use_program(Some(raw)),
                    None => return Err(ExecuteError::NoSuchResource {
                        draw: draw.label,
                        kind: "live program",
                        id: draw.pipeline,
                    }),
                }
            }
            self.bound_program = Some(draw.pipeline);
            self.stats.program_binds += 1;
        }
        for (id, value) in draw.uniforms.iter() {
            if apply_uniform(ctx, program, id, value) {
                self.stats.uniform_sets += 1;
            }
        }

        // 3. Geometry. The vertex format is re-applied when the buffer changes, never per draw:
        // without a vertex array object, `glVertexAttribPointer` is the expensive call.
        vertices.bind(ctx);
        self.format.apply(ctx);
        self.stats.buffer_binds += 1;

        if draw.geometry.indices == 0 {
            let first = (draw.geometry.vertex_offset / draw.geometry.stride) as i32;
            unsafe {
                gl.draw_arrays(
                    draw::PRIMITIVE_TRIANGLES_GL,
                    first,
                    draw.geometry.index_count as i32,
                );
            }
            self.stats.draw_calls += 1;
        } else {
            let Some(indices) = resources.index_buffer(draw.geometry.indices) else {
                return Err(ExecuteError::NoSuchResource {
                    draw: draw.label,
                    kind: "index buffer",
                    id: draw.geometry.indices,
                });
            };
            if let Some(detail) = index_problem(draw.geometry, indices.size()) {
                return Err(ExecuteError::BadGeometry {
                    draw: draw.label,
                    detail,
                });
            }
            indices.bind(ctx);
            let offset = draw.geometry.index_offset as i32;
            let count = draw.geometry.index_count as i32;
            // `instances > 1` is an extension on 2.1 and its absence is not a failure: the
            // same geometry is drawn once per instance, which the frame's section runs make
            // cheap enough (docs/gl21-x1650-plan.ru.md §11).
            for _ in 0..draw.instances.max(1) {
                unsafe {
                    gl.draw_elements(
                        draw::PRIMITIVE_TRIANGLES_GL,
                        count,
                        glow::UNSIGNED_SHORT,
                        offset,
                    );
                }
                self.stats.draw_calls += 1;
            }
        }
        self.stats.triangles +=
            u64::from(draw.geometry.triangles()) * u64::from(draw.instances.max(1));
        Ok(())
    }

    /// Binds a texture to a unit with the sampling a pass wants.
    ///
    /// On a 2.1 context wrap and filter are texture state, not a sampler object, so asking for
    /// a different sampling than the texture already has re-uploads its parameters. That is
    /// cheap, but it is not free, and a pass that alternates samplings pays it per draw: the
    /// reason `Sampling` is compared and the reason this lives in one function.
    pub fn bind_texture(
        &self,
        ctx: &Context,
        unit: u32,
        texture: &Texture,
        sampling: Sampling,
    ) -> Result<()> {
        let max = ctx.caps().limits.max_texture_units;
        if unit >= max {
            return Err(Error::budget(
                "texture unit",
                format!("unit {unit}, and this GPU has {max} texture units"),
            ));
        }
        let gl = ctx.gl();
        unsafe {
            gl.active_texture(glow::TEXTURE0 + unit);
            gl.bind_texture(glow::TEXTURE_2D, texture.raw());
        }
        if texture.sampling() != sampling {
            let mut copy = Texture::describe(
                texture,
                texture.sampling().with_wrap_and_filters(sampling),
            );
            copy.apply_sampling(ctx)?;
        }
        Ok(())
    }
}

/// The static check that a draw's vertex slice lies inside its buffer.
pub fn geometry_problem(geometry: Geometry, buffer_size: usize) -> Option<String> {
    let stride = geometry.stride.max(1);
    let start = u64::from(geometry.vertex_offset);
    let end = start + u64::from(stride);
    if end > buffer_size as u64 {
        return Some(format!(
            "vertex offset {} + stride {} is past the end of {} bytes",
            geometry.vertex_offset, geometry.stride, buffer_size
        ));
    }
    None
}

/// The static check that a draw's index slice lies inside its buffer: indices are `u16`.
pub fn index_problem(geometry: Geometry, buffer_size: usize) -> Option<String> {
    let start = u64::from(geometry.index_offset);
    let end = start + u64::from(geometry.index_count) * 2;
    if end > buffer_size as u64 {
        return Some(format!(
            "indices {}+{} pass the end of {} bytes",
            geometry.index_offset, geometry.index_count, buffer_size
        ));
    }
    None
}

/// Sets one uniform by its resolved location. Returns `false` for a uniform the linker dropped
/// (never an error: that is how an optional feature is compiled out).
fn apply_uniform(ctx: &Context, program: &Program, id: UniformId, value: Value) -> bool {
    let Some(location) = program.location(id).cloned() else {
        return false;
    };
    let gl = ctx.gl();
    unsafe {
        match value {
            Value::Float(v) => gl.uniform_1_f32(Some(&location), v),
            Value::Vec2(v) => gl.uniform_2_f32_slice(Some(&location), &v),
            Value::Vec3(v) => gl.uniform_3_f32_slice(Some(&location), &v),
            Value::Vec4(v) => gl.uniform_4_f32_slice(Some(&location), &v),
            Value::Mat4(m) => gl.uniform_matrix_4_f32_slice(Some(&location), false, &m),
            Value::Int(v) => gl.uniform_1_i32(Some(&location), v),
            // A sampler's value is the unit the pass bound it to.
            Value::Sampler(unit) => gl.uniform_1_i32(Some(&location), unit as i32),
        }
    }
    ctx.count_uniform();
    true
}

fn map(e: ExecuteError) -> Error {
    match e {
        ExecuteError::Missing { feature, .. } => Error::Missing(feature),
        other => Error::Unsupported(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{Blend, Cull, DepthState, DrawList, State, Target};

    #[test]
    fn a_resource_table_replaces_by_id_instead_of_growing() {
        let mut table: Vec<(u32, &str)> = Vec::new();
        replace(&mut table, 7, "first");
        replace(&mut table, 7, "second");
        assert_eq!(table.len(), 1);
        assert_eq!(find(&table, 7), Some(&"second"));
        assert_eq!(find(&table, 8), None);
    }

    #[test]
    fn a_draw_naming_a_missing_program_says_which_draw_and_which_program() {
        let e = ExecuteError::NoSuchResource {
            draw: "terrain opaque",
            kind: "program",
            id: 12,
        };
        let text = e.to_string();
        assert!(text.contains("terrain opaque"), "{text}");
        assert!(text.contains("program 12"), "{text}");
    }

    #[test]
    fn a_missing_feature_becomes_the_feature_error_not_a_string() {
        let mapped = map(ExecuteError::Missing {
            pass: "water",
            feature: Feature::FloatRenderTargets,
        });
        assert!(matches!(mapped, Error::Missing(Feature::FloatRenderTargets)));
    }

    #[test]
    fn a_vertex_slice_past_the_end_of_its_buffer_is_refused() {
        let geometry = Geometry {
            buffer: 1,
            vertex_offset: 992,
            stride: 32,
            indices: 0,
            index_offset: 0,
            index_count: 6,
            base_vertex: 0,
        };
        assert_eq!(geometry_problem(geometry, 1024), None, "the last vertex fits");
        assert!(geometry_problem(geometry, 1000).is_some());
        let past = Geometry {
            vertex_offset: 1024,
            ..geometry
        };
        assert!(geometry_problem(past, 1024).is_some());
        // A stride of zero would divide by zero in the first-vertex arithmetic; treat it as 1.
        let zero = Geometry {
            stride: 0,
            vertex_offset: 0,
            ..geometry
        };
        assert_eq!(geometry_problem(zero, 32), None);
    }

    #[test]
    fn an_index_slice_past_the_end_of_its_buffer_is_refused() {
        let geometry = Geometry {
            buffer: 1,
            vertex_offset: 0,
            stride: 32,
            indices: 2,
            index_offset: 0,
            index_count: 512,
            base_vertex: 0,
        };
        assert_eq!(index_problem(geometry, 1024), None);
        assert!(index_problem(geometry, 1022).is_some());
    }

    #[test]
    fn a_draw_that_is_not_a_whole_number_of_triangles_is_refused() {
        let whole = |count: u32| count % 3 == 0;
        assert!(whole(6));
        assert!(!whole(5));
        assert!(whole(0), "an empty draw is legal and draws nothing");
    }

    #[test]
    fn the_state_key_of_a_draw_keeps_the_state_that_matters() {
        let state = State {
            blend: Blend::Alpha,
            depth: DepthState::ReadOnly,
            cull: Cull::Back,
        };
        let key = StateKey::from_draw(state);
        assert_eq!(key.blend, Blend::Alpha);
        assert_eq!(key.depth, DepthState::ReadOnly);
        assert_eq!(key.cull, Cull::Back);
        assert!(key.color_mask, "an ordinary draw always writes colour");
    }

    #[test]
    fn a_full_screen_pass_lands_at_the_bottom_of_the_window() {
        // The draw list's origin is top-left; GL's is bottom-left. This is the only place the
        // difference exists, so it is worth pinning.
        let flip = |window_height: u32, y: u32, h: u32| window_height.saturating_sub(y + h);
        assert_eq!(flip(600, 0, 600), 0, "a full-screen pass starts at GL's y=0");
        assert_eq!(flip(600, 450, 150), 0, "the bottom half of a 600px window");
        assert_eq!(flip(600, 0, 300), 300, "the top half of a 600px window");
        assert_eq!(flip(768, 700, 200), 0, "past the bottom clamps, never wraps");
    }

    #[test]
    fn the_path_draws_triangles_and_nothing_else() {
        // A merged section mesh packs its runs as triangles: strips would need a restart index,
        // which is 3.0.
        assert_eq!(draw::PRIMITIVE_TRIANGLES_GL, glow::TRIANGLES);
    }

    #[test]
    fn frame_statistics_start_at_zero_and_carry_the_last_frame() {
        let mut executor = Executor::new(VertexFormat::legacy());
        assert_eq!(executor.stats, FrameStats::default());
        assert_eq!(executor.last_frame, FrameStats::default());
        executor.stats.draw_calls = 812;
        executor.stats.triangles = 240_000;
        executor.begin_frame();
        assert_eq!(executor.last_frame.draw_calls, 812);
        assert_eq!(executor.last_frame.triangles, 240_000);
        assert_eq!(executor.stats.draw_calls, 0);
    }

    #[test]
    fn a_pass_with_no_draws_is_still_a_pass() {
        let mut list = DrawList::new();
        list.begin_pass(
            Target::Screen,
            Viewport::full(1024, 768),
            Clear::COLOR_DEPTH,
            "clear only",
        );
        assert_eq!(list.draw_count(), 0);
        assert_eq!(list.passes().len(), 1);
        assert_eq!(list.triangle_count(), 0);
    }

    #[test]
    fn the_vertex_format_the_executor_rebinds_is_one_layout() {
        let format = VertexFormat::legacy();
        assert_eq!(format.attributes.len(), 5);
        // Spans are a bytes question, not an offsets question: 12 + 8 + 4 + 4 + 12 is 40, and
        // the last attribute ends exactly at the stride (gl/buffer.rs explains the 12).
        let last = format.attributes.last().unwrap();
        assert_eq!(last.end(), format.stride);
        // A merged pass does not get a second stride: same bytes, different meaning.
        assert_eq!(VertexFormat::with_origin().stride, format.stride);
    }
}
