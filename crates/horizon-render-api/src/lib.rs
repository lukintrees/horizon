//! Backend-neutral SDR draw contract. No wgpu, OpenGL, windowing or resource decoder.
#![forbid(unsafe_code)]
pub mod atlas;
pub mod entity;
pub mod gui;
pub mod paged_gui;
pub mod platform_contract;
pub mod render_contract;
pub mod resource_contract;
pub mod scene;
pub mod screen;
pub mod screenshot;
pub mod streaming;
pub mod stroke;
pub mod terrain;

/// Expanded, camera-relative-at-draw-time vertex. Terrain RGB already includes
/// baked tint/AO/shade. Alpha is actual coverage, never mesher material flags.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub light_uv: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshId(pub u64);

/// RGBA8 rows in GL order (row zero is the bottom). No implicit conversion/sRGB.
pub struct Image<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Opaque,
    /// Camera-relative background geometry, no depth test/write. Flattening
    /// removes near/far clipping of analytic sky discs, but preserves XY/w.
    Sky {
        additive: bool,
        flatten_depth: bool,
    },
    /// Unlit, unfogged camera overlay with sprite-local sampler state.
    Screen(screen::Material),
    Entity(entity::Material),
    /// Typed GUI vertices with independently repeated foil coordinates.
    Gui(gui::Material),
    Cutout,
    Translucent,
    /// Pre-expanded clip-space strokes: LEQUAL, alpha blend, no depth writes.
    Stroke,
    /// Screen-space geometry uses the supplied projection, without depth testing.
    Overlay,
    OverlayInvert,
    OverlayAdditive,
    OverlayVignette,
    /// Alpha-blended GUI model with LEQUAL depth testing and depth writes.
    OverlayDepth,
}
impl Pass {
    /// Fixed-function state shared by ordinary and foil-aware GUI buffers.
    pub fn pipeline(self) -> Self {
        match self {
            Self::Gui(m) => m.kind.pipeline(),
            Self::Screen(m) => match m.blend {
                screen::Blend::Alpha => Self::Overlay,
                screen::Blend::Vignette => Self::OverlayVignette,
            },
            other => other,
        }
    }
    pub fn is_overlay(self) -> bool {
        matches!(
            self.pipeline(),
            Self::Overlay
                | Self::OverlayInvert
                | Self::OverlayAdditive
                | Self::OverlayVignette
                | Self::OverlayDepth
        )
    }
}
/// Load/store policy for one view. A GUI view can load the world's colour while
/// using its own projection; a hand/model view can clear depth without erasing it.
#[derive(Clone, Copy, Debug)]
pub enum Clear {
    Load,
    Depth,
    ColorDepth([f32; 4]),
}
#[derive(Clone, Copy, Debug)]
pub struct Draw {
    pub mesh: MeshId,
    pub texture: TextureId,
    pub lightmap: TextureId,
    /// Absolute world origin. Subtracted from Frame.camera in f64, then cast.
    pub origin: [f64; 3],
    pub pass: Pass,
    /// Bottom-left pixel origin, width, height. None disables clipping.
    pub scissor: Option<[u32; 4]>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameFog {
    /// Render-distance fog, evaluated on max(length(xz), abs(y)).
    pub start: f32,
    pub end: f32,
    /// The game's fog color alpha, applied AFTER max(environment, distance).
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    /// None retains the original spherical, opaque fog contract. Some enables
    /// the game's dual-distance fog and permits equal (step/disabled) ranges.
    pub game: Option<GameFog>,
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
}
pub struct Frame<'a> {
    /// Column-major desktop GL projection/view. Z clip range is -w..w.
    pub view_projection: [f32; 16],
    pub camera: [f64; 3],
    pub clear: Clear,
    pub fog: Option<Fog>,
    /// Order is authoritative, including back-to-front translucency and UI.
    pub draws: &'a [Draw],
}
pub const IDENTITY: [f32; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];

