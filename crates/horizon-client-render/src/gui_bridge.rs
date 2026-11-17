//! Real `HudBatch` -> backend-neutral GUI triangles. No GL/wgpu object crosses it.
//! A CPU-only bridge shared by the modern renderer's validation tests and the
//! legacy one's frame builder.
use crate::hud::{Blend, HudBatch};
use horizon_render_api::gui::{self, GuiError, Input, Run};

#[derive(Debug)]
pub enum Error {
    Gui(GuiError),
    /// Do not silently replace portal/CIT shaders with the ordinary atlas path.
    UnsupportedBlend(Blend),
}

/// Preserve the original batch's ordered ranges, GUI scaling and depth groups.
/// `uv_scale` is returned by `gui::upload_atlas` for a POT-padded HUD atlas.
/// The pixel data is available from `HudAtlas::pixels()`, including packed atlases.
pub fn runs(
    batch: &HudBatch,
    window: [u32; 2],
    uv_scale: [f32; 2],
    max_vertices: usize,
) -> Result<Vec<Run>, Error> {
    if batch.vertices.len() > max_vertices {
        return Err(Error::Gui(GuiError::VertexBudget));
    }
    let batches = batches(batch)?;
    let input = batch
        .vertices
        .iter()
        .map(|v| Input {
            pos: v.pos,
            uv: v.uv,
            color: v.color,
            depth: v.depth,
        })
        .collect::<Vec<_>>();
    gui::runs(
        &input,
        &batches,
        window,
        batch.scale,
        uv_scale,
        max_vertices,
    )
    .map_err(Error::Gui)
}

/// Every `Blend` the neutral contract can express. The blend variants that
/// belong to a shader of their own are refused rather than flattened: a
/// portal or CIT layer drawn as an ordinary atlas quad is a wrong picture,
/// not a slow one.
fn batches(batch: &HudBatch) -> Result<Vec<(std::ops::Range<usize>, gui::Blend)>, Error> {
    let mut batches = Vec::new();
    for (range, blend) in batch.runs() {
        let blend = match blend {
            Blend::Alpha => gui::Blend::Alpha,
            Blend::Invert => gui::Blend::Invert,
            Blend::Additive => gui::Blend::Additive,
            Blend::Vignette => gui::Blend::Vignette,
            Blend::Depth(clip) => gui::Blend::Depth(clip),
            unsupported => return Err(Error::UnsupportedBlend(unsupported)),
        };
        batches.push((range.start as usize..range.end as usize, blend));
    }
    Ok(batches)
}

/// Foil-aware path retaining real `HudVertex::glint_uv` and the HUD's own glint
/// strength. Portal/CIT remain explicit errors, not flattened atlas draws.
pub fn glint_runs(
    batch: &HudBatch,
    window: [u32; 2],
    max_vertices: usize,
    item_texture: Option<horizon_render_api::TextureId>,
) -> Result<Vec<Run<gui::GuiVertex>>, Error> {
    if batch.vertices.len() > max_vertices {
        return Err(Error::Gui(GuiError::VertexBudget));
    }
    let strength = horizon_render_api::entity::Strength::new(batch.glint_alpha)
        .map_err(|_| Error::Gui(GuiError::InvalidGlint))?;
    let batches = batches(batch)?;
    let input = batch
        .vertices
        .iter()
        .map(|v| gui::GlintInput {
            base: Input {
                pos: v.pos,
                uv: v.uv,
                color: v.color,
                depth: v.depth,
            },
            glint_uv: v.glint_uv,
        })
        .collect::<Vec<_>>();
    gui::glint_runs(
        &input,
        &batches,
        gui::Options {
            window,
            scale: batch.scale,
            uv_scale: [1.; 2],
            max_vertices,
            glint: item_texture
                .map(|texture| horizon_render_api::entity::Glint { texture, strength }),
        },
    )
    .map_err(Error::Gui)
}

#[derive(Debug)]
pub enum PatchError<E> {
    Gui(GuiError),
    Backend(E),
}

