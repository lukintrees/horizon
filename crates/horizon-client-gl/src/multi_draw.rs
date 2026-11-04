//! `GL_EXT_multi_draw_arrays`: several draws in one call, and what to do without it.
//!
//! On the target this is not an optimisation. A frame of 800 to 1500 sections at the 20 to
//! 60 microseconds a draw costs on a 2006 Catalyst driver is 30 to 60 ms of CPU per frame
//! before anything is drawn (docs/gl21-x1650-plan.ru.md §11), against a 16 ms budget. The
//! extension turns that into 1 to 5 calls.
//!
//! **glow does not have the function.** `glMultiDrawElementsEXT` and its siblings are
//! extension entry points, and glow's table is the core; looking for it in
//! `HasContext` finds nothing, which is why this module loads it itself through
//! [`GlLibrary::symbol`](crate::loader::GlLibrary::symbol) — the same path glow's own table
//! takes, `wglGetProcAddress` or `glXGetProcAddressARB` first and the library's exports
//! second. Calling a null pointer is a driver segfault with no message
//! (docs/gl21-dev-env.md §5), so the pointer is checked when the extension is claimed and the
//! fallback is used when it is not.
//!
//! The three names the extension defines, all of which a `GL_EXT_multi_draw_arrays` driver
//! must export: `glMultiDrawArraysEXT`, `glMultiDrawElementsEXT`, and
//! `glMultiDrawElementsBaseVertexEXT` if `GL_ARB_draw_elements_base_vertex` is also present.
//! This module wraps the first two.

use std::ffi::c_void;

use crate::caps::Feature;
use crate::error::{Error, Result};
use crate::gl::Context;

/// `glMultiDrawArraysEXT(mode, first, count, primcount)`.
type MultiDrawArraysFn = unsafe extern "system" fn(u32, *const i32, *const i32, i32);

/// `glMultiDrawElementsEXT(mode, count, type, indices, primcount)`.
type MultiDrawElementsFn = unsafe extern "system" fn(u32, *const i32, u32, *const *const c_void, i32);

/// Which of the extension's entry points a driver actually exports.
///
/// Claimed separately from [`Feature::MultiDrawArrays`] on purpose: the extension string is
/// the driver's statement of intent, and the pointers are the fact. A driver that lists the
/// extension and exports `glMultiDrawArraysEXT` but not the indexed one is a real case
/// (`GL_EXT_multi_draw_arrays` is the only place the indexed form exists on 2.1), so the two
/// are tracked apart and a pass that needs the indexed form falls back alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MultiDrawPointers {
    pub arrays: Option<MultiDrawArraysFn>,
    pub elements: Option<MultiDrawElementsFn>,
}

/// A run of draws in one buffer, in the form `glMultiDrawElementsEXT` takes: a count per draw
/// and a byte offset per draw, both counting from the same bound vertex and index buffers.
///
/// The offsets are `*const c_void` in the GL call because that is what the driver adds to the
/// bound index buffer's base. Storing them as `u32` here keeps the struct free of pointers, so
/// it can be built per frame and inspected in a test.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MultiDrawElements {
    /// Number of indices per draw.
    pub counts: Vec<i32>,
    /// Byte offset into the index buffer per draw.
    pub offsets: Vec<u32>,
}

impl MultiDrawElements {
    /// An empty run: a driver's `primcount` of zero is legal but pointless, so a pass with no
    /// draws calls nothing at all.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserves for `draws` entries, which is one allocation per run rather than one per draw.
    pub fn with_capacity(draws: usize) -> Self {
        Self {
            counts: Vec::with_capacity(draws),
            offsets: Vec::with_capacity(draws),
        }
    }

    /// Adds one draw. The two vectors must stay the same length: `primcount` is one number for
    /// both, so a mismatch reads past the shorter array in the driver.
    pub fn push(&mut self, index_count: u32, byte_offset: u32) {
        self.counts.push(index_count as i32);
        self.offsets.push(byte_offset);
    }

