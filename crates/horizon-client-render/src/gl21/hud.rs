//! The HUD on OpenGL 2.1: the same batch as `hud::HudBatch`, in a vertex GLSL 1.20 can read, and the
//! blending state each run of it asks for.
//!
//! `hud.rs` builds one vertex list per frame and records the runs that change state inside it
//! (`HudBatch::blends`): the crosshair's invert, a highlight's additive, the credits vignette, the
//! End portal, a CIT glint. The batching is untouched here — nothing about it needed wgpu — but two
//! things of the wgpu path do not exist on a 2006 driver:
//!
//! - **The vertex layout.** `HudVertex` is `bytemuck::Pod` over seven fields, 32 bytes, and a
//!   `@location` per field. GL 2.1 takes it as it is *if* the colour is a `vec4` and not an integer
//!   (`glVertexAttribIPointer` is GL 3.0): the `u32` colour is unpacked on the CPU into four
//!   normalized bytes, exactly as `vertex.rs` does for the terrain.
//! - **Sampler objects.** The wgpu shader samples the sprite atlas and the glint texture, and its
//!   linear sampler is a separate object for the vignette's blur. GL 2.1 binds filtering with the
//!   texture, so a run that wants the blur needs its own texture unit and its own `u_linear` sampler
//!   uniform rather than one more pipeline — `State::units` says which units that run reads.
//!
//! Everything a caller must not get wrong is data here, and `program` holds the matching attribute
//! and uniform tables next to the shader's own declarations, as it does for the terrain program.

// Re-exported because they are in this module's own signatures (`State` is `hud::Blend`'s, and a
// caller hands `HudDraws::of` a `HudBatch`): a private `use` would make the types unnameable from
// outside, which is what `private_interfaces` warns about.
pub use crate::hud::{Blend, HudBatch, HudVertex};

/// A HUD vertex in the GL 2.1 layout: 32 bytes, five attributes, no integers.
///
/// | attribute | type | `HudVertex` field |
/// | --- | --- | --- |
/// | 0 `a_pos` | `GL_FLOAT` ×2 | `pos`, in GUI pixels |
/// | 1 `a_uv` | `GL_FLOAT` ×2 | `uv`, into the sprite atlas, `>= 2` in x for a glint |
/// | 2 `a_color` | `GL_UNSIGNED_BYTE` ×4, normalized | the `u32` colour, red in its low byte |
/// | 3 `a_depth` | `GL_FLOAT` | `depth` |
/// | 4 `a_glint_uv` | `GL_FLOAT` ×2 | `glint_uv`, into the repeating glint texture |
///
/// Five attributes of the sixteen the R500 has. The `text` flag stays behind: a resource pack's text
/// shader is dropped on the GL 2.1 path (plan §5.4), so no vertex needs to say which runs it draws.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlHudVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    /// RGBA8, red first.
    pub color: [u8; 4],
    pub depth: f32,
    pub glint_uv: [f32; 2],
}

/// The stride between HUD vertices, and the byte offset of each attribute.
pub const HUD_STRIDE: u32 = 32;
pub const HUD_COLOR: u32 = 16;
pub const HUD_DEPTH: u32 = 20;
pub const HUD_GLINT_UV: u32 = 24;

impl GlHudVertex {
    /// `vertex` in the GL 2.1 layout: the colour unpacked, everything else as it is.
    pub fn of(vertex: &HudVertex) -> GlHudVertex {
        let color = vertex.color;
        GlHudVertex {
            pos: vertex.pos,
            uv: vertex.uv,
            color: [
                color as u8,
                (color >> 8) as u8,
                (color >> 16) as u8,
                (color >> 24) as u8,
            ],
            depth: vertex.depth,
            glint_uv: vertex.glint_uv,
        }
    }

    /// Whether this vertex samples the glint texture rather than the sprite atlas: `uv.x >= 2` is
    /// vanilla's `GLINT_TEXTURING` marker, which `hud.wgsl`'s fragment stage branches on too.
    pub fn is_glint(&self) -> bool {
        self.uv[0] >= 2.0
    }
}

