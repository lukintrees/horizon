//! Adapts the game's real frame camera, fog and lightmap to the partial GL
//! world draw planner. This does not yet install the backend in the game loop.
use super::{
    entity,
    hud::{self, Atlas, PatchAtlas},
    screen,
    world::{PlanError, SectionDraw, SectionKey, World},
};
use crate::gfx::{Frame, View};
use glam::{Mat4, Vec4};
use horizon_legacy_gl::Fog;

/// Generate exactly the same RGBA lightmap as the existing renderer. Gamma and
/// fullbright are renderer settings rather than fields in `Frame`.
pub fn lightmap(frame: &Frame<'_>, gamma: f32, fullbright: bool) -> [u8; 16 * 16 * 4] {
    crate::gfx::lightmap::pixels(
        &frame.sky,
        frame.darkness.1,
        (gamma - frame.darkness.0).max(0.0),
        frame.night_vision,
        frame.block_flicker,
        frame.boss_darken,
        fullbright,
    )
}

/// Convert reversed-Z wgpu clip depth (near=1, far=0) to conventional OpenGL
/// clip depth (near=-1, far=1). This operates on homogeneous clip coordinates,
/// not on a perspective-divided depth value.
fn gl_clip_from_wgpu(matrix: Mat4) -> Mat4 {
    let depth = Mat4::from_cols(
        Vec4::X,
        Vec4::Y,
        Vec4::new(0.0, 0.0, -2.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 1.0),
    );
    depth * matrix
}

pub fn gl_view_projection(view: View, pose: Mat4) -> Mat4 {
    gl_clip_from_wgpu(view.view_proj(pose))
}

/// Projection for the camera-relative first-person hand, excluding world spin.
pub fn hand_view_projection(frame: &Frame<'_>, viewport: [u32; 2]) -> Result<[f32; 16], PlanError> {
    if viewport.contains(&0)
        || !frame.hand_fov_degrees.is_finite()
        || !(0.0..180.0).contains(&frame.hand_fov_degrees)
        || frame.hand_fov_degrees == 0.0
        || !frame.depth_far.is_finite()
        || frame.depth_far <= 0.05
        || !frame.yaw.is_finite()
        || !frame.pitch.is_finite()
    {
        return Err(PlanError::InvalidTransform);
    }
    let projection = Mat4::from_cols_array(&crate::camera::hand_projection(
        frame.hand_fov_degrees,
        viewport[0] as f32,
        viewport[1] as f32,
        frame.depth_far,
    ));
    let view = crate::camera::view_matrix(frame.yaw, frame.pitch);
    let matrix = gl_clip_from_wgpu(projection * frame.bob * view).to_cols_array();
    if matrix.iter().any(|value| !value.is_finite()) {
        return Err(PlanError::InvalidTransform);
    }
    Ok(matrix)
}

/// Fog for camera-relative entities, or the base fog that the world planner
/// offsets per section.
pub fn frame_fog(frame: &Frame<'_>) -> Fog {
    let (color, ranges) = if let Some(fog) = frame.fog {
        (
            fog.color,
            [
                fog.environmental_start,
                fog.environmental_end,
                fog.render_distance_start,
                fog.render_distance_end,
            ],
        )
    } else if frame.underwater {
        (
            [
                frame.water_fog[0],
                frame.water_fog[1],
                frame.water_fog[2],
                1.0,
            ],
            [-8.0, 96.0, -8.0, 96.0],
        )
    } else {
        let (start, end) = frame.environment_fog.unwrap_or((f32::MAX, f32::MAX));
        let d = frame.distance;
        (
            [
                frame.sky.fog_color[0],
                frame.sky.fog_color[1],
                frame.sky.fog_color[2],
                1.0,
            ],
            [start, end, (d - d.min(64.0) * 0.25).max(0.0), d],
        )
    };
    Fog {
        color,
        ranges,
        section_offset: [0.0; 3],
    }
}

