//! CPU preparation from the game's actual frame, not a parallel game model.
//! Visibility and translucent order remain the world's responsibility. This
//! adapter neither discovers sections nor silently drops unimplemented passes.
use super::{Geometry, GlintParameters, Limits};
use crate::{gfx, hud::HudBatch, lightmap, mesher::Vertex};
use glam::Mat4;
use horizon_render_api::{
    Fog, GameFog, gl_reversed_projection,
    scene::{SceneFrame, SectionKey},
};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fmt,
};

// Compatibility exports while callers move to the common C01 contract.
pub use horizon_render_api::render_contract::{FrameOptions as Options, VisibleSections};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Unsupported(&'static str),
    Invalid(&'static str),
    Budget(&'static str),
    Allocation,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "legacy game frame: {self:?}")
    }
}
impl std::error::Error for Error {}

pub struct Prepared<'a> {
    pub world: SceneFrame<'a>,
    pub hud: &'a HudBatch,
    pub lightmap: [u8; 1024],
    pub screen: &'a [crate::screen_effects::ScreenQuad],
    pub weather: &'a [crate::weather::Vertex],
    pub weather_time: crate::weather::Time,
    entities: Cow<'a, [Vertex]>,
    pub additive: Cow<'a, [Vertex]>,
    hand: &'a [Vertex],
    see_through: &'a [Vertex],
    hand_view_projection: [f32; 16],
    nether_lighting: bool,
    glint: GlintParameters,
}
impl<'a> Prepared<'a> {
    /// No GL calls or resource changes. Shader/material validation is still
    /// performed by `render_geometry`, also before framebuffer submission.
    pub fn new(
        frame: &'a gfx::Frame<'_>,
        size: [u32; 2],
        visible: VisibleSections<'a>,
        limits: Limits,
        options: Options,
    ) -> Result<Self, Error> {
        unsupported(frame)?;
        if frame.screen.len() > limits.screen_quads {
            return Err(Error::Budget("screen quads"));
        }
        if size.contains(&0) || size.iter().any(|&n| n > 4096) {
            return Err(Error::Invalid(
                "drawable size (minimized frames must be skipped)",
            ));
        }
        if !frame.camera.is_finite()
            || !frame.yaw.is_finite()
            || !frame.pitch.is_finite()
            || !frame.depth_far.is_finite()
            || frame.depth_far <= 0.05
            || !frame.distance.is_finite()
            || frame.distance <= 0.
            || ![frame.fov_degrees, frame.hand_fov_degrees]
                .iter()
                .all(|f| f.is_finite() && *f > 0. && *f < 180.)
            || !frame.bob.is_finite()
            || !frame.spin.is_finite()
        {
            return Err(Error::Invalid("camera/projection"));
        }
        if !frame.hud.scale.is_finite()
            || frame.hud.scale <= 0.
            || frame.hud.width != (size[0] as f32 / frame.hud.scale).ceil()
            || frame.hud.height != (size[1] as f32 / frame.hud.scale).ceil()
        {
            return Err(Error::Invalid("HUD dimensions (rebuild after resize)"));
        }
        if !unit(frame.glint_alpha) || !frame.glint_ticks.is_finite() {
            return Err(Error::Invalid("glint"));
        }
        unique(visible.solid, limits.sections)?;
        unique(visible.translucent, limits.sections)?;
        let static_visible = unique(frame.visible_static_block_entity_sections, limits.sections)?;
        let mut snapshots = HashMap::new();
        snapshots
            .try_reserve(static_visible.len())
            .map_err(|_| Error::Allocation)?;
        for snapshot in frame.static_block_entities {
            if static_visible.contains(&snapshot.section)
                && snapshots.insert(snapshot.section, snapshot).is_some()
            {
                return Err(Error::Invalid("duplicate visible static snapshot"));
            }
        }
        let mut count = 0usize;
        for vertices in [
            frame.entities,
            frame.hand,
            frame.see_through,
            frame.additive,
        ]
        .into_iter()
        .chain(
            frame
                .visible_static_block_entity_sections
                .iter()
                .filter_map(|key| snapshots.get(key).map(|s| s.vertices.as_ref())),
        ) {
            if !vertices.len().is_multiple_of(4) {
                return Err(Error::Invalid("quad vertex count"));
            }
            count = count
                .checked_add(vertices.len())
                .filter(|&n| n <= limits.entity_vertices)
                .ok_or(Error::Budget("combined frame vertices"))?;
        }
        if !frame.weather.len().is_multiple_of(4) || frame.weather.len() / 4 > limits.weather_quads
        {
            return Err(Error::Budget("weather quads/count"));
        }
        count = count
            .checked_add(frame.weather.len())
            .filter(|&n| n <= limits.entity_vertices)
            .ok_or(Error::Budget("combined frame vertices"))?;
        let entity_count = count
            - frame.hand.len()
            - frame.see_through.len()
            - frame.additive.len()
            - frame.weather.len();
        if frame.lightning.bolts.len() > limits.lightning_bolts {
            return Err(Error::Budget("lightning bolts"));
        }
        let bolt_vertices = frame
            .lightning
            .bolts
            .len()
            .checked_mul(gfx::lightning::QUADS as usize * 4)
            .ok_or(Error::Budget("lightning vertices"))?;
        if count
            .checked_add(bolt_vertices)
            .is_none_or(|n| n > limits.entity_vertices)
        {
            return Err(Error::Budget("combined frame vertices"));
        }
        let additive = if frame.lightning.bolts.is_empty() {
            Cow::Borrowed(frame.additive)
        } else {
            if frame
                .lightning
                .uv
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            {
                return Err(Error::Invalid("lightning white UV"));
            }
            let mut vertices = Vec::new();
            vertices
                .try_reserve_exact(bolt_vertices + frame.additive.len())
                .map_err(|_| Error::Allocation)?;
            for bolt in frame.lightning.bolts {
                let offset = horizon_render_api::relative_origin(bolt.at, frame.camera.to_array())
                    .ok_or(Error::Invalid("lightning position"))?;
                // Match vs_lightning exactly: cast the f64 relative offset to
                // f32 first, THEN add each local f32 vertex, in input bolt order.
                for mut vertex in gfx::lightning::geometry(bolt.seed, frame.lightning.uv) {
                    vertex.pos = std::array::from_fn(|axis| vertex.pos[axis] + offset[axis]);
                    if vertex.pos.iter().any(|v| !v.is_finite()) {
                        return Err(Error::Invalid("lightning position arithmetic"));
                    }
                    vertices.push(vertex);
                }
            }
            vertices.extend_from_slice(frame.additive);
            Cow::Owned(vertices)
        };
        let has_static = snapshots.values().any(|s| !s.vertices.is_empty());
        let entities = if has_static {
            let mut vertices = Vec::new();
            vertices
                .try_reserve_exact(entity_count)
                .map_err(|_| Error::Allocation)?;
            // Retain the game's visible-static ordering, then dynamic entities.
            // The coordinator retains stable order within each material kind.
            for key in frame.visible_static_block_entity_sections {
                if let Some(snapshot) = snapshots.get(key) {
                    for vertex in snapshot.vertices.iter() {
                        let mut vertex = *vertex;
                        vertex.pos = std::array::from_fn(|axis| {
                            ((f64::from(snapshot.origin[axis]) - frame.camera[axis])
                                + f64::from(vertex.pos[axis])) as f32
                        });
                        if vertex.pos.iter().any(|v| !v.is_finite()) {
                            return Err(Error::Invalid("static camera-relative position"));
                        }
                        vertices.push(vertex);
                    }
                }
            }
            vertices.extend_from_slice(frame.entities);
            Cow::Owned(vertices)
        } else {
            Cow::Borrowed(frame.entities)
        };
        let view = gfx::View::screen(
            frame.yaw,
            frame.pitch,
            frame.fov_degrees,
            size[0] as f32 / size[1] as f32,
            frame.depth_far,
        );
        let world_projection =
            gl_reversed_projection(view.view_proj(frame.bob * frame.spin).to_cols_array());
        let hand_projection = Mat4::from_cols_array(&crate::camera::hand_projection(
            frame.hand_fov_degrees,
            size[0] as f32,
            size[1] as f32,
            frame.depth_far,
        ));
        let hand_view_projection =
            gl_reversed_projection((hand_projection * frame.bob * view.view).to_cols_array());
        if world_projection
            .iter()
            .chain(&hand_view_projection)
            .any(|v| !v.is_finite())
        {
            return Err(Error::Invalid("projection arithmetic"));
        }
        let (color, ranges) = crate::fog::world_parameters(frame);
        if !color.iter().all(|&c| unit(c))
            || !ranges.iter().all(|v| v.is_finite())
            || ranges[0] > ranges[1]
            || ranges[2] > ranges[3]
        {
            return Err(Error::Invalid("fog color/ranges"));
        }
        if ![
            options.gamma,
            frame.darkness.0,
            frame.darkness.1,
            frame.night_vision,
            frame.boss_darken,
            frame.sky.sky_light_factor,
        ]
        .into_iter()
        .all(unit)
            || !frame.block_flicker.is_finite()
            || !frame
                .sky
                .ambient_light_color
                .into_iter()
                .chain(frame.sky.sky_light_color)
                .chain(frame.sky.block_light_tint)
                .all(unit)
        {
            return Err(Error::Invalid("classic lightmap parameters"));
        }
        let lightmap = lightmap::pixels(
            &frame.sky,
            lightmap::Parameters {
                gamma: (options.gamma - frame.darkness.0).max(0.),
                darkness: frame.darkness.1,
                night_vision: frame.night_vision,
                flicker: frame.block_flicker,
                boss_darken: frame.boss_darken,
                fullbright: options.fullbright,
            },
        );
        Ok(Self {
            world: SceneFrame {
                view_projection: world_projection,
                camera: frame.camera.to_array(),
                clear: [color[0], color[1], color[2], 1.],
                fog: Some(Fog {
                    color: [color[0], color[1], color[2]],
                    start: ranges[0],
                    end: ranges[1],
                    game: Some(GameFog {
                        start: ranges[2],
                        end: ranges[3],
                        alpha: color[3],
                    }),
                }),
                solid: visible.solid,
                translucent: visible.translucent,
            },
            hud: frame.hud,
            lightmap,
            screen: frame.screen,
            weather: frame.weather,
            weather_time: frame.weather_time,
            entities,
            additive,
            hand: frame.hand,
            see_through: frame.see_through,
            hand_view_projection,
            nether_lighting: frame.nether_lighting,
            glint: GlintParameters {
                strength: frame.glint_alpha,
                ticks: frame.glint_ticks,
            },
        })
    }

