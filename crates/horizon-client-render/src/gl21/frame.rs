//! A frame as a list of driver calls: what to upload, and what to draw where.
//!
//! The GL 2.1 renderer's frame is three steps, and the first two are arithmetic, so they live here
//! instead of in the file that calls `glow`:
//!
//! 1. **Store** every section in view in the one vertex buffer (`arena.rs`), solid faces first and
//!    see-through ones after, and remember which sprite each run of vertices draws. A section whose
//!    `revision` did not change keeps the range it had, and nothing of it is uploaded twice.
//! 2. **Order** the sections' runs into the frame's passes (`draws.rs`): sky, solid, see-through
//!    farthest first, and the present quad last.
//! 3. **Draw** what that produced: one `glDrawArrays` of triangles per `Draw`, with the rest of
//!    its state (`Pass`, the section's fade) set from the same struct.
//!
//! So the renderer's loop is `for draw in frame.draws() { … }` and the interesting mistakes — a run
//! pointing at another section's vertices, a see-through face drawn before the solid one behind it,
//! vertices uploaded every frame because a revision was read wrong — are test failures here instead
//! of a picture no one can explain on a 2006 driver.
//!
//! Two behaviours come from `arena.rs` and are worth knowing when reading a frame: the buffer is
//! appended to, so a changed section's old vertices stay until it starts over; and when it has no
//! room left the whole frame is stored again in an empty one (`restarts`, a log line that in a
//! healthy frame is never more than one). A section that does not fit even then is dropped from
//! the frame: a hole in the distance, not a crash.

use std::collections::BTreeMap;

use crate::gl21::arena::{Arena, Slot, Upload};
use crate::gl21::draws::{self, Pass, SKY, SectionDraw, SectionRuns};
use crate::gl21::vertex::Batches;
use crate::mesher::TerrainVertex;

/// The sky's vertices for one frame, and the revision that says whether they changed.
pub type Sky<'a> = (&'a [TerrainVertex], u32);

/// One section to draw this frame: its vertices as the mesher built them, and what the frame needs
/// to place it.
#[derive(Clone, Copy, Debug)]
pub struct Section<'a> {
    /// What the caller names the section by: its index in the game's own list, typically. Stored
    /// sections are looked up by it from one frame to the next.
    pub handle: u32,
    /// Which version of the mesh this is. Equal to the one in the last frame that stored this
    /// handle, the vertices are not stored again and the frame reuses the range it had.
    pub revision: u32,
    /// The section's origin relative to the camera, which `u_origin` adds to every vertex: the
    /// mesher writes section-relative positions, and a camera-relative section origin keeps the
    /// floats small enough for the 24-bit mantissa the R500 multiplies with.
    pub origin: [f32; 3],
    /// The square of the distance from the camera to the section's centre (see-through faces are
    /// sorted by it).
    pub distance_sq: f32,
    /// The section's shade-in, 0..=1 (`u_fade`).
    pub visibility: f32,
    /// Its solid and cut-out faces, in the mesher's order.
    pub solid: &'a [TerrainVertex],
    /// Its see-through faces.
    pub translucent: &'a [TerrainVertex],
}

impl<'a> Section<'a> {
    /// A section with no see-through faces.
    pub fn solid_only(
        handle: u32,
        revision: u32,
        origin: [f32; 3],
        distance_sq: f32,
        solid: &'a [TerrainVertex],
    ) -> Section<'a> {
        Section {
            handle,
            revision,
            origin,
            distance_sq,
            visibility: 1.0,
            solid,
            translucent: &[],
        }
    }
}

/// A run of one section's vertices that one call draws: the sprite whose atlas rectangle is
/// `u_sprite`, and the range of the vertex buffer it is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    /// The sprite's layer, which `Atlas::rect` turns into the draw's `u_sprite`.
    pub sprite: u16,
    /// The first vertex in the arena's buffer.
    pub begin: u32,
    /// How many vertices: a multiple of three, as the mesher emits triangles.
    pub count: u32,
}

/// One driver call: `glDrawArrays(GL_TRIANGLES, begin, count)` and the state around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    pub sprite: u16,
    pub begin: u32,
    pub count: u32,
    pub pass: Pass,
    /// The section's shade-in for `u_fade`. The sky's and the present quad's calls carry 1.
    pub visibility: f32,
    /// The section's origin relative to the camera: the draw's `u_origin`. The sky's calls carry
    /// zeros, because the sky is drawn where the camera is.
    pub origin: [f32; 3],
    /// Which section this is, or `SKY`: for a log line and for a caller that wants to group its
    /// uniforms per section.
    pub section: u32,
}