    /// How many draws the run holds.
    pub fn len(&self) -> usize {
        self.counts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// Whether both arrays agree, which is what `primcount` requires. A false here is a caller
    /// bug and is refused rather than passed to the driver.
    pub fn is_consistent(&self) -> bool {
        self.counts.len() == self.offsets.len()
    }

    /// Total indices, for the frame statistics: a multi-draw reports the triangles it drew or
    /// the statistics quietly under-count.
    pub fn indices(&self) -> u64 {
        self.counts.iter().map(|&c| u64::from(c.max(0) as u32)).sum()
    }
}

/// The extension's entry points, loaded once per context.
///
/// Loading is a handful of `GetProcAddress` calls at startup; the struct is then a pair of
/// function pointers, which is what a per-frame call needs.
pub struct MultiDraw {
    pointers: MultiDrawPointers,
}

impl MultiDraw {
    /// Loads the extension's pointers if the context claims the extension.
    ///
    /// The extension string alone does not decide it: the pointer is looked up either way, and
    /// a claimed extension whose pointer is missing is logged, because that is a driver bug
    /// worth seeing in a bug report rather than a condition to discover mid-frame.
    pub fn load(ctx: &Context) -> Self {
        let claimed = ctx.caps().has(Feature::MultiDrawArrays);
        let arrays = ctx.library().symbol("glMultiDrawArraysEXT");
        let elements = ctx.library().symbol("glMultiDrawElementsEXT");
        if claimed && (arrays.is_null() || elements.is_null()) {
            log::warn!(
                "GL_EXT_multi_draw_arrays is advertised but {} is missing: \
                 falling back to one call per draw",
                if arrays.is_null() {
                    "glMultiDrawArraysEXT"
                } else {
                    "glMultiDrawElementsEXT"
                }
            );
        }
        Self {
            pointers: MultiDrawPointers {
                arrays: arrays_fn(arrays),
                elements: elements_fn(elements),
            },
        }
    }

    /// A `MultiDraw` whose pointers are all null, for a context that has none: the fallback
    /// path is then the only path, and it is the one the tests exercise.
    pub fn none() -> Self {
        Self {
            pointers: MultiDrawPointers::default(),
        }
    }

    /// What the driver exported, for the startup log and the tests.
    pub fn pointers(&self) -> MultiDrawPointers {
        self.pointers
    }

    /// Whether the indexed form is callable: the one terrain sections need.
    pub fn has_elements(&self) -> bool {
        self.pointers.elements.is_some()
    }

    /// Whether the non-indexed form is callable: fullscreen quads, HUD vertices, clouds.
    pub fn has_arrays(&self) -> bool {
        self.pointers.arrays.is_some()
    }

    /// Draws a run of indexed triangle lists with one call, or returns the number of calls the
    /// caller has to make instead.
    ///
    /// `Ok(1)` means the extension drew it, `Ok(n)` means the caller must loop `n` times over
    /// [`MultiDrawElements::counts`] and [`MultiDrawElements::offsets`], and `Ok(0)` means
    /// there was nothing to draw. The fallback is not an error: this target may not have the
    /// extension, and the frame is still correct without it, only slower.
    ///
    /// # Safety
    ///
    /// The caller must have a vertex and index buffer bound that the offsets are inside, and
    /// must have checked them (`crate::exec`'s `index_problem` does, before it gets here). The
    /// pointers this hands the driver are the caller's contract, and a bad offset is a driver
    /// fault that this function cannot see.
    pub unsafe fn draw_elements(&self, mode: u32, run: &MultiDrawElements) -> Result<u32> {
        if run.is_empty() {
            return Ok(0);
        }
        if !run.is_consistent() {
            return Err(Error::Gl {
                call: "glMultiDrawElementsEXT",
                message: format!(
                    "{} counts against {} offsets: primcount is one number for both",
                    run.counts.len(),
                    run.offsets.len()
                ),
            });
        }
        let Some(function) = self.pointers.elements else {
            return Ok(run.len() as u32);
        };
        // The offsets become pointers because that is the shape the driver adds to the bound
        // index buffer's base; nothing dereferences them on this side.
        let offsets: Vec<*const c_void> = run
            .offsets
            .iter()
            .map(|&offset| offset as usize as *const c_void)
            .collect();
        unsafe {
            function(
                mode,
                run.counts.as_ptr(),
                glow::UNSIGNED_SHORT,
                offsets.as_ptr(),
                run.len() as i32,
            );
        }
        Ok(1)
    }

