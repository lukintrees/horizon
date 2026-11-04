//! The OpenGL 2 renderer: the frame the wgpu renderer draws, on a legacy OpenGL context made on the
//! game's own window. M2 draws the terrain: each section's solid quads in one float buffer, the atlas
//! pages, the lightmap, the fog, CPU culling by section, and the HUD. Everything else the enum forwards
//! lives in `stubs.rs` and does nothing until its milestone (docs/plans/opengl21-radeon-x1650.md).

use std::collections::{HashMap, HashSet};

use glam::{DVec3, Mat4, Vec3, Vec4};
use glow::HasContext;
use winit::window::Window;

use super::atlas::{self, Atlas};
use super::batch;
use super::draw::{Fog, Section, Terrain};
use super::hud::Hud;
#[cfg(windows)]
use super::wgl;
use crate::Result;
use crate::chunk::ColumnLayout;
use crate::gfx::{Frame, Frustum, SectionKey, View, ViewSet};
use crate::hud::HudBatch;
use crate::mesher::SectionMesh;

/// The window's OpenGL context. On other systems there is none: `open` refuses before one is needed.
#[cfg(windows)]
type Platform = wgl::Context;
#[cfg(not(windows))]
type Platform = ();

/// A section on the GPU: its solid and translucent buffers (either may be absent), and its bounds in
/// section-local blocks.
struct Placed {
    solid: Option<Section>,
    translucent: Option<Section>,
    bounds: [Vec3; 2],
}

pub struct Renderer {
    gl: glow::Context,
    size: [u32; 2],
    atlas: Atlas,
    terrain: Terrain,
    hud: Hud,
    sections: HashMap<SectionKey, Placed>,
    /// The lightmap as last uploaded, so a frame uploads it only when it changed.
    lightmap: [u8; 16 * 16 * 4],
    drawn: usize,
    pub gamma: f32,
    pub fullbright: bool,
    pub paced_elsewhere: bool,
    /// The answer to a screenshot request, taken once by the game.
    pub screenshot_result: Option<std::result::Result<[u32; 2], String>>,
    /// Last, so the GL objects are freed before the context goes.
    platform: Platform,
}

impl Renderer {
    /// Makes an OpenGL context on `window` and uploads the block sprites (`sprite_pixels`: `layers`
    /// square RGBA images of `sprite_size`), the HUD atlas and the lightmap. Fails when the window's
    /// OpenGL is 3.3 or newer, since the wgpu renderer draws that GPU.
    pub fn new(
        window: &Window,
        sprite_pixels: &[u8],
        sprite_size: u32,
        layers: u32,
        hud_atlas: (&[u8], u32, u32),
    ) -> Result<Self> {
        let (platform, gl) = open(window)?;
        // SAFETY: `open` made the context current on this thread.
        let version = unsafe { gl.get_parameter_string(glow::VERSION) };
        match parse_version(&version) {
            Some(found) if found < (3, 3) => {}
            Some(_) => {
                return Err(
                    format!("OpenGL {version} is 3.3 or newer; wgpu draws this GPU").into(),
                );
            }
            None => return Err(format!("the OpenGL version {version:?} cannot be read").into()),
        }
        // A mesh of this path is six vertices per quad, as `batch::solid` reads it. The packed
        // quads of the wgpu path are not drawn here.
        crate::mesher::packed::pack_quads(false);
        let atlas = atlas::pack(sprite_pixels, sprite_size, layers)?;
        let lightmap = [255u8; 16 * 16 * 4];
        let terrain = Terrain::new(&gl, &atlas, &lightmap)?;
        let hud = Hud::new(&gl, hud_atlas.0, hud_atlas.1, hud_atlas.2)?;
        let inner = window.inner_size();
        Ok(Self {
            gl,
            size: [inner.width.max(1), inner.height.max(1)],
            atlas,
            terrain,
            hud,
            sections: HashMap::new(),
            lightmap,
            drawn: 0,
            gamma: 0.0,
            fullbright: false,
            paced_elsewhere: false,
            screenshot_result: None,
            platform,
        })
    }

