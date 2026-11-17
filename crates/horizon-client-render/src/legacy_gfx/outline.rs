//! Real block-selection edges, not GL_LINES (whose wide-line support varies).
use super::screen;
use crate::gfx;
use horizon_legacy_render::Renderer;
use horizon_render_api::{
    Clear, Draw, Fog, Frame, IDENTITY, Pass, RenderBackend, TextureId, stroke,
};

pub(super) struct Prepared {
    mesh: stroke::Mesh,
    pub after_terrain: bool,
}
impl Prepared {
    pub fn new(source: &gfx::Frame<'_>, size: [u32; 2], matrix: [f32; 16]) -> Result<Self, String> {
        let width = gfx::outline_width(size[0]);
        let ordinary = [(width, [0., 0., 0., 0.4])];
        // Draw ALL black edges first, then ALL bright edges, like draw_outline.
        let contrast = [
            (7., [0., 0., 0., 1.]),
            (width, [87. / 255., 1., 225. / 255., 1.]),
        ];
        Ok(Self {
            mesh: stroke::Mesh::new(
                &source.outline,
                source.camera.to_array(),
                matrix,
                size,
                if source.outline_high_contrast {
                    &contrast
                } else {
                    &ordinary
                },
            )
            .map_err(str::to_owned)?,
            after_terrain: source.outline_after_terrain,
        })
    }
    pub fn bytes(&self) -> usize {
        self.mesh.bytes()
    }
}
impl screen::Resources {
    pub fn render_outline(
        &mut self,
        backend: &mut Renderer<'_>,
        prepared: &Prepared,
        white: TextureId,
        fog: Option<Fog>,
    ) -> Result<(), String> {
        if prepared.mesh.indices.is_empty() {
            return Ok(());
        }
        self.collect(backend)?;
        let mesh = backend.create_stroke_mesh(&prepared.mesh.vertices, &prepared.mesh.indices)?;
        self.meshes.push(mesh);
        backend.render(&Frame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: Clear::Load,
            fog,
            draws: &[Draw {
                mesh,
                texture: white,
                lightmap: white,
                origin: [0.; 3],
                pass: Pass::Stroke,
                scissor: None,
            }],
        })?;
        self.collect(backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hud::{HudAtlas, HudBatch},
        legacy_gfx::{Limits, TerrainHudRenderer, frame},
    };
    use horizon_legacy_render::Context;
    use horizon_render_api::atlas::{SpriteSize, build_page, pack_atlas};
    #[test]
    fn actual_frame_accepts_block_edges_but_not_debug_passes_and_orders_contrast_layers() {
        let hud = HudBatch::new([64; 2]);
        let lines = [[0.; 8]];
        let mut source = frame::tests::source(&hud);
        source.camera = glam::DVec3::ZERO;
        source.outline = vec![[-0.5, 0., 0., 0.5, 0., 0.]];
        let prepare = |source: &gfx::Frame<'_>| {
            frame::Prepared::new(
                source,
                [64; 2],
                frame::VisibleSections::default(),
                Limits::default(),
                frame::Options::default(),
            )
            .is_ok()
        };
        assert!(prepare(&source));
        let ordinary = Prepared::new(&source, [64; 2], IDENTITY).unwrap();
        assert_eq!(ordinary.bytes(), 172);
        assert_eq!(ordinary.mesh.vertices[0].color, [0., 0., 0., 0.4]);
        source.outline_high_contrast = true;
        source.outline_after_terrain = true;
        let contrast = Prepared::new(&source, [64; 2], IDENTITY).unwrap();
        assert!(contrast.after_terrain);
        assert_eq!(contrast.bytes(), 344);
        assert_eq!(contrast.mesh.vertices[0].color, [0., 0., 0., 1.]);
        assert_eq!(
            contrast.mesh.vertices[4].color,
            [87. / 255., 1., 225. / 255., 1.]
        );
        for index in 0..3 {
            match index {
                0 => source.debug_lines = &lines,
                1 => source.debug_lines_on_top = &lines,
                _ => source.debug_crosshair = &lines,
            }
            assert!(!prepare(&source));
            source.debug_lines = &[];
            source.debug_lines_on_top = &[];
            source.debug_crosshair = &[];
        }
        source.outline = vec![[0.; 6]; stroke::MAX_EDGES + 1];
        assert!(Prepared::new(&source, [64; 2], IDENTITY).is_err());
    }
    #[test]
    #[ignore = "requires OSMesa and explicit GL2 overrides; real Frame adapter, not a Game launch"]
    fn actual_frame_outline_draws_both_modes_without_leaks_and_rejects_before_clear() {
        let atlas = HudAtlas::boot();
        let hud = HudBatch::new([64; 2]);
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
        let page = build_page(&layout, 0, &[&[255, 255, 255, 128]]).unwrap();
        let mut r = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[255; 1024],
            &atlas,
            Limits::default(),
        )
        .unwrap();
        let mut source = frame::tests::source(&hud);
        source.camera = glam::DVec3::ZERO;
        source.pitch = 0.;
        source.outline = vec![[-0.8, 0., 3., 0.8, 0., 3.]];
        source.fog = Some(crate::fog::FogData {
            color: [0., 0., 1., 1.],
            environmental_start: 100.,
            environmental_end: 200.,
            render_distance_start: 100.,
            render_distance_end: 200.,
            sky_end: 200.,
            cloud_end: 200.,
        });
        let baseline = r.world.backend_mut().stats();
        for repeat in 0..32 {
            source.outline_high_contrast = repeat % 2 == 1;
            source.outline_after_terrain = repeat % 3 == 0;
            r.render_game_frame(
                &source,
                frame::VisibleSections::default(),
                frame::Options::default(),
            )
            .unwrap();
            let p = r.read_rgba([32, 32, 1, 1]).unwrap();
            let expected = if source.outline_high_contrast {
                [87, 255, 225]
            } else {
                [0, 0, 153]
            };
            assert!(
                p[..3]
                    .iter()
                    .zip(expected)
                    .all(|(&a, b)| a.abs_diff(b) <= 2),
                "{repeat}: {p:?}"
            );
            assert_eq!(r.world.backend_mut().stats(), baseline);
        }
        // The source flag really moves the stroke across translucent terrain,
        // rather than merely recording metadata in Prepared.
        let mut water = crate::mesher::SectionMesh::default();
        water
            .translucent
            .extend([[-2., -2.], [2., -2.], [2., 2.], [-2., 2.]].map(|[x, y]| {
                crate::mesher::TerrainVertex::new([x, y, 2.], [0.5; 2], 0, 0xff00ff00, 0)
            }));
        let keys = [[0; 3]];
        r.upload_section(keys[0], &water).unwrap();
        let visible = frame::VisibleSections {
            solid: &[],
            translucent: &keys,
        };
        let options = frame::Options {
            fullbright: true,
            ..frame::Options::default()
        };
        source.outline_after_terrain = false;
        r.render_game_frame(&source, visible, options).unwrap();
        let behind = r.read_rgba([32, 32, 1, 1]).unwrap();
        source.outline_after_terrain = true;
        r.render_game_frame(&source, visible, options).unwrap();
        let front = r.read_rgba([32, 32, 1, 1]).unwrap();
        assert_ne!(front, behind);
        assert!(
            front[..3]
                .iter()
                .zip([87, 255, 225])
                .all(|(&a, b)| a.abs_diff(b) <= 2),
            "{front:?}"
        );
        r.unload_section(keys[0]).unwrap();
        r.world.collect_garbage().unwrap();
        assert_eq!(r.world.backend_mut().stats(), baseline);
        let before = r.read_rgba([0, 0, 64, 64]).unwrap();
        source.outline.push([f64::NAN; 6]);
        assert!(
            r.render_game_frame(
                &source,
                frame::VisibleSections::default(),
                frame::Options::default()
            )
            .is_err()
        );
        assert_eq!(r.read_rgba([0, 0, 64, 64]).unwrap(), before);
        source.outline.pop();
        r.world
            .backend_mut()
            .set_budget(horizon_legacy_render::Budget {
                texture_bytes: baseline.texture_bytes,
                mesh_bytes: 343,
            })
            .unwrap();
        assert!(
            r.render_game_frame(
                &source,
                frame::VisibleSections::default(),
                frame::Options::default()
            )
            .is_err()
        );
        assert_eq!(r.read_rgba([0, 0, 64, 64]).unwrap(), before);
        assert_eq!(r.world.backend_mut().stats(), baseline);
    }
}