/// A stored section: the version of its mesh, where it is, and its runs.
#[derive(Clone, Debug)]
struct Entry {
    revision: u32,
    slot: Slot,
    /// Solid runs first, then see-through ones: the order `draws::plan` names them in.
    runs: Vec<Run>,
    /// How many of `runs` are solid faces.
    solid_runs: u32,
}

impl Entry {
    /// The `SectionDraw` for this entry: its run counts, by pass.
    fn draw(&self, distance_sq: f32, visibility: f32) -> SectionDraw {
        SectionDraw {
            distance_sq,
            visibility,
            solid: SectionRuns::new(0, self.solid_runs),
            translucent: SectionRuns::new(
                self.solid_runs,
                self.runs.len() as u32 - self.solid_runs,
            ),
        }
    }
}

/// The frame's vertex buffer and the calls that draw it.
#[derive(Clone, Debug)]
pub struct Frame {
    arena: Arena,
    /// The stored sections, by handle: what the next frame reuses when a revision matches.
    entries: BTreeMap<u32, Entry>,
    /// The sky's own vertices and runs, if the frame draws any.
    sky: Option<(Entry, u32)>,
    draws: Vec<Draw>,
    present: bool,
    dropped: u32,
    restarts: u32,
}

impl Frame {
    /// A frame whose vertex buffer holds `capacity` vertices: the plan's §5.2 budget for the whole
    /// view distance is a couple of megabytes, which is 65 536 vertices and change.
    pub fn new(capacity: u32) -> Frame {
        Frame {
            arena: Arena::new(capacity),
            entries: BTreeMap::new(),
            sky: None,
            draws: Vec::new(),
            present: false,
            dropped: 0,
            restarts: 0,
        }
    }

