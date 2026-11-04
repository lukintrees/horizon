//! PR 7: the complete GL2 blend/alpha-test table and translucent sorting.
//!
//! `BlendMode` replaces the GPU-side blend states shader packs would normally
//! request. Every mode is expressed as plain `GlCommand`s so the legacy pass
//! builder can be tested without a window. Translucent batches are sorted on
//! the CPU because GL 2.1 has no compute culling or indirect draws.

use super::state::{
    GL_ALPHA_TEST, GL_BLEND, GL_DST_COLOR, GL_GREATER, GL_ONE, GL_ONE_MINUS_SRC_ALPHA,
    GL_SRC_ALPHA, GL_ZERO, GlCommand,
};

pub const CUTOUT_ALPHA_REF: u8 = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlendMode {
    Opaque,
    Cutout,
    Alpha,
    Additive,
    Multiply,
    Premultiplied,
}

impl BlendMode {
    pub const ALL: [BlendMode; 6] = [
        BlendMode::Opaque,
        BlendMode::Cutout,
        BlendMode::Alpha,
        BlendMode::Additive,
        BlendMode::Multiply,
        BlendMode::Premultiplied,
    ];

    pub fn state(self) -> BlendState {
        let (src, dst, blend, depth_write, alpha_test) = match self {
            Self::Opaque => (GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, false, true, false),
            Self::Cutout => (GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, false, true, true),
            Self::Alpha => (GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, true, false, false),
            Self::Additive => (GL_SRC_ALPHA, GL_ONE, true, false, false),
            Self::Multiply => (GL_DST_COLOR, GL_ZERO, true, false, false),
            Self::Premultiplied => (GL_ONE, GL_ONE_MINUS_SRC_ALPHA, true, false, false),
        };
        BlendState {
            src,
            dst,
            blend,
            depth_write,
            alpha_test,
            alpha_func: GL_GREATER,
            alpha_ref: CUTOUT_ALPHA_REF,
        }
    }

    pub fn gl_commands(self) -> Vec<GlCommand> {
        let state = self.state();
        let mut commands = Vec::with_capacity(5);
        if state.blend {
            commands.push(GlCommand::Enable(GL_BLEND));
            commands.push(GlCommand::BlendFunc(state.src, state.dst));
        } else {
            commands.push(GlCommand::Disable(GL_BLEND));
        }
        if state.alpha_test {
            commands.push(GlCommand::Enable(GL_ALPHA_TEST));
            commands.push(GlCommand::AlphaFunc(state.alpha_func, state.alpha_ref));
        } else {
            commands.push(GlCommand::Disable(GL_ALPHA_TEST));
        }
        commands.push(GlCommand::DepthMask(state.depth_write));
        commands
    }

    pub const fn needs_sort(self) -> bool {
        matches!(
            self,
            Self::Alpha | Self::Additive | Self::Multiply | Self::Premultiplied
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlendState {
    pub src: u32,
    pub dst: u32,
    pub blend: bool,
    pub depth_write: bool,
    pub alpha_test: bool,
    pub alpha_func: u32,
    pub alpha_ref: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortState {
    pub mode: BlendMode,
    pub distance: u32,
    pub order: u32,
}

impl SortState {
    /// Far-to-near, stable for equal distances through the original order.
    pub fn sort(entries: &mut [SortState]) {
        entries.sort_by(|a, b| b.distance.cmp(&a.distance).then(a.order.cmp(&b.order)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx21::state::GlState;

    #[test]
    fn all_six_modes_have_independent_states() {
        assert_eq!(BlendMode::ALL.len(), 6);
        assert_eq!(
            BlendMode::Opaque.state(),
            BlendState {
                src: GL_SRC_ALPHA,
                dst: GL_ONE_MINUS_SRC_ALPHA,
                blend: false,
                depth_write: true,
                alpha_test: false,
                alpha_func: GL_GREATER,
                alpha_ref: 128,
            }
        );
        assert_eq!(BlendMode::Cutout.state().alpha_ref, 128);
        assert!(BlendMode::Cutout.state().alpha_test);
        assert_eq!(BlendMode::Additive.state().dst, GL_ONE);
        assert_eq!(BlendMode::Multiply.state().src, GL_DST_COLOR);
        assert_eq!(BlendMode::Premultiplied.state().src, GL_ONE);
    }

    #[test]
    fn cutout_commands_enable_alpha_test_and_write_depth() {
        assert_eq!(
            BlendMode::Cutout.gl_commands(),
            vec![
                GlCommand::Disable(GL_BLEND),
                GlCommand::Enable(GL_ALPHA_TEST),
                GlCommand::AlphaFunc(GL_GREATER, 128),
                GlCommand::DepthMask(true),
            ]
        );
    }

    #[test]
    fn alpha_commands_blend_and_disable_depth_write() {
        assert_eq!(
            BlendMode::Alpha.gl_commands(),
            vec![
                GlCommand::Enable(GL_BLEND),
                GlCommand::BlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA),
                GlCommand::Disable(GL_ALPHA_TEST),
                GlCommand::DepthMask(false),
            ]
        );
        assert!(BlendMode::Alpha.needs_sort());
        assert!(!BlendMode::Opaque.needs_sort());
        assert!(!BlendMode::Cutout.needs_sort());
    }

    #[test]
    fn state_cache_drops_redundant_blend_setup() {
        let commands = BlendMode::Alpha.gl_commands();
        let mut state = GlState::default();
        let first = state.apply_all(&commands);
        let second = state.apply_all(&commands);
        assert_eq!(first.len(), 2);
        assert!(second.is_empty());
        assert!(state.blend);
        assert!(!state.depth_mask);
    }

    #[test]
    fn translucent_sort_is_far_to_near_and_stable() {
        let mut entries = [
            SortState {
                mode: BlendMode::Alpha,
                distance: 10,
                order: 0,
            },
            SortState {
                mode: BlendMode::Alpha,
                distance: 30,
                order: 1,
            },
            SortState {
                mode: BlendMode::Alpha,
                distance: 20,
                order: 2,
            },
            SortState {
                mode: BlendMode::Alpha,
                distance: 30,
                order: 3,
            },
        ];
        SortState::sort(&mut entries);
        assert_eq!(
            entries.map(|e| (e.distance, e.order)),
            [(30, 1), (30, 3), (20, 2), (10, 0)]
        );
    }
}
