//! Opt-in integration with normal prepared resources, the real shared worker
//! pool/mesher and OSMesa. Not a Game/world/save acceptance test. No mesh or
//! ticket is fabricated; chunks enter the existing remote-column API so this
//! isolates mesher delivery from terrain generation and persistence.
use super::*;
use crate::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    hud::{HudAtlas, HudBatch},
    legacy_gfx::{Limits, ResourceLimits},
    light::LightTables,
    mesher::MeshTables,
    streaming::WorkerContext,
    testgen::TestTerrain,
};
use horizon_render_api::scene::SceneFrame;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn native(owner: &mut RendererOwner) -> &mut TerrainHudRenderer {
    let Owner::Legacy(renderer) = &mut owner.inner else {
        panic!("native owner lost")
    };
    renderer
}
fn settle(
    state: &mut NativeStream,
    owner: &mut RendererOwner,
    stream: &mut Streaming,
    world: &mut World,
    obsolete_at_least: u64,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let result = state.update(
            owner,
            stream,
            world,
            UploadBudget {
                jobs: 1,
                ..UploadBudget::default()
            },
        );
        assert_eq!(result.error, None);
        assert!(
            result.unloaded.is_empty(),
            "remote columns must only leave via forget_columns"
        );
        assert!(
            stream
                .stages()
                .into_iter()
                .find(|(name, _)| *name == "meshing")
                .unwrap()
                .1
                <= 32,
            "one-worker dispatch cap was applied to columns instead of sections"
        );
        let usage = native(owner).stream_usage().unwrap();
        assert!(
            usage.pending <= 1,
            "real completions escaped pending backpressure"
        );
        if !stream.busy()
            && world.dirty_sections.is_empty()
            && usage.pending == 0
            && usage.removals == 0
            && stream.obsolete_mesh_completions >= obsolete_at_least
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "real mesher did not settle: {:?}, dirty={}",
            stream.stages(),
            world.dirty_sections.len()
        );
        // Use the production mailbox wait, not a fabricated completion or a
        // worker run on the GL thread. All GL ownership stays on this thread.
        stream.wait(Duration::from_millis(2));
    }
}
fn image(owner: &mut RendererOwner) -> Vec<u8> {
    let renderer = native(owner);
    let keys: Vec<_> = renderer.section_keys().unwrap().collect();
    renderer
        .render(
            &SceneFrame {
                view_projection: glam::Mat4::perspective_rh_gl(60f32.to_radians(), 1., 0.1, 50.)
                    .to_cols_array(),
                camera: [8.5, 0.5, 12.],
                clear: [0., 0., 0., 1.],
                fog: None,
                solid: &keys,
                translucent: &[],
            },
            &HudBatch::new([64; 2]),
        )
        .unwrap();
    renderer.read_rgba([0, 0, 64, 64]).unwrap()
}