/// Convert the ordinary entity quads actually supplied by the game. Special
/// materials are rejected by the adapter; this does not yet render the hand,
/// static block-entity cache, particles, or other separate `Frame` arrays.
pub fn entities(
    atlas: &Atlas,
    frame: &Frame<'_>,
    lightmap: &[u8; 1024],
) -> Result<entity::Mesh, entity::Error> {
    entity::build(atlas, frame.entities, lightmap, frame.nether_lighting)
}

/// Camera-relative world/entity transform for a flat-screen game frame.
pub fn frame_view_projection(
    frame: &Frame<'_>,
    viewport: [u32; 2],
) -> Result<[f32; 16], PlanError> {
    if viewport.contains(&0)
        || !frame.fov_degrees.is_finite()
        || !(0.0..180.0).contains(&frame.fov_degrees)
        || frame.fov_degrees == 0.0
        || !frame.depth_far.is_finite()
        || frame.depth_far <= 0.05
        || !frame.yaw.is_finite()
        || !frame.pitch.is_finite()
    {
        return Err(PlanError::InvalidTransform);
    }
    let aspect = viewport[0] as f32 / viewport[1] as f32;
    let view = View::screen(
        frame.yaw,
        frame.pitch,
        frame.fov_degrees,
        aspect,
        frame.depth_far,
    );
    let matrix = gl_view_projection(view, frame.bob * frame.spin).to_cols_array();
    if matrix.iter().any(|v| !v.is_finite()) {
        return Err(PlanError::InvalidTransform);
    }
    Ok(matrix)
}

/// Plan the world's solid and translucent passes from an actual game frame.
/// This is flat-screen only, uses externally supplied visibility, and leaves
/// entities, hand, sky, far terrain and HUD to subsequent passes.
pub fn plan<'a>(
    world: &'a mut World,
    frame: &Frame<'_>,
    viewport: [u32; 2],
    min_y: i32,
    visible: &[SectionKey],
) -> Result<Vec<SectionDraw<'a>>, PlanError> {
    let matrix = frame_view_projection(frame, viewport)?;
    world.plan(
        visible,
        frame.camera.to_array(),
        min_y,
        matrix,
        Some(frame_fog(frame)),
    )
}

/// Errors in the partial native frame bridge. Unsupported entity and HUD
/// materials are returned explicitly instead of silently drawing them wrong.
#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    World(PlanError),
    Entity(entity::Error),
    Screen(screen::Error),
    Hud(hud::Error),
}

/// One visible static block-entity section, using section-local vertices.
pub struct StaticEntityDraw {
    pub mesh: entity::Mesh,
    pub transform: [f32; 16],
    pub fog: Fog,
}

/// CPU data for a partial frame: solid terrain, static and ordinary entities,
/// translucent terrain, hand, then HUD. Many other game passes are missing.
pub struct FramePlan<'a> {
    pub terrain: Vec<SectionDraw<'a>>,
    pub solid_count: usize,
    pub entities: entity::Mesh,
    pub static_entities: Vec<StaticEntityDraw>,
    pub omitted_static_overlays: Vec<(u32, usize)>,
    /// Nonempty only for `prepare_supported_frame`; these quads are not drawn.
    pub omitted_entity_overlays: Vec<(u32, usize)>,
    /// Camera-relative hand geometry, drawn after clearing the world's depth.
    pub hand: entity::Mesh,
    pub omitted_hand_overlays: Vec<(u32, usize)>,
    pub hand_view_projection: Option<[f32; 16]>,
    pub screen: screen::Mesh,
    pub hud: hud::Mesh,
    /// Unsupported GUI blends and omitted triangle counts in partial mode.
    pub omitted_hud_blends: Vec<(crate::hud::Blend, usize)>,
    pub view_projection: [f32; 16],
    pub fog: Fog,
}

/// Prepare ordinary passes directly from the game's `Frame`. HUD atlas patches
/// are applied first, even when a later conversion fails, and remain dirty
/// until the caller confirms their GPU upload. The caller owns visibility,
/// texture animation, GL context, framebuffer clear and presentation.
pub fn prepare_frame<'a>(
    world: &'a mut World,
    gui: &mut PatchAtlas,
    frame: &Frame<'_>,
    viewport: [u32; 2],
    min_y: i32,
    visible: &[SectionKey],
    gamma: f32,
    fullbright: bool,
) -> Result<FramePlan<'a>, FrameError> {
    prepare_frame_inner(
        world, gui, frame, viewport, min_y, visible, gamma, fullbright, false,
    )
}