    pub fn geometry(&self) -> Geometry<'_> {
        Geometry {
            entities: &self.entities,
            hand: self.hand,
            see_through: self.see_through,
            hand_view_projection: self.hand_view_projection,
            nether_lighting: self.nether_lighting,
            glint: self.glint,
        }
    }
}
fn unit(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
fn unique(keys: &[SectionKey], limit: usize) -> Result<HashSet<SectionKey>, Error> {
    if keys.len() > limit {
        return Err(Error::Budget("visible section records"));
    }
    let mut set = HashSet::new();
    set.try_reserve(keys.len()).map_err(|_| Error::Allocation)?;
    for key in keys {
        if !set.insert(*key) {
            return Err(Error::Invalid("duplicate visible section"));
        }
    }
    Ok(set)
}
fn unsupported(frame: &gfx::Frame<'_>) -> Result<(), Error> {
    // These are real requested passes, not quality knobs this adapter may ignore.
    // Metadata for modern-only optimizations (motion, shadows, pack uniforms)
    // does not request an additional classic pass and is intentionally unused.
    for (requested, name) in [
        (frame.title, "title panorama"),
        (frame.clouds.is_some(), "clouds"),
        (
            !frame.debug_lines.is_empty()
                || !frame.debug_lines_on_top.is_empty()
                || !frame.debug_crosshair.is_empty(),
            "debug lines/crosshair",
        ),
        (
            frame.skin.is_some(),
            "GPU-posed entities (request CPU poses)",
        ),
        (!frame.glowing.is_empty(), "glowing silhouettes"),
        (
            !frame.dynamic_lights.is_empty() || !frame.colored_lights.is_empty(),
            "dynamic/colored lights",
        ),
        (!frame.emissive_layers.is_empty(), "emissive layers"),
        (frame.border.is_some(), "world border"),
        (
            frame.menu_blur.is_some() || !frame.post_effects.effects.is_empty(),
            "post effects/menu blur",
        ),
        (frame.chunk_fade != 0., "chunk fade"),
        (frame.water_plane.is_some(), "water mirror"),
    ] {
        if requested {
            return Err(Error::Unsupported(name));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{block_entity_render::StaticSectionSnapshot, environment::Sky};
    use glam::{DVec3, Vec3, Vec4};
    use std::sync::Arc;

    pub(crate) fn source(hud: &HudBatch) -> gfx::Frame<'_> {
        gfx::Frame {
            weather: &[],
            weather_time: crate::weather::Time::new(0, 0.0),
            camera: glam::DVec3::new(0.5, 100.0, 0.5),
            yaw: 0.0,
            pitch: 10.0,
            fov_degrees: 85.0,
            eye: glam::DVec3::new(0.5, 100.0, 0.5),
            eye_rotation: [0.0, 10.0],
            first_person: true,
            bob: glam::Mat4::IDENTITY,
            spin: glam::Mat4::IDENTITY,
            sky: crate::environment::Sky::default(),
            shadow_scene_revision: (0, 0),
            shadow_cache: false,
            distance: 12.0,
            detail_distance: 12.0,
            depth_far: 1000.0,
            underwater: false,
            water_plane: None,
            cave_culling: false,
            title: false,
            sky_blocked: false,
            skybox: false,
            end_sky: false,
            end_sky_sprite: None,
            environment_fog: None,
            water_fog: [0.0; 3],
            fog: None,
            dark_disc: false,
            outline: Vec::new(),
            outline_after_terrain: false,
            outline_high_contrast: false,
            debug_lines: &[],
            debug_lines_on_top: &[],
            debug_crosshair: &[],
            hud,
            glint_alpha: 0.75,
            glint_ticks: 0.0,
            nether_lighting: false,
            entities: &[],
            static_block_entities: &[],
            visible_static_block_entity_sections: &[],
            static_block_entity_stats: Default::default(),
            entity_motion: &[],
            entity_spans: &[],
            skin: None,
            glowing: &[],
            dynamic_lights: &[],
            colored_lights: &[],
            emissive_layers: &[],
            clouds: None,
            lightning: Default::default(),
            additive: &[],
            see_through: &[],
            shadow_only: &[],
            celestial: &[],
            stars: None,
            hand: &[],
            hand_fov_degrees: 70.0,
            screen: &[],
            pack: Default::default(),
            border: None,
            game_time: 0.0,
            shader_game_time: 0.0,
            end_portal_sprites: [[0.0; 4]; 2],
            crumbling_sprite: [0.0; 4],
            ticks: 0,
            darkness: (0.0, 0.0),
            night_vision: 0.0,
            block_flicker: 0.0,
            boss_darken: 0.0,
            chunk_fade: 0.0,
            post_effects: crate::post_chain::Request::none(),
            menu_blur: None,
        }
    }
    fn prepare<'a>(frame: &'a gfx::Frame<'_>) -> Result<Prepared<'a>, Error> {
        Prepared::new(
            frame,
            [64; 2],
            VisibleSections::default(),
            Limits::default(),
            Options::default(),
        )
    }
    fn quad(pos: [f32; 3]) -> [Vertex; 4] {
        [Vertex {
            pos,
            uv: [0.5; 2],
            layer: 0,
            color: 0xffffffff,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.; 2],
        }; 4]
    }
    #[test]
    fn actual_lightning_uses_shared_seed_geometry_offset_order_and_combined_limits() {
        let hud = HudBatch::new([64; 2]);
        let mut frame = source(&hud);
        frame.camera = DVec3::new(30_000_000., 0., 0.);
        let bolts = [
            gfx::lightning::Bolt {
                at: [30_000_000.25, 2., 3.],
                seed: 9,
            },
            gfx::lightning::Bolt {
                at: [30_000_001.5, 4., 5.],
                seed: 9,
            },
        ];
        frame.lightning = gfx::lightning::Input {
            bolts: &bolts,
            uv: [0.25, 0.5],
        };
        let extra = quad([9.; 3]);
        frame.additive = &extra;
        let plan = prepare(&frame).unwrap();
        let template = gfx::lightning::geometry(9, [0.25, 0.5]);
        assert_eq!(template.len(), 896);
        assert_eq!(plan.additive.len(), 1796);
        for (index, offset) in [[0.25, 2., 3.], [1.5, 4., 5.]].into_iter().enumerate() {
            for (actual, local) in plan.additive[index * 896..(index + 1) * 896]
                .iter()
                .zip(&template)
            {
                assert_eq!(
                    actual.pos,
                    std::array::from_fn(|axis| local.pos[axis] + offset[axis])
                );
                assert_eq!(actual.color, local.color);
                assert_eq!(actual.layer, local.layer);
                assert_eq!(actual.uv, [0.25, 0.5]);
            }
        }
        assert_eq!(plan.additive[1792].pos, [9.; 3]);
        let mut limits = Limits {
            entity_vertices: 1795,
            ..Limits::default()
        };
        assert!(
            Prepared::new(
                &frame,
                [64; 2],
                VisibleSections::default(),
                limits,
                Options::default()
            )
            .is_err()
        );
        limits.entity_vertices = 1796;
        limits.lightning_bolts = 1;
        assert_eq!(
            Prepared::new(
                &frame,
                [64; 2],
                VisibleSections::default(),
                limits,
                Options::default()
            )
            .err(),
            Some(Error::Budget("lightning bolts"))
        );
        frame.lightning.uv = [f32::NAN, 0.];
        assert_eq!(
            prepare(&frame).err(),
            Some(Error::Invalid("lightning white UV"))
        );
        frame.lightning = gfx::lightning::Input {
            bolts: &[],
            uv: [f32::NAN; 2],
        };
        assert!(matches!(
            prepare(&frame).unwrap().additive,
            Cow::Borrowed(_)
        ));
        let bad = [gfx::lightning::Bolt {
            at: [f64::INFINITY, 0., 0.],
            seed: 1,
        }];
        frame.lightning = gfx::lightning::Input {
            bolts: &bad,
            uv: [0.; 2],
        };
        assert_eq!(
            prepare(&frame).err(),
            Some(Error::Invalid("lightning position"))
        );
    }
    #[test]
    fn game_matrices_preserve_xy_bob_spin_and_unreverse_depth() {
        let hud = HudBatch::new([64; 2]);
        let mut frame = source(&hud);
        frame.camera = DVec3::ZERO;
        frame.yaw = 37.;
        frame.pitch = -18.;
        frame.bob =
            Mat4::from_rotation_z(0.07) * Mat4::from_translation(Vec3::new(0.02, -0.04, 0.));
        frame.spin = Mat4::from_rotation_x(-0.03);
        let plan = prepare(&frame).unwrap();
        let original = gfx::View::screen(
            frame.yaw,
            frame.pitch,
            frame.fov_degrees,
            1.,
            frame.depth_far,
        );
        let input = Vec4::new(0.7, -0.9, 4., 1.);
        let old = original.view_proj(frame.bob * frame.spin) * input;
        let new = Mat4::from_cols_array(&plan.world.view_projection) * input;
        assert!((new.x - old.x).abs() < 0.00001);
        assert!((new.y - old.y).abs() < 0.00001);
        assert!((new.z - (old.w - 2. * old.z)).abs() < 0.00001);
        assert!((new.w - old.w).abs() < 0.00001);
        let hand = Mat4::from_cols_array(&crate::camera::hand_projection(
            70.,
            64.,
            64.,
            frame.depth_far,
        ));
        let expected = gl_reversed_projection((hand * frame.bob * original.view).to_cols_array());
        assert_eq!(plan.geometry().hand_view_projection, expected);
        frame.bob = Mat4::IDENTITY;
        frame.spin = Mat4::IDENTITY;
        frame.yaw = 0.;
        frame.pitch = 0.;
        let plan = prepare(&frame).unwrap();
        let matrix = Mat4::from_cols_array(&plan.world.view_projection);
        let depth = |z| {
            let p = matrix * Vec4::new(0., 0., z, 1.);
            p.z / p.w
        };
        assert!((depth(0.05) + 1.).abs() < 0.00001);
        assert!((depth(frame.depth_far) - 1.).abs() < 0.00001);
        assert!(depth(1.) < depth(2.));
    }
    #[test]
    fn visible_static_vertices_keep_precision_order_and_combined_budget() {
        let hud = HudBatch::new([64; 2]);
        let mut frame = source(&hud);
        frame.camera = DVec3::new(29_999_999.75, 0., 0.);
        let snapshots = [
            StaticSectionSnapshot {
                section: [1, 0, 0],
                origin: [30_000_000, 0, 0],
                revision: 1,
                entities: 1,
                vertices: Arc::from(quad([0.5, 0., 0.])),
            },
            StaticSectionSnapshot {
                section: [2, 0, 0],
                origin: [30_000_001, 0, 0],
                revision: 2,
                entities: 1,
                vertices: Arc::from(quad([0.25, 0., 0.])),
            },
            // Invisible malformed geometry must not consume the visible budget.
            StaticSectionSnapshot {
                section: [3, 0, 0],
                origin: [0; 3],
                revision: 3,
                entities: 1,
                vertices: Arc::from(vec![quad([f32::NAN; 3])[0]; 5]),
            },
        ];
        let dynamic = quad([3.; 3]);
        frame.static_block_entities = &snapshots;
        frame.visible_static_block_entity_sections = &[[2, 0, 0], [1, 0, 0], [4, 0, 0]];
        frame.entities = &dynamic;
        let plan = prepare(&frame).unwrap();
        let geometry = plan.geometry();
        assert_eq!(geometry.entities.len(), 12);
        assert_eq!(geometry.entities[0].pos, [1.5, 0., 0.]);
        assert_eq!(geometry.entities[4].pos, [0.75, 0., 0.]);
        assert_eq!(geometry.entities[8].pos, [3.; 3]);
        let limits = Limits {
            entity_vertices: 11,
            ..Limits::default()
        };
        assert_eq!(
            Prepared::new(
                &frame,
                [64; 2],
                VisibleSections::default(),
                limits,
                Options::default()
            )
            .err(),
            Some(Error::Budget("combined frame vertices"))
        );
        frame.visible_static_block_entity_sections = &[[1, 0, 0], [1, 0, 0]];
        assert_eq!(
            prepare(&frame).err(),
            Some(Error::Invalid("duplicate visible section"))
        );
        frame.visible_static_block_entity_sections = &[];
        assert!(matches!(
            prepare(&frame).unwrap().entities,
            Cow::Borrowed(_)
        ));
        frame.hand = &dynamic;
        frame.see_through = &dynamic;
        assert!(
            Prepared::new(
                &frame,
                [64; 2],
                VisibleSections::default(),
                limits,
                Options::default()
            )
            .is_err()
        );
    }
    #[test]
    fn actual_fog_overrides_fallbacks_and_shared_lightmap_is_uploaded_input() {
        let hud = HudBatch::new([64; 2]);
        let mut frame = source(&hud);
        frame.sky = Sky {
            fog_color: [0.25, 0.5, 0.75],
            ambient_light_color: [0.25, 0.5, 0.75],
            ..Sky::default()
        };
        frame.distance = 32.;
        let plan = prepare(&frame).unwrap();
        assert_eq!(plan.world.clear, [0.25, 0.5, 0.75, 1.]);
        let fog = plan.world.fog.unwrap();
        assert_eq!((fog.start, fog.end), (f32::MAX, f32::MAX));
        assert_eq!(
            fog.game,
            Some(GameFog {
                start: 24.,
                end: 32.,
                alpha: 1.
            })
        );
        assert_eq!(&plan.lightmap[..4], &[63, 127, 191, 255]);
        frame.underwater = true;
        frame.water_fog = [0.1, 0.2, 0.3];
        let fog = prepare(&frame).unwrap().world.fog.unwrap();
        assert_eq!((fog.start, fog.end, fog.color), (-8., 96., [0.1, 0.2, 0.3]));
        frame.fog = Some(crate::fog::FogData {
            color: [0.4, 0.3, 0.2, 0.5],
            environmental_start: 2.,
            environmental_end: 2.,
            render_distance_start: 4.,
            render_distance_end: 8.,
            sky_end: 100.,
            cloud_end: 100.,
        });
        let plan = prepare(&frame).unwrap();
        assert_eq!(plan.world.clear, [0.4, 0.3, 0.2, 1.]);
        assert_eq!(plan.world.fog.unwrap().game.unwrap().alpha, 0.5);
        assert_eq!(plan.world.fog.unwrap().start, 2.);
        frame.darkness = (0.3, 0.1);
        frame.night_vision = 0.7;
        frame.block_flicker = -0.1;
        frame.boss_darken = 0.6;
        let options = Options {
            gamma: 0.8,
            fullbright: false,
        };
        let plan = Prepared::new(
            &frame,
            [64; 2],
            VisibleSections::default(),
            Limits::default(),
            options,
        )
        .unwrap();
        assert_eq!(
            plan.lightmap,
            lightmap::pixels(
                &frame.sky,
                lightmap::Parameters {
                    darkness: 0.1,
                    gamma: 0.5,
                    night_vision: 0.7,
                    flicker: -0.1,
                    boss_darken: 0.6,
                    fullbright: false
                }
            )
        );
    }
    #[test]
    fn rejects_requested_missing_passes_invalid_cameras_and_stale_hud() {
        let hud = HudBatch::new([64; 2]);
        let mut frame = source(&hud);
        frame.title = true;
        assert_eq!(
            prepare(&frame).err(),
            Some(Error::Unsupported("title panorama"))
        );
        frame.title = false;
        frame.skybox = true;
        assert!(prepare(&frame).is_ok());
        frame.sky_blocked = true;
        assert!(prepare(&frame).is_ok());
        frame.chunk_fade = 0.3;
        assert_eq!(
            prepare(&frame).err(),
            Some(Error::Unsupported("chunk fade"))
        );
        frame.chunk_fade = 0.;
        frame.menu_blur = Some(0.);
        assert_eq!(
            prepare(&frame).err(),
            Some(Error::Unsupported("post effects/menu blur"))
        );
        frame.menu_blur = None;
        frame.fov_degrees = 180.;
        assert_eq!(
            prepare(&frame).err(),
            Some(Error::Invalid("camera/projection"))
        );
        frame.fov_degrees = 70.;
        frame.glint_ticks = f32::NAN;
        assert_eq!(prepare(&frame).err(), Some(Error::Invalid("glint")));
        frame.glint_ticks = 0.;
        for size in [[0, 64], [4097, 64], [96, 48]] {
            assert!(
                Prepared::new(
                    &frame,
                    size,
                    VisibleSections::default(),
                    Limits::default(),
                    Options::default()
                )
                .is_err()
            );
        }
        let duplicate = [[0; 3]; 2];
        let visible = VisibleSections {
            solid: &duplicate,
            translucent: &[],
        };
        assert!(
            Prepared::new(
                &frame,
                [64; 2],
                visible,
                Limits::default(),
                Options::default()
            )
            .is_err()
        );
    }
}
