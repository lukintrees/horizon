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
    /// How long one frame may spend converting and uploading finished sections. The
    /// count above is only a cap: it used to be the whole budget (four sections a
    /// tick), so a view that does not fit in a few frames — anything past a couple
    /// of chunks — filled at five sections a second, and an edit queued behind that
    /// fill took minutes to show at a wide render distance. Time-bounded like the
    /// modern path's `UploadPace`, the view fills as fast as frames allow and the
    /// frame keeps the rest of its time.
    pub micros: u64,
}
impl Default for UploadBudget {
    fn default() -> Self {
        Self {
            // The byte budget admits ~55 sections of a normal world; the count must not
            // be the binding limit below it.
            jobs: 64,
            bytes: 8 << 20,
            micros: 6_000,
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
    /// How far resident meshes currently reach, when the mesh ledger holds fewer
    /// sections than the interest table. `None` starts at the whole interest view.
    residency_radius: Option<i32>,
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
        self.residency_radius = None;
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
                // The interest table holds what it holds; a view past it would starve
                // sections forever. Draw the largest view that fits and say so once, rather
                // than ending the run: the player can still play and lower the distance.
                let fit = fitted_radius(world.layout().sections(), renderer.stream_limits().slots);
                if self.interest_radius != Some(fit) {
                    log::warn!(
                        "Horizon: the native view of {} chunks does not fit the section interest \
                         table; drawing {fit} chunks until the render distance is lowered",
                        streamer.radius
                    );
                }
                self.interest_radius = Some(fit);
            }
        }
        if self.error.is_none() {
            // Interest, admission and pruning all use the fitted radius: a column outside it
            // would be a ticket the table cannot keep. Resident meshes may hold less than the
            // interest table: `residency_radius` is how far the mesh ledger reaches, and is
            // what the empty space of the view's outer rings shrinks when it is full.
            let interest = self.interest_radius.unwrap_or(streamer.radius);
            let residency = self.residency_radius.unwrap_or(interest).min(interest);
            let mut hook = Hook {
                state: self,
                renderer,
                view: (streamer.center, residency),
                interest,
                budget,
            };
            hook.synchronize_membership(world);
            hook.prune(world, streamer.center, residency);
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
                hook.prune(world, streamer.center, residency);
                if hook.state.error.is_none() {
                    match hook
                        .renderer
                        .drain_streaming_budget(budget.jobs, budget.bytes, budget.micros)
                    {
                        Ok(drain) => hook.relieve_pressure(world, drain.deferred),
                        Err(error) => hook.fail(error),
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
    /// The radius this pass draws and keeps sections for: the residency cap.
    view: ((i32, i32), i32),
    /// The largest radius the interest table allows, above the current cap.
    interest: i32,
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
        self.view = (center, radius);
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
    /// A drain deferred its head because resident sections reached their share of
    /// the mesh ledger: the view's meshes do not fit what streaming may hold. The
    /// draw radius steps in by one and the ring outside it is released, so the next
    /// drain has room for the newest edits. Two things this deliberately does not do:
    /// evict single sections (they are still inside the view, so admission would
    /// remesh and re-queue them every frame), and grow back afterwards (the mesh
    /// that did not fit did not get smaller, so growing would shrink again on the
    /// next frame and remesh that whole ring each time). Lowering the render
    /// distance still lowers `interest`, below this cap.
    fn relieve_pressure(&mut self, world: &mut World, deferred: usize) {
        if deferred == 0 || self.state.error.is_some() {
            return;
        }
        let (center, current) = self.view;
        if current > 1 {
            self.state.residency_radius = Some(current - 1);
            if current == self.interest {
                log::warn!(
                    "Horizon: the native mesh budget holds about {} chunks of terrain, not {}; \
                     drawing the smaller view instead of stalling uploads",
                    current - 1,
                    self.interest
                );
            }
            self.prune(world, center, current - 1);
        }
    }
}
/// The largest radius whose whole view the interest table can hold around a level of
/// `sections` vertical sections. The table counts every section of a drawn column, so a
/// column is `sections` slots and a radius needs one slot per section of its in-view columns.
pub(super) fn fitted_radius(sections: usize, slots: usize) -> i32 {
    (0..=256)
        .rev()
        .find(|&radius| interest_fits(radius, sections, slots))
        .unwrap_or(0)
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
    fn wants_column(&self, x: i32, z: i32) -> bool {
        self.state.error.is_none() && within_view(self.view.0, self.view.1, x, z)
    }
    fn bounded_section_dispatch(&self) -> bool {
        true
    }
    fn request(&mut self, key: SectionKey) -> Admission {
        // Same scope as prune, including forced generation columns. Never mint
        // a ticket for work the end-of-frame prune will immediately revoke.
        if !self.wants_column(key[0], key[2]) {
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
        if bytes > self.renderer.stream_limits().bytes {
            // A single section bigger than the whole pending-input budget cannot ever be
            // queued. That used to end the stream for the rest of the session, which froze
            // every later edit; rejecting this one mesh and asking for a fresh snapshot
            // leaves a bad section unmeshed instead of a world that never updates again.
            log::warn!(
                "Horizon: native mesh of section {key:?} is {} bytes, past the streamed mesh \
                 input budget; leaving it unmeshed",
                bytes
            );
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
    fn a_view_past_the_table_fits_the_largest_radius_that_holds_every_column() {
        // Every radius up to the fitted one holds its whole view; the next one does not.
        for (sections, slots) in [(24, 4096), (24, 16384), (16, 4096)] {
            let fit = fitted_radius(sections, slots);
            assert!(interest_fits(fit, sections, slots), "{sections}/{slots}");
            assert!(
                !interest_fits(fit + 1, sections, slots),
                "{sections}/{slots} stops at {fit}"
            );
        }
        // Vanilla's 24-section overworld: 4096 slots is the old radius 6; 16384 (682 columns)
        // reaches 13, past the default render distance of 12.
        assert_eq!(fitted_radius(24, 4096), 6);
        assert_eq!(fitted_radius(24, 16384), 13);
        assert!(fitted_radius(24, 16384) >= 12);
        // A table that cannot even hold one column draws nothing rather than erroring.
        assert_eq!(fitted_radius(0, 4096), 0);
        assert_eq!(fitted_radius(24, 0), 0);
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
            view: ((0, 0), 2),
            interest: 2,
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
        assert!(matches!(hook.request(key), Admission::Defer));
        hook.prune(&mut world, (0, 0), 2); // actual readmission into the draw view
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
            view: ((0, 0), 2),
            interest: 2,
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
            view: ((0, 0), 2),
            interest: 2,
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
    fn consumer_scope_never_mints_off_view_forced_tickets_and_readmits_after_move() {
        let mut renderer = renderer();
        let mut state = NativeStream::default();
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        world.chunks.insert((40, 0), Arc::new(Chunk::empty(40, 0)));
        state.bind(&mut renderer, Arc::new(()), &mut world).unwrap();
        let mut hook = Hook {
            state: &mut state,
            renderer: &mut renderer,
            view: ((0, 0), 2),
            interest: 2,
            budget: UploadBudget::default(),
        };
        hook.synchronize_membership(&mut world);
        hook.prune(&mut world, (0, 0), 2);
        let a = [0, 0, 0];
        let b = [0, 1, 0];
        let far = [40, 0, 0];
        let first = ticket(&mut hook, a);
        assert!(matches!(
            hook.complete(a, Some(first), mesh()),
            Delivery::Queued
        ));
        let before = hook.renderer.stream_usage().unwrap();
        assert_eq!((before.slots, before.pending), (1, 1)); // one spare metadata slot
        for _ in 0..1024 {
            assert!(!hook.wants_column(far[0], far[2]));
            assert!(matches!(hook.request(far), Admission::Defer));
            assert_eq!(hook.renderer.stream_usage().unwrap(), before);
            assert_eq!(hook.state.tickets.len(), 1);
        }
        let second = ticket(&mut hook, b);
        assert_eq!(
            second.revision,
            first.revision + 1,
            "rejected work minted hidden tickets"
        );
        let retry = match hook.complete(b, Some(second), mesh()) {
            Delivery::Retry(mesh) => mesh,
            _ => panic!("pending=1 must still apply"),
        };
        hook.prune(&mut world, (40, 0), 2);
        assert!(!hook.wants_column(0, 0));
        assert!(matches!(hook.request(a), Admission::Defer));
        assert!(matches!(
            hook.complete(b, Some(second), retry),
            Delivery::Discard(_)
        ));
        let usage = hook.renderer.stream_usage().unwrap();
        assert_eq!((usage.slots, usage.pending, usage.bytes), (0, 0, 0));
        assert!(hook.wants_column(40, 0));
        let remote = ticket(&mut hook, far);
        assert!(remote.revision > second.revision);
        assert!(matches!(
            hook.complete(far, Some(remote), mesh()),
            Delivery::Queued
        ));
        hook.renderer.drain_streaming(1, hook.budget.bytes).unwrap();
        assert_eq!(
            hook.renderer.section_keys().unwrap().collect::<Vec<_>>(),
            [far]
        );
        hook.prune(&mut world, (0, 0), 0);
        assert!(!hook.wants_column(0, 0));
        assert!(matches!(hook.request(a), Admission::Defer));
        assert_eq!(hook.renderer.section_keys().unwrap().count(), 0);
        hook.prune(&mut world, (0, 0), 2);
        let fresh = ticket(&mut hook, a);
        assert!(fresh.revision > remote.revision);
        assert!(matches!(
            hook.complete(a, Some(first), mesh()),
            Delivery::Discard(_)
        ));
        assert!(matches!(
            hook.complete(a, Some(fresh), mesh()),
            Delivery::Queued
        ));
        hook.renderer.drain_streaming(1, hook.budget.bytes).unwrap();
        assert_eq!(
            hook.renderer.section_keys().unwrap().collect::<Vec<_>>(),
            [a]
        );
        assert!(world.dirty_sections.contains(&(0, 0, 0)));
        assert!(hook.state.error.is_none());
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
            view: ((0, 0), 2),
            interest: 2,
            budget: UploadBudget {
                jobs: 1,
                bytes: 1,
                micros: 1_000,
            },
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
