//! Caller-owned native section order. Reuse the existing distance predicate and
//! clip planes, not the modern renderer's GPU/cave graph or a second resident index.
#[cfg(any(feature = "legacy-gl", test))]
use crate::{gfx::Frustum, streaming::in_view_distance};
#[cfg(any(feature = "legacy-gl", test))]
use glam::{DVec3, Vec3};
use horizon_render_api::{render_contract::VisibleSections, scene::SectionKey};

#[derive(Default)]
pub struct SectionOrder {
    #[cfg(any(feature = "legacy-gl", test))]
    candidates: Vec<(u64, SectionKey)>,
    solid: Vec<SectionKey>,
    translucent: Vec<SectionKey>,
}
impl SectionOrder {
    #[cfg(any(feature = "legacy-gl", test))]
    pub(super) fn clear(&mut self) {
        self.candidates.clear();
        self.solid.clear();
        self.translucent.clear();
    }

    /// The owner's exact-size membership is bounded by its resident section cap.
    /// Reuse fallibly reserved scratch; publish neither list on a failed walk.
    /// Full 16³ AABBs conservatively retain geometry. Intra-section translucent
    /// quad sorting still belongs to the producer, not this section-order seam.
    #[cfg(any(feature = "legacy-gl", test))]
    pub(super) fn update(
        &mut self,
        keys: impl ExactSizeIterator<Item = SectionKey>,
        min_y: i32,
        camera: DVec3,
        radius: i32,
        frustum: &Frustum,
    ) -> Result<(), &'static str> {
        self.clear();
        if !camera.is_finite() || radius <= 0 || min_y % 16 != 0 {
            return Err("invalid native section visibility camera/distance/origin");
        }
        let count = keys.len();
        self.candidates
            .try_reserve(count)
            .map_err(|_| "native section visibility allocation failed")?;
        let center = (
            (camera.x.floor() as i32) >> 4,
            (camera.z.floor() as i32) >> 4,
        );
        for key in keys {
            // Bound differences in i64 before the shared i32 predicate. Distant
            // arbitrary keys must not overflow its subtraction/absolute value.
            if (i64::from(key[0]) - i64::from(center.0)).abs() > i64::from(radius)
                || (i64::from(key[2]) - i64::from(center.1)).abs() > i64::from(radius)
                || !in_view_distance(center, radius, key[0], key[2])
            {
                continue;
            }
            let origin = DVec3::new(
                f64::from(key[0]) * 16.,
                f64::from(min_y) + f64::from(key[1]) * 16.,
                f64::from(key[2]) * 16.,
            ) - camera;
            let min = origin.as_vec3();
            let distance = (origin + DVec3::splat(8.)).length_squared();
            if !min.is_finite() || !distance.is_finite() {
                return Err("invalid native section visibility relative coordinates");
            }
            if frustum.intersects(min, min + Vec3::splat(16.)) {
                self.candidates.push((distance.to_bits(), key));
            }
        }
        // Same centre-distance/key tie rule as the modern CPU resident walk.
        self.candidates.sort_unstable();
        if self
            .candidates
            .windows(2)
            .any(|pair| pair[0].1 == pair[1].1)
        {
            return Err("duplicate native resident section key");
        }
        self.solid
            .try_reserve(self.candidates.len())
            .map_err(|_| "native section visibility allocation failed")?;
        self.translucent
            .try_reserve(self.candidates.len())
            .map_err(|_| "native section visibility allocation failed")?;
        self.solid
            .extend(self.candidates.iter().map(|&(_, key)| key));
        self.translucent.extend(self.solid.iter().rev().copied());
        Ok(())
    }

    pub fn visible(&self) -> VisibleSections<'_> {
        VisibleSections {
            solid: &self.solid,
            translucent: &self.translucent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;

    #[test]
    fn camera_walk_culls_aabbs_and_orders_both_phases_independently_of_storage() {
        let keys = [
            [0, 0, 8], // In the frustum, outside the render distance.
            [1, 0, 2],
            [0, 4, 1], // Above the frustum.
            [0, 0, 2],
            [0, 0, -2], // Behind the camera.
            [0, 3, 2],  // Centre outside, but an AABB corner touches the frustum.
            [0, 0, 1],
            [-1, 0, 2],
            [0, 0, 0],
        ];
        let camera = DVec3::new(8., -56., 8.);
        let frustum = crate::gfx::camera_frustum([128; 2], 90., 0., 0., Mat4::IDENTITY, 256.);
        let expected = [
            [0, 0, 0],
            [0, 0, 1],
            [0, 0, 2],
            [-1, 0, 2],
            [1, 0, 2],
            [0, 3, 2],
        ];
        let mut order = SectionOrder::default();
        for shift in 0..keys.len() {
            let mut permuted = keys;
            permuted.rotate_left(shift);
            if shift % 2 == 0 {
                permuted.reverse();
            }
            order
                .update(permuted.into_iter(), -64, camera, 3, &frustum)
                .unwrap();
            assert_eq!(order.visible().solid, expected);
            assert_eq!(
                order.visible().translucent,
                [
                    [0, 3, 2],
                    [1, 0, 2],
                    [-1, 0, 2],
                    [0, 0, 2],
                    [0, 0, 1],
                    [0, 0, 0]
                ]
            );
        }
        order
            .update(
                keys.into_iter(),
                -64,
                DVec3::new(8., -56., 40.),
                3,
                &frustum,
            )
            .unwrap();
        assert_eq!(order.visible().solid, [[0, 0, 2], [-1, 0, 2], [1, 0, 2]]);
        order
            .update([].into_iter(), -64, camera, 3, &frustum)
            .unwrap();
        assert!(order.visible().solid.is_empty());
        assert!(order.visible().translucent.is_empty());
    }

    #[test]
    fn dimension_origin_and_subtraction_before_float_conversion_survive_world_edges() {
        let frustum =
            Frustum::from_view_projection(Mat4::from_scale(Vec3::new(10., 1. / 8., 1. / 64.)));
        for base in [-1_875_000, 1_875_000] {
            let left = [base, 0, base + 1];
            let right = [base + 1, 0, base + 1];
            let keys = [left, right, [i32::MIN, 0, i32::MAX]];
            let camera = DVec3::new(
                f64::from(base) * 16. + 16.25,
                -56.,
                f64::from(base) * 16. + 8.,
            );
            let mut order = SectionOrder::default();
            order
                .update(keys.into_iter(), -64, camera, 2, &frustum)
                .unwrap();
            // The left AABB ends at x=-0.25; clip x is only [-0.1, 0.1].
            // Casting world positions first would move that edge to x=0.
            assert_eq!(order.visible().solid, [right]);
            order
                .update(keys.into_iter(), 0, camera, 2, &frustum)
                .unwrap();
            assert!(order.visible().solid.is_empty());
        }
    }

    #[test]
    fn render_distance_keeps_column_slack_and_floors_negative_camera_positions() {
        let frustum = Frustum::from_view_projection(Mat4::from_scale(Vec3::splat(1. / 512.)));
        let mut order = SectionOrder::default();
        order
            .update(
                [[3, 0, 0], [2, 0, 2], [0, 0, 3], [-2, 0, 2], [2, 0, 0]].into_iter(),
                0,
                DVec3::splat(8.),
                2,
                &frustum,
            )
            .unwrap();
        assert_eq!(order.visible().solid, [[2, 0, 0], [-2, 0, 2], [2, 0, 2]]);
        order
            .update(
                [[-1, 0, -1], [1, 0, 1], [2, 0, -1]].into_iter(),
                0,
                DVec3::new(-0.25, 8., -0.25),
                2,
                &frustum,
            )
            .unwrap();
        assert_eq!(order.visible().solid, [[-1, 0, -1], [1, 0, 1]]);
    }

    #[test]
    fn failed_visibility_never_reuses_the_previous_draw_list() {
        let frustum = crate::gfx::camera_frustum([128; 2], 90., 0., 0., Mat4::IDENTITY, 256.);
        let mut order = SectionOrder::default();
        for (camera, radius, min_y, keys) in [
            (DVec3::new(f64::NAN, 8., 8.), 2, 0, vec![[0; 3]]),
            (DVec3::splat(8.), 0, 0, vec![[0; 3]]),
            (DVec3::splat(8.), 2, -63, vec![[0; 3]]),
            (DVec3::splat(8.), 2, 0, vec![[0; 3], [0; 3]]),
        ] {
            order
                .update([[0; 3]].into_iter(), 0, DVec3::splat(8.), 2, &frustum)
                .unwrap();
            assert_eq!(order.visible().solid, [[0; 3]]);
            assert!(
                order
                    .update(keys.into_iter(), min_y, camera, radius, &frustum)
                    .is_err()
            );
            assert!(order.visible().solid.is_empty());
            assert!(order.visible().translucent.is_empty());
        }
    }
}

