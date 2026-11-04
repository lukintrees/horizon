//! The frame's calls, in order, against a device the renderer does not know.
//!
//! `frame.rs` works out the numbers (what to upload, what to draw with which state); this walks them
//! and talks to a `Device`. The whole point of the indirection is that the GL 2.1 renderer's
//! sequence — clear, upload, terrain pass by pass, HUD run by run, present — is then a test: a
//! `Recording` device writes down every call, and the expectations below are what the real driver
//! must see too. The `glow` implementation (Phase 1's last piece, which needs `unsafe` for the
//! function pointers and so lives outside this crate) is a `Device` and nothing else.
//!
//! Three rules of the sequence, each of which has cost a frame somewhere:
//!
//! - **State is set when the pass changes, not per draw.** A pass's draws are contiguous in
//!   `frame.draws()`, so the loop compares the pass with the last one and only then applies
//!   `state::setup`. On a 2006 driver a `glEnable`/`glDepthMask` pair is cheap but not free, and a
//!   section is one draw per sprite.
//! - **The uploads happen before the first draw**, in one `glBufferSubData` each, and a frame that
//!   changed nothing uploads nothing.
//! - **The HUD after the world, the present quad last.** The present quad reads the frame texture, so
//!   it cannot be drawn while the frame is being written into it.

use crate::gl21::draws::Pass;
use crate::gl21::frame::{Draw, Frame, Section, Sky};
use crate::gl21::hud::HudDraws;
use crate::gl21::state::{self, State};
use crate::gl21::vertex::GlVertex;
use crate::gl21::{Upload, frame};

/// Which program a draw uses: the driver's `glUseProgram`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    /// `terrain.vert`/`terrain.frag`: the world.
    Terrain,
    /// `hud.vert`/`hud.frag`: the overlay.
    Hud,
    /// `present.vert`/`present.frag`: the frame texture onto the window.
    Present,
}

/// A texture a program samples, by `glActiveTexture` unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Texture {
    /// The sprite atlas, bound to `u_atlas`.
    SpriteAtlas,
    /// The 16×16 lightmap, bound to `u_lightmap`.
    Lightmap,
    /// The frame Horizon drew, bound to `u_frame`.
    Frame,
    /// The glyph atlas, bound to the HUD's `u_atlas`.
    Glyphs,
}

impl Texture {
    /// The unit the program expects it on, as `program`'s `set_by` says.
    pub fn unit(self) -> u32 {
        match self {
            Texture::SpriteAtlas => 0,
            Texture::Lightmap => 1,
            Texture::Frame => 0,
            Texture::Glyphs => 0,
        }
    }
}

/// What one draw of the terrain program needs besides its vertices: the section's place and the
/// sprite it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainUniforms {
    /// `u_origin`: the section's origin relative to the camera.
    pub origin: [f32; 3],
    /// `u_fade`: the section's shade-in.
    pub visibility: f32,
    /// The sprite the run draws, as a layer of the atlas. The device turns it into `u_sprite` with
    /// `Atlas::rect`, because the atlas (its side, its padding) is the device's, not the frame's.
    pub layer: u32,
    /// `u_alpha_test`, from `Pass::alpha_test`.
    pub alpha_test: f32,
}

/// The values a frame shares between its draws: the camera, the fog, the sampler bias.
///
/// One call a frame, right after `use_program(Program::Terrain)`: a driver keeps a program's
/// uniforms, so setting them once costs a few dozen `glUniform` calls per frame rather than per
/// draw, which on a 2006 CPU is the difference between drawing and stuttering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameUniforms {
    /// `u_view_proj`: the camera's view-projection, column major, as GLSL reads a `mat4`.
    pub view_proj: [f32; 16],
    /// `u_fog_color`: the fog colour, its strength in the alpha.
    pub fog_color: [f32; 4],
    /// `u_fog`: environmental start/end, then the render distance's start/end.
    pub fog: [f32; 4],
    /// `u_bias`: the LOD bias `texture2D` is passed, which GL 2.1 drivers do not apply themselves.
    pub bias: f32,
}

