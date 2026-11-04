//! One frame, described once, drawn by either backend.
//!
//! Today the frame is described inside `Renderer::render` (4 015 lines in `gfx.rs`) with
//! wgpu calls interleaved, so a second renderer would mean a second copy of it. This module
//! is the small shared vocabulary that lets the legacy path record the frame and a backend
//! (this crate's GL 2.1 one now, the wgpu one when it moves) execute it:
//!
//! * a **pass** is a target, a viewport and the clears that start it;
//! * a **draw** is a program, a slice of a vertex buffer, a handful of uniforms, and — this
//!   is the part the target forces — an **origin**, because a 2.1 context cannot put
//!   per-section data in a vertex array divisor: the section's origin is a uniform on this
//!   path and an instance attribute on the wgpu path, and both are the same field here;
//! * **state** is recorded only when it changes. On a 2006 driver every state change can
//!   flush the pipeline, and the sections arrive sorted by texture and side precisely so the
//!   changes happen once per run rather than once per section.
//!
//! Nothing here touches GL or wgpu: it is plain data, which is why it is also the part of
//! the port that can be unit-tested on a machine with no GPU at all.

use crate::caps::Feature;

/// Something a pass needs from the device, so a backend can refuse it once, before drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// Render to a texture rather than the window.
    RenderToTexture,
    /// More than one colour attachment.
    MultipleRenderTargets,
    /// A float colour target (HDR, TAA history).
    FloatTarget,
    /// Several draws in one call, for the frame's big passes.
    MultiDraw,
    /// The same geometry repeated with per-instance data.
    Instancing,
    /// Depth read back or sampled.
    DepthTexture,
}

impl Requirement {
    /// The capabilities this needs. Empty means every 2.1 context can draw it.
    pub fn features(self) -> &'static [Feature] {
        match self {
            Requirement::RenderToTexture => &[Feature::FramebufferObject],
            Requirement::MultipleRenderTargets => &[Feature::FramebufferObject],
            Requirement::FloatTarget => &[Feature::FloatRenderTargets],
            Requirement::MultiDraw => &[Feature::MultiDrawArrays],
            Requirement::Instancing => &[Feature::InstancedArrays],
            Requirement::DepthTexture => &[Feature::DepthTexture],
        }
    }
}

/// Where a pass draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The window's back buffer.
    Screen,
    /// A texture, by the renderer's own id.
    Texture(u32),
}

/// The clears that start a pass. `None` keeps what was there (a pass over the world's depth).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clear {
    pub color: Option<[f32; 4]>,
    pub depth: Option<f32>,
    pub stencil: Option<u8>,
}

impl Clear {
    pub const NONE: Clear = Clear {
        color: None,
        depth: None,
        stencil: None,
    };
    pub const COLOR_DEPTH: Clear = Clear {
        color: Some([0.0, 0.0, 0.0, 1.0]),
        depth: Some(1.0),
        stencil: None,
    };
}

/// A viewport in pixels, origin top-left as the game's screens think of it (the backend flips
/// it: GL's origin is bottom-left, and that one line lives in the backend, once).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }
}

/// Fixed-function state a draw depends on. Compared by value, so a backend can skip the GL
/// calls when nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub blend: Blend,
    pub depth: DepthState,
    pub cull: Cull,
}