/// Common upload/draw boundary. IDs are backend-owned and must be validated;
/// supplying a fabricated/stale ID is an error, never an unchecked GPU handle.
pub trait RenderBackend {
    type Error;
    fn create_texture(&mut self, image: Image<'_>) -> Result<TextureId, Self::Error>;
    fn update_texture(
        &mut self,
        id: TextureId,
        at: [u32; 2],
        image: Image<'_>,
    ) -> Result<(), Self::Error>;
    fn remove_texture(&mut self, id: TextureId) -> Result<(), Self::Error>;
    fn create_mesh(&mut self, vertices: &[Vertex], indices: &[u16]) -> Result<MeshId, Self::Error>;
    fn create_entity_mesh(
        &mut self,
        vertices: &[entity::LitVertex],
        indices: &[u16],
    ) -> Result<MeshId, Self::Error>;
    fn create_gui_mesh(
        &mut self,
        vertices: &[gui::GuiVertex],
        indices: &[u16],
    ) -> Result<MeshId, Self::Error>;
    fn remove_mesh(&mut self, id: MeshId) -> Result<(), Self::Error>;
    fn render(&mut self, frame: &Frame<'_>) -> Result<(), Self::Error>;
    fn read_rgba(&mut self, rect: [u32; 4]) -> Result<Vec<u8>, Self::Error>;
}

// Owning wrappers may also hold an exclusive reference during a bounded test or
// render pass. This keeps one lifecycle implementation for both ownership modes.
impl<B: RenderBackend + ?Sized> RenderBackend for &mut B {
    type Error = B::Error;
    fn create_texture(&mut self, image: Image<'_>) -> Result<TextureId, Self::Error> {
        (**self).create_texture(image)
    }
    fn update_texture(
        &mut self,
        id: TextureId,
        at: [u32; 2],
        image: Image<'_>,
    ) -> Result<(), Self::Error> {
        (**self).update_texture(id, at, image)
    }
    fn remove_texture(&mut self, id: TextureId) -> Result<(), Self::Error> {
        (**self).remove_texture(id)
    }
    fn create_mesh(&mut self, vertices: &[Vertex], indices: &[u16]) -> Result<MeshId, Self::Error> {
        (**self).create_mesh(vertices, indices)
    }
    fn create_entity_mesh(
        &mut self,
        vertices: &[entity::LitVertex],
        indices: &[u16],
    ) -> Result<MeshId, Self::Error> {
        (**self).create_entity_mesh(vertices, indices)
    }
    fn create_gui_mesh(
        &mut self,
        vertices: &[gui::GuiVertex],
        indices: &[u16],
    ) -> Result<MeshId, Self::Error> {
        (**self).create_gui_mesh(vertices, indices)
    }
    fn remove_mesh(&mut self, id: MeshId) -> Result<(), Self::Error> {
        (**self).remove_mesh(id)
    }
    fn render(&mut self, frame: &Frame<'_>) -> Result<(), Self::Error> {
        (**self).render(frame)
    }
    fn read_rgba(&mut self, rect: [u32; 4]) -> Result<Vec<u8>, Self::Error> {
        (**self).read_rgba(rect)
    }
}

pub fn relative_origin(origin: [f64; 3], camera: [f64; 3]) -> Option<[f32; 3]> {
    let relative = std::array::from_fn(|i| (origin[i] - camera[i]) as f32);
    relative.iter().all(|v| v.is_finite()).then_some(relative)
}

/// Convert wgpu's 0..w depth clip to desktop GL's -w..w. Columns, not rows.
pub fn gl_projection(mut matrix: [f32; 16]) -> [f32; 16] {
    for column in matrix.chunks_exact_mut(4) {
        column[2] = 2.0 * column[2] - column[3];
    }
    matrix
}

/// Convert REVERSED wgpu depth (near=w, far=0) to conventional desktop GL
/// (near=-w, far=w). Unlike `gl_projection`, this also unreverses depth for
/// the GL2 backend's LESS test and depth-clear value 1. Columns, not rows.
pub fn gl_reversed_projection(mut matrix: [f32; 16]) -> [f32; 16] {
    for column in matrix.chunks_exact_mut(4) {
        column[2] = column[3] - 2.0 * column[2];
    }
    matrix
}

/// Existing mesher fixed-point sky/block light -> texel-centred 16x16 lightmap UV.
pub fn light_uv(light: u16) -> [f32; 2] {
    let coord = |v: u16| (f32::from(v) / 256.0 + 0.5 / 16.0).clamp(0.5 / 16.0, 15.5 / 16.0);
    [coord(light >> 8), coord(light & 255)]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_subtraction_precedes_float_conversion() {
        assert_eq!(
            relative_origin([30_000_000.; 3], [29_999_999.75; 3]),
            Some([0.25; 3])
        );
        assert!(relative_origin([f64::INFINITY; 3], [0.; 3]).is_none());
    }
    #[test]
    fn gl_depth_maps_near_and_far_to_minus_and_plus_w() {
        let gl = gl_projection(IDENTITY);
        // For identity wgpu projection, input z=0 is near and z=1 is far.
        assert_eq!(gl[14], -1.);
        assert_eq!(gl[10] + gl[14], 1.);
        assert_eq!(gl[15], 1.);
    }
    #[test]
    fn reversed_depth_maps_near_to_minus_w_and_far_to_plus_w() {
        let gl = gl_reversed_projection(IDENTITY);
        assert_eq!(gl[10] + gl[14], -1.); // source near z=w
        assert_eq!(gl[14], 1.); // source far z=0
        assert_eq!(gl[15], 1.);
        let source = std::array::from_fn(|i| (i as f32 - 3.) / 7.);
        let converted = gl_reversed_projection(source);
        for i in 0..16 {
            assert_eq!(
                converted[i],
                if i % 4 == 2 {
                    source[i + 1] - 2. * source[i]
                } else {
                    source[i]
                }
            );
        }
    }
    #[test]
    fn lightmap_centres_and_intermediate_smooth_light() {
        assert_eq!(light_uv(0), [1. / 32.; 2]);
        assert_eq!(light_uv(0xf0f0), [31. / 32.; 2]);
        assert_eq!(light_uv(0x8040), [17. / 32., 9. / 32.]);
    }
}