/// A frame's camera, window and clear colour: everything the same for every draw in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// The window the frame draws into, in pixels.
    pub viewport: Viewport,
    /// The colour the frame starts at, as `glClearColor` takes it.
    pub clear: [f32; 4],
    /// The camera's view-projection (`FrameUniforms::view_proj`).
    pub view_proj: [f32; 16],
    /// The fog's colour and strength (`FrameUniforms::fog_color`).
    pub fog_color: [f32; 4],
    /// The fog's two distance pairs (`FrameUniforms::fog`).
    pub fog: [f32; 4],
    /// The sampler's LOD bias (`FrameUniforms::bias`).
    pub bias: f32,
}

impl View {
    /// A view of `viewport` cleared to `clear`, with `view_proj` as the camera: no fog, no bias.
    /// What a caller without a camera yet (a loading screen, a test) draws with.
    pub fn new(viewport: Viewport, clear: [f32; 4], view_proj: [f32; 16]) -> View {
        View {
            viewport,
            clear,
            view_proj,
            fog_color: [0.0, 0.0, 0.0, 0.0],
            fog: [0.0; 4],
            bias: 0.0,
        }
    }

    /// The uniforms this view sets once a frame.
    pub fn uniforms(&self) -> FrameUniforms {
        FrameUniforms {
            view_proj: self.view_proj,
            fog_color: self.fog_color,
            fog: self.fog,
            bias: self.bias,
        }
    }
}

/// The window a frame draws into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    /// A window `width`×`height` pixels.
    pub const fn new(width: u32, height: u32) -> Viewport {
        Viewport { width, height }
    }
}

/// What the renderer needs of a graphics device, one method per call it makes.
///
/// Deliberately small and free of GL types: no `GLuint`, no `glow::Context`. A driver implements it,
/// a test records it, and neither has to know the other.
pub trait Device {
    /// `glViewport` and `glClear` at the start of the frame.
    fn begin_frame(&mut self, viewport: Viewport, clear: [f32; 4]);
    /// The frame's shared uniforms, once, before its first draw.
    fn frame_uniforms(&mut self, uniforms: FrameUniforms);
    /// `glBufferSubData`: `vertices` go to `upload`'s range of the one vertex buffer.
    fn upload(&mut self, upload: Upload, vertices: &[GlVertex]);
    /// `glUseProgram` and its vertex attributes.
    fn use_program(&mut self, program: Program);
    /// The per-draw uniforms of the terrain program.
    fn terrain(&mut self, uniforms: TerrainUniforms);
    /// The state changes of one pass, from `state::setup`.
    fn set_state(&mut self, states: &[State]);
    /// `glActiveTexture` and `glBindTexture`.
    fn bind_texture(&mut self, texture: Texture);
    /// `glDrawArrays(GL_TRIANGLES, begin, count)` of the world's buffer.
    fn draw(&mut self, begin: u32, count: u32);
    /// The same for the HUD's own buffer, which is uploaded whole each frame.
    fn draw_hud(&mut self, begin: u32, count: u32);
    /// The HUD's vertices for this frame: one upload, as `HudBatch` is rebuilt every frame anyway.
    fn upload_hud(&mut self, vertices: &[crate::gl21::GlHudVertex]);
    /// The HUD's own uniforms: the window, the GUI scale, the atlas's place in a pixel.
    fn hud_frame(&mut self, hud: &HudFrame);
    /// `glUniform1f(u_use_linear, ..)` before a HUD run.
    fn hud_linear(&mut self, linear: bool);
    /// The full-screen quad that shows the frame texture in the window: the device's own four
    /// vertices, its own attribute layout and its own `glDrawArrays`. A call of its own rather than
    /// `draw(0, 4)` because the world's buffer is read with the terrain program's layout, which is
    /// not the present quad's.
    fn present(&mut self);
    /// `SwapBuffers`: the frame is done.
    fn end_frame(&mut self);
}