impl Default for State {
    fn default() -> Self {
        Self {
            blend: Blend::Off,
            depth: DepthState::Off,
            cull: Cull::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Off,
    /// `src * a + dst * (1 - a)`, the HUD, glass and water.
    Alpha,
    /// Premultiplied source over destination, used by sprite frames.
    Premultiplied,
    /// `src + dst`, glint, lightning, the energy swirl.
    Additive,
    /// The vignette's `dst * (1 - src)`.
    Multiply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthState {
    Off,
    /// Test and write.
    ReadWrite,
    /// Test, no write: translucent terrain, the hand over the world.
    ReadOnly,
    /// No test, write: the depth reset before the hand.
    WriteOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cull {
    None,
    Back,
    Front,
}

/// `GL_TRIANGLES`, named here so the draw list owns the one primitive the path uses and the
/// executor does not reach into `glow` for it.
pub const PRIMITIVE_TRIANGLES_GL: u32 = 0x0004;

/// A uniform's identity in a program: resolved when the program is linked, then an index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UniformId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    /// Column-major, as GL wants it.
    Mat4([f32; 16]),
    Int(i32),
    /// A texture unit already bound by the pass; the value is the unit number.
    Sampler(u32),
}

/// One draw's uniforms. Most draws share most of them: a run of sections differs only in its
/// origin, so this is a short list, not a copy of the whole block.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Uniforms {
    values: Vec<(UniformId, Value)>,
}

impl Uniforms {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, id: UniformId, value: Value) -> &mut Self {
        match self.values.iter_mut().find(|(i, _)| *i == id) {
            Some(slot) => slot.1 = value,
            None => self.values.push((id, value)),
        }
        self
    }

    pub fn get(&self, id: UniformId) -> Option<Value> {
        self.values.iter().find(|(i, _)| *i == id).map(|(_, v)| *v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (UniformId, Value)> + '_ {
        self.values.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// A slice of a vertex buffer: where its vertices start and how many indices to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// Buffer id, as the mesh uploader allocated it.
    pub buffer: u32,
    /// First byte of the vertices.
    pub vertex_offset: u32,
    /// Byte stride: 32 for a terrain vertex (`crate::mesh::VERTEX_BYTES`), which is not the
    /// 24 the mesher uses on the wgpu side (gl/buffer.rs has the layout and why).
    pub stride: u32,
    /// Index buffer id, or `0` for a non-indexed draw (fullscreen quads).
    pub indices: u32,
    /// First index.
    pub index_offset: u32,
    pub index_count: u32,
    /// Added to every index by the draw, so sections can share one index buffer.
    pub base_vertex: i32,
}

impl Geometry {
    pub fn triangles(&self) -> u32 {
        self.index_count / 3
    }
}

/// One draw: what to draw, with what program, where, and with which uniforms.
#[derive(Clone, Debug, PartialEq)]
pub struct Draw {
    /// The pipeline: program plus its fixed-function state and vertex layout. An index into
    /// the renderer's table, resolved once at startup.
    pub pipeline: u32,
    pub geometry: Geometry,
    pub state: State,
    /// Section origin in blocks, relative to the camera. A 2.1 context gets it as a uniform
    /// (`u_origin`); a backend with instancing can put it in a buffer instead. The terrain
    /// shader subtracts it in integer space before converting to float, which is how the
    /// wgpu path avoids the jitter that large coordinates cause in single precision — the
    /// same arithmetic has to survive the port, so it is a field here rather than a build
    /// detail of a backend.
    pub origin: [f32; 3],
    /// 1 for an ordinary draw; more repeats it with the same state.
    pub instances: u32,
    pub uniforms: Uniforms,
    /// A debug label, shown by a validation layer or by `HORIZON_GL21_TRACE`.
    pub label: &'static str,
}

impl Draw {
    pub fn new(pipeline: u32, geometry: Geometry, label: &'static str) -> Self {
        Self {
            pipeline,
            geometry,
            state: State::default(),
            origin: [0.0; 3],
            instances: 1,
            uniforms: Uniforms::new(),
            label,
        }
    }
}

/// What a pass needs, gathered while it is recorded: a backend refuses the pass before its
/// first draw (and the game shows the reason) instead of drawing half of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Needs(Vec<Requirement>);

impl Needs {
    pub fn add(&mut self, requirement: Requirement) {
        if !self.0.contains(&requirement) {
            self.0.push(requirement);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = Requirement> + '_ {
        self.0.iter().copied()
    }

    pub fn features(&self) -> impl Iterator<Item = Feature> + '_ {
        self.0.iter().flat_map(|r| r.features().iter().copied())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A render pass: a target, a viewport, its clears, and the draws in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    pub target: Target,
    pub viewport: Viewport,
    pub clear: Clear,
    pub draws: Vec<Draw>,
    pub needs: Needs,
    pub label: &'static str,
}

impl Pass {
    pub fn new(target: Target, viewport: Viewport, clear: Clear, label: &'static str) -> Self {
        Self {
            target,
            viewport,
            clear,
            draws: Vec::new(),
            needs: Needs::default(),
            label,
        }
    }
}

/// The frame. Passes run in order; within a pass, draws run in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    passes: Vec<Pass>,
}

impl DrawList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin_pass(
        &mut self,
        target: Target,
        viewport: Viewport,
        clear: Clear,
        label: &'static str,
    ) -> &mut Pass {
        self.passes.push(Pass::new(target, viewport, clear, label));
        self.passes.last_mut().expect("just pushed")
    }

    pub fn passes(&self) -> &[Pass] {
        &self.passes
    }

    pub fn passes_mut(&mut self) -> &mut [Pass] {
        &mut self.passes
    }

    pub fn clear(&mut self) {
        self.passes.clear();
    }

    pub fn draw_count(&self) -> usize {
        self.passes.iter().map(|p| p.draws.len()).sum()
    }

    pub fn triangle_count(&self) -> u64 {
        self.passes
            .iter()
            .flat_map(|p| p.draws.iter())
            .map(|d| u64::from(d.geometry.triangles()) * u64::from(d.instances))
            .sum()
    }

    /// Every requirement of the whole frame, for one check before anything is drawn.
    pub fn needs(&self) -> Needs {
        let mut all = Needs::default();
        for pass in &self.passes {
            for r in pass.needs.iter() {
                all.add(r);
            }
        }
        all
    }

    /// The requirements the device does not meet; the caller turns this into one sentence.
    pub fn unmet(&self, caps: &crate::caps::Caps) -> Vec<Feature> {
        let mut missing = Vec::new();
        for feature in self.needs().features() {
            if !caps.has(feature) && !missing.contains(&feature) {
                missing.push(feature);
            }
        }
        missing.sort();
        missing
    }

    /// State changes a run of draws costs, for the frame-time budget: on the target every
    /// one can flush the pipeline, so this number matters more than the draw count.
    pub fn state_changes(&self) -> usize {
        let mut changes = 0;
        for pass in &self.passes {
            let mut previous: Option<State> = None;
            for draw in &pass.draws {
                if previous != Some(draw.state) {
                    changes += 1;
                    previous = Some(draw.state);
                }
            }
        }
        changes
    }
}

/// Records a frame, keeping the state runs that the target needs: a `draw` with the state the
/// previous draw already had costs no state change, and the recorded pass therefore says what
/// the backend should do rather than what the caller happened to call.
pub struct Recorder<'a> {
    list: &'a mut DrawList,
    current: Option<State>,
    debug: bool,
}

impl<'a> Recorder<'a> {
    /// `debug`: keep every draw's label and uniform list (the log's trace, and tests); off in
    /// release frames, where the label is only needed for the first frame of a pass.
    pub fn new(list: &'a mut DrawList, debug: bool) -> Self {
        Self {
            list,
            current: None,
            debug,
        }
    }

