//! Standalone developer integration test for the experimental GL 2.1 backend.
//! Uses Mesa OSMesa and minimal stand-ins for the game's mesher types so it can
//! run without downloading the full workspace's Cargo dependencies. The real
//! atlas/mesh conversion and GL renderer source files are used unchanged.
#![allow(dead_code, unused_imports)]
use std::{
    ffi::{CStr, c_char, c_void},
    mem, ptr,
};

mod mesher {
    pub const OPAQUE_LAYER: u16 = 1 << 15;
    #[derive(Clone, Copy)]
    pub struct TerrainVertex {
        pub pos: [f32; 3],
        uv: [f32; 2],
        pub layer: u16,
        pub color: u32,
        pub light: u16,
    }
    impl TerrainVertex {
        pub fn new(pos: [f32; 3], uv: [f32; 2], layer: u16, color: u32, light: u16) -> Self {
            Self {
                pos,
                uv,
                layer,
                color,
                light,
            }
        }
        pub fn uv(&self) -> [f32; 2] {
            self.uv
        }
    }
    #[derive(Default)]
    pub struct SectionMesh {
        pub solid: Vec<TerrainVertex>,
        pub translucent: Vec<TerrainVertex>,
        pub _other: (),
    }
}
mod sprites {
    pub struct Sprites {
        pub size: u32,
        pub pixels: Vec<u8>,
    }
    impl Sprites {
        pub fn count(&self) -> usize {
            self.pixels.len() / (self.size * self.size * 4) as usize
        }
    }
}
mod font {
    pub struct Patch {
        pub x: u32,
        pub y: u32,
        pub width: u32,
        pub height: u32,
        pub rgba: Vec<u8>,
    }
}
mod hud {
    use std::{borrow::Cow, ops::Range};
    pub struct HudAtlas {
        pub width: u32,
        pub height: u32,
        pub pixels: Vec<u8>,
    }
    impl HudAtlas {
        pub fn pixels(&self) -> Cow<'_, [u8]> {
            Cow::Borrowed(&self.pixels)
        }
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Blend {
        Alpha,
        Invert,
        Additive,
        Depth([u32; 4]),
        Vignette,
        EndPortal,
        CitGlint(u8),
    }
    #[derive(Clone, Copy)]
    pub struct HudVertex {
        pub pos: [f32; 2],
        pub uv: [f32; 2],
        pub color: u32,
        pub depth: f32,
        pub glint_uv: [f32; 2],
        pub text: u32,
    }
    pub struct HudBatch {
        pub vertices: Vec<HudVertex>,
        pub scale: f32,
        pub blends: Vec<(Range<u32>, Blend)>,
    }
    impl HudBatch {
        pub fn new(_: [u32; 2]) -> Self {
            Self {
                vertices: vec![],
                scale: 1.0,
                blends: vec![],
            }
        }
        pub fn quad(&mut self, p: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: u32) {
            for i in [0, 1, 2, 0, 2, 3] {
                self.vertices.push(HudVertex {
                    pos: p[i],
                    uv: uv[i],
                    color,
                    depth: 0.,
                    glint_uv: [0., 0.],
                    text: 0,
                });
            }
        }
        pub fn runs(&self) -> Vec<(Range<u32>, Blend)> {
            let mut out = Vec::new();
            let mut at = 0;
            for (range, blend) in &self.blends {
                if range.start > at {
                    out.push((at..range.start, Blend::Alpha));
                }
                out.push((range.clone(), *blend));
                at = range.end;
            }
            if self.vertices.len() as u32 > at {
                out.push((at..self.vertices.len() as u32, Blend::Alpha));
            }
            out
        }
    }
}
#[path = "../crates/horizon-client-render/src/legacy/mod.rs"]
mod legacy;

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(name: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> i32;
}

unsafe fn symbol(handle: *mut c_void, name: &CStr) -> *mut c_void {
    let p = unsafe { dlsym(handle, name.as_ptr()) };
    assert!(!p.is_null(), "missing {}", name.to_string_lossy());
    p
}
macro_rules! function {
    ($handle:expr, $name:expr, $ty:ty) => {{
        let p = symbol($handle, $name);
        // SAFETY: OSMesa exports this function with the declared signature.
        mem::transmute::<*mut c_void, $ty>(p)
    }};
}

fn quad(x1: f32, x2: f32, z: f32, layer: u16) -> Vec<mesher::TerrainVertex> {
    [
        ([x1, -0.9, z], [0., 0.]),
        ([x2, -0.9, z], [1., 0.]),
        ([x2, 0.9, z], [1., 1.]),
        ([x1, 0.9, z], [0., 1.]),
    ]
    .into_iter()
    .map(|(p, uv)| mesher::TerrainVertex::new(p, uv, layer, 0xffff_ffff, 0))
    .collect()
}