    /// The surface's size in pixels.
    pub fn surface_size(&self) -> [u32; 2] {
        self.size
    }

    /// Sections drawn by the last frame.
    pub fn drawn_sections(&self) -> usize {
        self.drawn
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = [width.max(1), height.max(1)];
    }

    pub fn has_section(&self, key: SectionKey) -> bool {
        self.sections.contains_key(&key)
    }

    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    pub fn resident_sections(&self) -> usize {
        self.sections.len()
    }

    /// Replaces a section's vertices. An empty mesh only removes the section.
    pub fn upload(&mut self, key: SectionKey, mesh: &SectionMesh) {
        make_current(&self.platform);
        self.free_section(key);
        if mesh.solid.is_empty() && mesh.translucent.is_empty() {
            return;
        }
        let solid = self.upload_batch(&batch::solid(&mesh.solid, &self.atlas));
        let translucent = self.upload_batch(&batch::translucent(&mesh.translucent, &self.atlas));
        if solid.is_none() && translucent.is_none() {
            return;
        }
        let bounds = mesh.bounds.unwrap_or_else(full_section);
        self.sections.insert(
            key,
            Placed {
                solid,
                translucent,
                bounds,
            },
        );
    }

    /// Uploads one batch into a new buffer; an empty batch has none.
    fn upload_batch(&self, batch: &batch::Batch) -> Option<Section> {
        if batch.vertices.is_empty() {
            return None;
        }
        match self.terrain.upload(&self.gl, batch) {
            Ok(section) => Some(section),
            Err(error) => {
                log::warn!("a section does not upload: {error}");
                None
            }
        }
    }

    pub fn remove_columns(&mut self, columns: impl IntoIterator<Item = (i32, i32)>) {
        make_current(&self.platform);
        let columns: HashSet<(i32, i32)> = columns.into_iter().collect();
        let gone: Vec<SectionKey> = self
            .sections
            .keys()
            .copied()
            .filter(|key| columns.contains(&(key.0, key.2)))
            .collect();
        for key in gone {
            self.free_section(key);
        }
    }

    /// Frees every section; the layout is the wgpu path's and this path keeps none.
    pub fn clear_sections(&mut self, _layout: ColumnLayout) {
        make_current(&self.platform);
        let all: Vec<SectionKey> = self.sections.keys().copied().collect();
        for key in all {
            self.free_section(key);
        }
    }

    /// The view of a flat screen, from the frame's camera.
    pub fn views(&self, frame: &Frame) -> ViewSet {
        ViewSet::single(View::screen(
            frame.yaw,
            frame.pitch,
            frame.fov_degrees,
            self.aspect(),
            frame.depth_far,
        ))
    }

    /// Camera-relative clip planes, as the wgpu path builds them.
    pub fn camera_frustum(
        &self,
        fov_degrees: f32,
        yaw: f32,
        pitch: f32,
        bob: Mat4,
        depth_far: f32,
    ) -> Frustum {
        Frustum::of(View::screen(yaw, pitch, fov_degrees, self.aspect(), depth_far).view_proj(bob))
    }

    /// Draws one frame: the lightmap, the sections inside the frustum, then the HUD, and presents it.
    /// Returns whether the frame was presented.
    pub fn render(&mut self, frame: &Frame) -> Result<bool> {
        if !make_current(&self.platform) {
            return Err("the OpenGL context cannot be made current".into());
        }
        let view_proj = self.views(frame).primary().view_proj(frame.bob);
        let (fog_color, fog_range) = fog_of(frame);

        let pixels = crate::gfx::lightmap_pixels(
            &frame.sky,
            frame.darkness.1,
            (self.gamma - frame.darkness.0).max(0.0),
            frame.night_vision,
            frame.block_flicker,
            frame.boss_darken,
            self.fullbright,
        );
        if pixels != self.lightmap {
            self.terrain.set_lightmap(&self.gl, &pixels);
            self.lightmap = pixels;
        }

        self.draw_world(view_proj, frame.camera, fog_color, fog_range);

        // SAFETY: the context is current; the HUD is drawn without depth testing, over the terrain.
        unsafe { self.gl.disable(glow::DEPTH_TEST) };
        self.draw_hud(frame.hud);
        present(&self.platform);
        Ok(true)
    }

