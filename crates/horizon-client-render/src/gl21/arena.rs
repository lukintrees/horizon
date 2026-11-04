//! One vertex buffer for every section in view, and what of it to upload this frame.
//!
//! GL 2.1 has no buffer orphaning and the X1650's driver copies what it is given, so the renderer
//! keeps one buffer the size of the whole view distance and refills the parts that changed
//! (`docs/opengl21-plan.md`, phase 3 item 3: "one VBO arena per page, a draw per section"). What the
//! driver needs from it is a list: which ranges of the buffer to send to the GPU this frame, at which
//! byte offset. That list is what this works out, and it is pure arithmetic, so it is tested without
//! a GPU like the rest of `gl21`.
//!
//! The policy is a bump allocator on purpose, and it is the one vanilla's own chunk renderer used:
//!
//! - **Grow by appending.** A section that changed is written at the end of the buffer and its old
//!   range is forgotten. Nothing is copied, and the frame that changed one section uploads one
//!   section.
//! - **Start over when it fills.** When the buffer has no room, `store` says so (`Ok(false)`) instead
//!   of growing: the caller clears the frame's sections and stores them again, which re-uploads
//!   everything once rather than copying on every frame.
//! - **Nothing is uploaded twice.** `uploads` drains the list, so a frame that changed nothing uploads
//!   nothing — with a 2–4 MB buffer and 2006 bandwidth, that is the difference between 30 and 5
//!   frames a second when the player stands still.
//!
//! The renderer above owns the buffer's lifetime; this owns which sections are in it and which parts
//! the GPU has not seen.

use crate::gl21::vertex::GlVertex;

/// One section in the arena: where its vertices are, and which sprite runs they hold. The runs
/// themselves stay with the caller (`gl21::draws::SectionDraw`); a run's vertices are the section's
/// own range plus the run's offset, so inserting a section shifts no run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    /// The first vertex in the arena's buffer.
    pub begin: u32,
    /// How many vertices.
    pub len: u32,
    /// How many draw calls the section's runs are, both lists together.
    pub runs: u32,
}

/// A range of the buffer the GPU has not seen: (first vertex, how many).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Upload {
    pub begin: u32,
    pub len: u32,
}

impl Slot {
    /// A section at `begin`, `len` vertices long, in `runs` draw calls.
    pub const fn at(begin: u32, len: u32, runs: u32) -> Slot {
        Slot { begin, len, runs }
    }
}

impl Upload {
    /// The range from `begin`, `len` vertices long.
    pub const fn at(begin: u32, len: u32) -> Upload {
        Upload { begin, len }
    }

    /// The byte offset `glBufferSubData` takes, for a stride of `gl21::vertex::STRIDE`.
    pub fn byte_offset(self) -> usize {
        self.begin as usize * crate::gl21::vertex::STRIDE as usize
    }

    /// The range's length in bytes.
    pub fn byte_len(self) -> usize {
        self.len as usize * crate::gl21::vertex::STRIDE as usize
    }
}

/// The frame's vertex buffer: fixed capacity, appended to, started over when full.
#[derive(Clone, Debug)]
pub struct Arena {
    capacity: u32,
    vertices: Vec<GlVertex>,
    slots: Vec<Option<Slot>>,
    free_handles: Vec<u32>,
    /// Ranges the GPU has not seen yet, in the order they were written.
    pending: Vec<Upload>,
    /// How many vertices have moved to the GPU since the last `reset`.
    written: u32,
    /// Sections stored since the last `reset`: what a caller re-stores when it starts over.
    stored: u32,
}

impl Arena {
    /// An arena of `capacity` vertices: 65 536 sections of a few thousand vertices each live in a
    /// couple of megabytes, which is the plan's §5.2 budget for the whole view distance.
    pub fn new(capacity: u32) -> Arena {
        Arena {
            capacity,
            vertices: Vec::with_capacity(capacity as usize),
            slots: Vec::new(),
            free_handles: Vec::new(),
            pending: Vec::new(),
            written: 0,
            stored: 0,
        }
    }

    /// The buffer's capacity, in vertices.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// How many vertices are in the buffer, uploaded or not.
    pub fn len(&self) -> u32 {
        self.vertices.len() as u32
    }

    /// Whether the buffer holds no vertices.
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// How many sections are stored.
    pub fn stored(&self) -> u32 {
        self.stored
    }

    /// The vertices from `begin` on: what `glBufferSubData` sends, sliced.
    pub fn vertices(&self, begin: u32, len: u32) -> &[GlVertex] {
        &self.vertices[begin as usize..(begin + len) as usize]
    }

