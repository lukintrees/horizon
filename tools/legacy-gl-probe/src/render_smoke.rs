//! Exercises the reusable backend through its safe, backend-neutral API.
use horizon_legacy_render::{Budget, Context, Renderer, Stats};
use horizon_render_api::{Clear, Draw, Fog, Frame, IDENTITY, Image, Pass, RenderBackend, Vertex};

fn quad(color: [f32; 4], z: f32, extent: f32) -> [Vertex; 4] {
    [
        ([-extent, -extent, z], [0., 0.]),
        ([extent, -extent, z], [1., 0.]),
        ([extent, extent, z], [1., 1.]),
        ([-extent, extent, z], [0., 1.]),
    ]
    .map(|(pos, uv)| Vertex {
        pos,
        uv,
        light_uv: [0.5; 2],
        color,
    })
}
fn frame(draws: &[Draw]) -> Frame<'_> {
    Frame {
        view_projection: IDENTITY,
        camera: [0.; 3],
        clear: Clear::ColorDepth([0., 0., 0., 1.]),
        fog: None,
        draws,
    }
}
fn pixel(renderer: &mut Renderer<'_>, at: [u32; 2], expected: [u8; 3]) -> Result<(), String> {
    let actual = renderer.read_rgba([at[0], at[1], 1, 1])?;
    if actual[..3]
        .iter()
        .zip(expected)
        .any(|(&a, b)| a.abs_diff(b) > 2)
    {
        Err(format!(
            "backend readback {at:?}: expected {expected:?}, got {actual:?}"
        ))
    } else {
        Ok(())
    }
}
fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