#[test]
#[ignore = "requires normal HORIZON_TEST_RESOURCES plus OSMesa; not run by fixture-only CI"]
fn prepared_resources_real_mesher_rejects_inflight_replaced_columns() -> crate::Result<()> {
    let root = PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("normal prepared HORIZON_TEST_RESOURCES required"),
    );
    let assets = Arc::new(Assets::load(&root)?);
    let render = Arc::new(assets.reload(&root)?);
    let facts = BlockFacts::load(&root, assets.registry)?;
    let biomes = Arc::new(Biomes::load(&root)?);
    let tables = Arc::new(MeshTables::new(
        &assets,
        &render,
        &facts,
        crate::models::load_visibility(&root, assets.registry)?,
    )?);
    let context = Arc::new(WorkerContext {
        light: Arc::new(LightTables::load(&root, assets.registry, &facts)?),
        tables,
        assets: assets.clone(),
        render: render.clone(),
        biomes: biomes.clone(),
    });
    // Existing generator is required by Streaming's generic worker state, but
    // remote=true prevents generation. No replacement mesher/scheduler exists.
    let mut stream = Streaming::new(
        Arc::new(TestTerrain::new(assets.registry, &biomes)?),
        context,
        1,
    );
    stream.remote = true;
    stream.radius = 1;
    let air = assets
        .registry
        .default_state("minecraft:air")
        .ok_or("missing air")?;
    let stone = assets
        .registry
        .default_state("minecraft:stone")
        .ok_or("missing stone")?;
    let gold = assets
        .registry
        .default_state("minecraft:gold_block")
        .ok_or("missing gold block")?;
    let mut world = World::new(air, 0);
    for z in -2..=2 {
        for x in -2..=2 {
            let mut chunk = Chunk::empty(x, z);
            if (x, z) == (0, 0) {
                chunk.set(8, 0, 8, stone.0);
            }
            stream.insert_remote(&mut world, chunk);
        }
    }
    let atlas = HudAtlas::boot(); // unused boot HUD; world textures/models are real
    let renderer = TerrainHudRenderer::stage_assets(
        horizon_legacy_render::Context::new("osmesa", [64; 2])?,
        &render,
        &atlas,
        ResourceGeneration(render.generation),
        &[255; 1024],
        Limits {
            stream_pending: 1,
            ..Limits::default()
        },
        ResourceLimits::default(),
    )?
    .commit()?;
    let mut owner = RendererOwner::from_legacy(renderer);
    let mut state = NativeStream::default();
    settle(&mut state, &mut owner, &mut stream, &mut world, 0);
    let key = [0, (-world.layout().min_y()).div_euclid(16), 0];
    assert!(
        native(&mut owner).has_section(key),
        "actual stone mesh never reached the GPU"
    );
    assert!(stream.meshed_total > 0, "no worker completion consumed");
    let stone_image = image(&mut owner);
    assert!(
        stone_image.chunks_exact(4).any(|p| p[..3] != [0, 0, 0]),
        "real worker geometry was not drawn"
    );
    let before = stream.meshed_total;
    let stale_before = stream.obsolete_mesh_completions;

    world.dirty_sections.insert((key[0], key[1], key[2]));
    let result = state.update(&mut owner, &mut stream, &mut world, UploadBudget::default());
    assert_eq!(result.error, None);
    assert!(
        stream.is_meshing((key[0], key[1], key[2])),
        "old snapshot was not dispatched"
    );
    let obsolete = state.tickets[&key]; // issued by actual owner before worker dispatch
    // Do not collect another completion between dispatch and replacement.
    // Even a very fast worker still has its old snapshot in the real mailbox.
    let gone = [(0, 0)].into_iter().collect();
    assert_eq!(stream.forget_columns(&mut world, &gone).len(), 1);
    let mut replacement = Chunk::empty(0, 0);
    replacement.set(8, 0, 8, gold.0);
    stream.insert_remote(&mut world, replacement);
    settle(
        &mut state,
        &mut owner,
        &mut stream,
        &mut world,
        stale_before + 1,
    );
    let current = state.tickets[&key];
    assert_ne!(
        current, obsolete,
        "same-coordinate replacement reused an old ticket"
    );
    assert!(stream.meshed_total > before);
    assert!(native(&mut owner).has_section(key));
    let gold_image = image(&mut owner);
    assert!(gold_image.chunks_exact(4).any(|p| p[..3] != [0, 0, 0]));
    assert_ne!(
        gold_image, stone_image,
        "obsolete stone mesh won over the replacement gold block (requires distinct normal textures)"
    );

    assert_eq!(stream.forget_columns(&mut world, &gone).len(), 1);
    settle(
        &mut state,
        &mut owner,
        &mut stream,
        &mut world,
        stale_before + 1,
    );
    assert!(!native(&mut owner).has_section(key));
    let empty_image = image(&mut owner);
    assert!(
        empty_image.chunks_exact(4).all(|p| p == [0, 0, 0, 255]),
        "forgotten geometry remained visible"
    );
    if let Some(path) = std::env::var_os("HORIZON_NATIVE_WORKER_REPORT") {
        let report = serde_json::json!({
            "scenario": "native-prepared-real-mesher",
            "obsolete_completions": stream.obsolete_mesh_completions - stale_before,
            "normal_resources": true, "worker_completions": stream.meshed_total,
            "received_columns": stream.generated_total, "pending_limit": 1,
            "old_revision": obsolete.revision, "new_revision": current.revision,
            "resource_generation": current.resources.0,
            "nonempty_draw": true, "replacement_draw_changed": true,
            "forgotten_draw_empty": true, "game_acceptance": false,
            "terrain_generation_tested": false, "save_reopen_tested": false,
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}