/// What the HUD program needs of the frame: `hud.vert`'s two uniforms and where the sprite atlas
/// begins, which a GL 2.1 driver has to be told because its glyphs are uploaded into the atlas
/// rather than into an atlas of their own (`GlHudVertex::uv`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudFrame {
    /// The window in pixels, and the GUI scale: `u_hud.xy` and `u_hud.z`.
    pub window: [f32; 2],
    /// The GUI scale: pixels per GUI pixel.
    pub scale: f32,
    /// The tick's fraction, which the glint's animation reads (`u_hud.w`).
    pub time: f32,
    /// `u_aspect.x`: 1 for a target whose origin is the top left (the window), -1 for one whose
    /// origin is the bottom left (a frame texture on a GL 2.1 driver).
    pub flip: f32,
}

/// The frame's vertex buffer, and the sequence that draws it.
#[derive(Clone, Debug)]
pub struct Renderer {
    frame: Frame,
    /// What the last frame drew, so that a run of draws in one pass sets its state once.
    last_pass: Option<Pass>,
    /// The frames drawn, for a log line once a second rather than once a frame.
    frames: u64,
}

impl Renderer {
    /// A renderer whose vertex buffer holds `capacity` vertices.
    pub fn new(capacity: u32) -> Renderer {
        Renderer {
            frame: Frame::new(capacity),
            last_pass: None,
            frames: 0,
        }
    }