/// Prepare an explicitly incomplete frame, omitting unsupported entity
/// materials and returning their codes/counts in `omitted_entity_overlays`.
/// Unsupported HUD blends are reported separately; malformed ranges and
/// supported geometry still fail. No omission is silent.
pub fn prepare_supported_frame<'a>(
    world: &'a mut World,
    gui: &mut PatchAtlas,
    frame: &Frame<'_>,
    viewport: [u32; 2],
    min_y: i32,
    visible: &[SectionKey],
    gamma: f32,
    fullbright: bool,
) -> Result<FramePlan<'a>, FrameError> {
    prepare_frame_inner(
        world, gui, frame, viewport, min_y, visible, gamma, fullbright, true,
    )
}

fn prepare_frame_inner<'a>(
    world: &'a mut World,
    gui: &mut PatchAtlas,
    frame: &Frame<'_>,
    viewport: [u32; 2],
    min_y: i32,
    visible: &[SectionKey],
    gamma: f32,
    fullbright: bool,
    skip_unsupported: bool,
) -> Result<FramePlan<'a>, FrameError> {
    gui.apply(&frame.hud.patches).map_err(FrameError::Hud)?;
    let view_projection = frame_view_projection(frame, viewport).map_err(FrameError::World)?;
    let lights = lightmap(frame, gamma, fullbright);
    let entities = if frame.title {
        entity::BuildReport::default()
    } else if skip_unsupported {
        entity::build_supported(gui.atlas(), frame.entities, &lights, frame.nether_lighting)
            .map_err(FrameError::Entity)?
    } else {
        entity::BuildReport {
            mesh: entities(gui.atlas(), frame, &lights).map_err(FrameError::Entity)?,
            omitted: Vec::new(),
        }
    };
    let mut static_entities = Vec::new();
    let mut omitted_static_overlays: Vec<(u32, usize)> = Vec::new();
    if !frame.title {
        static_entities
            .try_reserve(frame.visible_static_block_entity_sections.len())
            .map_err(|_| FrameError::Entity(entity::Error::AllocationFailed))?;
        for section in frame.visible_static_block_entity_sections {
            // Like the wgpu path, snapshots are sorted by section key; missing
            // sections are skipped and each visible snapshot is drawn in order.
            let Ok(index) = frame
                .static_block_entities
                .binary_search_by_key(section, |s| s.section)
            else {
                continue;
            };
            let snapshot = &frame.static_block_entities[index];
            let report = if skip_unsupported {
                entity::build_supported(
                    gui.atlas(),
                    &snapshot.vertices,
                    &lights,
                    frame.nether_lighting,
                )
                .map_err(FrameError::Entity)?
            } else {
                entity::BuildReport {
                    mesh: entity::build(
                        gui.atlas(),
                        &snapshot.vertices,
                        &lights,
                        frame.nether_lighting,
                    )
                    .map_err(FrameError::Entity)?,
                    omitted: Vec::new(),
                }
            };
            for (overlay, count) in report.omitted {
                if let Some((_, total)) = omitted_static_overlays
                    .iter_mut()
                    .find(|(code, _)| *code == overlay)
                {
                    *total = total.saturating_add(count);
                } else {
                    omitted_static_overlays
                        .try_reserve(1)
                        .map_err(|_| FrameError::Entity(entity::Error::AllocationFailed))?;
                    omitted_static_overlays.push((overlay, count));
                }
            }
            if report.mesh.vertices.is_empty() {
                continue;
            }
            let offset =
                (glam::DVec3::from_array(snapshot.origin.map(f64::from)) - frame.camera).as_vec3();
            if !offset.is_finite() {
                return Err(FrameError::World(PlanError::InvalidTransform));
            }
            let transform =
                Mat4::from_cols_array(&view_projection) * Mat4::from_translation(offset);
            let mut fog = frame_fog(frame);
            fog.section_offset = offset.to_array();
            static_entities.push(StaticEntityDraw {
                mesh: report.mesh,
                transform: transform.to_cols_array(),
                fog,
            });
        }
    }
    let hand = if frame.title || frame.hand.is_empty() {
        entity::BuildReport::default()
    } else if skip_unsupported {
        entity::build_supported(gui.atlas(), frame.hand, &lights, frame.nether_lighting)
            .map_err(FrameError::Entity)?
    } else {
        entity::BuildReport {
            mesh: entity::build(gui.atlas(), frame.hand, &lights, frame.nether_lighting)
                .map_err(FrameError::Entity)?,
            omitted: Vec::new(),
        }
    };
    let hand_view_projection = if hand.mesh.vertices.is_empty() {
        None
    } else {
        Some(hand_view_projection(frame, viewport).map_err(FrameError::World)?)
    };
    let screen =
        screen::build(gui.atlas(), frame.screen, skip_unsupported).map_err(FrameError::Screen)?;
    let hud = if skip_unsupported {
        hud::build_supported(gui.atlas(), frame.hud, viewport).map_err(FrameError::Hud)?
    } else {
        hud::BuildReport {
            mesh: hud::build(gui.atlas(), frame.hud, viewport).map_err(FrameError::Hud)?,
            omitted: Vec::new(),
        }
    };
    world.set_lightmap(lights);
    let terrain = if frame.title {
        Vec::new()
    } else {
        world
            .plan(
                visible,
                frame.camera.to_array(),
                min_y,
                view_projection,
                Some(frame_fog(frame)),
            )
            .map_err(FrameError::World)?
    };
    let solid_count = terrain
        .iter()
        .position(|draw| draw.translucent)
        .unwrap_or(terrain.len());
    Ok(FramePlan {
        terrain,
        solid_count,
        entities: entities.mesh,
        static_entities,
        omitted_static_overlays,
        omitted_entity_overlays: entities.omitted,
        hand: hand.mesh,
        omitted_hand_overlays: hand.omitted,
        hand_view_projection,
        screen,
        hud: hud.mesh,
        omitted_hud_blends: hud.omitted,
        view_projection,
        fog: frame_fog(frame),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn test_frame(hud: &crate::hud::HudBatch) -> Frame<'_> {
        Frame {
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

    #[test]
    fn game_frame_prepares_ordered_passes_and_retains_patches_on_error() {
        let block_atlas = super::super::atlas::Atlas::new(1, 1, 4, &[255; 4]).unwrap();
        let mut world = World::new(block_atlas, [255; 1024]);
        let quad = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
            .map(|uv| crate::mesher::TerrainVertex::new([uv[0], uv[1], 0.], uv, 0, u32::MAX, 0));
        let mut section = crate::mesher::SectionMesh::default();
        section.solid.extend_from_slice(&quad);
        section.translucent.extend_from_slice(&quad);
        world.upload((0, 0, 0), &section).unwrap();
        let gui_atlas = hud::Atlas::new(1, 1, &[255; 4], 4).unwrap();
        let mut gui = PatchAtlas::new(gui_atlas);
        let mut hud_batch = crate::hud::HudBatch::new([32, 32]);
        hud_batch.patches.push(crate::font::Patch {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            rgba: vec![128; 4],
        });
        hud_batch.quad(
            [[0., 0.], [16., 0.], [16., 16.], [0., 16.]],
            [[0.5, 0.5]; 4],
            u32::MAX,
        );
        let entities = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]].map(|uv| crate::mesher::Vertex {
            pos: [uv[0], uv[1], 0.],
            uv,
            layer: 0,
            color: u32::MAX,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.; 2],
        });
        let mut frame = test_frame(&hud_batch);
        let snapshots = [crate::block_entity_render::StaticSectionSnapshot {
            section: [1, 0, 0],
            origin: [16, 0, 0],
            revision: 1,
            entities: 1,
            vertices: entities.to_vec().into(),
        }];
        let visible_static = [[1, 0, 0]];
        frame.static_block_entities = &snapshots;
        frame.visible_static_block_entity_sections = &visible_static;
        frame.entities = &entities;
        frame.hand = &entities;
        let plan = prepare_frame(
            &mut world,
            &mut gui,
            &frame,
            [32, 32],
            0,
            &[(0, 0, 0)],
            0.,
            false,
        )
        .unwrap();
        assert_eq!(plan.terrain.len(), 2);
        assert_eq!(plan.solid_count, 1);
        assert!(!plan.terrain[0].translucent);
        assert!(plan.terrain[1].translucent);
        assert!(!plan.entities.vertices.is_empty());
        assert_eq!(plan.static_entities.len(), 1);
        assert_eq!(
            plan.static_entities[0].fog.section_offset,
            [15.5, -100., -0.5]
        );
        assert!(!plan.static_entities[0].mesh.vertices.is_empty());
        assert!(!plan.hand.vertices.is_empty());
        assert_ne!(plan.hand_view_projection.unwrap(), plan.view_projection);
        assert!(!plan.hud.vertices.is_empty());
        assert_eq!(gui.dirty().count(), 1);
        drop(plan);
        gui.uploaded(0).unwrap();
        assert!(matches!(
            prepare_frame(&mut world, &mut gui, &frame, [0, 32], 0, &[], 0., false),
            Err(FrameError::World(PlanError::InvalidTransform))
        ));
        assert_eq!(gui.dirty().count(), 1); // glyphs survive a skipped frame
        let mut unsupported = entities;
        for vertex in &mut unsupported {
            vertex.layer = 7;
        }
        let mixed = [entities, unsupported, entities].concat();
        frame.entities = &mixed;
        frame.hand = &unsupported;
        assert!(matches!(
            prepare_frame(
                &mut world,
                &mut gui,
                &frame,
                [32, 32],
                0,
                &[(0, 0, 0)],
                0.,
                false
            ),
            Err(FrameError::Entity(entity::Error::UnsupportedOverlay(7)))
        ));
        let partial = prepare_supported_frame(
            &mut world,
            &mut gui,
            &frame,
            [32, 32],
            0,
            &[(0, 0, 0)],
            0.,
            false,
        )
        .unwrap();
        assert_eq!(partial.omitted_entity_overlays, [(7, 1)]);
        assert_eq!(partial.omitted_hand_overlays, [(7, 1)]);
        assert!(partial.hand_view_projection.is_none());
        assert_eq!(partial.terrain.len(), 2);
        assert!(!partial.entities.vertices.is_empty());
        drop(partial);
        frame.title = true;
        let title = prepare_frame(
            &mut world,
            &mut gui,
            &frame,
            [32, 32],
            0,
            &[(0, 0, 0)],
            0.,
            false,
        )
        .unwrap();
        assert!(title.terrain.is_empty());
        assert!(title.entities.vertices.is_empty());
        assert!(title.static_entities.is_empty());
        assert!(title.hand.vertices.is_empty());
        assert!(!title.hud.vertices.is_empty());
    }

    #[test]
    fn reversed_wgpu_depth_becomes_normal_gl_depth() {
        let view = View::screen(0.0, 0.0, 70.0, 1.0, 512.0);
        let wgpu = view.projection;
        let gl = gl_view_projection(
            View {
                view: Mat4::IDENTITY,
                ..view
            },
            Mat4::IDENTITY,
        );
        for z in [-0.05, -1.0, -512.0] {
            let input = Vec4::new(0.0, 0.0, z, 1.0);
            let a = wgpu * input;
            let b = gl * input;
            assert!((b.z / b.w - (1.0 - 2.0 * a.z / a.w)).abs() < 0.00001);
        }
        let near = gl * Vec4::new(0.0, 0.0, -0.05, 1.0);
        let far = gl * Vec4::new(0.0, 0.0, -512.0, 1.0);
        assert!((near.z / near.w + 1.0).abs() < 0.00001);
        assert!((far.z / far.w - 1.0).abs() < 0.00001);
    }
}
