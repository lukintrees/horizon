//! PR 7: CPU entity posing for the GL2 `entity` program.
//!
//! GL 2.1 has no vertex texture fetch, instancing or compute skinning, so the
//! four billboard corners of every entity are expanded here. The module emits
//! plain `GlVertex` records and `GL_UNSIGNED_SHORT` indices; it makes no GL
//! calls and has no external crate dependencies.

use std::error::Error;
use std::fmt;

use super::atlas::{Atlas, AtlasError};
use super::mesh::{GlVertex, OPAQUE_LAYER};
use super::state::GlCommand;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityPose {
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub scale: f32,
    pub layer: u16,
    pub color: [u8; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityCamera {
    pub offset: [f32; 3],
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityDraw {
    pub pose_index: usize,
    pub first_index: u32,
    pub index_count: u32,
    pub layer: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityBatch {
    pub vertices: Vec<GlVertex>,
    pub indices: Vec<u16>,
    pub draws: Vec<EntityDraw>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EntityError {
    Atlas(AtlasError),
    LayerOutOfRange { layer: u16, layers: u32 },
    TooManyVertices,
}

impl fmt::Display for EntityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Atlas(err) => write!(f, "atlas: {err}"),
            Self::LayerOutOfRange { layer, layers } => {
                write!(f, "layer {layer} is outside 0..{layers}")
            }
            Self::TooManyVertices => write!(f, "entity batch exceeds GL_UNSIGNED_SHORT"),
        }
    }
}

impl Error for EntityError {}

impl From<AtlasError> for EntityError {
    fn from(err: AtlasError) -> Self {
        Self::Atlas(err)
    }
}

impl EntityPose {
    /// Four screen-space corners, far-to-near order unchanged.
    pub fn screen_quad(&self, camera: &EntityCamera) -> [[f32; 3]; 4] {
        let (sin, cos) = self.yaw.sin_cos();
        let corners = [[-0.5_f32, -0.5_f32], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]];
        let mut out = [[0.0_f32; 3]; 4];
        for (dst, [cx, cy]) in out.iter_mut().zip(corners) {
            let rx = cx * self.scale * cos - cy * self.scale * sin;
            let ry = cx * self.scale * sin + cy * self.scale * cos + self.pitch * 0.25;
            let x = (self.position[0] + rx + camera.offset[0]) * camera.scale;
            let y = (self.position[1] + ry + camera.offset[1]) * camera.scale;
            *dst = [x, y, self.position[2] + camera.offset[2]];
        }
        out
    }

    pub fn reference() -> [EntityPose; 2] {
        [
            EntityPose {
                position: [1.0, 2.0, 0.0],
                yaw: 0.0,
                pitch: 0.0,
                scale: 2.0,
                layer: 1,
                color: [200, 100, 50, 255],
            },
            EntityPose {
                position: [4.0, 3.0, 0.0],
                yaw: 0.0,
                pitch: 0.0,
                scale: 1.0,
                layer: 2,
                color: [50, 100, 200, 255],
            },
        ]
    }
}

impl EntityCamera {
    pub fn reference() -> Self {
        Self {
            offset: [0.0, 0.0, 0.0],
            scale: 4.0,
        }
    }
}

impl EntityBatch {
    pub fn build(
        poses: &[EntityPose],
        camera: &EntityCamera,
        atlas: &Atlas,
    ) -> Result<Self, EntityError> {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut draws = Vec::new();
        for (pose_index, pose) in poses.iter().enumerate() {
            let layer = pose.layer & !OPAQUE_LAYER;
            if u32::from(layer) >= atlas.layers {
                return Err(EntityError::LayerOutOfRange {
                    layer,
                    layers: atlas.layers,
                });
            }
            let rect = atlas.uv_rect(u32::from(layer))?;
            let quad = pose.screen_quad(camera);
            let uv = [
                [rect[0], rect[1]],
                [rect[2], rect[1]],
                [rect[2], rect[3]],
                [rect[0], rect[3]],
            ];
            if vertices.len() + 4 > usize::from(u16::MAX) {
                return Err(EntityError::TooManyVertices);
            }
            for (i, pos) in quad.iter().enumerate() {
                vertices.push(GlVertex {
                    pos: *pos,
                    uv: uv[i],
                    color: pose.color,
                    light_uv: [0.5, 0.5],
                    layer,
                });
            }
            let first_index = indices.len() as u32;
            let base = (vertices.len() - 4) as u16;
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            draws.push(EntityDraw {
                pose_index,
                first_index,
                index_count: 6,
                layer,
            });
        }
        Ok(Self {
            vertices,
            indices,
            draws,
        })
    }

    pub fn reference() -> Self {
        let atlas = Atlas::new(4, 16, 4).expect("reference atlas must build");
        Self::build(&EntityPose::reference(), &EntityCamera::reference(), &atlas)
            .expect("reference entities must build")
    }

    pub fn draw_call_count(&self) -> u32 {
        self.draws.len() as u32
    }

    pub fn gl_commands(&self) -> Vec<GlCommand> {
        vec![GlCommand::UseProgram(1), GlCommand::BindTexture(0, 1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_posing_with_zero_yaw_has_explicit_screen_corners() {
        let pose = EntityPose::reference()[0];
        let quad = pose.screen_quad(&EntityCamera::reference());
        assert_eq!(
            quad,
            [
                [0.0, 4.0, 0.0],
                [8.0, 4.0, 0.0],
                [8.0, 12.0, 0.0],
                [0.0, 12.0, 0.0],
            ]
        );
    }

    #[test]
    fn reference_entity_batch_has_two_draws_and_explicit_uvs() {
        let batch = EntityBatch::reference();
        assert_eq!(batch.vertices.len(), 8);
        assert_eq!(batch.indices.len(), 12);
        assert_eq!(batch.draw_call_count(), 2);
        assert_eq!(batch.draws[0].first_index, 0);
        assert_eq!(batch.draws[1].first_index, 6);
        assert_eq!(
            batch.vertices[0].uv,
            [28.5_f32 / 48.0_f32, 4.5_f32 / 48.0_f32]
        );
        assert_eq!(
            batch.vertices[2].uv,
            [43.5_f32 / 48.0_f32, 19.5_f32 / 48.0_f32]
        );
        assert_eq!(batch.vertices[0].color, [200, 100, 50, 255]);
        assert_eq!(batch.vertices[0].light_uv, [0.5, 0.5]);
    }

    #[test]
    fn entity_pass_commands_are_explicit() {
        assert_eq!(
            EntityBatch::reference().gl_commands(),
            vec![GlCommand::UseProgram(1), GlCommand::BindTexture(0, 1)]
        );
    }

    #[test]
    fn out_of_range_entity_layer_is_rejected() {
        let atlas = Atlas::new(4, 16, 4).unwrap();
        let mut poses = EntityPose::reference();
        poses[0].layer = 9;
        assert_eq!(
            EntityBatch::build(&poses, &EntityCamera::reference(), &atlas),
            Err(EntityError::LayerOutOfRange {
                layer: 9,
                layers: 4
            })
        );
    }
}
