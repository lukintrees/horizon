//! The frame's draw calls, worked out without a GPU.
//!
//! Everything the GL 2.1 path draws is decided here and then handed to `glow` by the caller: which
//! passes run and in what order, one draw call per sprite per section, back to front for what is
//! see-through, and the one full-screen quad that presents the frame. The rules are the ones the wgpu
//! path keeps, written down where a test can hold them:
//!
//! - **Opaque before see-through**, and see-through sorted back to front by the distance to its
//!   section's centre: `rgba` blending is not commutative, so the order is the picture.
//! - **One solid pass, not two**: wgpu splits solid faces into an opaque pass and a cutout pass so
//!   that a tiled GPU's hidden-surface removal keeps working; GL 2.1 has no such pass to protect, so
//!   one pass with `Pass::alpha_test` draws both, as `terrain.frag` decides it per fragment. Opaque
//!   quads never discard, so this is the picture wgpu draws, in one pass and one call per sprite.
//! - **Sections behind the camera are still listed**: a box touching the near plane is visible and
//!   the driver clips it. Culling is the caller's business (`occlusion.rs`), not this list's.
//! - **The sky comes first**, with depth off, before the world covers it.
//!
//! GL 2.1 has no instancing and no indirect draws, so the counts are what to watch: one call per
//! (section, sprite) run, which `Batches::of` measures on the real mesh.

use crate::gl21::vertex::Batches;

/// How a run of triangles blends and tests depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    /// Depth test and write on, no blending, vanilla's 0.5 alpha test: solid and cut-out faces.
    Solid,
    /// Depth test on, depth write off, `src alpha` blending, back to front, the 0.01 alpha test.
    Translucent,
    /// Depth test off, depth write off, blending on: behind the world.
    Sky,
}

impl Pass {
    /// The threshold `terrain.frag` cuts a face away below, as `solid_color` (0.5) and
    /// `fs_translucent` (0.01) set it: one shader, one uniform, three passes.
    pub fn alpha_test(self) -> f32 {
        match self {
            Pass::Solid => 0.5,
            Pass::Translucent => 0.01,
            Pass::Sky => 0.0,
        }
    }

    /// Whether the pass writes depth: what covers what after it.
    pub fn writes_depth(self) -> bool {
        self == Pass::Solid
    }

    /// Whether the pass blends src alpha over the frame.
    pub fn blends(self) -> bool {
        self != Pass::Solid
    }
}

/// The runs of one kind within a section: their first run and their count, indexes into its solid
/// or its see-through list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionRuns {
    pub first_run: u32,
    pub runs: u32,
}

impl SectionRuns {
    /// The run indices this range names, for a loop over them.
    pub fn runs(&self) -> std::ops::Range<u32> {
        self.first_run..self.first_run + self.runs
    }

    pub const fn new(first_run: u32, runs: u32) -> Self {
        SectionRuns { first_run, runs }
    }
}

/// A section to draw: where it is and what each pass needs of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionDraw {
    /// The square of the distance from the camera to the section's centre, taken once by the
    /// caller.
    pub distance_sq: f32,
    /// The shade-in of the section: 1 whole, 0 gone (`RenderSection.getVisibility`), mixed toward
    /// the fog colour by `terrain.frag`.
    pub visibility: f32,
    /// Its solid faces, from `Batches::of`.
    pub solid: SectionRuns,
    /// Its see-through ones, from the same split.
    pub translucent: SectionRuns,
}

/// The frame's calls, in the order they are bound and drawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// The section each call draws and the run within it: (section, run). The sky's calls name
    /// `SKY` and its own runs.
    pub calls: Vec<(u32, u32)>,
    pub pass: Vec<Pass>,
    /// The full-screen quad that presents the frame, drawn last: nothing when the frame goes
    /// straight to the window.
    pub present: bool,
}

/// The section index of a sky run: past every section a frame holds.
pub const SKY: u32 = u32::MAX;

/// `plan` from the sections in view: their solid faces in the order given, then their see-through
/// faces farthest first. `sky`'s runs, when the frame draws any, come first.
pub fn plan(sections: &[SectionDraw], sky: Option<SectionRuns>, present: bool) -> Plan {
    let mut plan = Plan::default();
    if let Some(sky) = sky {
        for run in sky.runs() {
            plan.calls.push((SKY, run));
            plan.pass.push(Pass::Sky);
        }
    }
    for (index, section) in sections.iter().enumerate() {
        for run in section.solid.runs() {
            plan.calls.push((index as u32, run));
            plan.pass.push(Pass::Solid);
        }
    }
    let mut translucent: Vec<(usize, u32)> = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        translucent.extend(section.translucent.runs().map(|run| (index, run)));
    }
    translucent.sort_by(|(a, _), (b, _)| {
        sections[*b]
            .distance_sq
            .total_cmp(&sections[*a].distance_sq)
            // Two sections at one distance keep their order, so the list is deterministic.
            .then(a.cmp(b))
    });
    for (index, run) in translucent {
        plan.calls.push((index as u32, run));
        plan.pass.push(Pass::Translucent);
    }
    plan.present = present;
    plan
}

