//! Actual camera overlays -> bounded standalone sprite draws. Never repeat the
//! whole atlas or interpolate wrapped UVs on the CPU (both lose sprite seams).
use crate::{
    paged_hud::Backing,
    screen_effects::{self as source, ScreenQuad},
};
use horizon_render_api::{
    Clear, Draw, Frame, IDENTITY, Image, MeshId, Pass, RenderBackend, TextureId, Vertex,
    screen::{Blend, Material},
};
use std::collections::HashMap;

pub(super) struct Sprite {
    pub(super) size: [u32; 2],
    pub(super) pixels: Vec<u8>,
}
struct Run {
    sprite: usize,
    vertices: [Vertex; 4],
    material: Material,
}
pub(super) struct Prepared {
    sprites: Vec<Sprite>,
    runs: Vec<Run>,
}
impl Prepared {
    pub fn new(
        quads: &[ScreenQuad],
        backing: &Backing,
        quad_limit: usize,
        byte_limit: usize,
        max_texture: u32,
    ) -> Result<Self, String> {
        if quads.len() > quad_limit {
            return Err("screen quad budget exceeded".into());
        }
        let mut runs = Vec::new();
        runs.try_reserve_exact(quads.len())
            .map_err(|_| "screen allocation failed")?;
        let mut sprites = Vec::new();
        sprites
            .try_reserve_exact(quads.len())
            .map_err(|_| "screen allocation failed")?;
        let mut indices = HashMap::new();
        indices
            .try_reserve(quads.len())
            .map_err(|_| "screen allocation failed")?;
        // One decoded page may coexist with ALL retained extracted sprites.
        let mut bytes = if quads.is_empty() {
            0
        } else {
            backing.layout().page_bytes()
        };
        for quad in quads {
            if quad.sampling & !(source::SAMPLE_BLUR | source::SAMPLE_CLAMP) != 0 {
                return Err("invalid screen sampling flags".into());
            }
            let rect = quad.vertices[0].rect;
            if quad.vertices.iter().any(|v| {
                v.rect != rect
                    || v.pos.iter().chain(&v.uv).any(|f| !f.is_finite())
                    || v.color
                        .iter()
                        .any(|f| !f.is_finite() || !(0.0..=1.0).contains(f))
            }) {
                return Err("invalid screen vertices/rectangle/color".into());
            }
            let rect = pixel_rect(rect, backing.layout().size())?;
            if rect[2] > max_texture || rect[3] > max_texture {
                return Err("screen sprite exceeds native texture limit".into());
            }
            let sprite = if let Some(&index) = indices.get(&rect) {
                index
            } else {
                let image_bytes = (rect[2] as usize)
                    .checked_mul(rect[3] as usize)
                    .and_then(|n| n.checked_mul(4))
                    .ok_or("screen size overflow")?;
                bytes = bytes
                    .checked_add(image_bytes)
                    .filter(|&n| n <= byte_limit)
                    .ok_or("screen decoded-byte budget exceeded")?;
                let pixels = backing.extract(rect, image_bytes)?;
                let index = sprites.len();
                sprites.push(Sprite {
                    size: [rect[2], rect[3]],
                    pixels,
                });
                indices.insert(rect, index);
                index
            };
            runs.push(Run {
                sprite,
                vertices: quad.vertices.map(|v| Vertex {
                    pos: [v.pos[0], v.pos[1], 0.],
                    uv: v.uv,
                    color: v.color,
                    light_uv: [0.; 2],
                }),
                material: Material {
                    blend: match quad.blend {
                        source::Blend::Alpha => Blend::Alpha,
                        source::Blend::Vignette => Blend::Vignette,
                    },
                    linear: quad.sampling & source::SAMPLE_BLUR != 0,
                    clamp: quad.sampling & source::SAMPLE_CLAMP != 0,
                },
            });
        }
        Ok(Self { sprites, runs })
    }
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
    pub fn largest_texture(&self) -> usize {
        self.sprites
            .iter()
            .map(|s| s.pixels.len())
            .max()
            .unwrap_or(0)
    }
}
pub(super) fn pixel_rect(rect: [f32; 4], size: [u32; 2]) -> Result<[u32; 4], String> {
    let mut edges = [0; 4];
    for i in 0..4 {
        let value = rect[i];
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("invalid screen atlas rect".into());
        }
        let pixel = f64::from(value) * f64::from(size[i % 2]);
        // Recover integer texel boundaries rounded by the atlas's f32 division.
        // Arbitrary subtexel crops cannot become independent textures faithfully.
        if (pixel - pixel.round()).abs() > 0.01 {
            return Err("subtexel screen rectangle unsupported".into());
        }
        edges[i] = pixel.round() as u32;
    }
    if edges[0] >= edges[2] || edges[1] >= edges[3] {
        return Err("empty/reversed screen atlas rect".into());
    }
    Ok([edges[0], edges[1], edges[2] - edges[0], edges[3] - edges[1]])
}