/// Stage and validate all glyph patches before modifying the atlas. Padding at
/// a non-POT atlas edge must change with the source edge, otherwise clamped glyphs
/// would sample stale colours. The temporary CPU allocation is batch-bounded.
pub fn upload_patches<B: horizon_render_api::RenderBackend>(
    backend: &mut B,
    texture: horizon_render_api::TextureId,
    atlas_size: [u32; 2],
    batch: &HudBatch,
    max_bytes: usize,
) -> Result<(), PatchError<B::Error>> {
    let mut prepared = Vec::new();
    let mut remaining = max_bytes;
    for p in &batch.patches {
        let patch = gui::prepare_patch(
            atlas_size,
            [p.x, p.y],
            horizon_render_api::Image {
                width: p.width,
                height: p.height,
                rgba: &p.rgba,
            },
            remaining,
        )
        .map_err(PatchError::Gui)?;
        remaining -= patch.rgba.len();
        prepared.push(patch);
    }
    for p in prepared {
        backend
            .update_texture(
                texture,
                p.at,
                horizon_render_api::Image {
                    width: p.size[0],
                    height: p.size[1],
                    rgba: &p.rgba,
                },
            )
            .map_err(PatchError::Backend)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_render_api::Pass;
    #[test]
    fn actual_gui_foil_retains_marker_uv_and_uses_hud_not_world_strength() {
        let mut batch = HudBatch::scratch([64; 2]);
        batch.glint_alpha = 0.37;
        batch.quad_glint(
            [[0., 0.], [32., 0.], [32., 32.], [0., 32.]],
            [[2.25, 0.5]; 4],
            [[-0.25, 1.25]; 4],
            0,
        );
        assert!(glint_runs(&batch, [64; 2], 6, None).is_err());
        let output =
            glint_runs(&batch, [64; 2], 6, Some(horizon_render_api::TextureId(7))).unwrap();
        let Pass::Gui(material) = output[0].pass else {
            panic!("GUI material")
        };
        assert_eq!(material.glint.unwrap().strength.value(), 0.37);
        assert_eq!(output[0].vertices[0].base.uv, [0.25, 0.5]);
        assert_eq!(output[0].vertices[0].glint_uv, [-0.25, 1.25]);
        batch.glint_alpha = f32::NAN;
        assert!(glint_runs(&batch, [64; 2], 6, Some(horizon_render_api::TextureId(7))).is_err());
    }
    #[test]
    fn actual_hud_batch_keeps_invert_depth_alpha_and_triangle_order() {
        let mut batch = HudBatch::scratch([100, 80]);
        let quad = [[10., 20.], [40., 20.], [40., 50.], [10., 50.]];
        batch.quad(quad, [[0.5; 2]; 4], 0x8040_2010);
        batch.invert_since(0);
        batch.quad_3d(quad.map(|[x, y]| [x, y, x]), [[0.5; 2]; 4], u32::MAX);
        batch.depth_since(6, [10., 20., 40., 50.]);
        let runs = runs(&batch, [100, 80], [1.; 2], 12).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].pass, Pass::OverlayInvert);
        assert_eq!(runs[0].vertices.len(), 6);
        assert_eq!(runs[0].vertices[0].color[3], 128. / 255.);
        assert_eq!(runs[1].pass, Pass::OverlayDepth);
        assert_eq!(runs[1].scissor, Some([10, 30, 30, 30]));
        assert_eq!(runs[1].vertices[0].pos[2], 1.);
        assert_eq!(runs[1].vertices[1].pos[2], -1.);
        assert!(runs[1].clear_depth);
    }
    #[test]
    fn advanced_materials_are_not_drawn_as_plain_gui() {
        let mut batch = HudBatch::scratch([100; 2]);
        batch.quad([[0.; 2]; 4], [[0.5; 2]; 4], u32::MAX);
        batch.blend_since(0, Blend::EndPortal);
        assert!(matches!(
            runs(&batch, [100; 2], [1.; 2], 6),
            Err(Error::UnsupportedBlend(Blend::EndPortal))
        ));
    }
}
