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
        // A run-time glyph uses the real font::Patch path, not direct GL calls.
        let mut glyph = HudBatch::new([64; 2]);
        glyph.patches.push(crate::font::Patch {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        });
        let uv = [0.5 / atlas.width as f32, 0.5 / atlas.height as f32];
        glyph.quad([[0., 0.], [8., 0.], [8., 8.], [0., 8.]], [uv; 4], u32::MAX);
        upload_patches(
            &mut renderer,
            texture,
            [atlas.width, atlas.height],
            &glyph,
            1024,
        )
        .unwrap();
        let runs = super::runs(&glyph, [64; 2], uv_scale, 1000).unwrap();
        gui::render(&mut renderer, &runs, texture, white).unwrap();
        assert_eq!(renderer.read_rgba([4, 60, 1, 1]).unwrap(), [255, 0, 0, 255]);
        assert_eq!(renderer.stats().meshes, 0);
        renderer.remove_texture(texture).unwrap();
        renderer.remove_texture(white).unwrap();
        assert_eq!(renderer.stats(), Stats::default());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod owning_gpu_tests {
    use super::*;
    use crate::{
        hud::HudAtlas,
        mesher::{SectionMesh, TerrainVertex},
        render_bridge,
    };
    use horizon_legacy_render::{Budget, Context, Renderer};
    use horizon_render_api::{
        IDENTITY, Image, RenderBackend,
        atlas::{SpriteSize, build_page, pack_atlas},
        gui,
        scene::{SceneFrame, WorldScene},
    };

    fn game_renderer() -> WorldScene<Renderer<'static>> {
        let renderer =
            Renderer::owned(Context::new("osmesa", [64; 2]).unwrap(), Budget::default()).unwrap();
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[64, 128, 192, 255]]).unwrap();
        WorldScene::new(renderer, layout, &[&page], &[255; 16 * 16 * 4], 4).unwrap()
    }

    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn owned_game_mesh_and_hud_share_the_same_context() {
        // Context/layout were local variables in the factory, not leaked or
        // borrowed by a self-referential Game. Move the whole renderer again.
        let mut scene = Box::new(game_renderer());
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
                .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, u32::MAX, 0xf0f0)),
        );
        render_bridge::upload_section(&mut scene, [0; 3], &mesh, 4).unwrap();
        scene
            .render(&SceneFrame {
                view_projection: IDENTITY,
                camera: [0.; 3],
                clear: [0., 0., 0., 1.],
                fog: None,
                solid: &[[0; 3]],
                translucent: &[],
            })
            .unwrap();
        let atlas = HudAtlas::boot();
        let (texture, uv_scale) = gui::upload_atlas(
            scene.backend_mut(),
            [atlas.width, atlas.height],
            &atlas.pixels(),
            8 << 20,
        )
        .unwrap();
        let white = scene
            .backend_mut()
            .create_texture(Image {
                width: 1,
                height: 1,
                rgba: &[255; 4],
            })
            .unwrap();
        let mut hud = HudBatch::new([64; 2]);
        hud.fill(&atlas, 16., 16., 32., 32., u32::MAX);
        hud.invert_since(0);
        let runs = runs(&hud, [64; 2], uv_scale, 100).unwrap();
        gui::render(scene.backend_mut(), &runs, texture, white).unwrap();
        for (at, expected) in [([4, 4], [64, 128, 192]), ([32, 32], [191, 127, 63])] {
            let pixel = scene.read_rgba([at[0], at[1], 1, 1]).unwrap();
            assert!(
                pixel[..3]
                    .iter()
                    .zip(expected)
                    .all(|(&a, b)| a.abs_diff(b) <= 2),
                "pixel {at:?}: {pixel:?}"
            );
        }
        scene.backend_mut().remove_texture(texture).unwrap();
        scene.backend_mut().remove_texture(white).unwrap();
    }
}