    /// Clears to the fog colour and draws the sections whose bounds lie inside the frustum of
    /// `view_proj`, seen from `camera`. Returns how many were drawn.
    fn draw_world(
        &mut self,
        view_proj: Mat4,
        camera: DVec3,
        fog_color: [f32; 3],
        fog_range: [f32; 4],
    ) -> usize {
        let frustum = Frustum::of(view_proj);
        let clip = to_gl_clip() * view_proj;
        let [width, height] = self.size;
        // SAFETY: the context is current on this thread (`make_current` in `render`).
        unsafe {
            self.gl.viewport(0, 0, width as i32, height as i32);
            self.gl.disable(glow::BLEND);
            self.gl.enable(glow::DEPTH_TEST);
            // Reversed depth: near is 1, far is 0, so the cleared depth is 0 and the nearer surface wins
            // on GEQUAL.
            self.gl.depth_func(glow::GEQUAL);
            self.gl.depth_mask(true);
            self.gl
                .clear_color(fog_color[0], fog_color[1], fog_color[2], 1.0);
            self.gl.clear_depth_f64(0.0);
            self.gl
                .clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }

        let fog = Fog {
            range: fog_range,
            color: fog_color,
        };
        self.terrain.begin(&self.gl, &clip.to_cols_array(), fog);
        let mut drawn = 0;
        // The visible sections with their distance, for the translucent pass.
        let mut visible: Vec<(f32, [f32; 3], &Placed)> = Vec::new();
        for (key, placed) in &self.sections {
            let origin =
                (DVec3::new(key.0 as f64, key.1 as f64, key.2 as f64) * 16.0 - camera).as_vec3();
            if !frustum.intersects(origin + placed.bounds[0], origin + placed.bounds[1]) {
                continue;
            }
            if let Some(section) = &placed.solid {
                self.terrain.draw(&self.gl, section, origin.to_array());
                drawn += 1;
            }
            visible.push((origin.length_squared(), origin.to_array(), placed));
        }
        self.drawn = drawn;

        // The translucent quads last: blended over the solid world, section by section from the
        // farthest, without depth writes so that a section does not hide the one behind it.
        if visible
            .iter()
            .any(|(_, _, placed)| placed.translucent.is_some())
        {
            visible.sort_by(|a, b| b.0.total_cmp(&a.0));
            // SAFETY: the context is current on this thread.
            unsafe {
                self.gl.enable(glow::BLEND);
                self.gl
                    .blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                self.gl.depth_mask(false);
            }
            self.terrain.set_translucent(&self.gl, true);
            for (_, origin, placed) in &visible {
                if let Some(section) = &placed.translucent {
                    self.terrain.draw(&self.gl, section, *origin);
                }
            }
            self.terrain.set_translucent(&self.gl, false);
            // SAFETY: as above.
            unsafe {
                self.gl.disable(glow::BLEND);
                self.gl.depth_mask(true);
            }
        }
        drawn
    }

    /// Writes the HUD's changed atlas patches and draws its batch.
    fn draw_hud(&self, hud: &HudBatch) {
        for patch in &hud.patches {
            self.hud.patch(&self.gl, patch);
        }
        self.hud.draw(&self.gl, hud, self.size);
    }

    fn aspect(&self) -> f32 {
        self.size[0] as f32 / self.size[1].max(1) as f32
    }

