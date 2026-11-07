//! Terrain upload conversion. Consumes real mesher attributes, not GPU objects.
use crate::{Pass, Vertex, atlas::AtlasLayout, light_uv};

#[derive(Clone, Copy, Debug)]
pub struct TerrainInput {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub layer: u16,
    pub color: u32,
    pub light: u16,
}
#[derive(Debug, PartialEq, Eq)]
pub enum TerrainError {
    IncompleteQuad,
    MissingSprite(u16),
    MixedLayers,
    RepeatingUv,
    NonFinite,
    VertexBudget,
}
#[derive(Debug)]
pub struct Run {
    pub page: usize,
    pub pass: Pass,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
}

/// Preserve page/pass changes in input order; never regroup translucent quads by
/// texture. Mesher material/HDR alpha bits are NOT coverage. OPAQUE_LAYER selects
/// the opaque rather than cutout pass; the low 15 bits name a sprite.
pub fn runs(
    layout: &AtlasLayout,
    vertices: impl IntoIterator<Item = TerrainInput>,
    translucent: bool,
    max_vertices: usize,
) -> Result<Vec<Run>, TerrainError> {
    let mut iterator = vertices.into_iter();
    let mut output: Vec<Run> = Vec::new();
    let mut total = 0usize;
    while let Some(first) = iterator.next() {
        let quad = [
            first,
            iterator.next().ok_or(TerrainError::IncompleteQuad)?,
            iterator.next().ok_or(TerrainError::IncompleteQuad)?,
            iterator.next().ok_or(TerrainError::IncompleteQuad)?,
        ];
        total = total.checked_add(4).ok_or(TerrainError::VertexBudget)?;
        if total > max_vertices {
            return Err(TerrainError::VertexBudget);
        }
        let layer = first.layer & 0x7fff;
        let place = layout
            .sprites()
            .get(layer as usize)
            .ok_or(TerrainError::MissingSprite(layer))?;
        if quad.iter().any(|v| v.layer != first.layer) {
            return Err(TerrainError::MixedLayers);
        }
        if quad
            .iter()
            .any(|v| v.pos.iter().chain(&v.uv).any(|v| !v.is_finite()))
        {
            return Err(TerrainError::NonFinite);
        }
        if quad
            .iter()
            .any(|v| v.uv.iter().any(|v| !(0.0..=1.0).contains(v)))
        {
            return Err(TerrainError::RepeatingUv);
        }
        let pass = if translucent {
            Pass::Translucent
        } else if first.layer & 0x8000 != 0 {
            Pass::Opaque
        } else {
            Pass::Cutout
        };
        let side = layout.page_size() as f32;
        let converted = quad.map(|v| {
            let [r, g, b, _material] = v.color.to_le_bytes();
            Vertex {
                pos: v.pos,
                uv: [
                    (place.x as f32 + v.uv[0] * place.width as f32) / side,
                    (place.y as f32 + v.uv[1] * place.height as f32) / side,
                ],
                light_uv: light_uv(v.light),
                color: [
                    f32::from(r) / 255.,
                    f32::from(g) / 255.,
                    f32::from(b) / 255.,
                    1.,
                ],
            }
        });
        if output
            .last()
            .is_none_or(|r| r.page != place.page || r.pass != pass || r.vertices.len() > 65532)
        {
            output.push(Run {
                page: place.page,
                pass,
                vertices: Vec::new(),
                indices: Vec::new(),
            });
        }
        let run = output.last_mut().unwrap();
        let base = run.vertices.len() as u16;
        run.vertices.extend(converted);
        run.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::{SpriteSize, build_page, pack_atlas};
    fn input(layer: u16) -> TerrainInput {
        TerrainInput {
            pos: [1., 2., 3.],
            uv: [0.5, 1.],
            layer,
            color: 0x8d403020,
            light: 0xf080,
        }
    }
    fn layout() -> AtlasLayout {
        pack_atlas(
            &[SpriteSize {
                width: 2,
                height: 2,
            }; 2],
            4,
            1,
            2,
        )
        .unwrap()
    }
    #[test]
    fn keeps_order_alpha_flags_light_and_atlas_coordinates() {
        let input = [input(0x8000), input(1), input(0x8000)]
            .into_iter()
            .flat_map(|v| [v; 4]);
        let runs = runs(&layout(), input, false, 12).unwrap();
        assert_eq!(
            runs.iter().map(|r| (r.page, r.pass)).collect::<Vec<_>>(),
            vec![(0, Pass::Opaque), (1, Pass::Cutout), (0, Pass::Opaque)]
        );
        assert_eq!(runs[0].vertices[0].uv, [0.5, 0.75]);
        assert_eq!(
            runs[0].vertices[0].color,
            [32. / 255., 48. / 255., 64. / 255., 1.]
        );
        assert_eq!(runs[0].vertices[0].light_uv, [31. / 32., 17. / 32.]);
    }
    #[test]
    fn bounds_indices_before_u16_wrap_and_rejects_bad_input() {
        let all = std::iter::repeat_n(input(0), 65540);
        let runs = runs(&layout(), all, true, 65540).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].indices.last(), Some(&65535));
        assert_eq!(runs[1].indices, vec![0, 1, 2, 0, 2, 3]);
        assert!(matches!(
            super::runs(&layout(), [input(0); 4], false, 3),
            Err(TerrainError::VertexBudget)
        ));
        assert!(matches!(
            super::runs(&layout(), [input(0); 3], false, 4),
            Err(TerrainError::IncompleteQuad)
        ));
        let mut v = input(0);
        v.uv[0] = 1.5;
        assert!(matches!(
            super::runs(&layout(), [v; 4], false, 4),
            Err(TerrainError::RepeatingUv)
        ));
    }
    #[test]
    fn padding_is_extruded_without_cross_page_bleed() {
        let layout = layout();
        let red = [255, 0, 0, 255].repeat(4);
        let green = [0, 255, 0, 255].repeat(4);
        let first = build_page(&layout, 0, &[&red, &green]).unwrap();
        let second = build_page(&layout, 1, &[&red, &green]).unwrap();
        assert_eq!(first, [255, 0, 0, 255].repeat(16));
        assert_eq!(second, [0, 255, 0, 255].repeat(16));
    }
}
