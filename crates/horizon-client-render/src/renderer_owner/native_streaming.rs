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
    membership: u64,
    interest_radius: Option<i32>,
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
        world: &mut World,
    ) -> Result<(), String> {
        let layout = world.layout();
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
        self.membership = 0;
        self.interest_radius = None;
        // New GPU epoch has no residents, even for a previously meshed CPU world.
        for &(x, z) in world.chunks.keys() {
            for y in 0..layout.sections() as i32 {
                world.dirty_sections.insert((x, y, z));
            }
        }
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
        if self.error.is_none()
            && let Err(error) = self.bind(renderer, streamer.mesh_epoch(), world)
        {
            self.error = Some(error);
        }
        if self.error.is_none() && self.interest_radius != Some(streamer.radius) {
            if interest_fits(
                streamer.radius,
                world.layout().sections(),
                renderer.stream_limits().slots,
            ) {
                self.interest_radius = Some(streamer.radius);
            } else {
                self.error = Some("native view exceeds section interest capacity; select a smaller render distance before world entry".into());
            }
        }
        if self.error.is_none() {
            let mut hook = Hook {
                state: self,
                renderer,
                budget,
            };
            hook.synchronize_membership(world);
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
                hook.synchronize_membership(world);
                hook.prune(world, streamer.center, streamer.radius);
                if hook.state.error.is_none()
                    && let Err(error) = hook.renderer.drain_streaming(budget.jobs, budget.bytes)
                {
                    hook.fail(error);
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
    fn revoke(&mut self, keys: Vec<(SectionKey, SectionStamp)>, world: &mut World) {
        for (key, stamp) in keys {
            if self.state.error.is_some() {
                break;
            }
            match self.renderer.unload_stream_section(key, stamp) {
                Ok(true) => {
                    self.state.tickets.remove(&key);
                    // A CPU column can remain in the generation margin after
                    // leaving the draw area. It MUST remesh on readmission.
                    if world.chunks.contains_key(&(key[0], key[2])) {
                        world.dirty_sections.insert((key[0], key[1], key[2]));
                    }
                }
                Ok(false) => self.fail("native interest ticket is no longer current".into()),
                Err(error) => self.fail(error),
            }
        }
    }
    fn synchronize_membership(&mut self, world: &mut World) {
        use crate::chunk_map::ChunkMembershipDelta;
        if self.state.error.is_some() {
            return;
        }
        let revision = world.chunks.membership_revision();
        if revision == self.state.membership {
            return;
        }
        if self.state.membership != 0 {
            // Removed/re-added at the SAME coordinates is not the old column.
            // A lost delta ring/map replacement invalidates all interests;
            // re-dirtying loaded keys makes this conservative rebuild complete.
            let removed = world
                .chunks
                .deltas_since(self.state.membership)
                .map(|deltas| {
                    deltas
                        .filter_map(|(_, change)| match change {
                            ChunkMembershipDelta::Removed(key) => Some(key),
                            _ => None,
                        })
                        .collect::<std::collections::HashSet<_>>()
                });
            let keys = self
                .state
                .tickets
                .iter()
                .filter(|(key, _)| {
                    removed
                        .as_ref()
                        .is_none_or(|gone| gone.contains(&(key[0], key[2])))
                })
                .map(|(&key, &stamp)| (key, stamp))
                .collect();
            self.revoke(keys, world);
        }
        if self.state.error.is_none() {
            self.state.membership = revision;
        }
    }
    fn prune(&mut self, world: &mut World, center: (i32, i32), radius: i32) {
        if self.state.error.is_some() {
            return;
        }
        let gone = self
            .state
            .tickets
            .iter()
            .filter(|(key, _)| {
                !world.chunks.contains_key(&(key[0], key[2]))
                    || !within_view(center, radius, key[0], key[2])
            })
            .map(|(&key, &stamp)| (key, stamp))
            .collect();
        self.revoke(gone, world);
    }
}
fn interest_fits(radius: i32, sections: usize, slots: usize) -> bool {
    if radius == 0 {
        return true;
    }
    if !(1..=256).contains(&radius) || sections == 0 {
        return false;
    }
    let columns = slots / sections;
    if (radius as usize * 2 + 1) > columns {
        return false;
    }
    let mut count = 0;
    for x in -radius..=radius {
        for z in -radius..=radius {
            if in_view_distance((0, 0), radius, x, z) {
                count += 1;
                if count > columns {
                    return false;
                }
            }
        }
    }
    true
}
fn within_view(center: (i32, i32), radius: i32, x: i32, z: i32) -> bool {
    radius > 0
        && (i64::from(x) - i64::from(center.0)).abs() <= i64::from(radius)
        && (i64::from(z) - i64::from(center.1)).abs() <= i64::from(radius)
        && in_view_distance(center, radius, x, z)
}
impl MeshAdmission for Hook<'_> {
    fn bounded_section_dispatch(&self) -> bool {
        true
    }
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
    fn impossible_static_views_fail_instead_of_starving_dirty_sections_forever() {
        assert!(interest_fits(6, 24, 4096));
        assert!(!interest_fits(7, 24, 4096));
        assert!(interest_fits(0, 24, 4096));
        assert!(!interest_fits(i32::MAX, 1, usize::MAX));
        assert!(!interest_fits(2, 0, 4096));
        assert!(!interest_fits(2, 24, 24));
    }
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

    pub(super) fn renderer() -> TerrainHudRenderer {
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
    pub(super) fn ticket(hook: &mut Hook<'_>, key: SectionKey) -> SectionStamp {
        match hook.request(key) {
            Admission::Stamped(stamp) => stamp,
            _ => panic!("no ticket"),
        }
    }
    #[test]
    #[ignore = "requires OSMesa and explicit GL2 environment"]
    fn view_readmission_and_same_coordinate_column_replacement_never_reuse_residency() {
        let mut renderer = renderer();
        let mut state = NativeStream::default();
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        state.bind(&mut renderer, Arc::new(()), &mut world).unwrap();
        let mut hook = Hook {
            state: &mut state,
            renderer: &mut renderer,
            budget: UploadBudget::default(),
        };
        let key = [0, 0, 0];
        hook.synchronize_membership(&mut world);
        let first = ticket(&mut hook, key);
        assert!(matches!(
            hook.complete(key, Some(first), mesh()),
            Delivery::Queued
        ));
        hook.renderer.drain_streaming(1, hook.budget.bytes).unwrap();
        world.dirty_sections = Default::default();
        hook.prune(&mut world, (20, 20), 2);
        assert_eq!(hook.renderer.section_keys().unwrap().count(), 0);
        assert!(
            world
                .dirty_sections
                .columns()
                .any(|(column, sections)| column == (0, 0) && sections.contains(0))
        );
        let second = ticket(&mut hook, key);
        assert_ne!(first, second);
        assert!(matches!(
            hook.complete(key, Some(second), mesh()),
            Delivery::Queued
        ));
        hook.renderer.drain_streaming(1, hook.budget.bytes).unwrap();
        world.dirty_sections = Default::default();
        // A forget+reload between polls still revokes the previous GPU section.
        world.chunks.remove(&(0, 0));
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        hook.synchronize_membership(&mut world);
        assert_eq!(hook.renderer.section_keys().unwrap().count(), 0);
        assert!(matches!(
            hook.complete(key, Some(second), mesh()),
            Delivery::Discard(_)
        ));
        let third = ticket(&mut hook, key);
        assert!(matches!(
            hook.complete(key, Some(third), mesh()),
            Delivery::Queued
        ));
        hook.renderer.drain_streaming(1, hook.budget.bytes).unwrap();
        assert!(
            world
                .dirty_sections
                .columns()
                .any(|(column, sections)| column == (0, 0) && sections.contains(0))
        );
        world.dirty_sections = Default::default();
        // New map/evicted delta history: conservatively rebuild all interests.
        world.chunks = Default::default();
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        hook.synchronize_membership(&mut world);
        assert_eq!(hook.renderer.stream_usage().unwrap().slots, 0);
        assert_eq!(hook.renderer.section_keys().unwrap().count(), 0);
        assert!(matches!(
            hook.complete(key, Some(third), mesh()),
            Delivery::Discard(_)
        ));
        assert!(
            world
                .dirty_sections
                .columns()
                .any(|(column, sections)| column == (0, 0) && sections.contains(0))
        );
        let fourth = ticket(&mut hook, key);
        assert!(matches!(
            hook.complete(key, Some(fourth), mesh()),
            Delivery::Queued
        ));
        hook.renderer.drain_streaming(1, hook.budget.bytes).unwrap();
        world.dirty_sections = Default::default();
        let fixture = Arc::new(Chunk::empty(100, 100));
        for _ in 0..300 {
            world.chunks.insert((100, 100), fixture.clone());
            world.chunks.remove(&(100, 100));
        }
        hook.synchronize_membership(&mut world);
        assert_eq!(hook.renderer.section_keys().unwrap().count(), 0);
        assert!(matches!(
            hook.complete(key, Some(fourth), mesh()),
            Delivery::Discard(_)
        ));
        assert!(
            world
                .dirty_sections
                .columns()
                .any(|(column, sections)| column == (0, 0) && sections.contains(0))
        );
        assert!(hook.state.error.is_none());
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
            .bind(&mut renderer, epoch.clone(), &mut world)
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
        hook.prune(&mut world, (0, 0), 2);
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
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        world.dirty_sections = Default::default();
        state.bind(&mut renderer, epoch, &mut world).unwrap();
        assert!(world.dirty_sections.columns().next().is_none());
        assert_eq!(state.generation, 1);
        // New streamer identity cannot ABA-reuse the first world stamp.
        state.bind(&mut renderer, Arc::new(()), &mut world).unwrap();
        assert_eq!(state.generation, 2);
        assert!(
            world
                .dirty_sections
                .columns()
                .any(|(column, sections)| column == (0, 0) && sections.contains(0))
        );
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
        let mut world = World::new(BlockStateId(0), 0);
        state.bind(&mut renderer, Arc::new(()), &mut world).unwrap();
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

#[cfg(test)]
mod worker_tests;

#[cfg(test)]
mod gpu_epoch_tests;
