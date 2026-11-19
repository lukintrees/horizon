//! The 3D debug gizmos (`GizmoFeatureRenderer.buildLines`): F3+B hitboxes,
//! F3+G chunk borders and the 3D crosshair, as pre-expanded pixel-width
//! strokes with a colour per line. The modern path draws them with
//! `vs_debug_line`; this is the classic backend's own copy of that geometry.
use horizon_legacy_render::Renderer;
use horizon_render_api::{
    Clear, Draw, Fog, Frame, IDENTITY, Pass, RenderBackend, TextureId, stroke,
};

/// The camera-relative lines the game asked for, in its order.
pub(crate) struct Source<'a> {
    /// `[x, y, z, x, y, z, argb, width]` per line, camera relative.
    pub(crate) depth: &'a [[f32; 8]],
    pub(crate) on_top: &'a [[f32; 8]],
    pub(crate) crosshair: &'a [[f32; 8]],
}

/// Ceiling on the meshes one group may spend. `MAX_LINES` per mesh already
/// caps a group far above any debug view the game builds; this only keeps a
/// pathological list from eating the frame's whole mesh ledger.
const MAX_MESHES: usize = 16;

/// The debug gizmos as the modern pipelines' three groups: the opaque depth
/// group (`RenderPipelines.LINES`, which writes depth), the translucent depth
/// group (`linesTranslucentNoDepthWrite`), and everything drawn over the world
/// with no depth test at all. A group past `stroke::MAX_LINES` lines becomes
/// several meshes, drawn in order.
pub(crate) struct Prepared {
    opaque: Vec<stroke::Mesh>,
    translucent: Vec<stroke::Mesh>,
    over: Vec<stroke::Mesh>,
    pub(crate) bytes: usize,
}
impl Prepared {
    /// A debug overlay never fails the frame that carries the world: a line the
    /// stroke expander cannot draw is dropped, the way the GPU would clip it.
    /// Only an allocation failure, which no later frame would survive either,
    /// is an error.
    pub fn new(source: &Source<'_>, matrix: [f32; 16], size: [u32; 2]) -> Result<Self, String> {
        // `DrawableGizmoPrimitives.getGroup`: a line whose ARGB alpha is 255
        // joins the opaque group; the rest keep their order after it, so the
        // two groups drawn in sequence are one stable reordering.
        let (opaque, translucent): (Vec<_>, Vec<_>) = source
            .depth
            .iter()
            .partition(|line| line[6].to_bits() >> 24 == 0xFF);
        let mut over_lines: Vec<[f32; 8]> = Vec::new();
        if over_lines
            .try_reserve(source.on_top.len() + source.crosshair.len())
            .is_err()
        {
            return Err("debug gizmo allocation failed".into());
        }
        over_lines.extend_from_slice(source.on_top);
        over_lines.extend_from_slice(source.crosshair);
        let (opaque, translucent, over) = (
            build(&opaque, matrix, size)?,
            build(&translucent, matrix, size)?,
            build(&over_lines, matrix, size)?,
        );
        let bytes = opaque
            .iter()
            .chain(&translucent)
            .chain(&over)
            .map(stroke::Mesh::bytes)
            .sum();
        Ok(Self {
            opaque,
            translucent,
            over,
            bytes,
        })
    }
    pub fn is_empty(&self) -> bool {
        self.opaque.is_empty() && self.translucent.is_empty() && self.over.is_empty()
    }
    /// How many meshes the draw will create: the frame reserves that many.
    pub fn mesh_count(&self) -> usize {
        self.opaque.len() + self.translucent.len() + self.over.len()
    }
}
fn build(
    lines: &[[f32; 8]],
    matrix: [f32; 16],
    size: [u32; 2],
) -> Result<Vec<stroke::Mesh>, String> {
    let mut usable = Vec::new();
    usable
        .try_reserve(lines.len())
        .map_err(|_| "debug gizmo allocation failed")?;
    usable.extend(segments(lines).filter(|(edge, _, width)| {
        edge.iter().all(|v| v.is_finite()) && width.is_finite() && *width > 0. && *width <= 64.
    }));
    let mut meshes = Vec::new();
    for chunk in usable.chunks(stroke::MAX_LINES) {
        if meshes.len() == MAX_MESHES {
            break;
        }
        let mesh = stroke::Mesh::lines(chunk, [0.; 3], matrix, size).map_err(str::to_owned)?;
        if !mesh.indices.is_empty() {
            meshes.push(mesh);
        }
    }
    Ok(meshes)
}
/// The game's `[x, y, z, x, y, z, argb, width]` lines as stroke segments.
fn segments(lines: &[[f32; 8]]) -> impl Iterator<Item = ([f64; 6], u32, f32)> + '_ {
    lines.iter().map(|line| {
        let ends = [
            f64::from(line[0]),
            f64::from(line[1]),
            f64::from(line[2]),
            f64::from(line[3]),
            f64::from(line[4]),
            f64::from(line[5]),
        ];
        (ends, line[6].to_bits(), line[7])
    })
}
impl super::screen::Resources {
    /// The depth-tested gizmos, after translucent terrain.
    pub fn render_debug_lines(
        &mut self,
        backend: &mut Renderer<'_>,
        prepared: &Prepared,
        white: TextureId,
        fog: Option<Fog>,
    ) -> Result<(), String> {
        self.stroke_all(backend, &prepared.opaque, white, fog, Pass::StrokeDepth)?;
        self.stroke_all(backend, &prepared.translucent, white, fog, Pass::Stroke)
    }
    /// `debug_lines_on_top` and the 3D crosshair: no depth test, so they show
    /// through the world and the hand.
    pub fn render_debug_overlay(
        &mut self,
        backend: &mut Renderer<'_>,
        prepared: &Prepared,
        white: TextureId,
        fog: Option<Fog>,
    ) -> Result<(), String> {
        self.stroke_all(backend, &prepared.over, white, fog, Pass::StrokeOverlay)
    }
    fn stroke_all(
        &mut self,
        backend: &mut Renderer<'_>,
        meshes: &[stroke::Mesh],
        white: TextureId,
        fog: Option<Fog>,
        pass: Pass,
    ) -> Result<(), String> {
        for mesh in meshes {
            self.stroke(backend, mesh, white, fog, pass)?;
        }
        Ok(())
    }
    fn stroke(
        &mut self,
        backend: &mut Renderer<'_>,
        mesh: &stroke::Mesh,
        white: TextureId,
        fog: Option<Fog>,
        pass: Pass,
    ) -> Result<(), String> {
        self.collect(backend)?;
        let id = backend.create_stroke_mesh(&mesh.vertices, &mesh.indices)?;
        self.meshes.push(id);
        backend.render(&Frame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: Clear::Load,
            fog,
            draws: &[Draw {
                mesh: id,
                texture: white,
                lightmap: white,
                origin: [0.; 3],
                pass,
                scissor: None,
            }],
        })?;
        self.collect(backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A horizontal line inside the GL frustum (`-w..w` in every axis).
    fn line(y: f32, color: u32, width: f32) -> [f32; 8] {
        [0., y, -0.5, 1., y, -0.5, f32::from_bits(color), width]
    }
    fn quads(meshes: &[stroke::Mesh]) -> Vec<[f32; 4]> {
        meshes
            .iter()
            .flat_map(|mesh| mesh.vertices.chunks(4))
            .map(|quad| quad[0].color)
            .collect()
    }
    #[test]
    fn the_three_lists_split_into_opaque_translucent_and_on_top_meshes() {
        let depth = [
            line(-0.5, 0x40FF_0000, 1.),
            line(-0.25, 0xFF00_FF00, 4.),
            line(0., 0x8000_0000, 1.),
        ];
        let on_top = [line(0.25, 0xFF00_9000, 4.)];
        let crosshair = [line(0.5, 0xFFFF_0000, 2.), line(0.75, 0xFF00_FF00, 2.)];
        let source = Source {
            depth: &depth,
            on_top: &on_top,
            crosshair: &crosshair,
        };
        let prepared = Prepared::new(&source, IDENTITY, [64; 2]).unwrap();
        assert!(!prepared.is_empty());
        // One opaque line, two translucent ones, three drawn over the world;
        // every line is a 172-byte quad whatever its colour.
        assert_eq!(quads(&prepared.opaque).len(), 1);
        assert_eq!(quads(&prepared.translucent).len(), 2);
        assert_eq!(quads(&prepared.over).len(), 3);
        assert_eq!(prepared.bytes, 172 * 6);
        // The opaque group keeps its own line, in submission order.
        assert_eq!(prepared.opaque[0].vertices[0].color, [0., 1., 0., 1.]);
        // The translucent group keeps the game's order for the lines it holds.
        let translucent = quads(&prepared.translucent);
        assert_eq!(translucent[0][3], 64. / 255.);
        assert_eq!(translucent[1][3], 128. / 255.);
        // On-top lines keep the game's order: the border, then the axes.
        let over = quads(&prepared.over);
        assert_eq!(over[0], [0., 0x90 as f32 / 255., 0., 1.]);
        assert_eq!(over[1], [1., 0., 0., 1.]);
        assert_eq!(over[2], [0., 1., 0., 1.]);
    }
    #[test]
    fn nothing_requested_builds_nothing_and_an_undrawable_line_is_dropped() {
        let source = Source {
            depth: &[],
            on_top: &[],
            crosshair: &[],
        };
        let prepared = Prepared::new(&source, IDENTITY, [64; 2]).unwrap();
        assert!(prepared.is_empty() && prepared.bytes == 0);
        let good = line(0., 0xFFFF_FFFF, 1.);
        let bad = [
            line(0., 0xFFFF_FFFF, f32::NAN),
            line(0., 0xFFFF_FFFF, 0.),
            line(0., 0xFFFF_FFFF, 65.),
            [
                f32::NAN,
                0.,
                -0.5,
                1.,
                0.,
                -0.5,
                f32::from_bits(0xFFFF_FFFF),
                1.,
            ],
        ];
        // A debug overlay never fails the frame: the lines that cannot be
        // expanded are skipped and the rest still draw.
        for list in [&bad[..1], &bad[1..2], &bad[2..3], &bad[3..]] {
            let lines: Vec<_> = std::iter::once(good).chain(list.iter().copied()).collect();
            for source in [
                Source {
                    depth: &lines,
                    on_top: &[],
                    crosshair: &[],
                },
                Source {
                    depth: &[],
                    on_top: &lines,
                    crosshair: &[],
                },
                Source {
                    depth: &[],
                    on_top: &[],
                    crosshair: &lines,
                },
            ] {
                let prepared = Prepared::new(&source, IDENTITY, [64; 2]).unwrap();
                assert_eq!(prepared.bytes, 172);
                assert_eq!(
                    quads(&prepared.opaque).len()
                        + quads(&prepared.translucent).len()
                        + quads(&prepared.over).len(),
                    1
                );
            }
        }
    }
}
