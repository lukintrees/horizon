//! Game scheduler -> owner-issued stamps -> bounded native GPU drain.
//! All hooks run on the owner thread; workers see only the copied stamp.
use super::{Owner, RendererOwner};
use crate::{
    chunk::{Chunk, ColumnLayout},
    fast_hash::FxHashMap,
    legacy_gfx::{TerrainHudRenderer, retained_stream_mesh},
    mesher::SectionMesh,
    streaming::{
        Streaming,
        admission::{Admission, Delivery, MeshAdmission},
        in_view_distance,
    },
    world::World,
};
use horizon_render_api::{
    render_contract::{ResourceGeneration, SectionStamp, WorldGeneration},
    scene::SectionKey,
    streaming::Queued,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct UploadBudget {
    pub jobs: usize,
    pub bytes: usize,
}
impl Default for UploadBudget {
    fn default() -> Self {
        Self {
            jobs: 4,
            bytes: 8 << 20,
        }
    }
}
#[derive(Default)]
pub struct NativeStream {
    binding: Option<(Arc<()>, ColumnLayout, ResourceGeneration)>,
    generation: u64,
    tickets: FxHashMap<SectionKey, SectionStamp>,
    error: Option<String>,
}
pub struct Update {
    /// Never lose unloaded modified chunks on a later GPU error: Game must
    /// finish its existing entity/persistence cleanup before propagating error.
    pub unloaded: Vec<Arc<Chunk>>,
    pub error: Option<String>,
}
impl NativeStream {
    fn bind(
        &mut self,
        renderer: &mut TerrainHudRenderer,
        epoch: Arc<()>,
        layout: ColumnLayout,
    ) -> Result<(), String> {
        let resources = renderer
            .resource_generation()
            .ok_or("native streaming requires committed assets")?;
        if self.binding.as_ref().is_some_and(|(old, shape, assets)| {
            Arc::ptr_eq(old, &epoch) && *shape == layout && *assets == resources
        }) {
            return Ok(());
        }
        let next = self
            .generation
            .checked_add(1)
            .ok_or("native world generation exhausted")?;
        renderer.begin_streaming(WorldGeneration(next), layout)?;
        self.tickets.clear();
        self.binding = Some((epoch, layout, resources));
        self.generation = next;
        Ok(())
    }
    pub fn update(
        &mut self,
        owner: &mut RendererOwner,
        streamer: &mut Streaming,
        world: &mut World,
        budget: UploadBudget,
    ) -> Update {
        let mut unloaded = Vec::new();
        let Owner::Legacy(renderer) = &mut owner.inner else {
            return Update {
                unloaded,
                error: Some("native streaming requires a native owner".into()),
            };
        };
        if self.error.is_none() {
            self.error = self
                .bind(renderer, streamer.mesh_epoch(), world.layout())
                .err();
        }
        if self.error.is_none() {
            let mut hook = Hook {
                state: self,
                renderer,
                budget,
            };
            hook.prune(world, streamer.center, streamer.radius);
            if hook.state.error.is_none() {
                let (unexpected, removed) = streamer.update_admitted(world, &mut hook);
                unloaded = removed;
                // Native delivery consumes accepted work; never turn it into a
                // modern upload list or silently discard a second payload path.
                if !unexpected.is_empty() {
                    streamer.free(unexpected);
                    hook.fail("native admission produced unstamped uploads".into());
                }
                hook.prune(world, streamer.center, streamer.radius);
                if hook.state.error.is_none() {
                    if let Err(error) = hook.renderer.drain_streaming(budget.jobs, budget.bytes) {
                        hook.fail(error);
                    }
                }
            }
        }
        Update {
            unloaded,
            error: self.error.clone(),
        }
    }
}
struct Hook<'a> {
    state: &'a mut NativeStream,
    renderer: &'a mut TerrainHudRenderer,
    budget: UploadBudget,
}
impl Hook<'_> {
    fn fail(&mut self, error: String) {
        if self.state.error.is_none() {
            self.state.error = Some(error);
        }
    }
    fn prune(&mut self, world: &World, center: (i32, i32), radius: i32) {
        let gone: Vec<_> = self
            .state
            .tickets
            .iter()
            .filter(|(key, _)| {
                !world.chunks.contains_key(&(key[0], key[2]))
                    || !within_view(center, radius, key[0], key[2])
            })
            .map(|(&key, &stamp)| (key, stamp))
            .collect();
        for (key, stamp) in gone {
            match self.renderer.unload_stream_section(key, stamp) {
                Ok(true) => {
                    self.state.tickets.remove(&key);
                }
                Ok(false) => self.fail("native interest ticket is no longer current".into()),
                Err(error) => self.fail(error),
            }
        }
    }
}
fn within_view(center: (i32, i32), radius: i32, x: i32, z: i32) -> bool {
    radius > 0
        && (i64::from(x) - i64::from(center.0)).abs() <= i64::from(radius)
        && (i64::from(z) - i64::from(center.1)).abs() <= i64::from(radius)
        && in_view_distance(center, radius, x, z)
}
impl MeshAdmission for Hook<'_> {
    fn request(&mut self, key: SectionKey) -> Admission {
        if self.state.error.is_some() {
            return Admission::Defer;
        }
        match self.renderer.request_stream_section(key) {
            Ok(Some(stamp)) => {
                self.state.tickets.insert(key, stamp);
                Admission::Stamped(stamp)
            }
            Ok(None) => Admission::Defer,
            Err(error) => {
                self.fail(error);
                Admission::Defer
            }
        }
    }
    fn complete(
        &mut self,
        key: SectionKey,
        stamp: Option<SectionStamp>,
        mesh: SectionMesh,
    ) -> Delivery {
        if self.state.error.is_some() {
            return Delivery::Retry(mesh);
        }
        let Some(stamp) = stamp else {
            return Delivery::Discard(mesh);
        };
        if self.state.tickets.get(&key) != Some(&stamp) {
            return Delivery::Discard(mesh);
        }
        let bytes = match retained_stream_mesh(&mesh) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.fail(error);
                return Delivery::Discard(mesh);
            }
        };
        if bytes > self.renderer.stream_limits().bytes || bytes > self.budget.bytes {
            self.fail("native mesh exceeds queue/per-drain byte budget; increase budget or reduce producer geometry".into());
            return Delivery::Discard(mesh);
        }
        let mesh = Arc::new(mesh);
        match self.renderer.queue_stream_section(key, stamp, &mesh) {
            Ok(Queued::Accepted) => Delivery::Queued,
            Ok(Queued::Backpressure) => Delivery::Retry(
                Arc::try_unwrap(mesh)
                    .unwrap_or_else(|_| unreachable!("rejected completion retained")),
            ),
            Ok(Queued::Stale) => Delivery::Discard(
                Arc::try_unwrap(mesh).unwrap_or_else(|_| unreachable!("stale completion retained")),
            ),
            Err(error) => {
                self.fail(error);
                Delivery::Discard(
                    Arc::try_unwrap(mesh)
                        .unwrap_or_else(|_| unreachable!("failed completion retained")),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interest_distance_is_shared_and_overflow_safe_on_teleports() {
        assert!(!within_view((i32::MAX, 0), 16, i32::MIN, 0));
        assert!(!within_view((0, 0), 0, 0, 0));
        for radius in [1, 2, 8, 32] {
            for x in -40..=40 {
                for z in -40..=40 {
                    assert_eq!(
                        within_view((0, 0), radius, x, z),
                        in_view_distance((0, 0), radius, x, z)
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::{
        hud::HudAtlas,
        legacy_gfx::{Limits, ResourceLimits, test_assets},
        mesher::TerrainVertex,
    };
    use horizon_world::registry::BlockStateId;

    fn renderer() -> TerrainHudRenderer {
        let atlas = HudAtlas::boot();
        TerrainHudRenderer::stage_assets(
            horizon_legacy_render::Context::new("osmesa", [64; 2]).unwrap(),
            &test_assets(),
            &atlas,
            ResourceGeneration(17),
            &[255; 1024],
            Limits {
                stream_slots: 2,
                stream_pending: 1,
                ..Limits::default()
            },
            ResourceLimits::default(),
        )
        .unwrap()
        .commit()
        .unwrap()
    }
    fn mesh() -> SectionMesh {
        let mut result = SectionMesh::default();
        result.solid.extend(
            [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]]
                .map(|pos| TerrainVertex::new(pos, [0.5; 2], 0, u32::MAX, 0xf0f0)),
        );
        result
    }
    fn ticket(hook: &mut Hook<'_>, key: SectionKey) -> SectionStamp {
        match hook.request(key) {
            Admission::Stamped(stamp) => stamp,
            _ => panic!("no ticket"),
        }
    }
    #[test]
    #[ignore = "requires OSMesa and explicit GL2 environment"]
    fn admission_retries_same_ticket_rejects_edits_and_revokes_pending_before_unload() {
        let mut renderer = renderer();
        let mut state = NativeStream::default();
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let epoch = Arc::new(());
        state
            .bind(&mut renderer, epoch.clone(), world.layout())
            .unwrap();
        let mut hook = Hook {
            state: &mut state,
            renderer: &mut renderer,
            budget: UploadBudget::default(),
        };
        let a = [0, 0, 0];
        let b = [0, 1, 0];
        let first = ticket(&mut hook, a);
        let second = ticket(&mut hook, b);
        assert!(matches!(hook.request([0, 2, 0]), Admission::Defer));
        assert!(matches!(
            hook.complete(a, Some(first), mesh()),
            Delivery::Queued
        ));
        let retry = match hook.complete(b, Some(second), mesh()) {
            Delivery::Retry(mesh) => mesh,
            _ => panic!("no bounded backpressure"),
        };
        assert_eq!(hook.renderer.stream_usage().unwrap().pending, 1);
        assert_eq!(
            hook.renderer
                .drain_streaming(1, hook.budget.bytes)
                .unwrap()
                .uploads,
            1
        );
        assert!(matches!(
            hook.complete(b, Some(second), retry),
            Delivery::Queued
        ));
        let edit = ticket(&mut hook, a);
        assert_ne!(edit, first);
        assert!(matches!(
            hook.complete(a, Some(first), mesh()),
            Delivery::Discard(_)
        ));
        assert!(matches!(
            hook.complete(a, None, mesh()),
            Delivery::Discard(_)
        ));
        hook.renderer.drain_streaming(1, hook.budget.bytes).unwrap();
        // Last-block removal is still an admitted/stamped empty mesh, not an
        // unstamped direct removal or a permanently occupied GPU air section.
        assert!(matches!(
            hook.complete(a, Some(edit), SectionMesh::default()),
            Delivery::Queued
        ));
        hook.renderer.drain_streaming(1, hook.budget.bytes).unwrap();
        assert_eq!(hook.renderer.section_keys().unwrap().count(), 1);
        let pending = ticket(&mut hook, b);
        assert!(matches!(
            hook.complete(b, Some(pending), mesh()),
            Delivery::Queued
        ));
        world.chunks.clear();
        hook.prune(&world, (0, 0), 2);
        let usage = hook.renderer.stream_usage().unwrap();
        assert_eq!(
            (usage.slots, usage.pending, usage.bytes, usage.removals),
            (0, 0, 0, 0)
        );
        assert_eq!(hook.renderer.section_keys().unwrap().count(), 0);
        assert!(matches!(
            hook.complete(b, Some(pending), mesh()),
            Delivery::Discard(_)
        ));
        assert!(hook.state.error.is_none());
        state.bind(&mut renderer, epoch, world.layout()).unwrap();
        assert_eq!(state.generation, 1);
        // New streamer identity cannot ABA-reuse the first world stamp.
        state
            .bind(&mut renderer, Arc::new(()), world.layout())
            .unwrap();
        assert_eq!(state.generation, 2);
        let mut hook = Hook {
            state: &mut state,
            renderer: &mut renderer,
            budget: UploadBudget::default(),
        };
        let next = ticket(&mut hook, a);
        assert_ne!(next.world, first.world);
        assert!(matches!(
            hook.complete(a, Some(first), mesh()),
            Delivery::Discard(_)
        ));
        assert!(matches!(
            hook.complete(a, Some(next), mesh()),
            Delivery::Queued
        ));
        hook.renderer.drain_streaming(1, hook.budget.bytes).unwrap();
        assert_eq!(hook.renderer.section_keys().unwrap().count(), 1);
    }
    #[test]
    #[ignore = "requires OSMesa and explicit GL2 environment"]
    fn oversized_completion_is_a_reported_error_not_an_immortal_pending_head() {
        let mut renderer = renderer();
        let mut state = NativeStream::default();
        state
            .bind(&mut renderer, Arc::new(()), ColumnLayout::VANILLA)
            .unwrap();
        let mut hook = Hook {
            state: &mut state,
            renderer: &mut renderer,
            budget: UploadBudget { jobs: 1, bytes: 1 },
        };
        let stamp = ticket(&mut hook, [0; 3]);
        assert!(matches!(
            hook.complete([0; 3], Some(stamp), mesh()),
            Delivery::Discard(_)
        ));
        assert!(hook.state.error.as_ref().unwrap().contains("byte budget"));
        assert_eq!(hook.renderer.stream_usage().unwrap().pending, 0);
        assert!(matches!(hook.request([0, 1, 0]), Admission::Defer));
    }
}