pub fn run(context: &Context) -> Result<Vec<&'static str>, String> {
    let mut renderer = Renderer::new(context, Budget::default())?;
    let mut textures = Vec::new();
    let mut meshes = Vec::new();
    let indices = [0, 1, 2, 0, 2, 3];
    macro_rules! texture {
        ($w:expr, $h:expr, $pixels:expr) => {{
            let id = renderer.create_texture(Image {
                width: $w,
                height: $h,
                rgba: $pixels,
            })?;
            textures.push(id);
            id
        }};
    }
    macro_rules! mesh {
        ($v:expr) => {{
            let id = renderer.create_mesh(&$v, &indices)?;
            meshes.push(id);
            id
        }};
    }
    let white = texture!(1, 1, &[255; 4]);
    let blue = texture!(1, 1, &[0, 0, 255, 255]);
    let red = texture!(1, 1, &[255, 0, 0, 255]);
    let pattern = texture!(
        2,
        2,
        &[
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255
        ]
    );
    let base = mesh!(quad([1.; 4], 0., 1.));
    let draw = Draw {
        mesh: base,
        texture: pattern,
        lightmap: white,
        origin: [0.; 3],
        pass: Pass::Opaque,
        scissor: None,
    };
    renderer.render(&frame(&[draw]))?;
    for (at, rgb) in [
        ([16, 16], [255, 0, 0]),
        ([48, 16], [0, 255, 0]),
        ([16, 48], [0, 0, 255]),
        ([48, 48], [255; 3]),
    ] {
        pixel(&mut renderer, at, rgb)?;
    }
    let mut checks = vec!["backend_rgba_texture"];

    renderer.update_texture(
        pattern,
        [0, 0],
        Image {
            width: 1,
            height: 1,
            rgba: &[255, 255, 0, 255],
        },
    )?;
    require(
        renderer
            .update_texture(
                pattern,
                [1, 1],
                Image {
                    width: 2,
                    height: 2,
                    rgba: &[0; 16],
                },
            )
            .is_err(),
        "out-of-bounds update accepted",
    )?;
    renderer.render(&frame(&[draw]))?;
    pixel(&mut renderer, [16, 16], [255, 255, 0])?;
    pixel(&mut renderer, [48, 16], [0, 255, 0])?;
    checks.push("backend_texture_update");

    let green_near = mesh!(quad([0., 1., 0., 1.], -0.5, 1.));
    let red_far = mesh!(quad([1., 0., 0., 1.], 0.5, 1.));
    renderer.render(&frame(&[
        Draw {
            mesh: green_near,
            texture: white,
            ..draw
        },
        Draw {
            mesh: red_far,
            texture: white,
            ..draw
        },
    ]))?;
    pixel(&mut renderer, [32, 32], [0, 255, 0])?;
    checks.push("backend_depth");

    let front = mesh!(quad([1.; 4], -0.25, 1.));
    let mask = texture!(
        2,
        2,
        &[255, 0, 0, 0, 255, 0, 0, 255, 255, 0, 0, 0, 255, 0, 0, 255]
    );
    renderer.render(&frame(&[
        Draw {
            texture: blue,
            ..draw
        },
        Draw {
            mesh: front,
            texture: mask,
            pass: Pass::Cutout,
            ..draw
        },
    ]))?;
    pixel(&mut renderer, [16, 32], [0, 0, 255])?;
    pixel(&mut renderer, [48, 32], [255, 0, 0])?;
    checks.push("backend_cutout");

    let background = mesh!(quad([1.; 4], 0.75, 1.));
    let red_half = mesh!(quad([1., 0., 0., 0.5], 0., 1.));
    let green_half = mesh!(quad([0., 1., 0., 0.5], 0.25, 1.));
    renderer.render(&frame(&[
        Draw {
            mesh: background,
            texture: blue,
            ..draw
        },
        Draw {
            mesh: red_half,
            texture: white,
            pass: Pass::Translucent,
            ..draw
        },
        Draw {
            mesh: green_half,
            texture: white,
            pass: Pass::Translucent,
            ..draw
        },
    ]))?;
    pixel(&mut renderer, [32, 32], [64, 128, 64])?;
    checks.push("backend_ordered_blend_no_depth_write");

    renderer.render(&frame(&[
        Draw {
            mesh: base,
            texture: blue,
            ..draw
        },
        Draw {
            mesh: red_far,
            texture: white,
            pass: Pass::Overlay,
            scissor: Some([0, 0, 32, 64]),
            ..draw
        },
    ]))?;
    pixel(&mut renderer, [16, 32], [255, 0, 0])?;
    pixel(&mut renderer, [48, 32], [0, 0, 255])?;
    renderer.render(&frame(&[Draw {
        texture: blue,
        ..draw
    }]))?;
    pixel(&mut renderer, [16, 32], [0, 0, 255])?;
    checks.push("backend_overlay_scissor_reset");

    let light = texture!(1, 1, &[128, 255, 64, 255]);
    renderer.render(&frame(&[Draw {
        texture: white,
        lightmap: light,
        ..draw
    }]))?;
    pixel(&mut renderer, [32, 32], [128, 255, 64])?;
    checks.push("backend_lightmap");
    let fog_draws = [Draw {
        texture: white,
        ..draw
    }];
    let mut fog_frame = frame(&fog_draws);
    fog_frame.fog = Some(Fog {
        color: [0.25, 0.5, 0.75],
        start: 0.,
        end: 1.,
    });
    renderer.render(&fog_frame)?;
    pixel(&mut renderer, [32, 32], [64, 128, 191])?;
    checks.push("backend_fog");

    let small = mesh!(quad([1.; 4], 0., 0.2));
    let distant_draws = [Draw {
        mesh: small,
        texture: red,
        origin: [30_000_000., 0., 0.],
        ..draw
    }];
    let mut distant = frame(&distant_draws);
    distant.camera = [29_999_999.5, 0., 0.];
    renderer.render(&distant)?;
    pixel(&mut renderer, [48, 32], [255, 0, 0])?;
    pixel(&mut renderer, [32, 32], [0, 0, 0])?;
    checks.push("backend_large_coordinates");

    let stale = renderer.create_mesh(&quad([1.; 4], 0., 1.), &indices)?;
    renderer.remove_mesh(stale)?;
    require(
        renderer.remove_mesh(stale).is_err(),
        "double removal accepted",
    )?;
    require(
        renderer
            .render(&frame(&[Draw {
                mesh: stale,
                ..draw
            }]))
            .is_err(),
        "stale draw accepted",
    )?;
    pixel(&mut renderer, [48, 32], [255, 0, 0])?; // rejected frame did not even clear
    require(
        renderer
            .create_mesh(&quad([1.; 4], 0., 1.), &[4, 0, 1])
            .is_err(),
        "bad indices accepted",
    )?;
    require(
        renderer.read_rgba([63, 63, 2, 2]).is_err(),
        "bad readback accepted",
    )?;
    {
        let mut foreign = Renderer::new(context, Budget::default())?;
        require(
            foreign.render(&frame(&[draw])).is_err(),
            "foreign handles accepted",
        )?;
        let mut limited = Renderer::new(
            context,
            Budget {
                texture_bytes: 0,
                mesh_bytes: 0,
            },
        )?;
        require(
            limited
                .create_texture(Image {
                    width: 1,
                    height: 1,
                    rgba: &[255; 4],
                })
                .is_err(),
            "texture budget ignored",
        )?;
        require(
            limited
                .create_mesh(&quad([1.; 4], 0., 1.), &indices)
                .is_err(),
            "mesh budget ignored",
        )?;
        require(
            limited.stats() == Stats::default(),
            "failed uploads charged memory",
        )?;
    }
    checks.push("backend_validation_budget");

    // End-to-end shared terrain conversion: the game adapter supplies this exact
    // TerrainInput contract. No stand-in mesher types or Mojang assets are used.
    {
        use horizon_render_api::{
            atlas::{SpriteSize, build_page, pack_atlas},
            terrain::{TerrainInput, runs},
        };
        let layout = pack_atlas(
            &[SpriteSize {
                width: 2,
                height: 2,
            }; 2],
            4,
            1,
            2,
        )
        .map_err(|e| format!("{e:?}"))?;
        let red_pixels = [255, 0, 0, 255].repeat(4);
        let green_pixels = [0, 255, 0, 255].repeat(4);
        let first =
            build_page(&layout, 0, &[&red_pixels, &green_pixels]).map_err(|e| format!("{e:?}"))?;
        let second =
            build_page(&layout, 1, &[&red_pixels, &green_pixels]).map_err(|e| format!("{e:?}"))?;
        let pages = [texture!(4, 4, &first), texture!(4, 4, &second)];
        let left = quad([1.; 4], 0.5, 1.).map(|v| TerrainInput {
            pos: [(v.pos[0] - 1.) * 0.5, v.pos[1], v.pos[2]],
            uv: v.uv,
            layer: 0x8000,
            color: 0x8dff_ffff,
            light: 0xf0f0,
        });
        let right = quad([1.; 4], 0.5, 1.).map(|v| TerrainInput {
            pos: [(v.pos[0] + 1.) * 0.5, v.pos[1], v.pos[2]],
            uv: v.uv,
            layer: 1,
            color: 0xffff_ffff,
            light: 0xf0f0,
        });
        let runs =
            runs(&layout, left.into_iter().chain(right), false, 8).map_err(|e| format!("{e:?}"))?;
        let mut draws = Vec::new();
        for run in &runs {
            let id = renderer.create_mesh(&run.vertices, &run.indices)?;
            meshes.push(id);
            draws.push(Draw {
                mesh: id,
                texture: pages[run.page],
                pass: run.pass,
                ..draw
            });
        }
        let mut converted_frame = frame(&draws);
        converted_frame.view_projection = horizon_render_api::gl_projection(IDENTITY);
        renderer.render(&converted_frame)?;
        pixel(&mut renderer, [16, 32], [255, 0, 0])?;
        pixel(&mut renderer, [48, 32], [0, 255, 0])?;
    }
    checks.push("backend_terrain_atlas_contract");
    scene_lifecycle(context)?;
    checks.push("backend_resident_sections");
    gui_compositor(context)?;
    checks.push("backend_gui_compositor");
    paged_gui_compositor(context)?;
    checks.push("backend_paged_gui");
    paged_entities(context)?;
    checks.push("backend_paged_entities");
    entity_layers(context)?;
    checks.push("backend_entity_layers");

    // Switch to an unrelated context, then alternate both renderers. Every safe
    // operation must bind the correct context, rather than use thread-local luck.
    let kind = if context.description().starts_with("OSMesa") {
        Some("osmesa")
    } else if context.description().starts_with("EGL") {
        Some("egl")
    } else {
        None
    };
    if let Some(kind) = kind {
        let other_context = Context::new(kind, [64, 64])?;
        let mut other = Renderer::new(&other_context, Budget::default())?;
        let mut empty = frame(&[]);
        empty.clear = Clear::ColorDepth([0., 1., 0., 1.]);
        other.render(&empty)?;
        renderer.render(&frame(&[Draw {
            texture: blue,
            ..draw
        }]))?;
        pixel(&mut renderer, [32, 32], [0, 0, 255])?;
        pixel(&mut other, [32, 32], [0, 255, 0])?;
        checks.push("backend_context_isolation");
    }
    for id in meshes {
        renderer.remove_mesh(id)?;
    }
    for id in textures {
        renderer.remove_texture(id)?;
    }
    require(
        renderer.stats() == Stats::default(),
        "resource removal did not reclaim counters",
    )?;
    checks.push("backend_resource_reclamation");
    Ok(checks)
}

