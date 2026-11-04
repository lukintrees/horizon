//! Real HudBatch -> backend-neutral GUI triangles. No GL/wgpu object crosses it.
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
/// The pixel data is available from HudAtlas::pixels(), including packed atlases.
pub fn runs(
    batch: &HudBatch,
    window: [u32; 2],
    uv_scale: [f32; 2],
    max_vertices: usize,
) -> Result<Vec<Run>, Error> {
    if batch.vertices.len() > max_vertices {
        return Err(Error::Gui(GuiError::VertexBudget));
    }
    let mut batches = Vec::new();
    for (range, blend) in batch.runs() {
        let blend = match blend {
            Blend::Alpha => gui::Blend::Alpha,
            Blend::Invert => gui::Blend::Invert,
            Blend::Additive => gui::Blend::Additive,
            Blend::Depth(clip) => gui::Blend::Depth(clip),
            Blend::Vignette => gui::Blend::Vignette,
            unsupported => return Err(Error::UnsupportedBlend(unsupported)),
        };
        batches.push((range.start as usize..range.end as usize, blend));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_render_api::Pass;
    #[test]
    fn actual_hud_batch_keeps_invert_depth_alpha_and_triangle_order() {
        let mut batch = HudBatch::new([100, 80]);
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
        let mut batch = HudBatch::new([100; 2]);
        batch.quad([[0.; 2]; 4], [[0.5; 2]; 4], u32::MAX);
        batch.blend_since(0, Blend::EndPortal);
        assert!(matches!(
            runs(&batch, [100; 2], [1.; 2], 6),
            Err(Error::UnsupportedBlend(Blend::EndPortal))
        ));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod gpu_tests {
    use super::*;
    use crate::hud::{HudAtlas, rgba};
    use horizon_legacy_render::{Budget, Context, Renderer, Stats};
    use horizon_render_api::{Clear, Frame, IDENTITY, Image, RenderBackend, gui};

    /// Opt-in because normal developer/CI machines do not necessarily install
    /// OSMesa. The branch workflow explicitly runs this ignored test on GL2.
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn boot_atlas_and_real_hud_batch_reach_gl2_pixels() {
        let context = Context::new("osmesa", [64; 2]).unwrap();
        let mut renderer = Renderer::new(&context, Budget::default()).unwrap();
        let atlas = HudAtlas::boot();
        let (texture, uv_scale) = gui::upload_atlas(
            &mut renderer,
            [atlas.width, atlas.height],
            &atlas.pixels(),
            8 << 20,
        )
        .unwrap();
        let white = renderer
            .create_texture(Image {
                width: 1,
                height: 1,
                rgba: &[255; 4],
            })
            .unwrap();
        let mut batch = HudBatch::new([64; 2]);
        batch.fill(&atlas, 8., 8., 48., 48., rgba(64, 128, 192, 255));
        let at = batch.vertices.len();
        batch.fill(&atlas, 16., 16., 32., 32., u32::MAX);
        batch.invert_since(at);
        renderer
            .render(&Frame {
                view_projection: IDENTITY,
                camera: [0.; 3],
                clear: Clear::ColorDepth([0., 0., 0., 1.]),
                fog: None,
                draws: &[],
            })
            .unwrap();
        let runs = runs(&batch, [64; 2], uv_scale, 1000).unwrap();
        gui::render(&mut renderer, &runs, texture, white).unwrap();
        for (at, expected) in [
            ([4, 4], [0, 0, 0]),
            ([12, 12], [64, 128, 192]),
            ([32, 32], [191, 127, 63]),
        ] {
            let pixel = renderer.read_rgba([at[0], at[1], 1, 1]).unwrap();
            assert!(
                pixel[..3]
                    .iter()
                    .zip(expected)
                    .all(|(&a, b)| a.abs_diff(b) <= 2),
                "pixel {at:?}: {pixel:?}"
            );
        }
        assert_eq!(renderer.stats().meshes, 0);
        renderer.remove_texture(texture).unwrap();
        renderer.remove_texture(white).unwrap();
        assert_eq!(renderer.stats(), Stats::default());
    }
}