fn main() {
    assert_eq!(std::env::var("LIBGL_ALWAYS_SOFTWARE").as_deref(), Ok("1"));
    assert_eq!(
        std::env::var("MESA_GL_VERSION_OVERRIDE").as_deref(),
        Ok("2.1")
    );
    assert_eq!(
        std::env::var("MESA_GLSL_VERSION_OVERRIDE").as_deref(),
        Ok("120")
    );
    // SAFETY: the OSMesa library and all resources stay live until teardown.
    unsafe {
        let handle = dlopen(c"libOSMesa.so.8".as_ptr(), 2);
        assert!(!handle.is_null(), "libOSMesa.so.8 not found");
        let create = function!(
            handle,
            c"OSMesaCreateContextExt",
            unsafe extern "C" fn(u32, i32, i32, i32, *mut c_void) -> *mut c_void
        );
        let make = function!(
            handle,
            c"OSMesaMakeCurrent",
            unsafe extern "C" fn(*mut c_void, *mut c_void, u32, i32, i32) -> i32
        );
        let destroy = function!(
            handle,
            c"OSMesaDestroyContext",
            unsafe extern "C" fn(*mut c_void)
        );
        let getproc = function!(
            handle,
            c"OSMesaGetProcAddress",
            unsafe extern "C" fn(*const c_char) -> *const c_void
        );
        let get_string = function!(
            handle,
            c"glGetString",
            unsafe extern "system" fn(u32) -> *const c_char
        );
        let viewport = function!(
            handle,
            c"glViewport",
            unsafe extern "system" fn(i32, i32, i32, i32)
        );
        let clear_color = function!(
            handle,
            c"glClearColor",
            unsafe extern "system" fn(f32, f32, f32, f32)
        );
        let clear = function!(handle, c"glClear", unsafe extern "system" fn(u32));
        let pixel_store = function!(
            handle,
            c"glPixelStorei",
            unsafe extern "system" fn(u32, i32)
        );
        let depth_range = function!(handle, c"glDepthRange", unsafe extern "system" fn(f64, f64));
        let color_mask = function!(
            handle,
            c"glColorMask",
            unsafe extern "system" fn(u8, u8, u8, u8)
        );
        let blend_equation = function!(handle, c"glBlendEquation", unsafe extern "system" fn(u32));
        let read = function!(
            handle,
            c"glReadPixels",
            unsafe extern "system" fn(i32, i32, i32, i32, u32, u32, *mut c_void)
        );
        let context = create(0x1908, 16, 0, 0, ptr::null_mut());
        assert!(!context.is_null());
        let mut pixels = vec![0u8; 32 * 32 * 4];
        assert_ne!(make(context, pixels.as_mut_ptr().cast(), 0x1401, 32, 32), 0);
        let renderer = CStr::from_ptr(get_string(0x1f01)).to_string_lossy();
        let version = CStr::from_ptr(get_string(0x1f02)).to_string_lossy();
        let glsl = CStr::from_ptr(get_string(0x8b8c)).to_string_lossy();
        println!("renderer={renderer}, GL={version}, GLSL={glsl}");
        assert!(renderer.to_lowercase().contains("llvmpipe"));
        assert!(version.starts_with("2.1 ") && glsl.starts_with("1.20"));
        let mut gpu = horizon_legacy_gl::Renderer::new(|name| {
            let p = getproc(name.as_ptr());
            if p.is_null() {
                dlsym(handle, name.as_ptr())
            } else {
                p
            }
        })
        .expect("initialize legacy backend");
        let mut atlas =
            legacy::atlas::Atlas::new(1, 3, 4, &[200, 0, 0, 255, 0, 200, 0, 255, 0, 0, 200, 128])
                .unwrap();
        let upload = |gpu: &mut horizon_legacy_gl::Renderer, atlas: &legacy::atlas::Atlas| {
            let pages: Vec<_> = atlas
                .pages()
                .iter()
                .map(|p| horizon_legacy_gl::Page {
                    side: p.side,
                    pixels: &p.pixels,
                })
                .collect();
            gpu.upload_pages(&pages).unwrap();
        };
        // The uploader must not inherit stale unpack state from another pass.
        pixel_store(0x0cf2, 8); // GL_UNPACK_ROW_LENGTH
        pixel_store(0x0cf3, 1); // GL_UNPACK_SKIP_ROWS
        pixel_store(0x0cf4, 1); // GL_UNPACK_SKIP_PIXELS
        upload(&mut gpu, &atlas);
        assert_eq!(
            gpu.upload_pages(&[horizon_legacy_gl::Page {
                side: 3,
                pixels: &[]
            }]),
            Err(horizon_legacy_gl::Error::InvalidPage)
        );
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        viewport(0, 0, 32, 32);
        clear_color(0., 0., 0., 1.);
        let sample = |x: i32| {
            let mut p = [0u8; 4];
            read(x, 16, 1, 1, 0x1908, 0x1401, p.as_mut_ptr().cast());
            p
        };
        let rgb = |p: [u8; 4]| [p[0], p[1], p[2]];
        let draw = |gpu: &mut horizon_legacy_gl::Renderer, mesh: &legacy::terrain::Mesh, pass| {
            gpu.draw(&mesh.vertices, &mesh.batches, &identity, pass)
                .unwrap();
        };
        let ordered = legacy::terrain::build(
            &atlas,
            &[
                quad(-0.95, -0.35, 0., 0),
                quad(-0.3, 0.3, 0., 1),
                quad(0.35, 0.95, 0., 0),
            ]
            .concat(),
        )
        .unwrap();
        assert_eq!(
            ordered.batches.iter().map(|b| b.page).collect::<Vec<_>>(),
            [0, 1, 0]
        );
        clear(0x4000 | 0x0100);
        color_mask(0, 0, 0, 0);
        depth_range(1.0, 0.0);
        draw(
            &mut gpu,
            &ordered,
            horizon_legacy_gl::Pass::Solid { alpha_cutoff: 0.1 },
        );
        assert_eq!(rgb(sample(5)), [200, 0, 0]);
        assert_eq!(rgb(sample(16)), [0, 200, 0]);
        assert_eq!(rgb(sample(26)), [200, 0, 0]);

        // Re-upload an animated frame and observe it in the same mesh.
        atlas.update(1, &[220, 220, 0, 255]).unwrap();
        let page = &atlas.pages()[1];
        assert_eq!(
            gpu.update_page(
                3,
                horizon_legacy_gl::Page {
                    side: page.side,
                    pixels: &page.pixels
                }
            ),
            Err(horizon_legacy_gl::Error::InvalidPage)
        );
        assert_eq!(
            gpu.update_page(
                1,
                horizon_legacy_gl::Page {
                    side: 2,
                    pixels: &[0; 16]
                }
            ),
            Err(horizon_legacy_gl::Error::InvalidPage)
        );
        gpu.update_page(
            1,
            horizon_legacy_gl::Page {
                side: page.side,
                pixels: &page.pixels,
            },
        )
        .unwrap();
        clear(0x4000 | 0x0100);
        draw(
            &mut gpu,
            &ordered,
            horizon_legacy_gl::Pass::Solid { alpha_cutoff: 0.1 },
        );
        assert_eq!(rgb(sample(16)), [220, 220, 0]);

        // Near translucent blue blends over red; a later far solid stays hidden.
        let red = legacy::terrain::build(&atlas, &quad(-0.9, 0.9, 0., 0)).unwrap();
        let blue = legacy::terrain::build(&atlas, &quad(-0.9, 0.9, -0.5, 2)).unwrap();
        let behind = legacy::terrain::build(&atlas, &quad(-0.9, 0.9, 0.5, 1)).unwrap();
        clear(0x4000 | 0x0100);
        draw(
            &mut gpu,
            &red,
            horizon_legacy_gl::Pass::Solid { alpha_cutoff: 0.1 },
        );
        blend_equation(0x800b); // GL_FUNC_REVERSE_SUBTRACT
        draw(
            &mut gpu,
            &blue,
            horizon_legacy_gl::Pass::Translucent { alpha_cutoff: 0.1 },
        );
        draw(
            &mut gpu,
            &behind,
            horizon_legacy_gl::Pass::Solid { alpha_cutoff: 0.1 },
        );
        let actual = rgb(sample(16));
        assert!(
            actual[0].abs_diff(100) <= 2 && actual[1] <= 2 && actual[2].abs_diff(100) <= 2,
            "blend/depth: {actual:?}"
        );

        clear(0x4000 | 0x0100);
        draw(
            &mut gpu,
            &blue,
            horizon_legacy_gl::Pass::Solid { alpha_cutoff: 0.75 },
        );
        assert_eq!(rgb(sample(16)), [0, 0, 0], "cutout");

        // Lighting is evaluated on the CPU to avoid optional vertex texture fetch.
        let lightmap = [128u8; 16 * 16 * 4];
        let lit = legacy::terrain::build_with_lightmap(&atlas, &quad(-0.9, 0.9, 0., 0), &lightmap)
            .unwrap();
        clear(0x4000 | 0x0100);
        draw(
            &mut gpu,
            &lit,
            horizon_legacy_gl::Pass::Solid { alpha_cutoff: 0.1 },
        );
        assert!(rgb(sample(16))[0].abs_diff(100) <= 1, "CPU lightmap");

        // A GUI atlas wider than the GPU limit is split; UV-crossing triangles
        // are clipped and still render over existing world depth.
        let mut gui_pixels = Vec::new();
        for _ in 0..4 {
            for x in 0..8 {
                gui_pixels.extend_from_slice(if x < 4 {
                    &[200, 0, 0, 255]
                } else {
                    &[0, 200, 0, 255]
                });
            }
        }
        let mut gui_atlas = legacy::hud::Atlas::new(8, 4, &gui_pixels, 4).unwrap();
        let mut gui = horizon_legacy_gl::Renderer::new(|name| {
            let p = getproc(name.as_ptr());
            if p.is_null() {
                dlsym(handle, name.as_ptr())
            } else {
                p
            }
        })
        .unwrap();
        let pages: Vec<_> = gui_atlas
            .pages()
            .iter()
            .map(|p| horizon_legacy_gl::Page {
                side: p.side,
                pixels: &p.pixels,
            })
            .collect();
        gui.upload_pages(&pages).unwrap();
        let mut gui_batch = hud::HudBatch::new([32, 32]);
        gui_batch.quad(
            [[0., 0.], [32., 0.], [32., 32.], [0., 32.]],
            [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
            u32::MAX,
        );
        let gui_mesh = legacy::hud::build(&gui_atlas, &gui_batch, [32, 32]).unwrap();
        let gui_draw = |gui: &mut horizon_legacy_gl::Renderer| {
            gui.draw_ordered(&gui_mesh.vertices, &gui_mesh.draws(), &identity)
                .unwrap();
        };
        gui_draw(&mut gui);
        assert_eq!(rgb(sample(5)), [200, 0, 0]);
        assert_eq!(rgb(sample(26)), [0, 200, 0]);
        let dirty = gui_atlas
            .update_rect(3, 0, 2, 4, &[0, 0, 200, 255].repeat(8))
            .unwrap();
        assert_eq!(dirty, [0, 1]);
        for i in dirty {
            let p = &gui_atlas.pages()[i];
            gui.update_page(
                i,
                horizon_legacy_gl::Page {
                    side: p.side,
                    pixels: &p.pixels,
                },
            )
            .unwrap();
        }
        gui_draw(&mut gui);
        assert_eq!(rgb(sample(14)), [0, 0, 200]);
        assert_eq!(rgb(sample(18)), [0, 0, 200]);

        // Exercise the GUI invert and additive blend functions against a
        // known background using one white atlas texel.
        gui_atlas.update_rect(0, 0, 1, 1, &[255; 4]).unwrap();
        let p = &gui_atlas.pages()[0];
        gui.update_page(
            0,
            horizon_legacy_gl::Page {
                side: p.side,
                pixels: &p.pixels,
            },
        )
        .unwrap();
        let mut overlay = hud::HudBatch::new([32, 32]);
        overlay.quad(
            [[0., 0.], [32., 0.], [32., 32.], [0., 32.]],
            [[0.0625, 0.125]; 4],
            u32::MAX,
        );
        overlay.blends.push((0..6, hud::Blend::Invert));
        let invert = legacy::hud::build(&gui_atlas, &overlay, [32, 32]).unwrap();
        clear_color(0.25, 0.5, 0.75, 1.0);
        clear(0x4000 | 0x0100);
        gui.draw_ordered(&invert.vertices, &invert.draws(), &identity)
            .unwrap();
        let inverted = rgb(sample(16));
        assert!(
            inverted[0].abs_diff(191) <= 1
                && inverted[1].abs_diff(127) <= 1
                && inverted[2].abs_diff(64) <= 1,
            "HUD invert {inverted:?}"
        );
        // Different blend modes share one upload and must remain in input order.
        let mut mixed = hud::HudBatch::new([32, 32]);
        let positions = [[0., 0.], [32., 0.], [32., 32.], [0., 32.]];
        let uvs = [[0.0625, 0.125]; 4];
        mixed.quad(positions, uvs, 0xff00_00ff); // red
        mixed.quad(positions, uvs, u32::MAX); // invert to cyan
        mixed.quad(positions, uvs, 0xff00_0028); // add red
        mixed.blends.push((6..12, hud::Blend::Invert));
        mixed.blends.push((12..18, hud::Blend::Additive));
        let mixed = legacy::hud::build(&gui_atlas, &mixed, [32, 32]).unwrap();
        clear_color(0., 0., 0., 1.);
        clear(0x4000 | 0x0100);
        gui.draw_ordered(&mixed.vertices, &mixed.draws(), &identity)
            .unwrap();
        assert_eq!(rgb(sample(16)), [40, 255, 255], "HUD mixed blend order");
        gui.destroy();
        gpu.destroy();
        destroy(context);
        dlclose(handle);
        println!(
            "PASS: Rust GL renderer + CPU atlas/mesh, pages, update, depth, blending and cutout"
        );
    }
}