fn scene_lifecycle(context: &Context) -> Result<(), String> {
    use horizon_render_api::{
        atlas::{SpriteSize, build_page, pack_atlas},
        scene::{SceneError, SceneFrame, WorldScene},
        terrain::{TerrainInput, runs},
    };
    let error = |e| format!("scene: {e:?}");
    let layout = pack_atlas(
        &[SpriteSize {
            width: 2,
            height: 2,
        }; 2],
        4,
        1,
        2,
    )
    .map_err(|e| format!("{e:?}"))?;
    let red = [255, 0, 0, 255].repeat(4);
    let green = [0, 255, 0, 255].repeat(4);
    let pages = [
        build_page(&layout, 0, &[&red, &green]),
        build_page(&layout, 1, &[&red, &green]),
    ]
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("{e:?}"))?;
    let pages = pages.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let white = [255; 16 * 16 * 4];
    let convert = |layer| {
        runs(
            &layout,
            quad([1.; 4], 0.5, 1.).map(|v| TerrainInput {
                pos: v.pos,
                uv: v.uv,
                layer,
                color: 0xffff_ffff,
                light: 0xf0f0,
            }),
            false,
            4,
        )
        .unwrap()
    };
    let red_run = convert(0x8000);
    let green_run = convert(0x8001);
    let one_mesh_bytes = 4 * std::mem::size_of::<Vertex>() + 6 * 2;
    let mut renderer = Renderer::new(
        context,
        Budget {
            mesh_bytes: one_mesh_bytes * 2,
            ..Budget::default()
        },
    )?;
    let key = [1_875_000, 0, 0];
    let mut scene =
        WorldScene::new(&mut renderer, layout.clone(), &pages, &white, 1).map_err(error)?;
    let mut frame = SceneFrame {
        view_projection: IDENTITY,
        camera: [30_000_000.25, 0., 0.],
        clear: [0., 0., 0., 1.],
        fog: None,
        solid: &[key],
        translucent: &[],
    };
    let check =
        |scene: &mut WorldScene<&mut Renderer<'_>>, at, expected: [u8; 3]| -> Result<(), String> {
            let actual = scene.read_rgba([at, 32, 1, 1]).map_err(error)?;
            require(
                actual[..3]
                    .iter()
                    .zip(expected)
                    .all(|(&a, b)| a.abs_diff(b) <= 2),
                &format!("resident scene pixel: {actual:?}, expected {expected:?}"),
            )
        };
    scene.replace_section(key, &red_run, &[]).map_err(error)?;
    scene.render(&frame).map_err(error)?;
    check(&mut scene, 32, [255, 0, 0])?;
    check(&mut scene, 54, [255, 0, 0])?;
    check(&mut scene, 58, [0, 0, 0])?;
    let double = convert(0x8001)
        .into_iter()
        .chain(convert(0x8000))
        .collect::<Vec<_>>();
    require(
        matches!(
            scene.replace_section(key, &double, &[]),
            Err(SceneError::Backend(_))
        ),
        "replacement must fail halfway through upload at the mesh budget",
    )?;
    scene.render(&frame).map_err(error)?;
    check(&mut scene, 32, [255, 0, 0])?;
    require(
        matches!(
            scene.replace_section([0; 3], &red_run, &[]),
            Err(SceneError::SectionLimit)
        ),
        "resident section ceiling was not enforced",
    )?;
    scene.replace_section(key, &green_run, &[]).map_err(error)?;
    scene.render(&frame).map_err(error)?;
    check(&mut scene, 32, [0, 255, 0])?;
    scene
        .update_lightmap(&[128, 128, 128, 255].repeat(256))
        .map_err(error)?;
    scene.render(&frame).map_err(error)?;
    check(&mut scene, 32, [0, 128, 0])?;
    frame.solid = &[[0; 3]];
    frame.clear = [0., 0., 1., 1.];
    require(
        matches!(scene.render(&frame), Err(SceneError::MissingSection(_))),
        "missing section accepted",
    )?;
    check(&mut scene, 32, [0, 128, 0])?; // failed frame did not clear
    require(
        scene.unload_section(key).map_err(error)?,
        "resident section not unloaded",
    )?;
    frame.solid = &[];
    frame.clear = [0., 0., 0., 1.];
    scene.render(&frame).map_err(error)?;
    check(&mut scene, 32, [0, 0, 0])?;
    drop(scene);
    require(
        renderer.stats() == Stats::default(),
        "scene left resident resources after drop",
    )?;
    let mut small = Renderer::new(
        context,
        Budget {
            texture_bytes: 64,
            ..Budget::default()
        },
    )?;
    require(
        WorldScene::new(&mut small, layout.clone(), &pages, &white, 1).is_err(),
        "constructor texture budget was not enforced",
    )?;
    require(
        small.stats() == Stats::default(),
        "failed scene constructor leaked atlas pages",
    )
}

