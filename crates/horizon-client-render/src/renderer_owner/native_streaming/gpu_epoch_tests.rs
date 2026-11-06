//! Actual owner Hook + same-context reload + GL2 readback with transport meshes.
//! Deliberately not a prepared-resource Mesher or Game/save acceptance test.
use super::{
    gpu_tests::{renderer, ticket},
    *,
};
use crate::{hud::HudBatch, legacy_gfx::test_assets, mesher::TerrainVertex};
use horizon_render_api::{IDENTITY, scene::SceneFrame};
use horizon_world::registry::BlockStateId;

fn coloured_mesh(layer: u16) -> SectionMesh {
    let mut mesh = SectionMesh::default();
    mesh.solid.extend(
        [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
            .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000 | layer, u32::MAX, 0xf0f0)),
    );
    mesh
}
fn pixel(renderer: &mut TerrainHudRenderer, key: SectionKey) -> Vec<u8> {
    // Keep renderer preflight strict: an unloaded key is NOT a drawable section.
    let visible: Vec<_> = renderer
        .section_keys()
        .unwrap()
        .filter(|k| *k == key)
        .collect();
    renderer
        .render(
            &SceneFrame {
                view_projection: IDENTITY,
                camera: [0., -64., 0.],
                clear: [0., 0., 0., 1.],
                fog: None,
                solid: &visible,
                translucent: &[],
            },
            &HudBatch::new([64; 2]),
        )
        .unwrap();
    renderer.read_rgba([32, 32, 1, 1]).unwrap()
}

#[test]
#[ignore = "requires OSMesa and explicit GL2 environment"]
fn backpressured_hook_payloads_survive_rollback_but_not_committed_resource_epochs() {
    let mut renderer = renderer();
    let mut state = NativeStream::default();
    let mut world = World::new(BlockStateId(0), 0);
    world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let epoch = Arc::new(());
    state
        .bind(&mut renderer, epoch.clone(), &mut world)
        .unwrap();
    let a = [0, 0, 0];
    let b = [0, 1, 0];
    let budget = UploadBudget {
        jobs: 1,
        ..Default::default()
    };
    let (old_b, mut retry) = {
        let mut hook = Hook {
            state: &mut state,
            renderer: &mut renderer,
            budget,
        };
        let first = ticket(&mut hook, a);
        let second = ticket(&mut hook, b);
        assert!(matches!(
            hook.complete(a, Some(first), coloured_mesh(0)),
            Delivery::Queued
        ));
        let retry = match hook.complete(b, Some(second), coloured_mesh(1)) {
            Delivery::Retry(mesh) => mesh,
            _ => panic!("expected pending=1 backpressure"),
        };
        hook.renderer
            .drain_streaming(budget.jobs, budget.bytes)
            .unwrap();
        let edit = ticket(&mut hook, a);
        assert!(matches!(
            hook.complete(a, Some(edit), coloured_mesh(1)),
            Delivery::Queued
        ));
        (second, retry)
    };
    assert_eq!(pixel(&mut renderer, a), [255, 0, 0, 255]);
    let usage = renderer.stream_usage().unwrap();
    assert_eq!(usage.pending, 1);
    let pointer = retry.solid.as_ptr();
    let old = ResourceGeneration(17);
    let next = ResourceGeneration(18);
    let assets = test_assets();
    let atlas = crate::hud::HudAtlas::boot();
    // Fallible staging and a canceled valid stage must both preserve resident
    // pixels, pending work, the CPU retry ticket and its original allocation.
    assert!(
        renderer
            .stage_reload_assets(
                &assets,
                &atlas,
                old,
                next,
                0,
                crate::legacy_gfx::ResourceLimits {
                    base_bytes: 1,
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(renderer.stream_usage().unwrap(), usage);
    drop(
        renderer
            .stage_reload_assets(&assets, &atlas, old, next, 0, Default::default())
            .unwrap(),
    );
    state
        .bind(&mut renderer, epoch.clone(), &mut world)
        .unwrap();
    assert_eq!(state.generation, 1);
    assert_eq!(state.tickets.len(), 2);
    assert_eq!(renderer.stream_usage().unwrap(), usage);
    assert_eq!(pixel(&mut renderer, a), [255, 0, 0, 255]);
    {
        let mut hook = Hook {
            state: &mut state,
            renderer: &mut renderer,
            budget,
        };
        retry = match hook.complete(b, Some(old_b), retry) {
            Delivery::Retry(mesh) => mesh,
            _ => panic!("rollback consumed the waiting ticket"),
        };
        assert_eq!(
            retry.solid.as_ptr(),
            pointer,
            "retry copied or replaced the payload"
        );
    }
    assert_eq!(
        renderer
            .stage_reload_assets(&assets, &atlas, old, next, 0, Default::default())
            .unwrap()
            .commit()
            .unwrap(),
        next
    );
    assert_eq!(renderer.stream_usage().unwrap().pending, 0);
    assert_eq!(renderer.stream_usage().unwrap().bytes, 0);
    assert_eq!(renderer.section_keys().unwrap().count(), 0);
    assert_eq!(pixel(&mut renderer, a), [0, 0, 0, 255]);
    // Even before binding catches up, renderer-issued resource validation must
    // reject the old ticket: matching the Hook's old map alone is insufficient.
    {
        let mut hook = Hook {
            state: &mut state,
            renderer: &mut renderer,
            budget,
        };
        retry = match hook.complete(b, Some(old_b), retry) {
            Delivery::Discard(mesh) => mesh,
            _ => panic!("old resources resurrected a mesh"),
        };
        assert_eq!(retry.solid.as_ptr(), pointer);
        assert!(hook.state.error.is_none());
    }
    world.dirty_sections.clear();
    state
        .bind(&mut renderer, epoch.clone(), &mut world)
        .unwrap();
    assert_eq!(state.generation, 2);
    assert!(state.tickets.is_empty());
    assert_eq!(
        world
            .dirty_sections
            .columns()
            .next()
            .unwrap()
            .1
            .iter()
            .count(),
        world.layout().sections()
    );
    {
        let mut hook = Hook {
            state: &mut state,
            renderer: &mut renderer,
            budget,
        };
        assert!(matches!(
            hook.complete(b, Some(old_b), retry),
            Delivery::Discard(_)
        ));
        let current = ticket(&mut hook, a);
        assert_eq!(current.resources, next);
        assert_ne!(current.world, old_b.world);
        assert!(matches!(
            hook.complete(a, Some(current), coloured_mesh(1)),
            Delivery::Queued
        ));
        hook.renderer
            .drain_streaming(budget.jobs, budget.bytes)
            .unwrap();
        assert!(hook.state.error.is_none());
    }
    assert_eq!(pixel(&mut renderer, a), [0, 255, 0, 255]);
    assert_eq!(renderer.stream_usage().unwrap().pending, 0);
    assert_eq!(renderer.stream_usage().unwrap().bytes, 0);
    state.bind(&mut renderer, epoch, &mut world).unwrap();
    assert_eq!(
        state.generation, 2,
        "stable epoch must not discard current residency"
    );
    assert_eq!(state.tickets.len(), 1);
    assert_eq!(pixel(&mut renderer, a), [0, 255, 0, 255]);
}