/// The draw calls a frame takes, the present quad included: the number to watch on a 2006 CPU, which
/// spends most of a frame here.
pub fn draw_calls(plan: &Plan) -> usize {
    plan.calls.len() + usize::from(plan.present)
}

/// The runs of a section's solid and see-through vertex lists, as `SectionDraw` wants them: the two
/// lists are uploaded as one, solid first.
pub fn section_runs(solid: &Batches, translucent: &Batches) -> (SectionRuns, SectionRuns) {
    let solid_runs = SectionRuns::new(0, solid.runs.len() as u32);
    let translucent_runs = SectionRuns::new(solid.runs.len() as u32, translucent.runs.len() as u32);
    (solid_runs, translucent_runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(distance_sq: f32, solid: u32, translucent: u32) -> SectionDraw {
        SectionDraw {
            distance_sq,
            visibility: 1.0,
            solid: SectionRuns::new(0, solid),
            translucent: SectionRuns::new(solid, translucent),
        }
    }

    #[test]
    fn solid_faces_go_before_see_through_ones_farthest_first() {
        let near = section(4.0, 1, 2);
        let far = section(100.0, 1, 1);
        let plan = plan(&[near, far], None, false);
        // Both sections' solid faces in the order given, then see-through: the far section's one
        // run, then the near one's two.
        assert_eq!(plan.calls, vec![(0, 0), (1, 0), (1, 1), (0, 1), (0, 2)]);
        assert_eq!(
            plan.pass,
            vec![
                Pass::Solid,
                Pass::Solid,
                Pass::Translucent,
                Pass::Translucent,
                Pass::Translucent
            ]
        );
        assert_eq!(draw_calls(&plan), 5);
    }

    #[test]
    fn the_sky_is_drawn_first_and_the_present_quad_last() {
        let plan = plan(&[section(1.0, 2, 0)], Some(SectionRuns::new(0, 1)), true);
        assert_eq!(plan.pass.first(), Some(&Pass::Sky));
        assert_eq!(plan.calls.first(), Some(&(SKY, 0)));
        assert_eq!(plan.pass.last(), Some(&Pass::Solid));
        assert!(plan.present);
        let calls = draw_calls(&plan);
        assert_eq!(calls, 4, "sky, two solid runs, the present quad");
    }

    #[test]
    fn sections_at_one_distance_keep_their_order() {
        let plan = plan(&[section(9.0, 1, 1), section(9.0, 1, 1)], None, false);
        let translucent: Vec<_> = plan
            .calls
            .iter()
            .zip(&plan.pass)
            .filter(|(_, pass)| **pass == Pass::Translucent)
            .map(|(call, _)| *call)
            .collect();
        assert_eq!(translucent, vec![(0, 1), (1, 1)]);
    }

    #[test]
    fn the_passes_differ_in_depth_writing_blending_and_the_alpha_test() {
        for pass in [Pass::Solid, Pass::Translucent, Pass::Sky] {
            let (depth, blend, test) = (pass.writes_depth(), pass.blends(), pass.alpha_test());
            match pass {
                Pass::Solid => assert_eq!((depth, blend, test), (true, false, 0.5)),
                Pass::Translucent => assert_eq!((depth, blend, test), (false, true, 0.01)),
                Pass::Sky => assert_eq!((depth, blend, test), (false, true, 0.0)),
            }
        }
    }

    #[test]
    fn nothing_in_view_is_nothing_to_draw() {
        let plan = plan(&[], None, false);
        assert_eq!(draw_calls(&plan), 0);
        assert!(!plan.present);
    }

    #[test]
    fn the_lists_are_one_buffer_with_the_solid_faces_first() {
        let solid = Batches {
            runs: vec![(1, 0, 4), (2, 4, 4)],
        };
        let translucent = Batches {
            runs: vec![(1, 8, 4)],
        };
        let (solid, translucent) = section_runs(&solid, &translucent);
        assert_eq!(solid, SectionRuns::new(0, 2));
        assert_eq!(translucent, SectionRuns::new(2, 1));
    }
}