fn gui_compositor(context: &Context) -> Result<(), String> {
    use horizon_render_api::gui::{self, Blend, Input};
    fn error(e: impl std::fmt::Debug) -> String {
        format!("GUI: {e:?}")
    }
    let mut renderer = Renderer::new(context, Budget::default())?;
    let white = renderer.create_texture(Image {
        width: 1,
        height: 1,
        rgba: &[255; 4],
    })?;
    let make = |color, depth| {
        let p = [[8., 8.], [56., 8.], [56., 56.], [8., 56.]];
        [0, 1, 2, 0, 2, 3].map(|i| Input {
            pos: p[i],
            uv: [0.5; 2],
            color,
            depth,
        })
    };
    let convert = |data: &[Input], blend| {
        gui::runs(data, &[(0..data.len(), blend)], [64; 2], 1., [1.; 2], 100).map_err(error)
    };
    let mut world = frame(&[]);
    world.clear = Clear::ColorDepth([64. / 255., 128. / 255., 192. / 255., 1.]);
    renderer.render(&world)?;
    gui::render(
        &mut renderer,
        &convert(&make(u32::MAX, 0.), Blend::Invert)?,
        white,
        white,
    )
    .map_err(error)?;
    pixel(&mut renderer, [32, 32], [191, 127, 63])?;
    pixel(&mut renderer, [4, 4], [64, 128, 192])?;
    gui::render(
        &mut renderer,
        &convert(&make(0xff28_1e14, 0.), Blend::Additive)?,
        white,
        white,
    )
    .map_err(error)?;
    pixel(&mut renderer, [32, 32], [211, 157, 103])?;
    // Vignette ignores sampled/tint alpha and preserves destination alpha.
    gui::render(
        &mut renderer,
        &convert(&make(0x0000_4080, 0.), Blend::Vignette)?,
        white,
        white,
    )
    .map_err(error)?;
    pixel(&mut renderer, [32, 32], [105, 118, 103])?;
    require(
        renderer.read_rgba([32, 32, 1, 1])?[3] == 255,
        "vignette changed destination alpha",
    )?;
    let outside = renderer.read_rgba([16, 32, 1, 1])?;
    let model = make(0xff00_00ff, 0.2)
        .into_iter()
        .chain(make(0xff00_ff00, 0.8))
        .chain(make(0xffff_0000, 0.2))
        .collect::<Vec<_>>();
    gui::render(
        &mut renderer,
        &convert(&model, Blend::Depth([20, 20, 24, 24]))?,
        white,
        white,
    )
    .map_err(error)?;
    pixel(&mut renderer, [32, 32], [0, 0, 255])?; // equal-depth blue passes LEQUAL
    require(
        renderer.read_rgba([16, 32, 1, 1])? == outside,
        "GUI depth scissor leaked",
    )?;
    gui::render(
        &mut renderer,
        &convert(&make(0xff00_ff00, 0.9), Blend::Depth([20, 20, 24, 24]))?,
        white,
        white,
    )
    .map_err(error)?;
    pixel(&mut renderer, [32, 32], [0, 255, 0])?; // new preview has isolated depth
    require(renderer.stats().meshes == 0, "GUI transient meshes leaked")?;
    let gradient = renderer.create_texture(Image {
        width: 2,
        height: 1,
        rgba: &[0, 0, 0, 0, 255, 255, 255, 0],
    })?;
    world.clear = Clear::ColorDepth([1.; 4]);
    renderer.render(&world)?;
    let full = [[0., 0.], [64., 0.], [64., 64.], [0., 64.]];
    let uv = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
    let linear = [0, 1, 2, 0, 2, 3].map(|i| Input {
        pos: full[i],
        uv: uv[i],
        color: u32::MAX,
        depth: 0.,
    });
    gui::render(
        &mut renderer,
        &convert(&linear, Blend::Vignette)?,
        gradient,
        white,
    )
    .map_err(error)?;
    pixel(&mut renderer, [32, 32], [124; 3])?;
    renderer.remove_texture(gradient)?;
    let (padded, uv_scale) =
        gui::upload_atlas(&mut renderer, [3, 3], &[255, 0, 0, 255].repeat(9), 64).map_err(error)?;
    let patch = gui::prepare_patch(
        [3, 3],
        [2, 2],
        Image {
            width: 1,
            height: 1,
            rgba: &[0, 0, 255, 255],
        },
        16,
    )
    .map_err(error)?;
    renderer.update_texture(
        padded,
        patch.at,
        Image {
            width: patch.size[0],
            height: patch.size[1],
            rgba: &patch.rgba,
        },
    )?;
    let edge = make(u32::MAX, 0.).map(|mut v| {
        v.uv = [1.; 2];
        v
    });
    let edge =
        gui::runs(&edge, &[(0..6, Blend::Alpha)], [64; 2], 1., uv_scale, 6).map_err(error)?;
    gui::render(&mut renderer, &edge, padded, white).map_err(error)?;
    pixel(&mut renderer, [32, 32], [0, 0, 255])?;
    renderer.remove_texture(padded)?;
    renderer.remove_texture(white)?;
    require(
        renderer.stats() == Stats::default(),
        "GUI resources not reclaimed",
    )?;

    // A failed multi-run upload must not paint a partial GUI over the world.
    let mut small = Renderer::new(
        context,
        Budget {
            mesh_bytes: 6 * 44 + 6 * 2,
            ..Budget::default()
        },
    )?;
    let white = small.create_texture(Image {
        width: 1,
        height: 1,
        rgba: &[255; 4],
    })?;
    world.clear = Clear::ColorDepth([0., 0., 1., 1.]);
    small.render(&world)?;
    let mut multiple = convert(&make(u32::MAX, 0.), Blend::Alpha)?;
    multiple.extend(convert(&make(u32::MAX, 0.), Blend::Invert)?);
    require(
        gui::render(&mut small, &multiple, white, white).is_err(),
        "GUI mesh budget not enforced",
    )?;
    pixel(&mut small, [32, 32], [0, 0, 255])?;
    require(
        small.stats().meshes == 0,
        "failed GUI upload leaked staging meshes",
    )?;
    small.remove_texture(white)?;
    Ok(())
}

