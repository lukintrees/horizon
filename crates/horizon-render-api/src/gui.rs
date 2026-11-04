//! GUI triangle-list conversion and transient uploads. No font/resource decoder.
use crate::{Clear, Draw, Frame, IDENTITY, Image, MeshId, Pass, RenderBackend, TextureId, Vertex};
use std::ops::Range;

#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: u32,
    /// wgpu's 0..1 GUI model depth, not terrain's material alpha.
    pub depth: f32,
}
#[derive(Clone, Copy, Debug)]
pub enum Blend {
    Alpha,
    Invert,
    Additive,
    Vignette,
    /// Top-left physical-pixel rectangle, already including GUI scale.
    Depth([u32; 4]),
}
#[derive(Debug, PartialEq, Eq)]
pub enum GuiError {
    InvalidTarget,
    InvalidRange,
    InvalidVertex,
    UnsupportedGlint,
    VertexBudget,
    InvalidAtlas,
}
pub struct Run {
    pub pass: Pass,
    pub scissor: Option<[u32; 4]>,
    pub clear_depth: bool,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
}

/// Preserve every blend boundary and triangle order, splitting before u16 wraps.
/// UV scale accounts for a non-POT atlas padded to a POT texture. The source atlas
/// must be edge-extended, not zero-padded, so its old clamp behaviour is preserved.
pub fn runs(
    vertices: &[Input],
    batches: &[(Range<usize>, Blend)],
    window: [u32; 2],
    scale: f32,
    uv_scale: [f32; 2],
    max_vertices: usize,
) -> Result<Vec<Run>, GuiError> {
    if window.contains(&0)
        || !scale.is_finite()
        || scale <= 0.
        || uv_scale
            .iter()
            .any(|v| !v.is_finite() || *v <= 0. || *v > 1.)
    {
        return Err(GuiError::InvalidTarget);
    }
    let mut output = Vec::new();
    let mut total = 0usize;
    for (range, blend) in batches {
        if range.start > range.end
            || range.end > vertices.len()
            || range.start % 3 != 0
            || range.len() % 3 != 0
        {
            return Err(GuiError::InvalidRange);
        }
        let (pass, scissor, depth) = match *blend {
            Blend::Alpha => (Pass::Overlay, None, false),
            Blend::Invert => (Pass::OverlayInvert, None, false),
            Blend::Additive => (Pass::OverlayAdditive, None, false),
            Blend::Vignette => (Pass::OverlayVignette, None, false),
            Blend::Depth([x, y, w, h]) => {
                let x = x.min(window[0]);
                let y = y.min(window[1]);
                let w = w.min(window[0] - x);
                let h = h.min(window[1] - y);
                if w == 0 || h == 0 {
                    continue;
                }
                (Pass::OverlayDepth, Some([x, window[1] - y - h, w, h]), true)
            }
        };
        total = total
            .checked_add(range.len())
            .ok_or(GuiError::VertexBudget)?;
        if total > max_vertices {
            return Err(GuiError::VertexBudget);
        }
        for (part, source) in vertices[range.clone()].chunks(65535).enumerate() {
            let mut converted = Vec::with_capacity(source.len());
            for v in source {
                if v.pos
                    .iter()
                    .chain(&v.uv)
                    .chain([&v.depth])
                    .any(|n| !n.is_finite())
                    || !(0.0..=1.0).contains(&v.depth)
                {
                    return Err(GuiError::InvalidVertex);
                }
                // This is the actual game's glint marker, not an atlas UV.
                if v.uv[0] >= 2. {
                    return Err(GuiError::UnsupportedGlint);
                }
                let [r, g, b, a] = v.color.to_le_bytes();
                let vertex = Vertex {
                    pos: [
                        v.pos[0] * scale / window[0] as f32 * 2. - 1.,
                        1. - v.pos[1] * scale / window[1] as f32 * 2.,
                        2. * v.depth - 1.,
                    ],
                    uv: std::array::from_fn(|i| v.uv[i].clamp(0., 1.) * uv_scale[i]),
                    color: [r, g, b, a].map(|c| f32::from(c) / 255.),
                    light_uv: [0.5; 2],
                };
                if vertex.pos.iter().any(|v| !v.is_finite()) {
                    return Err(GuiError::InvalidVertex);
                }
                converted.push(vertex);
            }
            output.push(Run {
                pass,
                scissor,
                clear_depth: depth && part == 0,
                indices: (0..source.len() as u16).collect(),
                vertices: converted,
            });
        }
    }
    Ok(output)
}

