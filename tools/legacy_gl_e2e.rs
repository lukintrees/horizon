//! Standalone developer integration test for the experimental GL 2.0/2.1 backend.
//! Uses Mesa OSMesa and minimal stand-ins for the game's mesher types so it can
//! run without downloading the full workspace's Cargo dependencies. The real
//! atlas/mesh conversion and GL renderer source files are used unchanged.
#![allow(dead_code, unused_imports)]
use std::{
    ffi::{CStr, c_char, c_void},
    mem, ptr,
};

mod fog {
    pub struct FogData {
        pub color: [f32; 4],
        pub environmental_start: f32,
        pub environmental_end: f32,
        pub render_distance_start: f32,
        pub render_distance_end: f32,
        pub sky_end: f32,
        pub cloud_end: f32,
    }
}
mod mesher {
    pub const OPAQUE_LAYER: u16 = 1 << 15;
    pub const UV_STEPS: f32 = 32768.0;
    pub const TILED_UV_STEPS: f32 = 32.0;
    #[derive(Clone, Copy)]
    pub struct Vertex {
        pub pos: [f32; 3],
        pub uv: [f32; 2],
        pub layer: u32,
        pub color: u32,
        pub light: u32,
        pub normal: [i8; 4],
        pub glint_uv: [f32; 2],
    }
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
        pub fn tiled(pos: [f32; 3], uv: [f32; 2], layer: u16, color: u32, light: u16) -> Self {
            Self::new(
                pos,
                uv.map(|v| (v * TILED_UV_STEPS).round().clamp(0.0, 65535.0) / UV_STEPS),
                layer,
                color,
                light,
            )
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
#[path = "../crates/horizon-client-render/src/overlay.rs"]
mod overlay;
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
    let gl_version = std::env::var("MESA_GL_VERSION_OVERRIDE").unwrap();
    let glsl_version = std::env::var("MESA_GLSL_VERSION_OVERRIDE").unwrap_or_default();
    assert!(matches!(
        (gl_version.as_str(), glsl_version.as_str()),
        ("2.0", "") | ("2.0", "110") | ("2.1", "120")
    ));
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
        let get_tex_parameter = function!(
            handle,
            c"glGetTexParameteriv",
            unsafe extern "system" fn(u32, u32, *mut i32)
        );
        let pixel_store = function!(
            handle,
            c"glPixelStorei",
            unsafe extern "system" fn(u32, i32)
        );
        let gen_buffers = function!(
            handle,
            c"glGenBuffers",
            unsafe extern "system" fn(i32, *mut u32)
        );
        let delete_buffers = function!(
            handle,
            c"glDeleteBuffers",
            unsafe extern "system" fn(i32, *const u32)
        );
        let bind_buffer = function!(handle, c"glBindBuffer", unsafe extern "system" fn(u32, u32));
        let buffer_data = function!(
            handle,
            c"glBufferData",
            unsafe extern "system" fn(u32, isize, *const c_void, u32)
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
        assert!(version.starts_with(&format!("{gl_version} ")));
        if !glsl_version.is_empty() {
            assert!(glsl.starts_with(if glsl_version == "110" {
                "1.10"
            } else {
                "1.20"
            }));
        }
        if gl_version == "2.0" {
            let extensions = CStr::from_ptr(get_string(0x1f03)).to_string_lossy();
            assert!(!extensions.split_ascii_whitespace().any(|e| e == "GL_ARB_pixel_buffer_object" || e == "GL_EXT_pixel_buffer_object"), "2.0 test requires PBO extensions disabled");
        }
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
        // GL 2.1 has core pixel unpack buffers. Ensure a buffer left bound by
        // another pass is unbound before passing a CPU pointer to TexImage.
        let mut unpack = 0;
        if gl_version == "2.1" {
            gen_buffers(1, &mut unpack);
            bind_buffer(0x88ec, unpack);
            buffer_data(0x88ec, 4096, ptr::null(), 0x88e0);
        }
        upload(&mut gpu, &atlas);
        if unpack != 0 {
            delete_buffers(1, &unpack);
        }
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
        // Exercise the same resident-section plan that accepts the game's
        // SectionMesh: opaque leaves bypass alpha cutout, ordinary cutout does not.
        let world_atlas = legacy::atlas::Atlas::new(1, 1, 4, &[0, 0, 200, 30]).unwrap();
        let mut world = legacy::world::World::new(world_atlas, [255; 1024]);
        let mut source = mesher::SectionMesh::default();
        source
            .solid
            .extend(quad(-0.95, -0.05, 0., mesher::OPAQUE_LAYER));
        source.solid.extend(quad(0.05, 0.95, 0., 0));
        world.upload((0, 0, 0), &source).unwrap();
        upload(&mut gpu, world.atlas());
        clear(0x4000 | 0x0100);
        let no_fog = horizon_legacy_gl::Fog {
            color: [1.; 4],
            ranges: [100., 101., 100., 101.],
            section_offset: [0.; 3],
        };
        for section in world
            .plan(&[(0, 0, 0)], [0.; 3], 0, identity, Some(no_fog))
            .unwrap()
        {
            gpu.draw_ordered_with_fog(
                section.vertices,
                &section.draws,
                &section.transform,
                section.fog.unwrap(),
            )
            .unwrap();
        }
        assert_eq!(
            rgb(sample(7)),
            [0, 0, 200],
            "opaque quad must ignore sprite alpha"
        );
        assert_eq!(rgb(sample(24)), [0, 0, 0], "ordinary cutout must discard");
        upload(&mut gpu, &atlas);

        let red = legacy::terrain::build(&atlas, &quad(-0.9, 0.9, 0., 0)).unwrap();
        // WGSL computes distances at vertices, then interpolates them. All four
        // corners are equidistant here, so expected values are independent of
        // the sample location and triangle diagonal.
        let mut fog = horizon_legacy_gl::Fog {
            color: [0., 0., 1., 1.],
            ranges: [2., 4., 100., 101.],
            section_offset: [0., 0., 2.],
        };
        let fogged = |gpu: &mut horizon_legacy_gl::Renderer, fog| {
            clear(0x4000 | 0x0100);
            gpu.draw_with_fog(
                &red.vertices,
                &red.batches,
                &identity,
                horizon_legacy_gl::Pass::Solid { alpha_cutoff: 0.1 },
                fog,
            )
            .unwrap();
            rgb(sample(16))
        };
        let spherical = fogged(&mut gpu, fog);
        let factor = ((5.62f32).sqrt() - 2.) / 2.;
        assert!(
            spherical[0].abs_diff((200. * (1. - factor)).round() as u8) <= 2
                && spherical[2].abs_diff((255. * factor).round() as u8) <= 2,
            "spherical fog {spherical:?}"
        );
        fog.ranges = [100., 101., 2., 3.];
        let cylindrical = fogged(&mut gpu, fog);
        let factor = (4.81f32).sqrt() - 2.;
        assert!(
            cylindrical[0].abs_diff((200. * (1. - factor)).round() as u8) <= 2
                && cylindrical[2].abs_diff((255. * factor).round() as u8) <= 2,
            "cylindrical fog {cylindrical:?}"
        );
        fog.ranges = [2., 2., 100., 101.]; // zero-width fog is a step, not a division by zero
        fog.color[3] = 0.5;
        let stepped = fogged(&mut gpu, fog);
        assert!(
            stepped[0].abs_diff(100) <= 1 && stepped[2].abs_diff(128) <= 1,
            "step fog {stepped:?}"
        );
        clear(0x4000 | 0x0100);
        draw(
            &mut gpu,
            &red,
            horizon_legacy_gl::Pass::Solid { alpha_cutoff: 0.1 },
        );
        assert_eq!(
            rgb(sample(16)),
            [200, 0, 0],
            "ordinary draws must reset fog"
        );

        let blue = legacy::terrain::build(&atlas, &quad(-0.9, 0.9, -0.5, 2)).unwrap();
        fog.ranges = [0., 0., 0., 0.];
        fog.color = [1., 1., 1., 1.];
        clear(0x4000 | 0x0100);
        gpu.draw_with_fog(
            &blue.vertices,
            &blue.batches,
            &identity,
            horizon_legacy_gl::Pass::Translucent { alpha_cutoff: 0.1 },
            fog,
        )
        .unwrap();
        let translucent_fog = rgb(sample(16));
        assert!(
            translucent_fog.iter().all(|c| c.abs_diff(128) <= 1),
            "fog must preserve texture alpha: {translucent_fog:?}"
        );
        clear(0x4000 | 0x0100);
        gpu.draw_with_fog(
            &red.vertices,
            &red.batches,
            &identity,
            horizon_legacy_gl::Pass::HudAlpha,
            fog,
        )
        .unwrap();
        assert_eq!(rgb(sample(16)), [200, 0, 0], "fog must not affect HUD");
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
        // Feed real-shaped entity quads through page clipping and face passes.
        let entity_quad = [
            ([-0.9, -0.9, 0.0], [0.0, 0.0]),
            ([0.9, -0.9, 0.0], [1.0, 0.0]),
            ([0.9, 0.9, 0.0], [1.0, 1.0]),
            ([-0.9, 0.9, 0.0], [0.0, 1.0]),
        ]
        .map(|(pos, uv)| mesher::Vertex {
            pos,
            uv,
            layer: 0,
            color: u32::MAX,
            light: 0,
            normal: [0, 127, 0, -127],
            glint_uv: [0.0; 2],
        });
        let front = legacy::entity::build(&gui_atlas, &entity_quad, &[255; 1024], false).unwrap();
        clear_color(0., 0., 0., 1.);
        clear(0x4000 | 0x0100);
        gui.draw_ordered(&front.vertices, &front.draws, &identity)
            .unwrap();
        assert_eq!(
            rgb(sample(5)),
            [200, 0, 0],
            "front-facing entity left atlas page"
        );
        assert_eq!(
            rgb(sample(26)),
            [0, 200, 0],
            "front-facing entity right atlas page"
        );
        let back_quad = [
            entity_quad[3],
            entity_quad[2],
            entity_quad[1],
            entity_quad[0],
        ];
        let back = legacy::entity::build(&gui_atlas, &back_quad, &[255; 1024], false).unwrap();
        clear(0x4000 | 0x0100);
        gui.draw_ordered(&back.vertices, &back.draws, &identity)
            .unwrap();
        assert_eq!(rgb(sample(5)), [80, 0, 0], "back-face directional shading");
        assert_eq!(rgb(sample(26)), [0, 80, 0], "back-face clipping");
        let mut culled_quad = back_quad;
        for v in &mut culled_quad {
            v.normal[3] = 1;
        }
        let culled = legacy::entity::build(&gui_atlas, &culled_quad, &[255; 1024], false).unwrap();
        clear(0x4000 | 0x0100);
        gui.draw_ordered(&culled.vertices, &culled.draws, &identity)
            .unwrap();
        assert_eq!(rgb(sample(5)), [0, 0, 0], "back-face culling");
        let mut hurt_quad = entity_quad;
        for v in &mut hurt_quad {
            v.layer = 1;
            v.normal = [0; 4];
        }
        let hurt = legacy::entity::build(&gui_atlas, &hurt_quad, &[128; 1024], false).unwrap();
        clear(0x4000 | 0x0100);
        gui.draw_ordered(&hurt.vertices, &hurt.draws, &identity)
            .unwrap();
        let hurt_pixel = rgb(sample(26));
        assert!(
            hurt_pixel[0].abs_diff(39) <= 1
                && hurt_pixel[1].abs_diff(70) <= 1
                && hurt_pixel[2] <= 1,
            "hurt overlay then lightmap: {hurt_pixel:?}"
        );
        for v in &mut hurt_quad {
            v.layer = 31;
        }
        let flash = legacy::entity::build(&gui_atlas, &hurt_quad, &[128; 1024], false).unwrap();
        clear(0x4000 | 0x0100);
        gui.draw_ordered(&flash.vertices, &flash.draws, &identity)
            .unwrap();
        let white_pixel = rgb(sample(26));
        assert!(
            white_pixel[0].abs_diff(96) <= 1
                && white_pixel[1].abs_diff(121) <= 1
                && white_pixel[2].abs_diff(96) <= 1,
            "white flash then lightmap: {white_pixel:?}"
        );
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
        // Vignette uses source RGB even at alpha zero, and preserves destination alpha.
        let mut vignette_batch = hud::HudBatch::new([32, 32]);
        vignette_batch.quad(
            [[0., 0.], [32., 0.], [32., 32.], [0., 32.]],
            [[0.0625, 0.125]; 4],
            0x0000_00ff,
        );
        vignette_batch.blends.push((0..6, hud::Blend::Vignette));
        let vignette = legacy::hud::build(&gui_atlas, &vignette_batch, [32, 32]).unwrap();
        clear_color(0.25, 0.5, 0.75, 0.5);
        clear(0x4000 | 0x0100);
        gui.draw_ordered(&vignette.vertices, &vignette.draws(), &identity)
            .unwrap();
        let mut filter = 0;
        get_tex_parameter(0x0de1, 0x2801, &mut filter);
        assert_eq!(filter, 0x2601, "vignette uses linear filtering");
        let darkened = sample(16);
        assert!(
            darkened[0] <= 1
                && darkened[1].abs_diff(128) <= 1
                && darkened[2].abs_diff(191) <= 1
                && darkened[3].abs_diff(128) <= 1,
            "HUD vignette {darkened:?}"
        );
        // Different blend modes share one upload and must remain in input order.
        gui.draw_ordered(&invert.vertices, &invert.draws(), &identity)
            .unwrap();
        get_tex_parameter(0x0de1, 0x2801, &mut filter);
        assert_eq!(filter, 0x2600, "ordinary HUD restores nearest filtering");
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
        // Two depth-tested quads, then a separately cleared preview with the
        // same scissor. Pixels outside the preview stay untouched.
        let mut depth = hud::HudBatch::new([32, 32]);
        depth.quad(positions, uvs, 0xff00_00ff);
        for v in &mut depth.vertices[0..6] {
            v.depth = 0.2;
        }
        depth.quad(positions, uvs, 0xff00_ff00);
        for v in &mut depth.vertices[6..12] {
            v.depth = 0.8;
        }
        depth
            .blends
            .push((0..12, hud::Blend::Depth([8, 8, 16, 16])));
        let first = legacy::hud::build(&gui_atlas, &depth, [32, 32]).unwrap();
        let invalid = [horizon_legacy_gl::Draw {
            batch: first.runs[0].batch,
            pass: horizon_legacy_gl::Pass::HudDepth {
                scissor: [0, 0, 0, 8],
                clear: true,
            },
        }];
        assert_eq!(
            gui.draw_ordered(&first.vertices, &invalid, &identity),
            Err(horizon_legacy_gl::Error::InvalidScissor)
        );
        clear_color(0.1, 0.2, 0.3, 1.);
        clear(0x4000 | 0x0100);
        gui.draw_ordered(&first.vertices, &first.draws(), &identity)
            .unwrap();
        assert_eq!(rgb(sample(16)), [255, 0, 0], "preview depth test");
        let outside = rgb(sample(4));
        assert!(
            outside[0].abs_diff(26) <= 1
                && outside[1].abs_diff(51) <= 1
                && outside[2].abs_diff(77) <= 1,
            "scissor {outside:?}"
        );
        depth.quad(positions, uvs, 0xffff_0000);
        for v in &mut depth.vertices[12..18] {
            v.depth = 0.9;
        }
        depth
            .blends
            .push((12..18, hud::Blend::Depth([8, 8, 16, 16])));
        let both = legacy::hud::build(&gui_atlas, &depth, [32, 32]).unwrap();
        gui.draw_ordered(&both.vertices, &both.draws(), &identity)
            .unwrap();
        assert_eq!(
            rgb(sample(16)),
            [0, 0, 255],
            "separate preview clears depth even for identical scissors"
        );
        assert_eq!(rgb(sample(4)), outside);
        // Drawing a plain HUD pass afterwards must not retain the scissor.
        gui.draw_ordered(&gui_mesh.vertices, &gui_mesh.draws(), &identity)
            .unwrap();
        assert_eq!(rgb(sample(4)), [200, 0, 0], "scissor leaked");
        gui.destroy();

        // Real tiled UV encoding on a merged face: a two-texel stripe must
        // repeat twice rather than stretching over the whole quad.
        let tiled_atlas = legacy::atlas::Atlas::new(
            2,
            1,
            8,
            &[
                200, 0, 0, 255, 0, 200, 0, 255, 200, 0, 0, 255, 0, 200, 0, 255,
            ],
        )
        .unwrap();
        let page = &tiled_atlas.pages()[0];
        gpu.upload_pages(&[horizon_legacy_gl::Page {
            side: page.side,
            pixels: &page.pixels,
        }])
        .unwrap();
        let tiled: Vec<_> = [
            ([-0.9, -0.9, 0.], [0., 0.25]),
            ([0.9, -0.9, 0.], [2., 0.25]),
            ([0.9, 0.9, 0.], [2., 0.25]),
            ([-0.9, 0.9, 0.], [0., 0.25]),
        ]
        .into_iter()
        .map(|(p, uv)| mesher::TerrainVertex::tiled(p, uv, 0, u32::MAX, 0))
        .collect();
        let tiled = legacy::terrain::build_tiled(&tiled_atlas, &tiled).unwrap();
        clear(0x4000 | 0x0100);
        draw(
            &mut gpu,
            &tiled,
            horizon_legacy_gl::Pass::Solid { alpha_cutoff: 0.1 },
        );
        assert_eq!(rgb(sample(4)), [200, 0, 0]);
        assert_eq!(rgb(sample(10)), [0, 200, 0]);
        assert_eq!(rgb(sample(20)), [200, 0, 0]);
        assert_eq!(rgb(sample(26)), [0, 200, 0]);
        gpu.destroy();
        destroy(context);
        dlclose(handle);
        println!(
            "PASS: Rust GL renderer + CPU atlas/mesh, pages, update, depth, blending, cutout and fog"
        );
    }
}