/// The blending a run draws with, as the two `glBlendFunc` factors and whether depth is tested.
///
/// `hud.rs` creates a wgpu pipeline per `Blend`; a GL 2.1 driver has `glBlendFunc` instead, so the
/// same list is a table of constants here. `Blend::Depth(clip)` is the one that needs more than
/// blending — an isolated depth buffer and a pixel scissor — and keeps the rectangle in
/// `State::scissor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    /// The source factor: `GL_ONE`, `GL_ONE_MINUS_DST_COLOR`, `GL_ZERO`, `GL_SRC_ALPHA`.
    pub src: BlendFactor,
    /// The destination factor: `GL_ONE_MINUS_SRC_COLOR`, `GL_ONE`, `GL_SRC_ALPHA`, …
    pub dst: BlendFactor,
    /// Whether blending is on at all: the End portal's run writes its colour.
    pub blend: bool,
    /// Whether the depth test is on, as `Blend::Depth` asks.
    pub depth: bool,
    /// Whether `glAlphaFunc(GL_GREATER, 0)` applies: the crosshair's `fs_crosshair` discards on zero
    /// alpha, which GL 2.1 does with the alpha test rather than a `discard` in the shader.
    pub alpha_test: bool,
    /// `Blend::Depth`'s pixel scissor, in window pixels: (x, y, width, height).
    pub scissor: Option<[u32; 4]>,
    /// Whether the run samples a linear-filtered copy of the atlas: the vignette's blur, which is a
    /// sampler object on wgpu and a second texture unit on GL 2.1 (`u_linear`).
    pub linear: bool,
}

/// A `glBlendFunc` factor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendFactor {
    Zero,
    One,
    SrcAlpha,
    OneMinusSrcColor,
    OneMinusDstColor,
    DstColor,
}

impl BlendFactor {
    /// The GL constant's name, as a log line and the driver's `glBlendFunc` name it.
    pub fn named(self) -> &'static str {
        match self {
            BlendFactor::Zero => "GL_ZERO",
            BlendFactor::One => "GL_ONE",
            BlendFactor::SrcAlpha => "GL_SRC_ALPHA",
            BlendFactor::OneMinusSrcColor => "GL_ONE_MINUS_SRC_COLOR",
            BlendFactor::OneMinusDstColor => "GL_ONE_MINUS_DST_COLOR",
            BlendFactor::DstColor => "GL_DST_COLOR",
        }
    }
}

/// The state `blend` asks for: the table `hud.rs`'s pipeline set is, as `glBlendFunc` arguments.
pub fn state(blend: Blend) -> State {
    let alpha = State {
        src: BlendFactor::SrcAlpha,
        dst: BlendFactor::SrcAlpha,
        blend: true,
        depth: false,
        alpha_test: false,
        scissor: None,
        linear: false,
    };
    match blend {
        // BlendFunction.TRANSLUCENT, as the game's GUI pipelines set it: `src alpha` over
        // `one minus src alpha`, which is what vanilla's `RenderPipelines.GUI` does (the WGSL path
        // uses `ALPHA_BLENDING`, whose destination factor for colour is the same).
        Blend::Alpha => alpha,
        // BlendFunction.INVERT: ONE_MINUS_DST_COLOR, ONE_MINUS_SRC_COLOR. `fs_crosshair` drops
        // alpha-zero texels, which is the alpha test here.
        Blend::Invert => State {
            src: BlendFactor::OneMinusDstColor,
            dst: BlendFactor::OneMinusSrcColor,
            blend: true,
            depth: false,
            alpha_test: true,
            scissor: None,
            linear: false,
        },
        // BlendFunction.ADDITIVE: ONE, ONE.
        Blend::Additive => State {
            src: BlendFactor::One,
            dst: BlendFactor::One,
            ..alpha
        },
        // The HUD model's own depth buffer, and the scissor that isolates it.
        Blend::Depth(clip) => State {
            depth: true,
            scissor: Some(clip),
            ..alpha
        },
        // RenderPipelines.VIGNETTE: ZERO, ONE_MINUS_SRC_COLOR, bilinear.
        Blend::Vignette => State {
            src: BlendFactor::Zero,
            dst: BlendFactor::OneMinusSrcColor,
            blend: true,
            depth: false,
            alpha_test: false,
            scissor: None,
            linear: true,
        },
        // RenderPipelines.END_PORTAL: opaque, one texture unit.
        Blend::EndPortal => State {
            src: BlendFactor::One,
            dst: BlendFactor::Zero,
            blend: false,
            depth: false,
            alpha_test: false,
            scissor: None,
            linear: false,
        },
        // A CIT glint layer: `cit_glint`'s own table picks the blend, and the GL 2.1 path drops the
        // layer's array texture (plan §5.4), so the run keeps its own texture unit and the one blend
        // the rule that is not additive or alphabetic asks for is approximated by alpha.
        Blend::CitGlint(_) => State {
            linear: true,
            ..alpha
        },
    }
}

