//! The GPU-posing payload: what the game hands the renderer so it can pose
//! entity models on the GPU. The vertices, parts and jobs here are plain data;
//! the compute pipeline, buffers and bind groups that consume them stay in
//! `gfx::entity_skin`. `gfx::entity_skin` re-exports these types.
use super::pack::EntitySpan;
use crate::mesher::Vertex;

/// A model vertex: position / 16, model UV, the face's normal as an axis (0..2
/// = +x, +y, +z; 3..5 = -x, -y, -z), the part's index in its model.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub axis: u32,
    pub part: u32,
}

/// A drawn part: its geometry matrix and its normal matrix, by columns, the
/// normal matrix's columns normalized when the part's scale is not uniform.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinPart {
    pub matrix: [f32; 16],
    pub normal: [f32; 9],
}

impl SkinPart {
    /// An item layer's part: its pose, and its quads' colours by axis.
    pub fn item(matrix: glam::Mat4, colors: [u32; 6]) -> Self {
        let mut normal = [0.0; 9];
        for (word, color) in normal.iter_mut().zip(colors) {
            *word = f32::from_bits(color);
        }
        Self {
            matrix: matrix.to_cols_array(),
            normal,
        }
    }
}

/// One run of a model's vertices drawn with one texture rect, colour, light
/// and overlay. `dst` counts from the frame's first posed vertex.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinJob {
    pub first: u32,
    pub count: u32,
    pub part: u32,
    pub dst: u32,
    pub rect: [f32; 4],
    pub scroll: f32,
    pub color: u32,
    pub light: u32,
    pub overlay: u32,
    /// 1 when the overlay is the armour glint: the model UV rides in `glint_uv`.
    pub glint: u32,
    /// How the vertices are finished: [`SkinJob::MODEL`], [`SkinJob::ITEM`] or
    /// [`SkinJob::FLAT`].
    pub mode: u32,
    /// 1 for a model whose render type culls back faces (`mobs::Polygon::cull_back`):
    /// the normal's w is `mesher::SHADED_CULL_W` instead of -127.
    pub cull: u32,
    /// Always 0: the shader reads it as a zero it cannot fold away.
    pub spare: u32,
}

impl SkinJob {
    /// A model's polygons: the face normal posed, the job's colour.
    pub const MODEL: u32 = 0;
    /// An item model's quads (`item_render::WorldSink`): the colour by the face's
    /// axis rides in the part's first 6 normal words ([`SkinPart::item`]), shaded on
    /// the CPU, and the normal is `mesher::CULL_BACK`.
    pub const ITEM: u32 = 1;
    /// A model's polygons with no normal at all (all four bytes 0): the eyes render
    /// type's layers, which entity.wgsl leaves unshaded.
    pub const FLAT: u32 = 2;
}

const _: () = assert!(std::mem::size_of::<Vertex>() == 44);
const _: () = assert!(std::mem::size_of::<SkinVertex>() == 7 * 4);
const _: () = assert!(std::mem::size_of::<SkinPart>() == 25 * 4);
const _: () = assert!(std::mem::size_of::<SkinJob>() == 16 * 4);

/// What the game hands the renderer for one frame.
#[derive(Default)]
pub struct SkinFrame {
    /// The model store restarted (models are registered again from 0).
    pub reset: bool,
    /// The store's size before `new_model`: the renderer takes the frame only
    /// when its own store is that large (it missed none of the earlier ones).
    pub base: u32,
    /// Model vertices registered since the last frame, at the store's end.
    pub new_model: Vec<SkinVertex>,
    pub parts: Vec<SkinPart>,
    pub jobs: Vec<SkinJob>,
    /// Shader identities in the GPU output vertex space; empty without a pack.
    pub spans: Vec<EntitySpan>,
    /// Vertices the jobs write: a whole number of quads.
    pub vertices: u32,
    /// Glowing entities' silhouettes posed here (`OutlineBufferSource`'s copy of
    /// their draws, [`SkinFrame::outline_since`]): `dst` counts from the first.
    pub outline_jobs: Vec<SkinJob>,
    /// Vertices the outline jobs write.
    pub outline_vertices: u32,
}

impl SkinFrame {
    /// Starts a frame's draws; models not yet handed to the renderer stay.
    pub fn clear_draws(&mut self) {
        self.parts.clear();
        self.jobs.clear();
        self.spans.clear();
        self.vertices = 0;
        self.outline_jobs.clear();
        self.outline_vertices = 0;
    }

    /// Copies the draws since job `first` into the glowing entities' outline
    /// (`Frame::glowing`'s, on the GPU), as `glowing.rs`'s `mark_outline` copies
    /// vertices: the same ones, in `color` (`Vertex::color`) with no normal.
    pub fn outline_since(&mut self, first: usize, color: u32) {
        for i in first..self.jobs.len() {
            let job = self.jobs[i];
            self.outline_jobs.push(SkinJob {
                dst: self.outline_vertices,
                color,
                mode: SkinJob::FLAT,
                ..job
            });
            self.outline_vertices += job.count;
        }
    }

    /// The renderer took the new models.
    pub fn handed(&mut self) {
        self.base += self.new_model.len() as u32;
        self.new_model.clear();
        self.reset = false;
    }