/// Edge-extend an RGBA8 atlas to POT dimensions. Bound the allocation before
/// allocating; oversized atlases need paging, not silently downscaled resources.
pub fn pad_atlas(
    size: [u32; 2],
    rgba: &[u8],
    max_bytes: usize,
) -> Result<([u32; 2], Vec<u8>), GuiError> {
    let bytes = (size[0] as usize)
        .checked_mul(size[1] as usize)
        .and_then(|v| v.checked_mul(4));
    if size.contains(&0) || bytes != Some(rgba.len()) {
        return Err(GuiError::InvalidAtlas);
    }
    let pot = [
        size[0].checked_next_power_of_two(),
        size[1].checked_next_power_of_two(),
    ];
    let pot = [
        pot[0].ok_or(GuiError::InvalidAtlas)?,
        pot[1].ok_or(GuiError::InvalidAtlas)?,
    ];
    let bytes = (pot[0] as usize)
        .checked_mul(pot[1] as usize)
        .and_then(|v| v.checked_mul(4))
        .filter(|&v| v <= max_bytes)
        .ok_or(GuiError::InvalidAtlas)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(bytes)
        .map_err(|_| GuiError::InvalidAtlas)?;
    result.resize(bytes, 0);
    for y in 0..pot[1] {
        for x in 0..pot[0] {
            let source =
                (y.min(size[1] - 1) as usize * size[0] as usize + x.min(size[0] - 1) as usize) * 4;
            let dest = (y as usize * pot[0] as usize + x as usize) * 4;
            result[dest..dest + 4].copy_from_slice(&rgba[source..source + 4]);
        }
    }
    Ok((pot, result))
}

#[derive(Debug)]
pub enum RenderError<E> {
    InvalidPass,
    Backend(E),
}
struct Uploads<'a, B: RenderBackend> {
    backend: &'a mut B,
    meshes: Vec<MeshId>,
}
impl<B: RenderBackend> Drop for Uploads<'_, B> {
    fn drop(&mut self) {
        for id in self.meshes.drain(..) {
            let _ = self.backend.remove_mesh(id);
        }
    }
}
/// Upload all geometry before drawing, then composite without clearing world
/// colour. Separate depth previews clear depth once per original run, even when
/// a run is split across multiple u16 meshes. Texture lifetime stays with caller.
/// `white` must be a white lightmap; GUI colours must not inherit world lighting.
pub fn render<B: RenderBackend>(
    backend: &mut B,
    runs: &[Run],
    atlas: TextureId,
    white: TextureId,
) -> Result<(), RenderError<B::Error>> {
    if runs
        .iter()
        .any(|r| !r.pass.is_overlay() || (r.clear_depth && r.pass != Pass::OverlayDepth))
    {
        return Err(RenderError::InvalidPass);
    }
    let mut uploads = Uploads {
        backend,
        meshes: Vec::new(),
    };
    for run in runs {
        let mesh = uploads
            .backend
            .create_mesh(&run.vertices, &run.indices)
            .map_err(RenderError::Backend)?;
        uploads.meshes.push(mesh);
    }
    for (run, &mesh) in runs.iter().zip(&uploads.meshes) {
        uploads
            .backend
            .render(&Frame {
                view_projection: IDENTITY,
                camera: [0.; 3],
                fog: None,
                clear: if run.clear_depth {
                    Clear::Depth
                } else {
                    Clear::Load
                },
                draws: &[Draw {
                    mesh,
                    texture: atlas,
                    lightmap: white,
                    origin: [0.; 3],
                    pass: run.pass,
                    scissor: run.scissor,
                }],
            })
            .map_err(RenderError::Backend)?;
    }
    Ok(())
}