    /// Store a section's vertices, appending them, and say where they went. `None` when the buffer
    /// has no room left: the caller then calls `reset` and stores the frame's sections again.
    ///
    /// `handle` is whatever the caller names the section by; storing the same handle twice replaces
    /// its slot and leaves the old range behind until the next `reset` (see the module's note on
    /// appending).
    pub fn store(&mut self, handle: u32, vertices: &[GlVertex], runs: u32) -> Option<Slot> {
        let slot = self.append(vertices, runs)?;
        self.claim(handle, slot);
        Some(slot)
    }

    /// Store vertices that belong to no handle — the sky's own — and say where they went. They are
    /// appended like a section's and uploaded the same way, but no slot names them, so nothing
    /// evicts them and only `reset` takes them back.
    pub fn store_unowned(&mut self, vertices: &[GlVertex], runs: u32) -> Option<Slot> {
        self.append(vertices, runs)
    }

    /// Append `vertices` at the end of the buffer, `None` when the buffer has no room.
    fn append(&mut self, vertices: &[GlVertex], runs: u32) -> Option<Slot> {
        let len = vertices.len() as u32;
        if self.written + len > self.capacity {
            return None;
        }
        let begin = self.vertices.len() as u32;
        self.vertices.extend_from_slice(vertices);
        self.pending.push(Upload { begin, len });
        self.written += len;
        Some(Slot::at(begin, len, runs))
    }

    /// The slot a handle names, if it is stored.
    pub fn slot(&self, handle: u32) -> Option<Slot> {
        self.slots.get(handle as usize).copied().flatten()
    }

    /// Forget a handle: it leaves the draw list and is uploaded no more. Its vertices stay in the
    /// buffer until the next `reset`, which costs room but no copy — a section leaving the view
    /// distance is common and copying for it is not.
    pub fn evict(&mut self, handle: u32) -> Option<Slot> {
        let index = handle as usize;
        let slot = self.slots.get_mut(index)?.take()?;
        self.free_handles.push(handle);
        self.stored = self.stored.saturating_sub(1);
        Some(slot)
    }

    /// The ranges to upload this frame, and mark as uploaded: `glBufferSubData` once per range,
    /// fewer when `coalesce` joins neighbours whose one copy is cheaper than two calls.
    pub fn uploads(&mut self) -> Vec<Upload> {
        let pending = std::mem::take(&mut self.pending);
        coalesce(&pending)
    }

    /// The ranges waiting to be uploaded, without taking them: for a log line or a test.
    pub fn pending(&self) -> &[Upload] {
        &self.pending
    }

    /// Start the buffer over: no vertices, no slots, no uploads waiting. The caller stores the
    /// frame's sections again, and the first `uploads` after it sends everything.
    pub fn reset(&mut self) {
        self.vertices.clear();
        self.slots.clear();
        self.free_handles.clear();
        self.pending.clear();
        self.written = 0;
        self.stored = 0;
    }

    /// Put `slot` under `handle`: the caller's numbering is the index into `slots`.
    fn claim(&mut self, handle: u32, slot: Slot) {
        let index = handle as usize;
        if self.slots.len() <= index {
            self.slots.resize(index + 1, None);
        }
        let replaced = self.slots[index].replace(slot).is_some();
        if !replaced {
            self.stored += 1;
        }
    }
}

/// `uploads` joined where one range ends where the next begins, or comes within `GAP` vertices of
/// it: a driver's per-call cost is worth more than a few hundred wasted vertices.
pub fn coalesce(pending: &[Upload]) -> Vec<Upload> {
    let mut out: Vec<Upload> = Vec::new();
    for upload in pending {
        match out.last_mut() {
            Some(last) if joins(last, upload) => {
                last.len = upload.begin + upload.len - last.begin;
            }
            _ => out.push(*upload),
        }
    }
    out
}

