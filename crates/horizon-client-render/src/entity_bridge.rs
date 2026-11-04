//! Actual camera-relative entity/item/particle vertices -> classic SDR GL2.
use crate::mesher::Vertex;
use horizon_render_api::{
    entity::{self, Error, LitVertex},
    gui,
};
pub fn runs(
    vertices: &[Vertex],
    lightmap: &[u8],
    nether: bool,
    max_vertices: usize,
) -> Result<Vec<gui::Run<LitVertex>>, Error> {
    entity::runs(vertices.iter().map(input), lightmap, nether, max_vertices)
}
fn input(v: &Vertex) -> entity::Input {
    entity::Input {
        pos: v.pos,
        uv: v.uv,
        layer: v.layer,
        color: v.color,
        light: v.light,
        normal: v.normal,
    }
}
#[derive(Clone, Copy)]
pub enum View {
    World,
    SeeThrough,
}
#[derive(Debug)]
pub enum PrepareError {
    Entity(Error),
    Atlas(horizon_render_api::paged_gui::Error),
}
/// Keep the temporary unpaged stream inside this scope, releasing it as soon as
/// clipping completes rather than retaining shadowed buffers for the whole frame.
pub fn prepare(
    vertices: &[Vertex],
    lightmap: &[u8],
    nether: bool,
    layout: horizon_render_api::paged_gui::Layout,
    max_vertices: usize,
    view: View,
) -> Result<Vec<horizon_render_api::paged_gui::Run<LitVertex>>, PrepareError> {
    let source = match view {
        View::World => runs(vertices, lightmap, nether, max_vertices),
        View::SeeThrough => {
            entity::see_through_runs(vertices.iter().map(input), lightmap, nether, max_vertices)
        }
    }
    .map_err(PrepareError::Entity)?;
    horizon_render_api::paged_gui::split_entities(layout, &source, max_vertices)
        .map_err(PrepareError::Atlas)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_entity_vertex_retains_coverage_culling_and_particle_material() {
        let vertex = Vertex {
            pos: [1., 2., 3.],
            uv: [0.5; 2],
            layer: 15,
            color: 0x80402010,
            light: 0x8040,
            normal: crate::mesher::CULL_BACK,
            glint_uv: [0.; 2],
        };
        let runs = runs(&[vertex; 4], &[255; 1024], false, 4).unwrap();
        assert_eq!(
            runs[0].vertices[0].base.color,
            [16. / 255., 32. / 255., 64. / 255., 128. / 255.]
        );
        assert_eq!(runs[0].vertices[0].base.light_uv, [17. / 32., 9. / 32.]);
        assert!(
            matches!(runs[0].pass, horizon_render_api::Pass::Entity(m) if m.cull && m.kind == entity::Kind::Particle)
        );
    }
    #[test]
    fn real_boat_mask_does_not_require_a_sampleable_atlas_uv() {
        let vertex = Vertex {
            pos: [0.; 3],
            uv: [f32::NAN; 2],
            layer: 13,
            color: 0,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.; 2],
        };
        let mask = runs(&[vertex; 4], &[255; 1024], false, 4).unwrap();
        assert_eq!(mask[0].vertices[0].base.uv, [0.; 2]);
        assert!(
            matches!(mask[0].pass,horizon_render_api::Pass::Entity(m) if m.kind==entity::Kind::WaterMask)
        );
    }
    #[test]
    fn real_quad_preparation_bounds_triangle_expansion_and_explicit_text_view() {
        let layout = horizon_render_api::paged_gui::Layout::new([3, 1], 4, 2).unwrap();
        let vertices =
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|pos| Vertex {
                pos,
                uv: [0.5; 2],
                layer: 0,
                color: u32::MAX,
                light: 0xf0f0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            });
        assert!(matches!(
            prepare(&vertices, &[255; 1024], false, layout, 5, View::World),
            Err(PrepareError::Atlas(
                horizon_render_api::paged_gui::Error::VertexBudget
            ))
        ));
        let text = prepare(&vertices, &[255; 1024], false, layout, 6, View::SeeThrough).unwrap();
        assert_eq!(
            text.iter()
                .map(|r| r.geometry.vertices.len())
                .sum::<usize>(),
            6
        );
        assert!(
            matches!(text[0].geometry.pass,horizon_render_api::Pass::Entity(m) if m.kind==entity::Kind::SeeThrough && m.cull)
        );
    }
}