    /// Starts a pass; the state of the previous pass does not carry over.
    pub fn begin_pass(
        &mut self,
        target: Target,
        viewport: Viewport,
        clear: Clear,
        label: &'static str,
        needs: impl IntoIterator<Item = Requirement>,
    ) {
        self.current = None;
        let pass = self.list.begin_pass(target, viewport, clear, label);
        if target != Target::Screen {
            pass.needs.add(Requirement::RenderToTexture);
        }
        for requirement in needs {
            pass.needs.add(requirement);
        }
    }

    pub fn draw(&mut self, draw: Draw) {
        let pass = self
            .list
            .passes
            .last_mut()
            .expect("a draw needs a pass: call begin_pass first");
        if draw.instances > 1 {
            pass.needs.add(Requirement::Instancing);
        }
        self.current = Some(draw.state);
        pass.draws.push(draw);
    }

    /// The state the next draw inherits, for a backend or a test.
    pub fn current_state(&self) -> Option<State> {
        self.current
    }

    pub fn debug(&self) -> bool {
        self.debug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::{Caps, Limits, Version};

    fn geometry() -> Geometry {
        Geometry {
            buffer: 1,
            vertex_offset: 0,
            stride: 24,
            indices: 2,
            index_offset: 0,
            index_count: 6,
            base_vertex: 0,
        }
    }

    fn caps_gl21() -> Caps {
        Caps::detect(
            Version::parse("2.1.8545 Release"),
            "1.20",
            "ATI",
            "Radeon X1600/X1650 Series",
            [
                "GL_ARB_framebuffer_object",
                "GL_EXT_multi_draw_arrays",
                "GL_ARB_vertex_array_object",
            ]
            .iter()
            .map(|s| s.to_string()),
            Limits::X1650,
        )
    }

    #[test]
    fn a_draw_with_the_same_state_is_not_a_state_change() {
        let mut list = DrawList::new();
        let mut rec = Recorder::new(&mut list, false);
        rec.begin_pass(Target::Screen, Viewport::full(800, 600), Clear::COLOR_DEPTH, "world", []);
        let mut draw = Draw::new(0, geometry(), "section");
        draw.state = State {
            blend: Blend::Off,
            depth: DepthState::ReadWrite,
            cull: Cull::Back,
        };
        rec.draw(draw.clone());
        rec.draw(draw.clone());
        rec.draw(draw);
        // Three draws, one state: the runs are what the target needs.
        assert_eq!(list.state_changes(), 1);
        assert_eq!(list.draw_count(), 3);
    }

    #[test]
    fn a_state_change_is_counted_once_per_transition() {
        let mut list = DrawList::new();
        let mut rec = Recorder::new(&mut list, false);
        rec.begin_pass(Target::Screen, Viewport::full(64, 64), Clear::NONE, "p", []);
        let mut a = Draw::new(0, geometry(), "a");
        a.state.depth = DepthState::ReadWrite;
        let mut b = Draw::new(0, geometry(), "b");
        b.state.depth = DepthState::ReadOnly;
        rec.draw(a.clone());
        rec.draw(b.clone());
        rec.draw(a);
        assert_eq!(list.state_changes(), 3);
    }

    #[test]
    fn a_texture_target_and_instancing_become_requirements() {
        let mut list = DrawList::new();
        let mut rec = Recorder::new(&mut list, false);
        rec.begin_pass(
            Target::Texture(7),
            Viewport::full(320, 200),
            Clear::COLOR_DEPTH,
            "blur",
            [],
        );
        let mut draw = Draw::new(1, geometry(), "quad");
        draw.instances = 4;
        rec.draw(draw);

        let needs: Vec<Requirement> = list.needs().iter().collect();
        assert!(needs.contains(&Requirement::RenderToTexture));
        assert!(needs.contains(&Requirement::Instancing));
    }

    #[test]
    fn unmet_requirements_are_named() {
        let caps = caps_gl21();
        let mut list = DrawList::new();
        list.begin_pass(Target::Texture(1), Viewport::full(4, 4), Clear::NONE, "t")
            .needs
            .add(Requirement::FloatTarget);
        assert_eq!(list.unmet(&caps), vec![Feature::FloatRenderTargets]);

        let mut ok = DrawList::new();
        ok.begin_pass(Target::Texture(1), Viewport::full(4, 4), Clear::NONE, "t")
            .needs
            .add(Requirement::RenderToTexture);
        assert!(ok.unmet(&caps).is_empty());
    }

    #[test]
    fn triangles_count_instances() {
        let mut list = DrawList::new();
        let mut pass = list.begin_pass(Target::Screen, Viewport::full(8, 8), Clear::NONE, "p");
        let mut draw = Draw::new(0, geometry(), "d");
        draw.instances = 3;
        pass.draws.push(draw);
        // 2 triangles, 3 instances.
        assert_eq!(list.triangle_count(), 6);
    }

    #[test]
    fn uniforms_keep_one_value_per_id() {
        let mut u = Uniforms::new();
        u.set(UniformId(3), Value::Float(1.0));
        u.set(UniformId(3), Value::Float(2.0));
        u.set(UniformId(4), Value::Vec4([0.0; 4]));
        assert_eq!(u.len(), 2);
        assert_eq!(u.get(UniformId(3)), Some(Value::Float(2.0)));
        assert_eq!(u.get(UniformId(9)), None);
    }

    #[test]
    fn a_draw_before_a_pass_is_a_bug_in_the_caller() {
        // Recording order is the caller's contract; the panic names it rather than
        // silently drawing into the previous frame's last pass.
        let mut list = DrawList::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut rec = Recorder::new(&mut list, false);
            rec.draw(Draw::new(0, geometry(), "orphan"));
        }));
        assert!(result.is_err());
    }
}