#[cfg(all(test, feature = "legacy-gl"))]
mod gpu_tests {
    use super::*;
    use crate::{
        chunk::ColumnLayout,
        gfx,
        hud::{HudAtlas, HudBatch},
        legacy_gfx::{Limits, ResourceLimits, TerrainHudRenderer, frame, test_assets},
        mesher::{SectionMesh, TerrainVertex},
        renderer_facade::RendererFacade,
        renderer_owner::{Owner, RendererOwner},
    };
    use horizon_render_api::{
        render_contract::{FrameOptions, FrameOutcome, ResourceGeneration, WorldGeneration},
        resource_contract::TerrainAtlasLimits,
        streaming::Queued,
    };
    use std::sync::Arc;

    fn facade(owner: &mut RendererOwner) -> RendererFacade<'_> {
        owner
            .facade(Default::default(), Default::default())
            .unwrap()
    }

    fn mesh(layer: u16, translucent: bool) -> Arc<SectionMesh> {
        let mut mesh = SectionMesh::default();
        let vertices = [[0., 0., 8.], [16., 0., 8.], [16., 16., 8.], [0., 16., 8.]]
            .map(|p| TerrainVertex::new(p, [0.5; 2], layer, u32::MAX, 0xf0f0));
        if translucent {
            mesh.translucent.extend(vertices);
        } else {
            mesh.solid.extend(vertices);
        }
        Arc::new(mesh)
    }

    fn draw(
        owner: &mut RendererOwner,
        order: &mut SectionOrder,
        source: &gfx::Frame<'_>,
    ) -> [u8; 3] {
        assert_eq!(
            owner
                .render_native_world(
                    source,
                    order,
                    3,
                    FrameOptions {
                        gamma: 0.,
                        fullbright: true
                    }
                )
                .unwrap(),
            FrameOutcome::Offscreen
        );
        let Owner::Legacy(renderer) = &mut owner.inner else {
            panic!("missing native owner")
        };
        renderer.read_rgba([32, 32, 1, 1]).unwrap()[..3]
            .try_into()
            .unwrap()
    }

    fn rgb(actual: [u8; 3], expected: [u8; 3]) {
        assert!(
            actual
                .into_iter()
                .zip(expected)
                .all(|(a, b)| a.abs_diff(b) <= 1),
            "RGB {actual:?}, expected {expected:?}"
        );
    }

    #[test]
    #[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
    fn native_world_frame_uses_clock_live_lightmap_cpu_poses_and_rejects_bad_requests() {
        let mut assets = test_assets();
        assets.animations.push(crate::sprites::test_frames(
            0,
            vec![vec![255, 0, 0, 255], vec![0, 0, 255, 255]],
            r#"{"frametime":1}"#,
        ));
        let atlas = HudAtlas::boot();
        let hud = HudBatch::new([64; 2]);
        let renderer = TerrainHudRenderer::stage_assets(
            horizon_legacy_render::Context::new("osmesa", [64; 2]).unwrap(),
            &assets,
            &atlas,
            ResourceGeneration(78),
            &[255; 1024],
            Limits::default(),
            ResourceLimits {
                terrain: TerrainAtlasLimits {
                    page_size: 8,
                    max_pages: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap()
        .commit()
        .unwrap();
        let mut owner = RendererOwner::from_legacy(renderer);
        let mut order = SectionOrder::default();
        let mut source = frame::tests::source(&hud);
        source.camera = DVec3::splat(8.);
        source.pitch = 0.;
        source.fov_degrees = 90.;
        source.distance = 512.; // isolate lightmap/clock from distance fog
        source.sky.ambient_light_color = [0.25; 3];
        // Initialized streaming is required even for an empty resident world.
        assert!(
            owner
                .render_native_world(&source, &mut order, 3, FrameOptions::default())
                .unwrap_err()
                .to_string()
                .contains("initialized world streaming")
        );
        let mut terrain = mesh(0, false);
        for vertex in &mut Arc::make_mut(&mut terrain).solid {
            vertex.light = 0;
        }
        {
            let mut f = facade(&mut owner);
            f.begin_streaming(WorldGeneration(1), ColumnLayout::new(0, 256).unwrap())
                .unwrap();
            let key = [0, 0, 1];
            let ticket = f.request_stream_section(key).unwrap().unwrap();
            f.queue_stream_section(key, ticket, &terrain).unwrap();
            f.drain_streaming(1, usize::MAX).unwrap();
        }
        fn submit(
            owner: &mut RendererOwner,
            order: &mut SectionOrder,
            source: &gfx::Frame<'_>,
            options: FrameOptions,
        ) -> [u8; 3] {
            assert_eq!(
                owner
                    .render_native_world(source, order, 3, options)
                    .unwrap(),
                FrameOutcome::Offscreen
            );
            pixel(owner)
        }
        fn pixel(owner: &mut RendererOwner) -> [u8; 3] {
            let Owner::Legacy(r) = &mut owner.inner else {
                unreachable!()
            };
            r.read_rgba([32, 32, 1, 1]).unwrap()[..3]
                .try_into()
                .unwrap()
        }
        rgb(
            submit(&mut owner, &mut order, &source, FrameOptions::default()),
            [63, 0, 0],
        );
        source.ticks = 1;
        source.sky.ambient_light_color = [0.5; 3];
        rgb(
            submit(&mut owner, &mut order, &source, FrameOptions::default()),
            [0, 0, 127],
        );
        // The same tick is idempotent, gamma/fullbright still come from caller.
        rgb(
            submit(
                &mut owner,
                &mut order,
                &source,
                FrameOptions {
                    gamma: 1.,
                    fullbright: false,
                },
            ),
            [0, 0, 239],
        );
        rgb(
            submit(
                &mut owner,
                &mut order,
                &source,
                FrameOptions {
                    gamma: 0.,
                    fullbright: true,
                },
            ),
            [0, 0, 255],
        );
        let entities = [[-4., -4., 8.], [4., -4., 8.], [4., 4., 8.], [-4., 4., 8.]].map(|pos| {
            crate::mesher::Vertex {
                pos,
                uv: [0.5; 2],
                layer: 1,
                color: u32::MAX,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            }
        });
        source.entities = &entities;
        rgb(
            submit(
                &mut owner,
                &mut order,
                &source,
                FrameOptions {
                    gamma: 0.,
                    fullbright: true,
                },
            ),
            [0, 255, 0],
        );
        let skin = crate::frame::SkinFrame::default();
        source.skin = Some(&skin);
        assert!(
            owner
                .render_native_world(&source, &mut order, 3, FrameOptions::default())
                .unwrap_err()
                .to_string()
                .contains("GPU-posed entities")
        );
        assert!(order.visible().solid.is_empty());
        rgb(pixel(&mut owner), [0, 255, 0]);
        source.skin = None;
        source.title = true;
        assert!(
            owner
                .render_native_world(&source, &mut order, 3, FrameOptions::default())
                .unwrap_err()
                .to_string()
                .contains("title panorama")
        );
        rgb(pixel(&mut owner), [0, 255, 0]);
        source.title = false;
        source.camera.x = f64::NAN;
        assert!(
            owner
                .render_native_world(&source, &mut order, 3, FrameOptions::default())
                .is_err()
        );
        assert!(order.visible().translucent.is_empty());
        rgb(pixel(&mut owner), [0, 255, 0]);
    }

    #[test]
    #[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
    fn native_resident_order_draws_actual_frame_across_camera_unload_and_world_reset() {
        let mut assets = test_assets();
        assets.pixels[3] = 128; // Red and green half-covered texels, not mesher material alpha.
        assets.pixels[7] = 128;
        assets.pixels.extend([0, 0, 255, 255]);
        assets.texture_layers = 3;
        let atlas = HudAtlas::boot();
        let hud = HudBatch::new([64; 2]);
        let renderer = TerrainHudRenderer::stage_assets(
            horizon_legacy_render::Context::new("osmesa", [64; 2]).unwrap(),
            &assets,
            &atlas,
            ResourceGeneration(77),
            &[255; 1024],
            Limits::default(),
            ResourceLimits {
                terrain: TerrainAtlasLimits {
                    page_size: 8,
                    max_pages: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap()
        .commit()
        .unwrap();
        let mut owner = RendererOwner::from_legacy(renderer);
        let mut order = SectionOrder::default();
        let mut source = frame::tests::source(&hud);
        source.camera = DVec3::new(8., -56., 8.);
        source.pitch = 0.;
        source.fov_degrees = 90.;
        // Keep even the out-of-radius blue quad unfogged: bad distance selection
        // would add blue at the centre behind both translucent quads.
        source.distance = 512.;
        let far_stamp;
        {
            let mut facade = facade(&mut owner);
            facade
                .begin_streaming(WorldGeneration(1), ColumnLayout::new(-64, 384).unwrap())
                .unwrap();
            far_stamp = facade.request_stream_section([0, 0, 2]).unwrap().unwrap();
            assert_eq!(
                facade
                    .queue_stream_section([0, 0, 2], far_stamp, &mesh(1, true))
                    .unwrap(),
                Queued::Accepted
            );
            // Deliberately not nearest-first membership insertion.
            for (key, layer, translucent) in [
                ([0, 0, -2], 0x8002, false),
                ([0, 0, 8], 0x8002, false),
                ([0, 0, 1], 0, true),
            ] {
                let stamp = facade.request_stream_section(key).unwrap().unwrap();
                assert_eq!(
                    facade
                        .queue_stream_section(key, stamp, &mesh(layer, translucent))
                        .unwrap(),
                    Queued::Accepted
                );
            }
        }
        // Completed CPU jobs are not residency until the owning GL drain commits.
        rgb(draw(&mut owner, &mut order, &source), [0; 3]);
        assert!(order.visible().solid.is_empty());
        assert_eq!(
            facade(&mut owner)
                .drain_streaming(4, usize::MAX)
                .unwrap()
                .uploads,
            4
        );
        // Green behind red: R=128, G=128*(127/255), B=0 (independent blend expectation).
        rgb(draw(&mut owner, &mut order, &source), [128, 64, 0]);
        assert_eq!(order.visible().solid, [[0, 0, 1], [0, 0, 2]]);
        assert_eq!(order.visible().translucent, [[0, 0, 2], [0, 0, 1]]);
        source.camera.z = 32.;
        rgb(draw(&mut owner, &mut order, &source), [0, 128, 0]);
        assert_eq!(order.visible().solid, [[0, 0, 2]]);
        assert!(
            facade(&mut owner)
                .unload_stream_section([0, 0, 2], far_stamp)
                .unwrap()
        );
        rgb(draw(&mut owner, &mut order, &source), [0; 3]);
        assert!(order.visible().translucent.is_empty());
        {
            let mut facade = facade(&mut owner);
            facade
                .begin_streaming(WorldGeneration(2), ColumnLayout::new(0, 256).unwrap())
                .unwrap();
            let stamp = facade.request_stream_section([0, 0, 1]).unwrap().unwrap();
            assert_eq!(
                facade
                    .queue_stream_section([0, 0, 1], stamp, &mesh(0x8002, false))
                    .unwrap(),
                Queued::Accepted
            );
            facade.drain_streaming(1, usize::MAX).unwrap();
        }
        source.camera = DVec3::splat(8.);
        rgb(draw(&mut owner, &mut order, &source), [0, 0, 255]);
        assert_eq!(order.visible().solid, [[0, 0, 1]]);
        // This uses real Frame/mesher/facade types, not a normal-resource Game launch.
    }
}