    /// The non-indexed form: `counts` are vertex counts and `firsts` are vertex indices, so
    /// there is one array rather than two and no index buffer to have bound.
    ///
    /// # Safety
    ///
    /// As [`Self::draw_elements`]: the bound vertex buffer must cover every `firsts[i] +
    /// counts[i]`.
    pub unsafe fn draw_arrays(&self, mode: u32, firsts: &[i32], counts: &[i32]) -> Result<u32> {
        if counts.is_empty() {
            return Ok(0);
        }
        if firsts.len() != counts.len() {
            return Err(Error::Gl {
                call: "glMultiDrawArraysEXT",
                message: format!(
                    "{} firsts against {} counts: primcount is one number for both",
                    firsts.len(),
                    counts.len()
                ),
            });
        }
        let Some(function) = self.pointers.arrays else {
            return Ok(counts.len() as u32);
        };
        unsafe {
            function(mode, firsts.as_ptr(), counts.as_ptr(), counts.len() as i32);
        }
        Ok(1)
    }
}

/// A raw entry point as `glMultiDrawArraysEXT`, or `None` for null.
///
/// A transmute between a data pointer and a function pointer is what every GL loader does
/// (`glow` included): the loader's symbol type cannot express a C signature, and the ABI is the
/// driver's, not Rust's, which is why the two function types above are `extern "system"`. Both
/// are pointer-sized, so the transmute is between equal sizes.
fn arrays_fn(pointer: *const c_void) -> Option<MultiDrawArraysFn> {
    if pointer.is_null() {
        return None;
    }
    Some(unsafe { std::mem::transmute::<*const c_void, MultiDrawArraysFn>(pointer) })
}

/// As [`arrays_fn`], for the indexed form.
fn elements_fn(pointer: *const c_void) -> Option<MultiDrawElementsFn> {
    if pointer.is_null() {
        return None;
    }
    Some(unsafe { std::mem::transmute::<*const c_void, MultiDrawElementsFn>(pointer) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_of_draws_keeps_its_two_arrays_in_step() {
        let mut run = MultiDrawElements::with_capacity(3);
        run.push(6, 0);
        run.push(9, 12);
        run.push(3, 30);
        assert_eq!(run.len(), 3);
        assert!(run.is_consistent());
        assert_eq!(run.counts, [6, 9, 3]);
        assert_eq!(run.offsets, [0, 12, 30]);
        assert_eq!(run.indices(), 18);
    }

    #[test]
    fn an_inconsistent_run_is_refused_before_the_driver_sees_it() {
        // The failure this prevents: `primcount` is one number for both arrays, so a short
        // `offsets` makes the driver read whatever follows it as an offset.
        let run = MultiDrawElements {
            counts: vec![6, 6],
            offsets: vec![0],
        };
        assert!(!run.is_consistent());
        let draw = MultiDraw::none();
        let error = unsafe { draw.draw_elements(0, &run) };
        assert!(matches!(error, Err(Error::Gl { call, .. }) if call == "glMultiDrawElementsEXT"));
    }

    #[test]
    fn without_the_extension_the_caller_gets_the_number_of_calls_to_make() {
        let draw = MultiDraw::none();
        assert!(!draw.has_elements() && !draw.has_arrays());
        let mut run = MultiDrawElements::new();
        run.push(6, 0);
        run.push(6, 12);
        // The fallback: two draws, in the order they were pushed.
        let calls = unsafe { draw.draw_elements(glow::TRIANGLES, &run) };
        assert_eq!(calls, Ok(2));
        let calls = unsafe { draw.draw_arrays(glow::TRIANGLES, &[0, 12], &[6, 6]) };
        assert_eq!(calls, Ok(2));
    }

    #[test]
    fn nothing_to_draw_is_zero_calls_and_not_an_error() {
        let draw = MultiDraw::none();
        let empty = MultiDrawElements::new();
        assert_eq!(unsafe { draw.draw_elements(glow::TRIANGLES, &empty) }, Ok(0));
        assert_eq!(unsafe { draw.draw_arrays(glow::TRIANGLES, &[], &[]) }, Ok(0));
    }

    #[test]
    fn an_arrays_run_with_mismatched_lengths_is_refused() {
        let draw = MultiDraw::none();
        let error = unsafe { draw.draw_arrays(glow::TRIANGLES, &[0], &[6, 6]) };
        assert!(matches!(error, Err(Error::Gl { call, .. }) if call == "glMultiDrawArraysEXT"));
    }

    #[test]
    fn a_null_entry_point_is_none_and_a_present_one_is_not() {
        // The check is on nullness only: a driver that exports the name gets a call, and the
        // alternative (calling a null pointer) is a segfault with no message.
        assert!(arrays_fn(std::ptr::null()).is_none());
        assert!(elements_fn(std::ptr::null()).is_none());
        // A real address, never called here.
        let address = 1 as usize as *const c_void;
        assert!(arrays_fn(address).is_some());
        assert!(elements_fn(address).is_some());
    }

    #[test]
    fn the_statistics_count_every_index_the_run_drew() {
        // A multi-draw that reported one call and six triangles would make the frame budget
        // look met while the GPU drew thousands: the count is the sum, not the call count.
        let mut run = MultiDrawElements::new();
        for _ in 0..800 {
            run.push(300, 0);
        }
        assert_eq!(run.len(), 800);
        assert_eq!(run.indices(), 240_000);
        assert_eq!(run.indices() / 3, 80_000);
    }
}