    /// Where the draws stand, to take back a mob's (`truncate`).
    pub fn mark(&self) -> (usize, usize, u32) {
        (self.jobs.len(), self.parts.len(), self.vertices)
    }

    /// Takes back the draws since `mark` (a mob culled after drawing).
    pub fn truncate(&mut self, (jobs, parts, vertices): (usize, usize, u32)) {
        self.jobs.truncate(jobs);
        self.parts.truncate(parts);
        self.vertices = vertices;
        self.spans.retain(|s| s.start <= vertices);
    }

    /// Whether the renderer has anything to take.
    pub fn pending(&self) -> bool {
        self.reset
            || !self.new_model.is_empty()
            || !self.jobs.is_empty()
            || !self.outline_jobs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_reset_retains_models_until_the_renderer_accepts_them() {
        let mut frame = SkinFrame {
            reset: true,
            base: 12,
            new_model: vec![SkinVertex {
                pos: [1., 2., 3.],
                uv: [0.25, 0.5],
                axis: 2,
                part: 0,
            }],
            parts: vec![SkinPart::item(glam::Mat4::IDENTITY, [0; 6])],
            jobs: vec![SkinJob {
                count: 4,
                ..Default::default()
            }],
            vertices: 4,
            outline_jobs: vec![SkinJob::default()],
            outline_vertices: 4,
            ..Default::default()
        };
        frame.clear_draws();
        assert!(frame.pending());
        assert!(frame.reset);
        assert_eq!(frame.base, 12);
        assert_eq!(frame.new_model.len(), 1);
        assert_eq!(frame.new_model[0].pos, [1., 2., 3.]);
        assert!(frame.parts.is_empty() && frame.jobs.is_empty() && frame.spans.is_empty());
        assert!(frame.outline_jobs.is_empty());
        assert_eq!((frame.vertices, frame.outline_vertices), (0, 0));

        frame.handed();
        assert_eq!(frame.base, 13);
        assert!(frame.new_model.is_empty());
        assert!(!frame.reset);
        assert!(!frame.pending());
    }

    #[test]
    fn culling_rolls_back_only_the_later_entity_draws() {
        let mut frame = SkinFrame {
            jobs: vec![SkinJob {
                first: 12,
                count: 4,
                ..Default::default()
            }],
            parts: vec![SkinPart::item(glam::Mat4::IDENTITY, [0; 6])],
            vertices: 4,
            spans: vec![EntitySpan {
                start: 0,
                id: 7,
                ..Default::default()
            }],
            ..Default::default()
        };
        let mark = frame.mark();
        frame.jobs.push(SkinJob {
            first: 24,
            count: 8,
            dst: 4,
            ..Default::default()
        });
        frame
            .parts
            .push(SkinPart::item(glam::Mat4::IDENTITY, [0; 6]));
        frame.vertices = 12;
        frame.spans.extend([
            EntitySpan {
                start: 4,
                id: 8,
                ..Default::default()
            },
            EntitySpan {
                start: 8,
                id: 9,
                ..Default::default()
            },
        ]);

        frame.truncate(mark);
        assert_eq!(frame.jobs.len(), 1);
        assert_eq!((frame.jobs[0].first, frame.jobs[0].count), (12, 4));
        assert_eq!(frame.parts.len(), 1);
        assert_eq!(frame.vertices, 4);
        assert_eq!(
            frame
                .spans
                .iter()
                .map(|s| (s.start, s.id))
                .collect::<Vec<_>>(),
            [(0, 7), (4, 8)]
        );
    }

    #[test]
    fn outline_jobs_use_their_own_offsets_without_changing_the_draws() {
        let mut frame = SkinFrame {
            jobs: vec![
                SkinJob {
                    count: 4,
                    color: 1,
                    ..Default::default()
                },
                SkinJob {
                    first: 20,
                    count: 8,
                    dst: 4,
                    part: 3,
                    color: 2,
                    ..Default::default()
                },
                SkinJob {
                    first: 40,
                    count: 4,
                    dst: 12,
                    part: 5,
                    color: 3,
                    ..Default::default()
                },
            ],
            outline_jobs: vec![SkinJob {
                count: 4,
                ..Default::default()
            }],
            outline_vertices: 4,
            ..Default::default()
        };
        frame.outline_since(1, 0xff123456);
        assert_eq!(frame.outline_vertices, 16);
        let copied = &frame.outline_jobs[1..];
        assert_eq!(
            copied
                .iter()
                .map(|j| (j.first, j.count, j.dst, j.part))
                .collect::<Vec<_>>(),
            [(20, 8, 4, 3), (40, 4, 12, 5)]
        );
        assert!(
            copied
                .iter()
                .all(|j| j.mode == SkinJob::FLAT && j.color == 0xff123456)
        );
        assert_eq!(
            frame
                .jobs
                .iter()
                .map(|j| (j.mode, j.color))
                .collect::<Vec<_>>(),
            [
                (SkinJob::MODEL, 1),
                (SkinJob::MODEL, 2),
                (SkinJob::MODEL, 3)
            ]
        );
    }
}