/// Retain any failed cleanup IDs until retry/Drop. No leaked context/ID casts.
#[derive(Default)]
pub(super) struct Resources {
    pub(super) meshes: Vec<MeshId>,
    pub(super) textures: Vec<TextureId>,
}
impl Resources {
    pub fn collect<B: RenderBackend<Error = String>>(
        &mut self,
        backend: &mut B,
    ) -> Result<(), String> {
        while let Some(&id) = self.meshes.last() {
            backend.remove_mesh(id)?;
            self.meshes.pop();
        }
        while let Some(&id) = self.textures.last() {
            backend.remove_texture(id)?;
            self.textures.pop();
        }
        Ok(())
    }
    /// One image + one 188-byte mesh live at a time, released before the HUD.
    /// CPU/space validation is done by the coordinator BEFORE world clear.
    pub fn render<B: RenderBackend<Error = String>>(
        &mut self,
        backend: &mut B,
        prepared: &Prepared,
        white: TextureId,
    ) -> Result<(), String> {
        self.collect(backend)?;
        for run in &prepared.runs {
            let sprite = &prepared.sprites[run.sprite];
            let texture = backend.create_texture(Image {
                width: sprite.size[0],
                height: sprite.size[1],
                rgba: &sprite.pixels,
            })?;
            self.textures.push(texture);
            let mesh = backend.create_mesh(&run.vertices, &[0, 1, 2, 0, 2, 3])?;
            self.meshes.push(mesh);
            backend.render(&Frame {
                view_projection: IDENTITY,
                camera: [0.; 3],
                clear: Clear::Load,
                fog: None,
                draws: &[Draw {
                    mesh,
                    texture,
                    lightmap: white,
                    origin: [0.; 3],
                    pass: Pass::Screen(run.material),
                    scissor: None,
                }],
            })?;
            self.collect(backend)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{font::Patch, screen_effects::ScreenVertex};
    use horizon_render_api::paged_gui::Layout;
    fn quad(rect: [f32; 4]) -> ScreenQuad {
        ScreenQuad {
            vertices: [ScreenVertex {
                pos: [0.; 2],
                uv: [4., -2.],
                rect,
                color: [1.; 4],
            }; 4],
            blend: source::Blend::Alpha,
            sampling: source::SAMPLE_BLUR,
            world_tint: true,
        }
    }
    #[test]
    fn extracts_across_page_seams_deduplicates_and_observes_patches() {
        let layout = Layout::new([7, 5], 4, 12).unwrap();
        let pixels: Vec<_> = (0..35u8).flat_map(|n| [n, 2 * n, 255 - n, 255]).collect();
        let mut backing = Backing::new(layout, &pixels, 4096).unwrap();
        for x in 0..7 {
            for y in 0..5 {
                for w in 1..=7 - x {
                    for h in 1..=5 - y {
                        let actual = backing.extract([x, y, w, h], 140).unwrap();
                        let expected: Vec<_> = (y..y + h)
                            .flat_map(|row| {
                                pixels[(row * 7 + x) as usize * 4..(row * 7 + x + w) as usize * 4]
                                    .iter()
                                    .copied()
                            })
                            .collect();
                        assert_eq!(actual, expected);
                    }
                }
            }
        }
        let q = quad([1. / 7., 1. / 5., 6. / 7., 4. / 5.]);
        let first = Prepared::new(&[q.clone(), q.clone()], &backing, 2, 124, 64).unwrap(); // 64 decoded + 60 extracted
        assert_eq!(first.sprites.len(), 1);
        assert_eq!(first.runs.len(), 2);
        assert_eq!(first.runs[0].vertices[0].uv, [4., -2.]); // NO CPU wrapping
        assert!(first.runs[0].material.linear);
        assert!(!first.runs[0].material.clamp);
        assert!(Prepared::new(std::slice::from_ref(&q), &backing, 1, 123, 64).is_err());
        backing
            .patch(
                &[Patch {
                    x: 2,
                    y: 2,
                    width: 1,
                    height: 1,
                    rgba: vec![9; 4],
                }],
                1024,
            )
            .unwrap();
        let patched = Prepared::new(&[q], &backing, 1, 124, 64).unwrap();
        assert_eq!(
            &patched.sprites[0].pixels[(5 + 1) * 4..(5 + 1) * 4 + 4],
            &[9; 4]
        );
    }
    #[test]
    fn rejects_invalid_inputs_and_limits_before_extraction_or_submission() {
        let layout = Layout::new([7, 5], 4, 12).unwrap();
        let backing = Backing::new(layout, &[0; 140], 4096).unwrap();
        let valid = quad([0., 0., 1., 1.]);
        assert!(Prepared::new(std::slice::from_ref(&valid), &backing, 0, 4096, 64).is_err());
        assert!(Prepared::new(std::slice::from_ref(&valid), &backing, 1, 4096, 4).is_err());
        for rect in [
            [0., 0., 0., 1.],
            [0., 0., f32::NAN, 1.],
            [-0.1, 0., 1., 1.],
            [0.01, 0., 1., 1.],
        ] {
            assert!(Prepared::new(&[quad(rect)], &backing, 1, 4096, 64).is_err());
        }
        for kind in 0..4 {
            let mut q = valid.clone();
            match kind {
                0 => q.sampling = 4,
                1 => q.vertices[2].uv[0] = f32::NAN,
                2 => q.vertices[1].rect[0] = 1.,
                _ => q.vertices[3].color[0] = 1.1,
            }
            assert!(Prepared::new(&[q], &backing, 1, 4096, 64).is_err());
        }
        assert!(Prepared::new(&[], &backing, 0, 0, 0).unwrap().is_empty());
        assert!(backing.extract([6, 4, 2, 1], 4096).is_err());
        assert!(backing.extract([0, 0, 7, 5], 139).is_err());
    }
}