/// Whether one copy can carry `next` as well as `last`: it starts after `last` ends, and no more
/// than `GAP` vertices later.
fn joins(last: &Upload, next: &Upload) -> bool {
    const GAP: u32 = 256;
    let end = last.begin + last.len;
    end <= next.begin && next.begin - end <= GAP
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl21::vertex::Batches;
    use crate::mesher::TerrainVertex;

    /// A section of `count` vertices of one sprite, so that it is one run.
    fn section(count: usize, sprite: u16) -> Vec<TerrainVertex> {
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let pos = [i as f32, 0.0, 0.0];
            out.push(TerrainVertex::new(pos, [0.0, 0.0], sprite, 0, 0));
        }
        out
    }

    /// `mesher` vertices as the arena stores them, and how many runs they are.
    fn gl(vertices: &[TerrainVertex]) -> (Vec<GlVertex>, u32) {
        let (out, batches) = Batches::of(vertices);
        (out, batches.runs.len() as u32)
    }

    #[test]
    fn a_stored_section_lands_at_the_end_and_is_uploaded_once() {
        let mut arena = Arena::new(1024);
        let (a, runs) = gl(&section(4, 1));
        let (b, _) = gl(&section(2, 1));
        let first = arena.store(0, &a, runs).unwrap();
        let second = arena.store(1, &b, 1).unwrap();
        assert_eq!(first, Slot::at(0, 4, 1));
        assert_eq!(second, Slot::at(4, 2, 1));
        assert_eq!(arena.len(), 6);
        assert_eq!(arena.vertices(4, 2), &b[..]);
        // The two ranges are adjacent, so one copy carries both.
        assert_eq!(arena.uploads(), vec![Upload::at(0, 6)]);
        // Drained: a second frame uploads nothing until something changes.
        assert!(arena.uploads().is_empty());
    }

    #[test]
    fn a_changed_section_is_appended_and_only_it_is_uploaded() {
        let mut arena = Arena::new(1024);
        let (a, _) = gl(&section(4, 1));
        arena.store(0, &a, 1).unwrap();
        arena.uploads();
        // The same handle again: the new vertices go to the end, the slot points there.
        let (changed, _) = gl(&section(4, 2));
        let slot = arena.store(0, &changed, 1).unwrap();
        assert_eq!(slot, Slot::at(4, 4, 1));
        assert_eq!(arena.slot(0), Some(slot));
        assert_eq!(arena.uploads(), vec![Upload::at(4, 4)]);
        // The old copy stayed: eight vertices for four live ones. A reset is what takes the room
        // back, which is cheaper than copying on the frame that changed.
        assert_eq!(arena.len(), 8);
        assert_eq!(arena.stored(), 1);
    }

    #[test]
    fn a_full_buffer_says_so_instead_of_growing() {
        let mut arena = Arena::new(6);
        let (a, _) = gl(&section(4, 1));
        let (b, _) = gl(&section(4, 1));
        assert!(arena.store(0, &a, 1).is_some());
        // Four and four do not fit six.
        assert_eq!(arena.store(1, &b, 1), None);
        assert_eq!(arena.len(), 4);
        // Starting over makes room for the frame's sections again.
        arena.reset();
        assert!(arena.is_empty());
        assert_eq!(arena.stored(), 0);
        let (small, _) = gl(&section(2, 1));
        assert!(arena.store(1, &small, 1).is_some());
        assert_eq!(arena.slot(1), Some(Slot::at(0, 2, 1)));
        assert_eq!(arena.uploads(), vec![Upload::at(0, 2)]);
    }

    #[test]
    fn an_evicted_section_leaves_the_list() {
        let mut arena = Arena::new(1024);
        let (a, _) = gl(&section(3, 1));
        arena.store(0, &a, 1).unwrap();
        arena.store(1, &a, 1).unwrap();
        assert_eq!(arena.evict(0), Some(Slot::at(0, 3, 1)));
        assert_eq!(arena.slot(0), None);
        assert_eq!(arena.evict(0), None, "evicting twice is not a fault");
        assert_eq!(arena.stored(), 1);
        // The next one goes to the buffer's end, not into the freed range: appending is the point,
        // and a hole is not a place to write.
        assert_eq!(arena.store(0, &a, 1).unwrap(), Slot::at(6, 3, 1));
        assert_eq!(arena.stored(), 2);
    }

    #[test]
    fn uploads_coalesce_only_what_touches() {
        let adjacent = [Upload::at(0, 4), Upload::at(4, 4)];
        assert_eq!(coalesce(&adjacent), vec![Upload::at(0, 8)]);
        // 96 skipped vertices are worth one copy.
        let near = [Upload::at(0, 4), Upload::at(100, 4)];
        assert_eq!(coalesce(&near), vec![Upload::at(0, 104)]);
        let far = [Upload::at(0, 4), Upload::at(1024, 4)];
        assert_eq!(coalesce(&far), far.to_vec(), "two copies");
        // Ranges that come backwards (or overlap) are never merged: a copy cannot go back in time.
        let backwards = [Upload::at(8, 4), Upload::at(0, 4)];
        assert_eq!(coalesce(&backwards), backwards.to_vec());
    }

    #[test]
    fn a_gap_of_two_hundred_and_fifty_six_vertices_is_the_cap() {
        // 256 skipped vertices join, 257 do not: the constant is the line, either side of it.
        let at_cap = [Upload::at(0, 4), Upload::at(260, 4)];
        assert_eq!(coalesce(&at_cap), vec![Upload::at(0, 264)]);
        let past_cap = [Upload::at(0, 4), Upload::at(261, 4)];
        assert_eq!(coalesce(&past_cap), past_cap.to_vec());
    }

    #[test]
    fn an_upload_knows_its_place_in_bytes() {
        let upload = Upload::at(10, 5);
        assert_eq!(upload.byte_offset(), 10 * 28);
        assert_eq!(upload.byte_len(), 5 * 28);
    }
}