    fn free_section(&mut self, key: SectionKey) {
        if let Some(placed) = self.sections.remove(&key) {
            free_placed(&self.terrain, &self.gl, placed);
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // The GL objects go while their context is current. `platform` drops after this body.
        make_current(&self.platform);
        for (_, placed) in self.sections.drain() {
            free_placed(&self.terrain, &self.gl, placed);
        }
        self.terrain.free(&self.gl);
        self.hud.free(&self.gl);
    }
}

/// The depth of `gfx::view`'s reversed clip space, rewritten for OpenGL, whose clip depth runs from
/// -w to w: z' = 2z - w. The depth values stay reversed, so the buffer keeps GEQUAL and clear 0.
fn to_gl_clip() -> Mat4 {
    Mat4::from_cols(
        Vec4::X,
        Vec4::Y,
        Vec4::new(0.0, 0.0, 2.0, 0.0),
        Vec4::new(0.0, 0.0, -1.0, 1.0),
    )
}

/// Frees both buffers of a section. A free function, so that the sections can be drained while the
/// terrain and the context are borrowed.
fn free_placed(terrain: &Terrain, gl: &glow::Context, placed: Placed) {
    for section in placed.solid.into_iter().chain(placed.translucent) {
        terrain.free_section(gl, section);
    }
}

fn full_section() -> [Vec3; 2] {
    [Vec3::ZERO, Vec3::splat(16.0)]
}

/// The fog colour and range of a frame, with the rules of terrain.wgsl's `Globals`: the range is
/// `[spherical start, end, cylindrical start, end]`.
fn fog_of(frame: &Frame) -> ([f32; 3], [f32; 4]) {
    if let Some(fog) = frame.fog {
        return (
            [fog.color[0], fog.color[1], fog.color[2]],
            [
                fog.environmental_start,
                fog.environmental_end,
                fog.render_distance_start,
                fog.render_distance_end,
            ],
        );
    }
    if frame.underwater {
        return (frame.water_fog, [-8.0, 96.0, -8.0, 96.0]);
    }
    let d = frame.distance;
    let (start, end) = frame.environment_fog.unwrap_or((f32::MAX, f32::MAX));
    (
        frame.sky.fog_color,
        [start, end, (d - d.min(64.0) * 0.25).max(0.0), d],
    )
}

/// `(major, minor)` from a `GL_VERSION` string such as "2.1.2 ATI-1.0" or "3.3.0 Mesa 23.0".
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut numbers = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty());
    let major = numbers.next()?.parse().ok()?;
    let minor = numbers.next()?.parse().ok()?;
    Some((major, minor))
}

#[cfg(windows)]
fn open(window: &Window) -> Result<(Platform, glow::Context)> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = window.window_handle()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err("the OpenGL 2 path needs a Win32 window".into());
    };
    // SAFETY: the window is the game's, on this thread, and outlives the renderer.
    let context = unsafe { wgl::Context::attach(win32.hwnd.get()) }?;
    let gl = context.glow();
    Ok((context, gl))
}

#[cfg(not(windows))]
fn open(_window: &Window) -> Result<(Platform, glow::Context)> {
    Err("the OpenGL 2 path runs on Windows only".into())
}

#[cfg(windows)]
fn make_current(platform: &Platform) -> bool {
    platform.make_current()
}

#[cfg(not(windows))]
fn make_current(_platform: &Platform) -> bool {
    true
}

#[cfg(windows)]
fn present(platform: &Platform) -> bool {
    platform.swap()
}

#[cfg(not(windows))]
fn present(_platform: &Platform) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_gl_version_strings() {
        assert_eq!(parse_version("2.1.2 ATI-1.0"), Some((2, 1)));
        assert_eq!(parse_version("3.3.0 Mesa 23.0"), Some((3, 3)));
        assert_eq!(parse_version("4.6.0 NVIDIA 550"), Some((4, 6)));
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn the_gl_clip_maps_reversed_depth_onto_gl_range() {
        // The near plane (depth 1 in wgpu's clip space) lands at +1 in GL's, and far (0) at -1, after
        // the perspective divide by w.
        let near = to_gl_clip() * Vec4::new(0.0, 0.0, 1.0, 1.0);
        let far = to_gl_clip() * Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!((near.z / near.w - 1.0).abs() < 1e-6);
        assert!((far.z / far.w + 1.0).abs() < 1e-6);
    }
}

/// Draws a section on the GPU the game's window gets, as the game would: on Windows, where the
/// context is WGL. CI runs it on Mesa llvmpipe capped to OpenGL 2.1 (`MESA_GL_VERSION_OVERRIDE`).
#[cfg(all(test, windows))]
mod gpu_tests {
    use std::time::Duration;