/// One run of a frame's HUD: the vertices it draws and the state around them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudRun {
    /// The first vertex of the batch's buffer.
    pub begin: u32,
    /// How many vertices: triangles, three to a call's worth.
    pub count: u32,
    pub state: State,
}

/// One HUD frame, ready to upload and draw: the vertices in the GL layout and the runs of state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudDraws {
    pub vertices: Vec<GlHudVertex>,
    pub runs: Vec<HudRun>,
    /// GUI pixels per window pixel (`HudBatch::scale`), which `hud.vert` needs to place a vertex in
    /// the window.
    pub scale: f32,
    /// The GUI's size in pixels, as `hud.wgsl`'s `hud.size`.
    pub size: [f32; 2],
}

impl HudDraws {
    /// A batch as the GL 2.1 driver draws it: every vertex converted once, and the batch's own blend
    /// runs turned into state.
    ///
    /// A run with no state (`Blend::Alpha`, which `HudBatch::blend_since` does not record) is not in
    /// `HudBatch::blends` at all: `hud.rs` draws everything in between with the plain pipeline, so
    /// this fills those gaps with `state(Blend::Alpha)`. Past the last recorded run the remaining
    /// vertices are one more alpha run — the common case, a menu drawn with no state change.
    pub fn of(batch: &HudBatch) -> HudDraws {
        let vertices: Vec<GlHudVertex> = batch.vertices.iter().map(GlHudVertex::of).collect();
        let total = batch.vertices.len() as u32;
        let mut runs = Vec::with_capacity(batch.blends.len() + 1);
        let mut at = 0;
        for (range, blend) in &batch.blends {
            let (begin, end) = (range.start.min(total), range.end.min(total));
            if begin > at {
                runs.push(HudRun {
                    begin: at,
                    count: begin - at,
                    state: state(Blend::Alpha),
                });
            }
            if end > begin {
                runs.push(HudRun {
                    begin,
                    count: end - begin,
                    state: state(*blend),
                });
            }
            at = end.max(at);
        }
        if total > at {
            runs.push(HudRun {
                begin: at,
                count: total - at,
                state: state(Blend::Alpha),
            });
        }
        HudDraws {
            vertices,
            runs,
            scale: batch.scale,
            size: [batch.width, batch.height],
        }
    }

    /// How many `glDrawArrays` calls the frame is.
    pub fn calls(&self) -> usize {
        self.runs.len()
    }