    /// The frame this renderer stored last.
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// How many frames it has drawn.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Draw one frame: the world's sections, the sky when there is one, then the HUD, then the
    /// present quad.
    ///
    /// `hud` is `None` for a frame with no overlay at all (a screenshot, a loading screen drawn by the
    /// game's own code). `present` draws the frame texture onto the window: a frame the game keeps in
    /// its own texture for the menu's blur does not, and then the last pass into the window is the
    /// game's own.
    pub fn draw(
        &mut self,
        device: &mut dyn Device,
        view: View,
        sections: &[Section<'_>],
        sky: Option<Sky<'_>>,
        hud: Option<&HudDraws>,
        present: bool,
    ) {
        self.frame.build(sections, sky, present);
        device.begin_frame(view.viewport, view.clear);
        // The ranges the GPU has not seen, before anything is drawn out of them.
        for upload in self.frame.uploads() {
            let vertices = self.frame.vertices(upload).to_vec();
            device.upload(upload, &vertices);
        }
        self.last_pass = None;
        device.use_program(Program::Terrain);
        // The camera and the fog, once: a program keeps its uniforms from one draw to the next.
        device.frame_uniforms(view.uniforms());
        device.bind_texture(Texture::SpriteAtlas);
        device.bind_texture(Texture::Lightmap);
        // The frame's draws are read while `last_pass` is written: both fields at once, so that the
        // loop does not borrow the renderer twice.
        let (frame, last_pass) = (&self.frame, &mut self.last_pass);
        for draw in frame.draws() {
            begin_pass(last_pass, device, draw);
            let uniforms = TerrainUniforms {
                origin: draw.origin,
                visibility: draw.visibility,
                layer: u32::from(draw.sprite),
                alpha_test: draw.pass.alpha_test(),
            };
            device.terrain(uniforms);
            device.draw(draw.begin, draw.count);
        }
        if let Some(hud) = hud {
            self.draw_hud(device, view.viewport, hud);
        }
        if self.frame.present() {
            device.use_program(Program::Present);
            device.bind_texture(Texture::Frame);
            device.set_state(&state::reset());
            device.present();
        }
        device.end_frame();
        self.frames += 1;
    }

    /// The overlay: its whole vertex buffer, then one draw per run of blending state.
    fn draw_hud(&mut self, device: &mut dyn Device, viewport: Viewport, hud: &HudDraws) {
        device.use_program(Program::Hud);
        device.bind_texture(Texture::Glyphs);
        device.upload_hud(&hud.vertices);
        let window = [viewport.width as f32, viewport.height as f32];
        device.hud_frame(&HudFrame {
            window,
            scale: hud.scale,
            time: hud.time,
            flip: 1.0,
        });
        let mut last: Option<(bool, bool, bool, bool)> = None;
        for run in &hud.runs {
            if run.count == 0 {
                continue;
            }
            let key = hud_state_key(run.state);
            if last.as_ref() != Some(&key) {
                device.set_state(&state::hud(
                    run.state.blend,
                    run.state.alpha_test,
                    run.state.depth,
                ));
                device.hud_linear(run.state.linear);
                last = Some(key);
            }
            device.draw_hud(run.begin, run.count);
        }
    }
}

/// Apply a pass's state when the pass changes: the draws arrive grouped, so this is one set of calls
/// per pass per frame, not per draw.
fn begin_pass(last_pass: &mut Option<Pass>, device: &mut dyn Device, draw: &Draw) {
    if *last_pass == Some(draw.pass) {
        return;
    }
    device.set_state(&state::setup(draw.pass));
    *last_pass = Some(draw.pass);
}

/// The part of a HUD run's state the sequence compares: the four values `state::hud` and
/// `hud_linear` read. Two runs with the same key need no calls between them.
fn hud_state_key(state: crate::gl21::hud::State) -> (bool, bool, bool, bool) {
    (state.blend, state.alpha_test, state.depth, state.linear)
}

/// The uploads and draws of one frame, as a `Recording` device saw them: for a test, and for the
/// one-line-per-second log a player can attach to a bug report.
pub fn summary(frame: &Frame) -> String {
    let draws = frame.draws();
    let mut passes = [0usize; 3];
    for draw in draws {
        let slot = match draw.pass {
            Pass::Solid => 0,
            Pass::Translucent => 1,
            Pass::Sky => 2,
        };
        passes[slot] += 1;
    }
    format!(
        "{} draw call(s): {} solid, {} see-through, {} sky; {} section(s) stored, {} dropped, {} restart(s); present: {}",
        frame::draw_calls_of(frame),
        passes[0],
        passes[1],
        passes[2],
        frame.stored(),
        frame.dropped(),
        frame.restarts(),
        frame.present()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl21::draws::SKY;
    use crate::gl21::hud::{Blend, HudDraws};
    use crate::mesher::TerrainVertex;

    /// A camera that leaves every vertex where it is, column major.
    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    /// A device that writes down every call, so that the sequence is what a test checks.
    #[derive(Default)]
    struct Recording {
        calls: Vec<String>,
    }

    impl Recording {
        fn push(&mut self, line: impl Into<String>) {
            self.calls.push(line.into());
        }

        fn index_of(&self, needle: &str) -> Option<usize> {
            self.calls.iter().position(|call| call.contains(needle))
        }
    }

    impl Device for Recording {
        fn begin_frame(&mut self, viewport: Viewport, clear: [f32; 4]) {
            self.push(format!(
                "begin {}x{} clear {clear:?}",
                viewport.width, viewport.height
            ));
        }

        fn frame_uniforms(&mut self, uniforms: FrameUniforms) {
            self.push(format!(
                "frame camera {:?} fog {:?} bias {}",
                uniforms.view_proj, uniforms.fog, uniforms.bias
            ));
        }

        fn upload(&mut self, upload: Upload, vertices: &[GlVertex]) {
            self.push(format!(
                "upload {}..{} ({})",
                upload.begin,
                upload.begin + upload.len,
                vertices.len()
            ));
        }

        fn use_program(&mut self, program: Program) {
            self.push(format!("program {program:?}"));
        }

        fn terrain(&mut self, uniforms: TerrainUniforms) {
            self.push(format!(
                "terrain origin {:?} fade {} sprite {} test {}",
                uniforms.origin, uniforms.visibility, uniforms.layer, uniforms.alpha_test
            ));
        }

        fn set_state(&mut self, states: &[State]) {
            self.push(format!("state {}", state::describe(states)));
        }

        fn bind_texture(&mut self, texture: Texture) {
            self.push(format!("texture {texture:?} unit {}", texture.unit()));
        }

        fn draw(&mut self, begin: u32, count: u32) {
            self.push(format!("draw {begin}+{count}"));
        }

        fn draw_hud(&mut self, begin: u32, count: u32) {
            self.push(format!("draw hud {begin}+{count}"));
        }

        fn upload_hud(&mut self, vertices: &[crate::gl21::GlHudVertex]) {
            self.push(format!("upload hud ({})", vertices.len()));
        }

        fn hud_frame(&mut self, hud: &HudFrame) {
            self.push(format!(
                "hud frame {:?} scale {} time {} flip {}",
                hud.window, hud.scale, hud.time, hud.flip
            ));
        }

        fn hud_linear(&mut self, linear: bool) {
            self.push(format!("hud linear {linear}"));
        }

        fn present(&mut self) {
            self.push("present".to_string());
        }

        fn end_frame(&mut self) {
            self.push("end");
        }
    }

    fn vertices(count: usize, sprite: u16) -> Vec<TerrainVertex> {
        (0..count)
            .map(|i| TerrainVertex::new([i as f32, 0.0, 0.0], [0.0, 0.0], sprite, 0, 0))
            .collect()
    }

    fn section<'a>(
        handle: u32,
        solid: &'a [TerrainVertex],
        translucent: &'a [TerrainVertex],
    ) -> Section<'a> {
        Section {
            handle,
            revision: 1,
            // Section-relative positions with the origin at the camera: what the mesher writes,
            // and what `u_origin` adds back.
            origin: [0.0; 3],
            distance_sq: 1.0,
            visibility: 1.0,
            solid,
            translucent,
        }
    }