    use glow::HasContext;
    use winit::application::ApplicationHandler;
    use winit::dpi::PhysicalSize;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::platform::pump_events::EventLoopExtPumpEvents;
    use winit::platform::windows::EventLoopBuilderExtWindows;
    use winit::window::WindowId;

    use super::*;
    use crate::mesher::TerrainVertex;

    /// Makes the window on the test's own thread, which is not the main thread.
    #[derive(Default)]
    struct Make {
        window: Option<Window>,
    }

    impl ApplicationHandler for Make {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_none() {
                let attributes =
                    Window::default_attributes().with_inner_size(PhysicalSize::new(128, 128));
                self.window = event_loop.create_window(attributes).ok();
            }
        }

        fn window_event(
            &mut self,
            _event_loop: &ActiveEventLoop,
            _id: WindowId,
            _event: WindowEvent,
        ) {
        }
    }

    /// The event loop is returned too: the window must not outlive it.
    fn window() -> (EventLoop<()>, Window) {
        let mut builder = EventLoop::builder();
        builder.with_any_thread(true);
        let mut event_loop = builder.build().expect("an event loop on the test thread");
        let mut app = Make::default();
        for _ in 0..100 {
            if app.window.is_some() {
                break;
            }
            let _ = event_loop.pump_app_events(Some(Duration::from_millis(10)), &mut app);
        }
        let window = app.window.expect("a window");
        (event_loop, window)
    }

    #[test]
    fn a_section_in_front_of_the_camera_is_drawn() {
        let (_event_loop, window) = window();
        let sprite = vec![255u8; 16 * 16 * 4];
        let hud = vec![255u8; 4 * 4 * 4];
        let mut renderer = Renderer::new(&window, &sprite, 16, 1, (&hud, 4, 4))
            .expect("the OpenGL 2 renderer starts on the window's context");

        // One white quad across the section's far face (z = 8 of 16), facing the camera.
        let corners = [
            [0.0, 0.0, 8.0],
            [16.0, 0.0, 8.0],
            [16.0, 16.0, 8.0],
            [0.0, 16.0, 8.0],
        ];
        let mut mesh = SectionMesh::default();
        mesh.solid = corners
            .map(|pos| TerrainVertex::new(pos, [0.5, 0.5], 0, 0xFF_FFFF, 0xF0F0))
            .to_vec();
        // A translucent quad in front of it, in the same section: the blended pass draws it too.
        let near = [
            [0.0, 0.0, 4.0],
            [16.0, 0.0, 4.0],
            [16.0, 16.0, 4.0],
            [0.0, 16.0, 4.0],
        ];
        mesh.translucent = near
            .map(|pos| TerrainVertex::new(pos, [0.5, 0.5], 0, 0xFF_FFFF, 0xF0F0))
            .to_vec();
        renderer.upload((0, 0, 0), &mesh);
        assert_eq!(renderer.resident_sections(), 1);

        let [width, height] = renderer.size;
        let aspect = width as f32 / height as f32;
        let view = View::screen(0.0, 0.0, 70.0, aspect, 512.0);
        let camera = DVec3::new(8.0, 8.0, 0.0);
        assert!(make_current(&renderer.platform));
        let drawn = renderer.draw_world(
            view.view_proj(Mat4::IDENTITY),
            camera,
            [0.5, 0.5, 0.5],
            [1.0e9; 4],
        );
        assert_eq!(drawn, 1, "the section lies in front of the camera");

        let mut pixels = vec![0u8; (width * height * 4) as usize];
        // SAFETY: the context is current on this thread and `pixels` holds the whole surface.
        unsafe {
            renderer.gl.read_pixels(
                0,
                0,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
            assert_eq!(
                renderer.gl.get_error(),
                glow::NO_ERROR,
                "the frame draws without a GL error"
            );
        }
        let bright = pixels
            .chunks_exact(4)
            .filter(|p| p[0] > 200 && p[1] > 200)
            .count();
        assert!(
            bright > 1000,
            "the quad covers the frame: {bright} bright pixels"
        );
    }
}