    /// Whether any run needs the vignette's linear copy of the atlas (`u_linear`).
    pub fn needs_linear(&self) -> bool {
        self.runs.iter().any(|run| run.state.linear)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(pos: [f32; 2], uv: [f32; 2], color: u32, depth: f32) -> HudVertex {
        HudVertex {
            pos,
            uv,
            color,
            depth,
            glint_uv: [0.0, 0.0],
            text: 0,
        }
    }

    fn batch(vertices: Vec<HudVertex>) -> HudBatch {
        let mut batch = HudBatch::new([854, 480]);
        batch.vertices = vertices;
        batch
    }

    #[test]
    fn a_vertex_carries_its_colour_as_four_normalized_bytes() {
        // #20 alpha 0x80 in the high byte, as `hud::rgba` packs it: a, b, g, r.
        let v = GlHudVertex::of(&vertex([4.0, 8.0], [0.5, 0.25], 0x8040_3020, 0.75));
        assert_eq!(v.pos, [4.0, 8.0]);
        assert_eq!(v.uv, [0.5, 0.25]);
        assert_eq!(v.color, [0x20, 0x30, 0x40, 0x80], "red first");
        assert_eq!(v.depth, 0.75);
        assert_eq!(v.glint_uv, [0.0, 0.0]);
    }

    #[test]
    fn the_stride_and_offsets_are_what_the_attribute_pointers_step() {
        assert_eq!(std::mem::size_of::<GlHudVertex>() as u32, HUD_STRIDE);
        assert_eq!(HUD_STRIDE, 2 * 4 + 2 * 4 + 4 + 4 + 2 * 4);
        assert_eq!(std::mem::offset_of!(GlHudVertex, color), HUD_COLOR as usize);
        let depth = std::mem::offset_of!(GlHudVertex, depth);
        let glint_uv = std::mem::offset_of!(GlHudVertex, glint_uv);
        assert_eq!(depth, HUD_DEPTH as usize);
        assert_eq!(glint_uv, HUD_GLINT_UV as usize);
    }

    #[test]
    fn a_glint_vertex_is_the_one_whose_u_is_past_two() {
        let plain = GlHudVertex::of(&vertex([0.0; 2], [0.5, 0.5], 0, 0.0));
        let glint = GlHudVertex::of(&vertex([0.0; 2], [2.5, 0.5], 0, 0.0));
        assert!(!plain.is_glint());
        assert!(glint.is_glint());
    }

    #[test]
    fn the_blend_table_is_the_pipeline_set() {
        // Invert: ONE_MINUS_DST_COLOR, ONE_MINUS_SRC_COLOR, alpha test on.
        let invert = state(Blend::Invert);
        assert_eq!(invert.src, BlendFactor::OneMinusDstColor);
        assert_eq!(invert.dst, BlendFactor::OneMinusSrcColor);
        assert!(invert.alpha_test);
        // Additive: ONE, ONE.
        let additive = state(Blend::Additive);
        let add = (additive.src, additive.dst);
        assert_eq!(add, (BlendFactor::One, BlendFactor::One));
        // Vignette: ZERO, ONE_MINUS_SRC_COLOR, and the linear unit.
        let vignette = state(Blend::Vignette);
        assert_eq!(vignette.src, BlendFactor::Zero);
        assert_eq!(vignette.dst, BlendFactor::OneMinusSrcColor);
        assert!(vignette.linear);
        // End portal: no blending at all.
        assert!(!state(Blend::EndPortal).blend);
        // Depth: the section's scissor kept, and the depth test on.
        let depth = state(Blend::Depth([3, 4, 5, 6]));
        assert!(depth.depth);
        assert_eq!(depth.scissor, Some([3, 4, 5, 6]));
        // Plain alpha: src alpha over one minus src alpha, no alpha test.
        let alpha = state(Blend::Alpha);
        assert!(alpha.blend);
        assert!(!alpha.alpha_test);
        assert_eq!(alpha.src.named(), "GL_SRC_ALPHA");
    }

    #[test]
    fn the_runs_cover_every_vertex_once_with_the_alpha_filling_the_gaps() {
        let mut batch = batch(vec![vertex([0.0; 2], [0.0; 2], 0, 0.0); 10]);
        // Vertices 2..6 are an inverted crosshair, 8..10 additive: the gaps are plain alpha.
        batch.blends = vec![(2..6, Blend::Invert), (8..10, Blend::Additive)];
        let draws = HudDraws::of(&batch);
        assert_eq!(draws.vertices.len(), 10);
        let runs: Vec<(u32, u32)> = draws
            .runs
            .iter()
            .map(|run| (run.begin, run.count))
            .collect();
        assert_eq!(runs, vec![(0, 2), (2, 4), (6, 2), (8, 2)]);
        assert_eq!(draws.runs[1].state, state(Blend::Invert));
        assert_eq!(draws.runs[0].state, state(Blend::Alpha));
        assert_eq!(draws.calls(), 4);
        assert!(!draws.needs_linear());
    }

    #[test]
    fn a_batch_with_no_state_change_is_one_alpha_run() {
        let draws = HudDraws::of(&batch(vec![vertex([0.0; 2], [0.0; 2], 0, 0.0); 6]));
        assert_eq!(draws.calls(), 1);
        assert_eq!(draws.runs[0].begin, 0);
        assert_eq!(draws.runs[0].count, 6);
        assert_eq!(draws.size, [854.0 / draws.scale, 480.0 / draws.scale]);
    }

    #[test]
    fn the_vignette_asks_for_the_linear_unit() {
        let mut batch = batch(vec![vertex([0.0; 2], [0.0; 2], 0, 0.0); 3]);
        batch.blends = vec![(0..3, Blend::Vignette)];
        let draws = HudDraws::of(&batch);
        assert!(draws.needs_linear());
        assert_eq!(draws.runs.len(), 1, "no alpha gap before the first run");
    }

    #[test]
    fn a_run_past_the_vertex_list_is_clipped_not_panicking() {
        let mut batch = batch(vec![vertex([0.0; 2], [0.0; 2], 0, 0.0); 4]);
        batch.blends = vec![(2..9, Blend::Invert)];
        let draws = HudDraws::of(&batch);
        assert_eq!(draws.runs.len(), 2);
        assert_eq!((draws.runs[1].begin, draws.runs[1].count), (2, 2));
    }
}