fn paged_gui_compositor(context: &Context) -> Result<(), String> {
    use horizon_render_api::{
        gui::{self, Blend, Input},
        paged_gui::{self, Cache, Layout},
    };
    fn error(e: impl std::fmt::Debug) -> String {
        format!("paged GUI: {e:?}")
    }
    // Three source tiles, but space for only one GPU tile plus white lightmap.
    let mut renderer = Renderer::new(
        context,
        Budget {
            texture_bytes: 4 * 4 * 4 + 4,
            ..Budget::default()
        },
    )?;
    let white = renderer.create_texture(Image {
        width: 1,
        height: 1,
        rgba: &[255; 4],
    })?;
    let layout = Layout::new([6, 2], 4, 3).map_err(error)?;
    let row = [
        255, 0, 0, 128, 255, 0, 0, 128, 0, 255, 0, 128, 0, 255, 0, 128, 0, 0, 255, 128, 0, 0, 255,
        128,
    ];
    let mut pixels = row.repeat(2);
    let mut cache = Cache::new(layout, 1).map_err(error)?;
    let mut clear = frame(&[]);
    clear.clear = Clear::ColorDepth([40. / 255., 40. / 255., 40. / 255., 1.]);
    let p = [[2., 2.], [62., 2.], [62., 62.], [2., 62.]];
    let uv = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
    let vertices = [0, 1, 2, 0, 2, 3].map(|i| Input {
        pos: p[i],
        uv: uv[i],
        color: u32::MAX,
        depth: 0.,
    });
    let runs =
        gui::runs(&vertices, &[(0..6, Blend::Alpha)], [64; 2], 1., [1.; 2], 6).map_err(error)?;
    let runs = paged_gui::split(layout, &runs, 100).map_err(error)?;
    renderer.render(&clear)?;
    cache
        .render(&mut renderer, &runs, white, |page| {
            layout.pixels(page, &pixels)
        })
        .map_err(error)?;
    // Check every pixel across both seams: holes or duplicate translucent draws
    // would produce 40 or ~200 rather than ~148 in the selected channel.
    for x in 2..62 {
        let mut expected = [20; 3];
        expected[((x - 2) / 20) as usize] = 148;
        pixel(&mut renderer, [x, 32], expected)?;
    }
    require(
        renderer.stats().texture_bytes == 68 && renderer.stats().meshes == 0,
        "paged GUI exceeded resident budget",
    )?;
    let mut layered = Vec::new();
    for u in [0.1, 0.9] {
        layered.extend([0, 1, 2, 0, 2, 3].map(|i| Input {
            pos: p[i],
            uv: [u, 0.5],
            color: u32::MAX,
            depth: 0.,
        }));
    }
    let runs =
        gui::runs(&layered, &[(0..12, Blend::Alpha)], [64; 2], 1., [1.; 2], 12).map_err(error)?;
    let runs = paged_gui::split(layout, &runs, 100).map_err(error)?;
    renderer.render(&clear)?;
    cache
        .render(&mut renderer, &runs, white, |page| {
            layout.pixels(page, &pixels)
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [74, 10, 138])?;
    // The last blue page is resident. Dirty it in CPU backing and make sure the
    // next frame cannot reuse the stale GPU texture.
    for rgba in pixels.chunks_exact_mut(4).skip(4).step_by(6) {
        rgba.copy_from_slice(&[255, 255, 0, 255]);
    }
    for rgba in pixels.chunks_exact_mut(4).skip(5).step_by(6) {
        rgba.copy_from_slice(&[255, 255, 0, 255]);
    }
    cache.invalidate(&[2]);
    let last = std::slice::from_ref(runs.last().unwrap());
    renderer.render(&clear)?;
    cache
        .render(&mut renderer, last, white, |page| {
            layout.pixels(page, &pixels)
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [255, 255, 0])?;
    // Linear sampling must be continuous across each gutter, not clamp to the
    // last interior texel. Vignette's sampled alpha is deliberately zero.
    let shades = [0, 32, 96, 160, 224, 255];
    pixels = shades
        .into_iter()
        .flat_map(|v| [v, v, v, 0])
        .collect::<Vec<_>>()
        .repeat(2);
    cache.invalidate(&[0, 1, 2]);
    let vertices = [0, 1, 2, 0, 2, 3].map(|i| Input {
        pos: p[i],
        uv: uv[i],
        color: u32::MAX,
        depth: 0.,
    });
    let linear = gui::runs(
        &vertices,
        &[(0..6, Blend::Vignette)],
        [64; 2],
        1.,
        [1.; 2],
        6,
    )
    .map_err(error)?;
    let linear = paged_gui::split(layout, &linear, 100).map_err(error)?;
    clear.clear = Clear::ColorDepth([1.; 4]);
    renderer.render(&clear)?;
    cache
        .render(&mut renderer, &linear, white, |page| {
            layout.pixels(page, &pixels)
        })
        .map_err(error)?;
    for x in 2..62 {
        let coordinate = ((x as f32 + 0.5 - 2.) / 60. * 6. - 0.5).clamp(0., 5.);
        let lo = coordinate.floor() as usize;
        let hi = (lo + 1).min(5);
        let shade =
            f32::from(shades[lo]) + f32::from(shades[hi] - shades[lo]) * (coordinate - lo as f32);
        pixel(&mut renderer, [x, 32], [(255. - shade).round() as u8; 3])?;
    }
    // Evicting a page is not a new depth group: the far blue model must stay
    // behind the near red one even when the two textures cannot coexist.
    pixels = row.repeat(2);
    for rgba in pixels.chunks_exact_mut(4) {
        rgba[3] = 255;
    }
    cache.invalidate(&[0, 1, 2]);
    let mut model = Vec::new();
    for (u, depth) in [(0.1, 0.2), (0.9, 0.8)] {
        model.extend([0, 1, 2, 0, 2, 3].map(|i| Input {
            pos: p[i],
            uv: [u, 0.5],
            color: u32::MAX,
            depth,
        }));
    }
    let model = gui::runs(
        &model,
        &[(0..12, Blend::Depth([0, 0, 64, 64]))],
        [64; 2],
        1.,
        [1.; 2],
        12,
    )
    .map_err(error)?;
    let model = paged_gui::split(layout, &model, 100).map_err(error)?;
    renderer.render(&clear)?;
    cache
        .render(&mut renderer, &model, white, |page| {
            layout.pixels(page, &pixels)
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [255, 0, 0])?;
    cache.release(&mut renderer)?;
    renderer.remove_texture(white)?;
    require(
        renderer.stats() == Stats::default(),
        "paged GUI cache did not release its textures",
    )
}

fn paged_entities(context: &Context) -> Result<(), String> {
    use horizon_render_api::{
        entity::{self, Input},
        paged_gui::{self, Cache, Layout, WorldView},
    };
    fn error(e: impl std::fmt::Debug) -> String {
        format!("paged entities: {e:?}")
    }
    let mut renderer = Renderer::new(
        context,
        Budget {
            texture_bytes: 68,
            ..Budget::default()
        },
    )?;
    let white = renderer.create_texture(Image {
        width: 1,
        height: 1,
        rgba: &[255; 4],
    })?;
    let layout = Layout::new([6, 2], 4, 3).map_err(error)?;
    let mut cache = Cache::new(layout, 1).map_err(error)?;
    let row = [
        255, 0, 0, 255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 255,
        255,
    ];
    let mut pixels = row.repeat(2);
    let p = [
        [-0.9, -0.9, 0.],
        [0.9, -0.9, 0.],
        [0.9, 0.9, 0.],
        [-0.9, 0.9, 0.],
    ];
    let uv = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
    let quad = std::array::from_fn::<_, 4, _>(|i| Input {
        pos: p[i],
        uv: uv[i],
        layer: 0,
        color: u32::MAX,
        light: 0xf0f0,
        normal: [0, 127, 0, -127],
    });
    let mut view = WorldView {
        view_projection: IDENTITY,
        camera: [30_000_000.; 3],
        fog: None,
        lightmap: white,
    };
    let clear = frame(&[]);
    let make = |input: &[Input], light: &[u8]| -> Result<_, String> {
        let runs = entity::runs(input.iter().copied(), light, false, 100).map_err(error)?;
        paged_gui::split_entities(layout, &runs, 300).map_err(error)
    };
    let runs = make(&quad, &[255; 1024])?;
    renderer.render(&clear)?;
    cache
        .render_world(&mut renderer, &runs, &view, |page| {
            layout.pixels(page, &pixels)
        })
        .map_err(error)?;
    for (x, expected) in [(10, [255, 0, 0]), (32, [0, 255, 0]), (54, [0, 0, 255])] {
        pixel(&mut renderer, [x, 32], expected)?;
    }
    let mut back = quad;
    back.reverse();
    let runs = make(&back, &[255; 1024])?;
    renderer.render(&clear)?;
    cache
        .render_world(&mut renderer, &runs, &view, |page| {
            layout.pixels(page, &pixels)
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [0, 102, 0])?;
    for v in &mut back {
        v.normal[3] = -126;
    }
    let runs = make(&back, &[255; 1024])?;
    renderer.render(&clear)?;
    cache
        .render_world(&mut renderer, &runs, &view, |page| {
            layout.pixels(page, &pixels)
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [0, 0, 0])?;
    pixels = [64, 128, 192, 255].repeat(12);
    cache.invalidate(&[0, 1, 2]);
    let hurt = quad.map(|v| Input {
        layer: 1,
        normal: [0; 4],
        ..v
    });
    let light = [128, 255, 64, 255].repeat(256);
    let runs = make(&hurt, &light)?;
    renderer.render(&clear)?;
    cache
        .render_world(&mut renderer, &runs, &view, |page| {
            layout.pixels(page, &pixels)
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [61, 89, 34])?;
    // A constant-X world plane remains visible after rotating the camera. Its
    // XY area is zero, but its 3D triangles must not be discarded by UV clipping.
    pixels = row.repeat(2);
    cache.invalidate(&[0, 1, 2]);
    let wall = std::array::from_fn::<_, 4, _>(|i| Input {
        pos: [0., p[i][1], p[i][0]],
        normal: [0; 4],
        ..quad[i]
    });
    view.view_projection = [
        0., 0., -1., 0., 0., 1., 0., 0., 1., 0., 0., 0., 0., 0., 0., 1.,
    ];
    let runs = make(&wall, &[255; 1024])?;
    renderer.render(&clear)?;
    cache
        .render_world(&mut renderer, &runs, &view, |page| {
            layout.pixels(page, &pixels)
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [0, 255, 0])?;
    view.view_projection = IDENTITY;
    // Lightmap samples are interpolated per vertex, not nearest-sampled per
    // fragment. The same gradient must survive all three clipped texture pages.
    pixels = [255; 4].repeat(12);
    cache.invalidate(&[0, 1, 2]);
    let mut light = vec![0; 1024];
    for c in light.chunks_exact_mut(4) {
        c[3] = 255;
    }
    light[1020..].fill(255);
    let gradient = std::array::from_fn::<_, 4, _>(|i| Input {
        normal: [0; 4],
        light: if i < 2 { 0 } else { 0xf0f0 },
        ..quad[i]
    });
    let runs = make(&gradient, &light)?;
    renderer.render(&clear)?;
    cache
        .render_world(&mut renderer, &runs, &view, |page| {
            layout.pixels(page, &pixels)
        })
        .map_err(error)?;
    for x in [10, 32, 54] {
        pixel(&mut renderer, [x, 32], [130; 3])?;
    }
    // Ordinary/translucent items write depth; translucent particles do not.
    pixels = row.repeat(2);
    for rgba in pixels.chunks_exact_mut(4) {
        rgba[3] = 128;
    }
    cache.invalidate(&[0, 1, 2]);
    for (layer, expected) in [(32, [128, 0, 0]), (15, [64, 0, 128])] {
        let mut pair = Vec::new();
        for (u, z) in [(0.1, -0.3), (0.9, 0.3)] {
            pair.extend(quad.map(|v| Input {
                pos: [v.pos[0], v.pos[1], z],
                uv: [u, 0.5],
                normal: [0; 4],
                layer,
                ..v
            }));
        }
        let runs = make(&pair, &[255; 1024])?;
        renderer.render(&clear)?;
        cache
            .render_world(&mut renderer, &runs, &view, |page| {
                layout.pixels(page, &pixels)
            })
            .map_err(error)?;
        pixel(&mut renderer, [32, 32], expected)?;
    }
    // Wrong mesh types and malformed uniforms must fail before the clear, not
    // reinterpret 44-byte terrain as a 76-byte entity buffer on the GPU.
    let base = horizon_render_api::Vertex {
        pos: [0.; 3],
        uv: [0.5; 2],
        light_uv: [0.5; 2],
        color: [1.; 4],
    };
    let mesh = renderer.create_mesh(&[base; 3], &[0, 1, 2])?;
    let material = entity::Material {
        overlay: entity::Overlay::None,
        cull: false,
        kind: entity::Kind::Body,
    };
    let mut bad = frame(&[]);
    bad.clear = Clear::ColorDepth([1., 1., 0., 1.]);
    let draw = Draw {
        mesh,
        texture: white,
        lightmap: white,
        origin: [0.; 3],
        pass: Pass::Entity(material),
        scissor: None,
    };
    bad.draws = std::slice::from_ref(&draw);
    require(
        renderer.render(&bad).is_err(),
        "entity pass accepted a terrain buffer",
    )?;
    pixel(&mut renderer, [32, 32], [64, 0, 128])?;
    renderer.remove_mesh(mesh)?;
    let mut vertex = entity::LitVertex {
        base,
        back_color: [1.; 4],
        light_color: [1.; 4],
    };
    vertex.light_color[0] = f32::NAN;
    require(
        renderer
            .create_entity_mesh(&[vertex; 3], &[0, 1, 2])
            .is_err(),
        "nonfinite entity light accepted",
    )?;
    vertex.light_color = [1.; 4];
    let mesh = renderer.create_entity_mesh(&[vertex; 3], &[0, 1, 2])?;
    let draw = Draw {
        mesh,
        pass: Pass::Entity(entity::Material {
            overlay: entity::Overlay::White(16),
            ..material
        }),
        ..draw
    };
    let mut bad = frame(std::slice::from_ref(&draw));
    bad.clear = Clear::ColorDepth([1., 1., 0., 1.]);
    require(
        renderer.render(&bad).is_err(),
        "invalid entity overlay accepted",
    )?;
    pixel(&mut renderer, [32, 32], [64, 0, 128])?;
    renderer.remove_mesh(mesh)?;
    require(
        renderer.stats().mesh_bytes == 0 && renderer.stats().texture_bytes == 68,
        "entity stream exceeded capacity or retained transient meshes",
    )?;
    cache.release(&mut renderer)?;
    renderer.remove_texture(white)?;
    require(
        renderer.stats() == Stats::default(),
        "entity stream cleanup leaked resources",
    )
}

fn entity_layers(context: &Context) -> Result<(), String> {
    use horizon_render_api::{
        entity::{self, Input},
        paged_gui::{self, Cache, Layout, WorldView},
    };
    fn error(e: impl std::fmt::Debug) -> String {
        format!("entity layers: {e:?}")
    }
    let mut renderer = Renderer::new(context, Budget::default())?;
    let white = renderer.create_texture(Image {
        width: 1,
        height: 1,
        rgba: &[255; 4],
    })?;
    let clear_rgba = renderer.create_texture(Image {
        width: 1,
        height: 1,
        rgba: &[0; 4],
    })?;
    let layout = Layout::new([1, 1], 4, 1).map_err(error)?;
    let mut cache = Cache::new(layout, 1).map_err(error)?;
    let view = WorldView {
        view_projection: IDENTITY,
        camera: [0.; 3],
        fog: None,
        lightmap: white,
    };
    let input = |layer, z, color| {
        quad([1.; 4], z, 0.9).map(|v| Input {
            pos: v.pos,
            uv: [0.5; 2],
            layer,
            color,
            light: 0xf0f0,
            normal: [0; 4],
        })
    };
    let prepare = |vertices: [Input; 4]| -> Result<_, String> {
        let runs = entity::runs(vertices, &[255; 1024], false, 4).map_err(error)?;
        paged_gui::split_entities(layout, &runs, 32).map_err(error)
    };
    let mut clear = frame(&[]);
    clear.clear = Clear::ColorDepth([1.; 4]);
    renderer.render(&clear)?;
    let shadow = prepare(input(14, -0.5, u32::MAX))?;
    cache
        .render_world(&mut renderer, &shadow, &view, |p| {
            layout.pixels(p, &[0, 0, 0, 8])
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [247; 3])?; // alpha 8/255 must not hit entity cutout
    // It darkens colour but must not write a near depth that hides later items.
    let red = prepare(input(32, 0.5, 0xff0000ff))?;
    cache.invalidate(&[0]);
    cache
        .render_world(&mut renderer, &red, &view, |p| layout.pixels(p, &[255; 4]))
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [255, 0, 0])?;
    let mut back = input(14, 0., u32::MAX);
    back.reverse();
    let back = prepare(back)?;
    renderer.render(&clear)?;
    cache.invalidate(&[0]);
    cache
        .render_world(&mut renderer, &back, &view, |p| {
            layout.pixels(p, &[0, 0, 0, 128])
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [255; 3])?;
    // A mask has no dependency on atlas colour/alpha or a resident page. Bind
    // a transparent texture as its otherwise-unused image to prove no discard.
    cache.release(&mut renderer)?;
    let mask = prepare(input(13, 0., 0))?;
    let mask_view = WorldView {
        lightmap: clear_rgba,
        ..view
    };
    let mut background = frame(&[]);
    background.clear = Clear::ColorDepth([32. / 255., 64. / 255., 96. / 255., 1.]);
    renderer.render(&background)?;
    cache
        .render_world(&mut renderer, &mask, &mask_view, |_| {
            Err::<Vec<u8>, _>("mask requested atlas pixels")
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [32, 64, 96])?;
    let indices = [0, 1, 2, 0, 2, 3];
    let blue = renderer.create_mesh(&quad([0., 0., 1., 1.], 0.3, 0.9), &indices)?;
    let red_mesh = renderer.create_mesh(&quad([1., 0., 0., 1.], -0.3, 0.9), &indices)?;
    let draw = |mesh| Draw {
        mesh,
        texture: white,
        lightmap: white,
        origin: [0.; 3],
        pass: Pass::Opaque,
        scissor: None,
    };
    let blue_draw = draw(blue);
    let red_draw = draw(red_mesh);
    let mut load = frame(std::slice::from_ref(&blue_draw));
    load.clear = Clear::Load;
    renderer.render(&load)?;
    pixel(&mut renderer, [32, 32], [32, 64, 96])?; // water behind the mask is hidden
    load.draws = std::slice::from_ref(&red_draw);
    renderer.render(&load)?;
    pixel(&mut renderer, [32, 32], [255, 0, 0])?; // colour writes restored in the same view
    renderer.render(&background)?;
    cache
        .render_world(&mut renderer, &mask, &mask_view, |_| {
            Err::<Vec<u8>, _>("mask requested atlas pixels")
        })
        .map_err(error)?;
    clear.clear = Clear::ColorDepth([0., 1., 0., 1.]);
    renderer.render(&clear)?;
    pixel(&mut renderer, [32, 32], [0, 255, 0])?; // colour mask restored before clear
    // Text pulls a very slightly farther plane in front. A later unbiased plane
    // stays behind it: leaking POLYGON_OFFSET_FILL would turn the pixel green.
    let plate = renderer.create_mesh(&quad([0., 0., 1., 1.], 0., 0.9), &indices)?;
    let green = renderer.create_mesh(&quad([0., 1., 0., 1.], 0., 0.9), &indices)?;
    let plate_draw = draw(plate);
    let green_draw = draw(green);
    let clear_plate = frame(std::slice::from_ref(&plate_draw));
    renderer.render(&clear_plate)?;
    let text = prepare(input(35, 0.0000005, 0xff0000ff))?;
    cache
        .render_world(&mut renderer, &text, &view, |p| layout.pixels(p, &[255; 4]))
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [255, 0, 0])?;
    let mut load = frame(std::slice::from_ref(&green_draw));
    load.clear = Clear::Load;
    renderer.render(&load)?;
    pixel(&mut renderer, [32, 32], [255, 0, 0])?;
    // Text is front-culled even with baked normals; outside its cutout there is
    // no hidden depth. Shadows do not inherit the text's alpha cutoff either.
    let mut back_text = input(35, 0., u32::MAX);
    back_text.reverse();
    let back_text = prepare(back_text)?;
    clear.clear = Clear::ColorDepth([0., 0., 0., 1.]);
    renderer.render(&clear)?;
    cache
        .render_world(&mut renderer, &back_text, &view, |p| {
            layout.pixels(p, &[255; 4])
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [0; 3])?;
    let name = entity::see_through_runs(input(0, 0.8, 0x800000ff), &[255; 1024], false, 4)
        .map_err(error)?;
    let name = paged_gui::split_entities(layout, &name, 32).map_err(error)?;
    renderer.render(&clear_plate)?;
    cache
        .render_world(&mut renderer, &name, &view, |p| layout.pixels(p, &[255; 4]))
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [128, 0, 127])?;
    let mut load = frame(std::slice::from_ref(&green_draw));
    load.clear = Clear::Load;
    renderer.render(&load)?;
    pixel(&mut renderer, [32, 32], [128, 0, 127])?; // original depth survived see-through text
    renderer.render(&clear_plate)?;
    let fogged = WorldView {
        fog: Some(Fog {
            color: [0., 1., 0.],
            start: 0.,
            end: 0.4,
        }),
        ..view
    };
    cache
        .render_world(&mut renderer, &name, &fogged, |p| {
            layout.pixels(p, &[255; 4])
        })
        .map_err(error)?;
    pixel(&mut renderer, [32, 32], [0, 128, 127])?; // world fog, not a screen overlay
    let mask_source = entity::runs(input(13, 0., 0), &[255; 1024], false, 4).map_err(error)?;
    let mask_mesh = renderer.create_entity_mesh(&mask_source[0].vertices, &indices)?;
    let mask_draw = Draw {
        mesh: mask_mesh,
        texture: clear_rgba,
        pass: mask_source[0].pass,
        ..plate_draw
    };
    let draws = [mask_draw, blue_draw, red_draw];
    let mixed = frame(&draws);
    renderer.render(&mixed)?;
    pixel(&mut renderer, [32, 32], [255, 0, 0])?; // per-draw colour mask/layout restored too
    let text_source =
        entity::runs(input(35, 0.0000005, 0xff0000ff), &[255; 1024], false, 4).map_err(error)?;
    let text_mesh = renderer.create_entity_mesh(&text_source[0].vertices, &indices)?;
    let text_draw = Draw {
        mesh: text_mesh,
        pass: text_source[0].pass,
        ..plate_draw
    };
    let draws = [plate_draw, text_draw, green_draw];
    renderer.render(&frame(&draws))?;
    pixel(&mut renderer, [32, 32], [255, 0, 0])?; // polygon offset reset within one draw list
    for mesh in [blue, red_mesh, plate, green, mask_mesh, text_mesh] {
        renderer.remove_mesh(mesh)?;
    }
    cache.release(&mut renderer)?;
    renderer.remove_texture(white)?;
    renderer.remove_texture(clear_rgba)?;
    require(
        renderer.stats() == Stats::default(),
        "special entity layers retained resources",
    )
}