    /// How many ranges of the vertex buffer the device was sent.
    fn uploads(device: &Recording) -> usize {
        device
            .calls
            .iter()
            .filter(|call| call.starts_with("upload "))
            .count()
    }

    /// `renderer.draw` for a frame of sections only: every test below wants that shape.
    fn draw_world(renderer: &mut Renderer, device: &mut Recording, sections: &[Section<'_>]) {
        let view = View::new(Viewport::new(320, 240), [0.0; 4], IDENTITY);
        renderer.draw(device, view, sections, None, None, true);
    }

    fn hud_batch(verts: usize, blend: Blend) -> HudDraws {
        use crate::hud::{HudBatch, HudVertex};
        let mut batch = HudBatch::new([320, 240]);
        batch.vertices = vec![
            HudVertex {
                pos: [0.0; 2],
                uv: [0.0; 2],
                color: 0,
                depth: 0.0,
                glint_uv: [0.0; 2],
                text: 0,
            };
            verts
        ];
        if blend != Blend::Alpha {
            batch.blends = vec![(0..verts as u32, blend)];
        }
        HudDraws::of(&batch)
    }

    #[test]
    fn the_sequence_is_clear_upload_state_draw_hud_present() {
        let solid = vertices(6, 3);
        let mut renderer = Renderer::new(1024);
        let mut device = Recording::default();
        let hud = hud_batch(6, Blend::Alpha);
        renderer.draw(
            &mut device,
            View::new(Viewport::new(800, 600), [0.1, 0.2, 0.3, 1.0], IDENTITY),
            &[section(1, &solid, &[])],
            None,
            Some(&hud),
            true,
        );
        let calls = &device.calls;
        let first = calls.first().map(String::as_str);
        assert_eq!(first, Some("begin 800x600 clear [0.1, 0.2, 0.3, 1.0]"));
        assert_eq!(calls.last().map(String::as_str), Some("end"));
        // The upload comes before the first draw, and the program before the state.
        let upload = device.index_of("upload ").expect("an upload");
        let program = device
            .index_of("program Terrain")
            .expect("the terrain program");
        let state = device.index_of("state ").expect("a pass's state");
        let draw = device.index_of("draw 0+6").expect("the world's draw");
        let ordered = upload < program && program < state && state < draw;
        assert!(ordered, "{calls:?}");
        // The HUD after the world, on its own program and buffer.
        let hud_upload = device.index_of("upload hud").expect("the hud's vertices");
        let hud_draw = device.index_of("draw hud").expect("the hud's draw");
        assert!(draw < hud_upload && hud_upload < hud_draw, "{calls:?}");
        // And the frame ends with the present quad, which the device draws from its own buffer.
        let present = device
            .index_of("program Present")
            .expect("the present program");
        assert!(hud_draw < present, "{calls:?}");
        assert!(calls.contains(&"present".to_string()), "{calls:?}");
        // The camera's uniforms are set once, after the program and before the first draw.
        let camera = device.index_of("frame camera").expect("the camera");
        assert!(program < camera && camera < state, "{calls:?}");
        assert_eq!(renderer.frames(), 1);
    }

    #[test]
    fn a_pass_sets_its_state_once_however_many_draws_it_has() {
        // Two sections, one sprite each: two solid draws in one pass, one `state` call.
        let a = vertices(6, 3);
        let b = vertices(6, 4);
        let mut renderer = Renderer::new(1024);
        let mut device = Recording::default();
        renderer.draw(
            &mut device,
            View::new(Viewport::new(320, 240), [0.0; 4], IDENTITY),
            &[section(1, &a, &[]), section(2, &b, &[])],
            None,
            None,
            true,
        );
        let states = device
            .calls
            .iter()
            .filter(|call| call.starts_with("state "))
            .count();
        let draws = device
            .calls
            .iter()
            .filter(|call| call.starts_with("draw "))
            .count();
        // One draw per section's run. The present quad is not one of these: the device draws it in
        // `present`, from its own buffer, which is why `Recording::present` logs "present".
        assert_eq!(draws, 2, "one draw per section, both in the solid pass");
        // One state for the solid pass, one for the present quad's reset.
        assert_eq!(states, 2, "{:?}", device.calls);
    }

    #[test]
    fn each_pass_of_a_frame_gets_its_own_state_in_order() {
        let solid = vertices(3, 3);
        let translucent = vertices(3, 4);
        let sky = vertices(6, 0);
        let mut renderer = Renderer::new(1024);
        let mut device = Recording::default();
        renderer.draw(
            &mut device,
            View::new(Viewport::new(320, 240), [0.0; 4], IDENTITY),
            &[section(1, &solid, &translucent)],
            Some((&sky, 7)),
            None,
            true,
        );
        let states: Vec<&String> = device
            .calls
            .iter()
            .filter(|call| call.starts_with("state "))
            .collect();
        let wanted = "sky, solid, see-through, and the present quad's reset";
        assert_eq!(states.len(), 4, "{wanted}: {states:?}");
        assert!(states[0].contains("glDisable(DEPTH_TEST)"), "{states:?}");
        assert!(states[1].contains("glDepthMask(1)"), "{states:?}");
        assert!(states[2].contains("glBlendFunc(GL_SRC_ALPHA"), "{states:?}");
        let reset = states[3].contains("glDepthMask(1)");
        assert!(reset, "the reset: {states:?}");
        // The sky's draw is the first one, before the world's.
        let draws: Vec<&String> = device
            .calls
            .iter()
            .filter(|call| call.starts_with("draw "))
            .collect();
        assert_eq!(draws[0], "draw 0+6", "the sky was stored first: {draws:?}");
        // And the translucent face's draw comes last of the world's: the section's solid run is
        // stored whole before the see-through one, so it is one run further on.
        let last_world = draws.last().expect("the see-through draw");
        assert_eq!(last_world, "draw 9+3", "solid then see-through: {draws:?}");
    }

    #[test]
    fn a_frame_that_changed_nothing_uploads_nothing() {
        let solid = vertices(6, 3);
        let mut renderer = Renderer::new(1024);
        let mut device = Recording::default();
        let sections = [section(1, &solid, &[])];
        draw_world(&mut renderer, &mut device, &sections);
        assert_eq!(uploads(&device), 1);
        device.calls.clear();
        draw_world(&mut renderer, &mut device, &sections);
        let uploads = uploads(&device);
        assert_eq!(uploads, 0, "{:?}", device.calls);
        // The picture is still drawn, though: nothing to send is not nothing to draw.
        assert!(device.calls.iter().any(|call| call == "draw 0+6"));
        assert_eq!(renderer.frames(), 2);
    }

    #[test]
    fn the_hud_sets_its_state_only_when_a_run_changes_it() {
        // Ten vertices, four of them an inverted crosshair: two runs, two states.
        use crate::hud::{HudBatch, HudVertex};
        let mut batch = HudBatch::new([320, 240]);
        batch.vertices = vec![
            HudVertex {
                pos: [0.0; 2],
                uv: [0.0; 2],
                color: 0,
                depth: 0.0,
                glint_uv: [0.0; 2],
                text: 0,
            };
            10
        ];
        batch.blends = vec![(2..6, Blend::Invert)];
        let hud = HudDraws::of(&batch);
        let mut renderer = Renderer::new(256);
        let mut device = Recording::default();
        renderer.draw(
            &mut device,
            View::new(Viewport::new(320, 240), [0.0; 4], IDENTITY),
            &[],
            None,
            Some(&hud),
            true,
        );
        // Only the HUD's own states: the present quad's reset is a state call too, after `program
        // Present`, which is where the HUD's section ends.
        let present = device
            .index_of("program Present")
            .expect("the present program");
        let hud_states = device.calls[..present]
            .iter()
            .filter(|call| call.starts_with("state"))
            .count();
        let hud_draws = device
            .calls
            .iter()
            .filter(|c| c.starts_with("draw hud"))
            .count();
        assert_eq!(hud_draws, 3, "0..2 alpha, 2..6 invert, 6..10 alpha");
        // Three runs, three states: the last alpha run repeats the first one's state, but the
        // invert run in between changed it, so it has to be set again.
        assert_eq!(hud_states, 3, "{:?}", device.calls);
        assert!(device.calls.iter().any(|call| call == "hud linear false"));
    }

    #[test]
    fn a_frame_with_nothing_in_view_still_clears_and_presents() {
        let mut renderer = Renderer::new(256);
        let mut device = Recording::default();
        renderer.draw(
            &mut device,
            View::new(Viewport::new(320, 240), [0.5; 4], IDENTITY),
            &[],
            None,
            None,
            true,
        );
        assert!(device.calls[0].starts_with("begin "));
        assert_eq!(device.calls.last().map(String::as_str), Some("end"));
        // No world draw, but the present quad is there: the cleared frame has to reach the window.
        let world = device
            .calls
            .iter()
            .filter(|c| c.starts_with("draw "))
            .count();
        assert_eq!(world, 0, "{:?}", device.calls);
        assert!(device.calls.iter().any(|call| call == "present"));
    }

    #[test]
    fn a_frame_kept_in_its_own_texture_has_no_present_pass() {
        let solid = vertices(6, 3);
        let mut renderer = Renderer::new(1024);
        let mut device = Recording::default();
        renderer.draw(
            &mut device,
            View::new(Viewport::new(320, 240), [0.0; 4], IDENTITY),
            &[section(1, &solid, &[])],
            None,
            None,
            false,
        );
        assert!(!device.calls.iter().any(|call| call.contains("Present")));
        assert!(!renderer.frame().present());
    }

    #[test]
    fn the_summary_counts_the_draws_by_pass() {
        let solid = vertices(3, 3);
        let translucent = vertices(3, 4);
        let sky = vertices(6, 0);
        let mut renderer = Renderer::new(1024);
        let mut device = Recording::default();
        renderer.draw(
            &mut device,
            View::new(Viewport::new(320, 240), [0.0; 4], IDENTITY),
            &[section(1, &solid, &translucent)],
            Some((&sky, 1)),
            None,
            true,
        );
        let line = summary(renderer.frame());
        assert!(line.contains("1 solid"), "{line}");
        assert!(line.contains("1 see-through"), "{line}");
        assert!(line.contains("1 sky"), "{line}");
        // The present quad makes the calls four: sky, solid, see-through, present.
        assert!(line.contains("4 draw call(s)"), "{line}");
        assert!(line.contains("1 section(s) stored"), "{line}");
        assert!(line.contains("0 dropped"), "{line}");
        assert!(line.contains("present: true"), "{line}");
    }

    #[test]
    fn a_texture_knows_its_unit() {
        assert_eq!(Texture::SpriteAtlas.unit(), 0);
        assert_eq!(Texture::Lightmap.unit(), 1);
        assert_eq!(Texture::Frame.unit(), 0);
        assert_eq!(Texture::Glyphs.unit(), 0);
    }

    #[test]
    fn the_sky_s_own_draw_names_no_section() {
        let sky = vertices(6, 0);
        let mut renderer = Renderer::new(256);
        let mut device = Recording::default();
        renderer.draw(
            &mut device,
            View::new(Viewport::new(320, 240), [0.0; 4], IDENTITY),
            &[],
            Some((&sky, 3)),
            None,
            true,
        );
        let draw = renderer.frame().draws()[0];
        assert_eq!(draw.section, SKY);
        // The sky is drawn where the camera is: no origin of its own.
        assert_eq!(draw.origin, [0.0; 3]);
    }

    /// The section's origin is the draw's: a mesher writes positions relative to a section, and
    /// without this the whole world would be drawn piled on the camera.
    #[test]
    fn a_section_s_origin_is_the_draw_s_u_origin() {
        let solid = vertices(6, 3);
        let mut renderer = Renderer::new(1024);
        let mut device = Recording::default();
        let mut placed = section(4, &solid, &[]);
        placed.origin = [16.0, 64.0, -32.0];
        renderer.draw(
            &mut device,
            View::new(Viewport::new(320, 240), [0.0; 4], IDENTITY),
            &[placed],
            None,
            None,
            false,
        );
        let draw = renderer.frame().draws()[0];
        assert_eq!(draw.origin, [16.0, 64.0, -32.0]);
        let origin = device
            .calls
            .iter()
            .find(|call| call.starts_with("terrain origin "))
            .expect("the draw's uniforms");
        assert!(origin.contains("[16.0, 64.0, -32.0]"), "{origin}");
        // And the sprite the run names, which the device turns into the atlas's rectangle.
        assert!(origin.contains("sprite 3"), "{origin}");
    }

    /// The HUD's uniforms carry the window, the GUI scale and the tick's fraction: without the
    /// first two the overlay is drawn at the wrong size, and without the third the glint freezes.
    #[test]
    fn the_hud_gets_the_window_the_scale_and_the_time() {
        let mut renderer = Renderer::new(256);
        let mut device = Recording::default();
        let mut hud = hud_batch(3, Blend::Alpha);
        hud.scale = 2.0;
        hud.time = 0.25;
        renderer.draw(
            &mut device,
            View::new(Viewport::new(640, 480), [0.0; 4], IDENTITY),
            &[],
            None,
            Some(&hud),
            true,
        );
        let frame = device.index_of("hud frame").expect("the hud's uniforms");
        let line = &device.calls[frame];
        assert!(line.contains("[640.0, 480.0]"), "{line}");
        assert!(line.contains("scale 2"), "{line}");
        assert!(line.contains("time 0.25"), "{line}");
        // The window's origin is its top left: the y flip `hud.vert` has.
        assert!(line.contains("flip 1"), "{line}");
    }
}