/// Upload a padded GUI atlas, returning the multiplier for original UVs.
pub fn upload_atlas<B: RenderBackend>(
    backend: &mut B,
    size: [u32; 2],
    rgba: &[u8],
    max_bytes: usize,
) -> Result<(TextureId, [f32; 2]), UploadError<B::Error>> {
    let (pot, pixels) = pad_atlas(size, rgba, max_bytes).map_err(UploadError::Atlas)?;
    let id = backend
        .create_texture(Image {
            width: pot[0],
            height: pot[1],
            rgba: &pixels,
        })
        .map_err(UploadError::Backend)?;
    Ok((id, std::array::from_fn(|i| size[i] as f32 / pot[i] as f32)))
}
#[derive(Debug)]
pub enum UploadError<E> {
    Atlas(GuiError),
    Backend(E),
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vertex() -> Input {
        Input {
            pos: [10., 20.],
            uv: [0.5, 1.],
            color: 0x8040_2010,
            depth: 0.25,
        }
    }
    #[test]
    fn real_gui_units_alpha_depth_and_top_left_scissor_are_preserved() {
        let runs = runs(
            &[vertex(); 6],
            &[(0..6, Blend::Depth([4, 6, 80, 200]))],
            [100, 80],
            2.,
            [0.75, 1.],
            6,
        )
        .unwrap();
        assert_eq!(runs[0].vertices[0].pos, [-0.6, 0., -0.5]);
        assert_eq!(
            runs[0].vertices[0].color,
            [16., 32., 64., 128.].map(|v| v / 255.)
        );
        assert_eq!(runs[0].vertices[0].uv, [0.375, 1.]);
        assert_eq!(runs[0].scissor, Some([4, 0, 80, 74]));
        assert!(runs[0].clear_depth);
        assert_eq!(runs[0].indices, [0, 1, 2, 3, 4, 5]);
    }
    #[test]
    fn split_preserves_depth_between_chunks_but_resets_between_models() {
        let data = vec![vertex(); 65538];
        let runs = runs(
            &data,
            &[
                (0..65538, Blend::Depth([0, 0, 100, 80])),
                (0..3, Blend::Depth([0, 0, 100, 80])),
            ],
            [100, 80],
            1.,
            [1.; 2],
            65541,
        )
        .unwrap();
        assert_eq!(
            runs.iter().map(|r| r.vertices.len()).collect::<Vec<_>>(),
            [65535, 3, 3]
        );
        assert_eq!(
            runs.iter().map(|r| r.clear_depth).collect::<Vec<_>>(),
            [true, false, true]
        );
        assert_eq!(runs[0].indices.last(), Some(&65534));
    }
    #[test]
    fn invalid_geometry_ranges_glint_and_budgets_are_explicit_errors() {
        let v = [vertex(); 6];
        for range in [0..7, 1..4, 0..2] {
            assert!(matches!(
                runs(&v, &[(range, Blend::Alpha)], [100; 2], 1., [1.; 2], 6),
                Err(GuiError::InvalidRange)
            ));
        }
        assert!(matches!(
            runs(&v, &[(0..6, Blend::Alpha)], [100; 2], 1., [1.; 2], 5),
            Err(GuiError::VertexBudget)
        ));
        let mut bad = v;
        bad[0].uv[0] = 2.;
        assert!(matches!(
            runs(&bad, &[(0..6, Blend::Alpha)], [100; 2], 1., [1.; 2], 6),
            Err(GuiError::UnsupportedGlint)
        ));
        bad[0].uv[0] = f32::NAN;
        assert!(matches!(
            runs(&bad, &[(0..6, Blend::Alpha)], [100; 2], 1., [1.; 2], 6),
            Err(GuiError::InvalidVertex)
        ));
    }
    #[test]
    fn padding_replicates_edge_and_checks_budget_before_allocating() {
        let pixels = [1, 2, 3, 4].repeat(3);
        let (size, result) = pad_atlas([3, 1], &pixels, 16).unwrap();
        assert_eq!(size, [4, 1]);
        assert_eq!(result, [1, 2, 3, 4].repeat(4));
        assert_eq!(pad_atlas([3, 1], &pixels, 15), Err(GuiError::InvalidAtlas));
    }
}