    /// Store and order this frame: `sections` in the order the caller wants their solid faces
    /// drawn, and `sky` (its vertices and a revision) when the frame draws any. `present` adds the
    /// full-screen quad that shows the frame in the window.
    pub fn build(&mut self, sections: &[Section<'_>], sky: Option<Sky<'_>>, present: bool) {
        self.draws.clear();
        self.dropped = 0;
        self.present = present;
        let mut attempt = 0;
        loop {
            if self.store_all(sections, sky) {
                break;
            }
            // The buffer filled up: store the frame again from the start, once, in an empty one.
            attempt += 1;
            if attempt > 1 {
                break;
            }
            self.restart();
            // What the first attempt could not fit says nothing about this one.
            self.dropped = 0;
        }
        self.order(sections, present);
        // Sections that left the view distance take their ranges with them (their vertices stay in
        // the buffer until it starts over, which is cheaper than copying).
        let live: Vec<u32> = sections.iter().map(|section| section.handle).collect();
        self.forget(&live);
    }

    /// The frame's calls, in the order the driver makes them.
    pub fn draws(&self) -> &[Draw] {
        &self.draws
    }

    /// The ranges to send to the GPU since the last call: what changed, and nothing else.
    pub fn uploads(&mut self) -> Vec<Upload> {
        self.arena.uploads()
    }

    /// The vertices a range names, for `glBufferSubData`.
    pub fn vertices(&self, upload: Upload) -> &[crate::gl21::vertex::GlVertex] {
        self.arena.vertices(upload.begin, upload.len)
    }

    /// The buffer, for its size and what it holds.
    pub fn buffer(&self) -> &Arena {
        &self.arena
    }

    /// How many sections are stored.
    pub fn stored(&self) -> u32 {
        self.arena.stored()
    }

    /// Where a section is in the buffer, which is also how a caller finds the vertices of a draw.
    pub fn slot(&self, handle: u32) -> Option<Slot> {
        self.entries.get(&handle).map(|entry| entry.slot)
    }

    /// How many sections this frame could not fit; 0 in a frame with room.
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    /// How many times the buffer was started over for this frame: 0 normally, and 1 when it filled
    /// up and the frame was stored again.
    pub fn restarts(&self) -> u32 {
        self.restarts
    }

    /// Whether the frame draws the present quad.
    pub fn present(&self) -> bool {
        self.present
    }

    /// Store every section, reusing what `revision` says is unchanged: `false` when the buffer had
    /// no room, so that the caller starts it over.
    fn store_all(&mut self, sections: &[Section<'_>], sky: Option<Sky<'_>>) -> bool {
        if let Some((vertices, revision)) = sky {
            let stored = match &self.sky {
                Some((entry, stored_revision)) if *stored_revision == revision => true,
                _ => self.store_sky(vertices, revision),
            };
            if !stored {
                return false;
            }
        } else {
            self.sky = None;
        }
        for section in sections {
            let unchanged = self
                .entries
                .get(&section.handle)
                .is_some_and(|entry| entry.revision == section.revision);
            if unchanged {
                continue;
            }
            if !self.store_section(section) {
                // Counted once, by the attempt that ran out of room; the restart before it is not
                // this section's fault.
                self.dropped += 1;
                return false;
            }
        }
        true
    }

    /// Store one section's vertices, solid faces first, and its runs.
    fn store_section(&mut self, section: &Section<'_>) -> bool {
        let solid = Batches::of(section.solid);
        let translucent = Batches::of(section.translucent);
        let (solid_runs, translucent_runs) = draws::section_runs(&solid.1, &translucent.1);
        let mut vertices = solid.0;
        let first_translucent = vertices.len() as u32;
        vertices.extend_from_slice(&translucent.0);
        let runs = runs_of(&solid.1, 0);
        let mut all = runs;
        all.extend(runs_of(&translucent.1, first_translucent));
        let Some(slot) = self
            .arena
            .store(section.handle, &vertices, all.len() as u32)
        else {
            return false;
        };
        let entry = Entry {
            revision: section.revision,
            slot,
            runs: shifted(all, slot.begin),
            solid_runs: solid_runs.runs,
        };
        debug_assert_eq!(translucent_runs.first_run, entry.solid_runs);
        self.entries.insert(section.handle, entry);
        true
    }

    /// Store the sky's vertices, which have no section and no revision of their own.
    fn store_sky(&mut self, vertices: &[TerrainVertex], revision: u32) -> bool {
        let (out, batches) = Batches::of(vertices);
        let runs = runs_of(&batches, 0);
        let Some(slot) = self.arena.store_unowned(&out, runs.len() as u32) else {
            return false;
        };
        let entry = Entry {
            revision,
            slot,
            runs: shifted(runs, slot.begin),
            solid_runs: 0,
        };
        self.sky = Some((entry, revision));
        true
    }

    /// Order the stored sections into the frame's calls.
    fn order(&mut self, sections: &[Section<'_>], present: bool) {
        let draws: Vec<SectionDraw> = sections
            .iter()
            .filter_map(|section| {
                let entry = self.entries.get(&section.handle)?;
                Some(entry.draw(section.distance_sq, section.visibility))
            })
            .collect();
        let sky = self
            .sky
            .as_ref()
            .map(|(entry, _)| SectionRuns::new(0, entry.runs.len() as u32));
        let plan = draws::plan(&draws, sky, present);
        let mut out = Vec::with_capacity(plan.calls.len());
        for (call, pass) in plan.calls.iter().zip(&plan.pass) {
            // The plan names a section by its place in `sections`; a draw names it by the handle the
            // caller gave it, which is what its uniforms are looked up by.
            let (index, run) = *call;
            let (handle, entry, visibility, origin) = if index == SKY {
                let Some((entry, _)) = self.sky.as_ref() else {
                    continue;
                };
                (SKY, entry, 1.0, [0.0; 3])
            } else {
                let Some(section) = sections.get(index as usize) else {
                    continue;
                };
                let Some(entry) = self.entries.get(&section.handle) else {
                    continue;
                };
                (section.handle, entry, section.visibility, section.origin)
            };
            let Some(run) = entry.runs.get(run as usize) else {
                continue;
            };
            out.push(Draw {
                sprite: run.sprite,
                begin: run.begin,
                count: run.count,
                pass: *pass,
                visibility,
                origin,
                section: handle,
            });
        }
        self.draws = out;
    }

    /// Start the buffer over and forget every stored section: the frame is stored again whole.
    fn restart(&mut self) {
        self.restarts += 1;
        self.arena.reset();
        self.entries.clear();
        self.sky = None;
    }

    /// Forget the stored sections whose handles are not `live`: they left the view distance.
    fn forget(&mut self, live: &[u32]) {
        let gone: Vec<u32> = self
            .entries
            .keys()
            .copied()
            .filter(|handle| !live.contains(handle))
            .collect();
        for handle in gone {
            self.entries.remove(&handle);
            self.arena.evict(handle);
        }
    }
}

/// One `Batches`' runs, as `Run`s whose `begin` is an offset from the start of its own list.
fn runs_of(batches: &Batches, offset: u32) -> Vec<Run> {
    batches
        .runs
        .iter()
        .map(|(sprite, first, count)| Run {
            sprite: *sprite,
            begin: offset + first,
            count: *count,
        })
        .collect()
}

/// The same runs, `base` vertices further on: where they are in the arena's buffer.
fn shifted(runs: Vec<Run>, base: u32) -> Vec<Run> {
    runs.into_iter()
        .map(|run| Run {
            begin: base + run.begin,
            ..run
        })
        .collect()
}

/// How many calls the frame's draws are, the present quad included.
pub fn draw_calls_of(frame: &Frame) -> usize {
    frame.draws.len() + usize::from(frame.present)
}

/// The passes a frame's draws make, in order: the plan without its section numbers.
pub fn passes(frame: &Frame) -> Vec<Pass> {
    frame.draws.iter().map(|draw| draw.pass).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `count` vertices of one sprite, x from 0 up, so that a range can be recognised by its count.
    fn vertices(count: usize, sprite: u16) -> Vec<TerrainVertex> {
        (0..count)
            .map(|i| TerrainVertex::new([i as f32, 0.0, 0.0], [0.0, 0.0], sprite, 0, 0))
            .collect()
    }

    fn section<'a>(
        handle: u32,
        distance_sq: f32,
        solid: &'a [TerrainVertex],
        translucent: &'a [TerrainVertex],
    ) -> Section<'a> {
        Section {
            handle,
            revision: 1,
            origin: [0.0; 3],
            distance_sq,
            visibility: 1.0,
            solid,
            translucent,
        }
    }

    #[test]
    fn a_section_is_stored_solid_first_and_its_runs_point_into_the_buffer() {
        let solid = vertices(6, 3);
        let translucent = vertices(3, 3);
        let mut frame = Frame::new(1024);
        frame.build(&[section(7, 4.0, &solid, &translucent)], None, false);
        // One buffer, solid faces then see-through ones.
        assert_eq!(frame.draws().len(), 2);
        let solid_draw = frame.draws()[0];
        let translucent_draw = frame.draws()[1];
        assert_eq!(solid_draw.pass, Pass::Solid);
        assert_eq!((solid_draw.begin, solid_draw.count), (0, 6));
        assert_eq!(translucent_draw.pass, Pass::Translucent);
        assert_eq!((translucent_draw.begin, translucent_draw.count), (6, 3));
        // Same sprite: the atlas rectangle is the same for both.
        assert_eq!(solid_draw.sprite, 3);
        assert_eq!(translucent_draw.sprite, 3);
        // The frame uploaded the section once.
        assert_eq!(frame.uploads(), vec![Upload::at(0, 9)]);
    }

    #[test]
    fn see_through_faces_are_drawn_after_the_solid_ones_farthest_first() {
        let near = vertices(3, 1);
        let far = vertices(3, 2);
        let mut frame = Frame::new(1024);
        frame.build(
            &[section(1, 4.0, &near, &near), section(2, 100.0, &far, &far)],
            None,
            false,
        );
        let passes = passes(&frame);
        let solids = [Pass::Solid, Pass::Solid];
        let blend = [Pass::Translucent, Pass::Translucent];
        assert_eq!(passes, solids.into_iter().chain(blend).collect::<Vec<_>>());
        // The see-through calls are the far section's, then the near one's.
        let translucent: Vec<u32> = frame
            .draws()
            .iter()
            .filter(|draw| draw.pass == Pass::Translucent)
            .map(|draw| draw.section)
            .collect();
        assert_eq!(translucent, vec![2, 1]);
    }

    #[test]
    fn an_unchanged_section_is_not_stored_again() {
        let solid = vertices(6, 3);
        let mut frame = Frame::new(1024);
        frame.build(&[section(9, 1.0, &solid, &[])], None, false);
        let begin = frame.draws()[0].begin;
        let _ = frame.uploads();
        // The same revision: the frame keeps the range it had and uploads nothing.
        frame.build(&[section(9, 1.0, &solid, &[])], None, false);
        assert_eq!(frame.draws()[0].begin, begin);
        let sent = frame.uploads();
        assert!(sent.is_empty(), "nothing changed, nothing to send");
        assert_eq!(frame.stored(), 1);
    }

    #[test]
    fn a_changed_section_is_appended_and_drawn_from_its_new_range() {
        let first = vertices(6, 3);
        let second = vertices(9, 3);
        let mut frame = Frame::new(1024);
        frame.build(&[section(9, 1.0, &first, &[])], None, false);
        let _ = frame.uploads();
        let mut moved = section(9, 1.0, &second, &[]);
        moved.revision = 2;
        frame.build(&[moved], None, false);
        assert_eq!(frame.draws()[0].begin, 6, "appended after the old range");
        assert_eq!(frame.draws()[0].count, 9);
        assert_eq!(frame.uploads(), vec![Upload::at(6, 9)]);
        assert_eq!(frame.stored(), 1, "one section, stored twice");
    }

    #[test]
    fn a_full_buffer_starts_over_once_and_still_draws_everything() {
        let a = vertices(6, 1);
        // Room for one section: the first frame takes `a`, a change to it needs the buffer again.
        let mut frame = Frame::new(8);
        frame.build(&[section(1, 1.0, &a, &[])], None, false);
        let mut changed = section(1, 1.0, &a, &[]);
        changed.revision = 2;
        frame.build(&[changed], None, false);
        // Appending `a` again does not fit past the first copy: the frame starts over.
        assert_eq!(frame.restarts(), 1);
        assert_eq!(frame.draws().len(), 1);
        assert_eq!(frame.draws()[0].begin, 0);
        assert_eq!(frame.dropped(), 0);
        // The restarted buffer has to send everything: one section, at the start.
        assert_eq!(frame.uploads(), vec![Upload::at(0, 6)]);
    }

    #[test]
    fn a_section_that_cannot_fit_is_dropped_and_the_rest_is_drawn() {
        let big = vertices(64, 1);
        let small = vertices(3, 2);
        let mut frame = Frame::new(16);
        frame.build(
            &[section(1, 1.0, &small, &[]), section(2, 2.0, &big, &[])],
            None,
            false,
        );
        // The first attempt fills up at `big`; the second runs out again at the same place, so the
        // frame is what fitted: the small section, drawn, and `big` missing from it.
        assert_eq!(frame.dropped(), 1);
        assert_eq!(frame.restarts(), 1, "it tried to make room once");
        assert_eq!(frame.draws().len(), 1);
        assert_eq!(frame.draws()[0].section, 1);
    }

    #[test]
    fn a_section_s_slot_says_where_its_vertices_are() {
        let solid = vertices(6, 3);
        let translucent = vertices(3, 4);
        let mut frame = Frame::new(1024);
        frame.build(&[section(5, 1.0, &solid, &translucent)], None, false);
        let slot = frame.slot(5).expect("stored");
        assert_eq!((slot.begin, slot.len, slot.runs), (0, 9, 2));
        // The range is the section's vertices, solid faces first.
        assert_eq!(frame.vertices(Upload::at(slot.begin, slot.len)).len(), 9);
        assert_eq!(frame.slot(6), None);
    }

    #[test]
    fn a_section_that_left_the_view_is_forgotten() {
        let a = vertices(6, 1);
        let b = vertices(6, 2);
        let mut frame = Frame::new(1024);
        frame.build(
            &[section(1, 1.0, &a, &[]), section(2, 2.0, &b, &[])],
            None,
            false,
        );
        assert_eq!(frame.stored(), 2);
        let _ = frame.uploads();
        frame.build(&[section(2, 2.0, &b, &[])], None, false);
        assert_eq!(frame.stored(), 1);
        assert_eq!(frame.draws().len(), 1);
        assert_eq!(frame.draws()[0].section, 2);
        // Its vertices stay in the buffer until it starts over: only the second is drawn.
        assert!(frame.uploads().is_empty());
    }

    #[test]
    fn the_sky_is_drawn_before_the_world_and_the_present_quad_is_the_flag() {
        let world = vertices(6, 3);
        let sky = vertices(6, 0);
        let mut frame = Frame::new(1024);
        frame.build(&[section(1, 1.0, &world, &[])], Some((&sky, 1)), true);
        assert_eq!(passes(&frame), vec![Pass::Sky, Pass::Solid]);
        assert_eq!(frame.draws()[0].section, SKY);
        assert_eq!(frame.draws()[0].begin, 0, "the sky is stored first");
        assert_eq!(frame.draws()[1].begin, 6);
        assert!(frame.present());
        // The same sky in the next frame is not stored again.
        let _ = frame.uploads();
        frame.build(&[section(1, 1.0, &world, &[])], Some((&sky, 1)), true);
        assert!(frame.uploads().is_empty());
        assert_eq!(frame.draws()[0].section, SKY);
    }

    #[test]
    fn a_section_with_nothing_to_draw_is_no_call() {
        let mut frame = Frame::new(64);
        frame.build(&[section(1, 1.0, &[], &[])], None, false);
        assert!(frame.draws().is_empty());
        assert!(!frame.present());
    }
}
