//! Opt-in stamped native section ownership. Existing direct uploads remain for
//! probes until enabled; thereafter all unstamped membership commands reject.
use super::*;
use horizon_render_api::{
    render_contract::{SectionStamp, WorldGeneration},
    streaming::{self, Queued, Stream, Usage},
};
use std::sync::Arc;

pub(super) type State = Stream<Arc<SectionMesh>>;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drain {
    pub uploads: usize,
    pub removals: usize,
    pub bytes: usize,
}
fn retained(mesh: &SectionMesh) -> Result<usize, String> {
    fn vector<T>(v: &Vec<T>) -> Option<usize> {
        v.capacity().checked_mul(std::mem::size_of::<T>())
    }
    [
        vector(&mesh.solid),
        vector(&mesh.translucent),
        vector(&mesh.packed),
        vector(&mesh.solid_tints),
        vector(&mesh.block_entities),
        vector(&mesh.lights),
        vector(&mesh.sprite_layers),
    ]
    .into_iter()
    .try_fold(
        std::mem::size_of::<SectionMesh>() + 2 * std::mem::size_of::<usize>(),
        |sum, n| sum.checked_add(n?),
    )
    .ok_or_else(|| "stream mesh retained-byte overflow".into())
}
impl TerrainHudRenderer {
    pub(super) fn streaming_ready(&self) -> Result<(), String> {
        if self
            .streaming
            .as_ref()
            .is_some_and(|s| s.usage().removals != 0)
        {
            return Err("stream section removal incomplete; drain/retry before drawing".into());
        }
        Ok(())
    }
    pub(super) fn unstamped_sections(&self) -> Result<(), String> {
        if self.streaming.is_some() {
            return Err("stamped streaming active; use issued section tickets".into());
        }
        Ok(())
    }
    /// Start a new world/teleport epoch before dispatching work. Success clears
    /// old resident sections and pending CPU jobs; failure does not publish it.
    pub fn begin_streaming(
        &mut self,
        world: WorldGeneration,
        layout: crate::chunk::ColumnLayout,
    ) -> Result<(), String> {
        let resources = self
            .resource_generation
            .ok_or("streaming requires tracked native resources")?;
        if self
            .streaming
            .as_ref()
            .is_some_and(|s| world.0 <= s.world().0)
        {
            return Err("streaming world generation must advance".into());
        }
        let state = Stream::new(
            world,
            resources,
            streaming::Limits {
                slots: self.limits.stream_slots,
                pending: self.limits.stream_pending,
                bytes: self.limits.stream_bytes,
            },
        )
        .map_err(diagnostic)?;
        self.world
            .reset_sections(layout.min_y())
            .map_err(diagnostic)?;
        self.game_layout = Some(layout);
        self.streaming = Some(state);
        Ok(())
    }
    fn stream_key(&self, key: SectionKey) -> Result<(), String> {
        let layout = self
            .game_layout
            .ok_or("streaming dimension not configured")?;
        if usize::try_from(key[1])
            .ok()
            .is_none_or(|y| y >= layout.sections())
        {
            return Err("stream section Y outside dimension".into());
        }
        Ok(())
    }
    /// None = bounded interest-table backpressure. Dispatch the existing mesher
    /// only after receiving Some(ticket), and carry that ticket unchanged.
    pub fn request_stream_section(
        &mut self,
        key: SectionKey,
    ) -> Result<Option<SectionStamp>, String> {
        self.stream_key(key)?;
        self.streaming
            .as_mut()
            .ok_or("stamped streaming not started")?
            .request(key)
            .map_err(diagnostic)
    }
    /// Arc is shared, never a second mesher or full vertex-buffer copy. Count all
    /// retained Vec capacities, auxiliary caches and Arc payload/header; allocator
    /// overhead and external producer/worker references are not a process budget.
    /// The caller retains its Arc on Stale/Backpressure and may retry/reclaim it.
    pub fn queue_stream_section(
        &mut self,
        key: SectionKey,
        stamp: SectionStamp,
        mesh: &Arc<SectionMesh>,
    ) -> Result<Queued, String> {
        self.stream_key(key)?;
        if !self
            .streaming
            .as_ref()
            .ok_or("stamped streaming not started")?
            .accepts_completion(key, stamp)
        {
            return Ok(Queued::Stale);
        }
        if mesh.keep {
            return Err("native streamed sections do not support far-mesh keep/fade".into());
        }
        let bytes = retained(mesh)?;
        self.streaming
            .as_mut()
            .ok_or("stamped streaming not started")?
            .complete(key, stamp, Arc::clone(mesh), bytes)
            .map_err(diagnostic)
    }
    pub fn stream_usage(&self) -> Option<Usage> {
        self.streaming.as_ref().map(Stream::usage)
    }
    /// Revoke BEFORE touching GPU membership. On cleanup failure the ticket
    /// stays revoked, draws reject, and drain_streaming retries the removal.
    pub fn unload_stream_section(
        &mut self,
        key: SectionKey,
        stamp: SectionStamp,
    ) -> Result<bool, String> {
        let stream = self
            .streaming
            .as_mut()
            .ok_or("stamped streaming not started")?;
        if !stream.revoke(key, stamp) {
            return Ok(false);
        }
        self.world.unload_section(key).map_err(diagnostic)?;
        stream.removed(key, stamp);
        Ok(true)
    }
    /// Hard per-call count + charged-input byte limits. Zero max_jobs does no
    /// work; zero max_bytes permits removals but no uploads.
    /// an oversized head waits for an explicit larger budget/new ticket. GPU
    /// conversion scratch stays bounded separately by section_vertices. Failed
    /// upload keeps the previous resident mesh AND the pending head for retry.
    pub fn drain_streaming(&mut self, max_jobs: usize, max_bytes: usize) -> Result<Drain, String> {
        if self.streaming.is_none() {
            return Err("stamped streaming not started".into());
        }
        let mut result = Drain::default();
        while result.uploads + result.removals < max_jobs {
            if let Some((key, stamp)) = self.streaming.as_ref().unwrap().removal() {
                self.world.unload_section(key).map_err(diagnostic)?;
                self.streaming.as_mut().unwrap().removed(key, stamp);
                result.removals += 1;
                continue;
            }
            let Some((key, stamp, mesh, bytes)) = self.streaming.as_ref().unwrap().front() else {
                break;
            };
            if result
                .bytes
                .checked_add(bytes)
                .is_none_or(|n| n > max_bytes)
            {
                break;
            }
            let mesh = Arc::clone(mesh);
            if mesh.solid.is_empty() && mesh.translucent.is_empty() {
                // Air/last-block edits release GPU membership, not the active
                // interest ticket. Do not fill the resident section cap with air.
                self.world.unload_section(key).map_err(diagnostic)?;
            } else {
                render_bridge::upload_section(
                    &mut self.world,
                    key,
                    &mesh,
                    self.limits.section_vertices,
                )
                .map_err(diagnostic)?;
            }
            self.streaming.as_mut().unwrap().uploaded(key, stamp);
            result.uploads += 1;
            result.bytes += bytes;
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_mesh_charges_capacity_and_every_auxiliary_buffer() {
        let mut mesh = SectionMesh::default();
        let base = retained(&mesh).unwrap();
        mesh.solid.reserve_exact(8);
        assert_eq!(
            retained(&mesh).unwrap(),
            base + mesh.solid.capacity() * std::mem::size_of::<crate::mesher::TerrainVertex>()
        );
        mesh.translucent.reserve_exact(4);
        mesh.packed.reserve_exact(2);
        mesh.solid_tints.reserve_exact(3);
        mesh.block_entities.reserve_exact(4);
        mesh.lights.reserve_exact(5);
        mesh.sprite_layers.reserve_exact(6);
        let expected = base
            + mesh.solid.capacity() * std::mem::size_of::<crate::mesher::TerrainVertex>()
            + mesh.translucent.capacity() * std::mem::size_of::<crate::mesher::TerrainVertex>()
            + mesh.packed.capacity() * 24
            + mesh.solid_tints.capacity() * 4
            + mesh.block_entities.capacity() * 16
            + mesh.lights.capacity() * 16
            + mesh.sprite_layers.capacity() * 2;
        assert_eq!(retained(&mesh).unwrap(), expected);
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::{mesher::TerrainVertex, renderer_facade::RendererFacade};
    use horizon_render_api::{
        IDENTITY, render_contract::ResourceGeneration, resource_contract::TerrainAtlasLimits,
    };
    fn facade(renderer: &mut TerrainHudRenderer) -> RendererFacade<'_> {
        RendererFacade::legacy(
            renderer,
            frame::VisibleSections::default(),
            frame::Options::default(),
        )
    }
    fn mesh(layer: u16) -> Arc<SectionMesh> {
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
                .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000 | layer, u32::MAX, 0xf0f0)),
        );
        Arc::new(mesh)
    }
    fn draw(renderer: &mut TerrainHudRenderer, key: SectionKey, hud: &HudBatch) -> Vec<u8> {
        renderer
            .render(
                &SceneFrame {
                    view_projection: IDENTITY,
                    camera: [key[0] as f64 * 16., -64., key[2] as f64 * 16.],
                    clear: [0., 0., 0., 1.],
                    fog: None,
                    solid: &[key],
                    translucent: &[],
                },
                hud,
            )
            .unwrap();
        renderer.read_rgba([32, 32, 1, 1]).unwrap()
    }
    #[test]
    #[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
    fn issued_jobs_survive_backpressure_not_unload_reload_or_world_change() {
        let assets = super::super::test_assets();
        let atlas = HudAtlas::boot();
        let hud = HudBatch::new([64; 2]);
        let red = mesh(0);
        let green = mesh(1);
        let charge = retained(&red).unwrap();
        let preparation = ResourceLimits {
            terrain: TerrainAtlasLimits {
                page_size: 8,
                max_pages: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut renderer = TerrainHudRenderer::stage_assets(
            Context::new("osmesa", [64; 2]).unwrap(),
            &assets,
            &atlas,
            ResourceGeneration(77),
            &[255; 1024],
            Limits {
                stream_slots: 2,
                stream_pending: 1,
                stream_bytes: charge,
                gpu: Budget {
                    mesh_bytes: 256,
                    ..Budget::default()
                },
                ..Limits::default()
            },
            preparation,
        )
        .unwrap()
        .commit()
        .unwrap();
        let layout = crate::chunk::ColumnLayout::new(-64, 384).unwrap();
        facade(&mut renderer)
            .begin_streaming(WorldGeneration(1), layout)
            .unwrap();
        assert!(facade(&mut renderer).upload_section([0; 3], &red).is_err());
        assert!(facade(&mut renderer).clear_sections(layout).is_err());
        assert!(facade(&mut renderer).remove_columns([(0, 0)]).is_err());
        assert!(renderer.unload_section([0; 3]).is_err());
        assert!(
            facade(&mut renderer)
                .request_stream_section([0, -1, 0])
                .is_err()
        );
        let first = facade(&mut renderer)
            .request_stream_section([0; 3])
            .unwrap()
            .unwrap();
        let other = facade(&mut renderer)
            .request_stream_section([1, 0, 0])
            .unwrap()
            .unwrap();
        assert!(
            facade(&mut renderer)
                .request_stream_section([2, 0, 0])
                .unwrap()
                .is_none()
        );
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([0; 3], first, &red)
                .unwrap(),
            Queued::Accepted
        );
        assert_eq!(Arc::strong_count(&red), 2);
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([1, 0, 0], other, &green)
                .unwrap(),
            Queued::Backpressure
        );
        assert_eq!(Arc::strong_count(&green), 1);
        assert_eq!(
            facade(&mut renderer)
                .drain_streaming(0, usize::MAX)
                .unwrap(),
            Drain::default()
        );
        assert_eq!(
            facade(&mut renderer)
                .drain_streaming(1, charge - 1)
                .unwrap(),
            Drain::default()
        );
        assert_eq!(
            facade(&mut renderer).drain_streaming(1, charge).unwrap(),
            Drain {
                uploads: 1,
                removals: 0,
                bytes: charge
            }
        );
        assert_eq!(Arc::strong_count(&red), 1);
        assert_eq!(draw(&mut renderer, [0; 3], &hud), [255, 0, 0, 255]);
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([0; 3], first, &green)
                .unwrap(),
            Queued::Stale
        );
        assert!(
            facade(&mut renderer)
                .unload_stream_section([1, 0, 0], other)
                .unwrap()
        );
        let fresh = facade(&mut renderer)
            .request_stream_section([0; 3])
            .unwrap()
            .unwrap();
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([0; 3], fresh, &green)
                .unwrap(),
            Queued::Accepted
        );
        // A real GPU replacement budget failure: old 188-byte VBO/IBO stays
        // resident; the extra replacement cannot fit the 256-byte owner budget.
        assert!(facade(&mut renderer).drain_streaming(1, charge).is_err());
        assert_eq!(renderer.stream_usage().unwrap().pending, 1);
        assert_eq!(draw(&mut renderer, [0; 3], &hud), [255, 0, 0, 255]);
        assert!(
            !facade(&mut renderer)
                .unload_stream_section([0; 3], first)
                .unwrap()
        );
        assert!(
            facade(&mut renderer)
                .unload_stream_section([0; 3], fresh)
                .unwrap()
        );
        let revisit = facade(&mut renderer)
            .request_stream_section([0; 3])
            .unwrap()
            .unwrap();
        assert!(revisit.revision > fresh.revision);
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([0; 3], fresh, &red)
                .unwrap(),
            Queued::Stale
        );
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([0; 3], revisit, &green)
                .unwrap(),
            Queued::Accepted
        );
        facade(&mut renderer).drain_streaming(1, charge).unwrap();
        assert_eq!(draw(&mut renderer, [0; 3], &hud), [0, 255, 0, 255]);
        // Exercise the pre-clear guard with an unacknowledged removal (the core
        // fault/retry tests deliberately omit the backend acknowledgement).
        let before = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        assert!(renderer.streaming.as_mut().unwrap().revoke([0; 3], revisit));
        assert!(renderer.section_keys().is_err());
        assert!(renderer.render_menu(&hud, [0.; 4]).is_err());
        assert!(
            renderer
                .render_game_frame(
                    &frame::tests::source(&hud),
                    frame::VisibleSections::default(),
                    frame::Options::default()
                )
                .is_err()
        );
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), before);
        assert_eq!(
            facade(&mut renderer)
                .drain_streaming(1, 0)
                .unwrap()
                .removals,
            1
        );
        assert!(renderer.section_keys().unwrap().next().is_none());
        let pending = facade(&mut renderer)
            .request_stream_section([0; 3])
            .unwrap()
            .unwrap();
        let mut spare = SectionMesh::default();
        spare.packed.reserve_exact(1000); // unused capacity must still be charged
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([0; 3], pending, &Arc::new(spare))
                .unwrap(),
            Queued::Backpressure
        );
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([0; 3], pending, &red)
                .unwrap(),
            Queued::Accepted
        );
        assert!(
            renderer
                .stage_reload_assets(
                    &assets,
                    &atlas,
                    ResourceGeneration(77),
                    ResourceGeneration(78),
                    0,
                    ResourceLimits {
                        base_bytes: 1,
                        ..preparation
                    }
                )
                .is_err()
        );
        assert_eq!(renderer.stream_usage().unwrap().pending, 1);
        // Discarding a valid same-context stage also leaves its CPU queue live.
        drop(
            renderer
                .stage_reload_assets(
                    &assets,
                    &atlas,
                    ResourceGeneration(77),
                    ResourceGeneration(78),
                    0,
                    preparation,
                )
                .unwrap(),
        );
        assert_eq!(renderer.stream_usage().unwrap().pending, 1);
        facade(&mut renderer)
            .reload_assets(
                &assets,
                &atlas,
                ResourceGeneration(77),
                ResourceGeneration(78),
                0,
                preparation,
            )
            .unwrap();
        assert_eq!(renderer.stream_usage().unwrap().pending, 0);
        assert_eq!(Arc::strong_count(&red), 1);
        let after_reload = facade(&mut renderer)
            .request_stream_section([0; 3])
            .unwrap()
            .unwrap();
        assert_eq!(after_reload.world, WorldGeneration(1));
        assert_eq!(after_reload.resources, ResourceGeneration(78));
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([0; 3], pending, &red)
                .unwrap(),
            Queued::Stale
        );
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([0; 3], after_reload, &green)
                .unwrap(),
            Queued::Accepted
        );
        assert!(
            facade(&mut renderer)
                .begin_streaming(WorldGeneration(1), layout)
                .is_err()
        );
        facade(&mut renderer)
            .begin_streaming(WorldGeneration(2), layout)
            .unwrap();
        let after_world = facade(&mut renderer)
            .request_stream_section([0; 3])
            .unwrap()
            .unwrap();
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([0; 3], after_reload, &red)
                .unwrap(),
            Queued::Stale
        );
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section([0; 3], after_world, &green)
                .unwrap(),
            Queued::Accepted
        );
        facade(&mut renderer).drain_streaming(1, charge).unwrap();
        assert_eq!(draw(&mut renderer, [0; 3], &hud), [0, 255, 0, 255]);
        facade(&mut renderer)
            .unload_stream_section([0; 3], after_world)
            .unwrap();
        // Travel needs no accumulating tombstone table. Interest slots are
        // reclaimed while fresh global ticket nonces prevent an ABA revisit.
        for x in 0..32 {
            let key = [x, 0, 0];
            let stamp = facade(&mut renderer)
                .request_stream_section(key)
                .unwrap()
                .unwrap();
            assert_eq!(
                facade(&mut renderer)
                    .queue_stream_section(key, stamp, &green)
                    .unwrap(),
                Queued::Accepted
            );
            facade(&mut renderer).drain_streaming(1, charge).unwrap();
            assert_eq!(draw(&mut renderer, key, &hud), [0, 255, 0, 255]);
            assert_eq!(renderer.world.backend_mut().stats().meshes, 1);
            facade(&mut renderer)
                .unload_stream_section(key, stamp)
                .unwrap();
            assert_eq!(renderer.stream_usage().unwrap().slots, 0);
        }
        let key = [0; 3];
        let solid = facade(&mut renderer)
            .request_stream_section(key)
            .unwrap()
            .unwrap();
        facade(&mut renderer)
            .queue_stream_section(key, solid, &green)
            .unwrap();
        facade(&mut renderer).drain_streaming(1, charge).unwrap();
        let empty = facade(&mut renderer)
            .request_stream_section(key)
            .unwrap()
            .unwrap();
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section(key, empty, &Arc::new(SectionMesh::default()))
                .unwrap(),
            Queued::Accepted
        );
        assert_eq!(
            facade(&mut renderer)
                .drain_streaming(1, charge)
                .unwrap()
                .uploads,
            1
        );
        assert!(!facade(&mut renderer).has_section(key));
        assert_eq!(renderer.resident_sections(), 0);
        assert_eq!(renderer.stream_usage().unwrap().slots, 1);
        assert_eq!(
            facade(&mut renderer)
                .queue_stream_section(key, empty, &green)
                .unwrap(),
            Queued::Stale
        );
        facade(&mut renderer)
            .unload_stream_section(key, empty)
            .unwrap();
        assert_eq!(renderer.world.backend_mut().stats().meshes, 0);
        assert_eq!(renderer.stream_usage().unwrap().slots, 0);
    }
}
