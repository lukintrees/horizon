//! Headless `gl21_clear` smoke command stream (PR 3, stage 1).
//!
//! The plan names an example that clears a GL 2.1 context and draws one
//! triangle. Until the GLX/WGL loader lands, this module produces the exact
//! command list that example will replay: it is pure data, runs in CI without
//! a GPU, and already fixes the reversed-Z-independent state (`glClearDepth`
//! 1.0, `GL_LEQUAL`) described in `docs/design/legacy-gl21.md` §4.6.

use super::context::GlContext;
use super::state::{
    GL_BLEND, GL_COLOR_BUFFER_BIT, GL_DEPTH_BUFFER_BIT, GL_DEPTH_TEST, GL_LEQUAL,
    GL_ONE_MINUS_SRC_ALPHA, GL_SRC_ALPHA, GL_TRIANGLES, GL_UNSIGNED_SHORT, GlCommand, GlState,
};

/// Dark blue-grey clear used by the smoke frame; RGBA8, no HDR target.
pub const CLEAR_COLOR: [u8; 4] = [16, 24, 32, 255];

/// The command stream for one 800×600-style legacy frame.
pub fn gl21_clear_commands(context: &GlContext, width: u32, height: u32) -> Vec<GlCommand> {
    let mut commands = vec![
        GlCommand::Viewport(0, 0, width, height),
        GlCommand::Disable(GL_BLEND),
        GlCommand::Enable(GL_DEPTH_TEST),
        GlCommand::DepthFunc(GL_LEQUAL),
        GlCommand::BlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA),
        GlCommand::ClearColor(CLEAR_COLOR),
        GlCommand::Clear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT),
    ];
    if context.dialect.needs_glsl() {
        commands.push(GlCommand::UseProgram(1));
    }
    commands.push(GlCommand::DrawRangeElements {
        mode: GL_TRIANGLES,
        start: 0,
        end: 2,
        count: 3,
        ty: GL_UNSIGNED_SHORT,
        offset: 0,
    });
    commands
}

/// Same stream with redundant state setters removed by [`GlState`].
pub fn gl21_clear_state_stream(context: &GlContext, width: u32, height: u32) -> Vec<GlCommand> {
    GlState::default().apply_all(&gl21_clear_commands(context, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use super::super::caps::{REQUIRED_BASE, REQUIRED_FBO, classify};

    fn context(glsl: Option<u32>) -> GlContext {
        let mut extensions: Vec<String> = REQUIRED_BASE
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        extensions.extend(REQUIRED_FBO.iter().map(|name| (*name).to_owned()));
        let mut limits = BTreeMap::new();
        limits.insert("GL_MAX_TEXTURE_SIZE".to_owned(), 4096_u64);
        GlContext::from_report(classify(Some((2, 1)), glsl, &extensions, limits)).unwrap()
    }

    #[test]
    fn l1_clear_stream_ends_with_a_triangle() {
        let commands = gl21_clear_commands(&context(Some(120)), 800, 600);
        assert_eq!(commands.len(), 9);
        assert_eq!(commands[0], GlCommand::Viewport(0, 0, 800, 600));
        assert!(commands.contains(&GlCommand::UseProgram(1)));
        assert_eq!(
            commands.last(),
            Some(&GlCommand::DrawRangeElements {
                mode: GL_TRIANGLES,
                start: 0,
                end: 2,
                count: 3,
                ty: GL_UNSIGNED_SHORT,
                offset: 0,
            })
        );
        // GlState drops the three setters that already match its defaults:
        // Disable(BLEND), DepthFunc(LEQUAL) and BlendFunc(SRC_ALPHA, …).
        let stream = gl21_clear_state_stream(&context(Some(120)), 800, 600);
        assert_eq!(stream.len(), 6);
        assert_eq!(stream[0], GlCommand::Viewport(0, 0, 800, 600));
        assert!(
            !stream
                .iter()
                .any(|c| matches!(c, GlCommand::DepthFunc(_) | GlCommand::BlendFunc(..)))
        );
        assert!(
            stream
                .iter()
                .any(|c| matches!(c, GlCommand::DrawRangeElements { .. }))
        );
    }

    #[test]
    fn f0_stream_has_no_program_but_still_draws() {
        let commands = gl21_clear_commands(&context(None), 640, 480);
        assert_eq!(commands.len(), 8);
        assert!(
            !commands
                .iter()
                .any(|c| matches!(c, GlCommand::UseProgram(_)))
        );
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, GlCommand::DrawRangeElements { .. }))
        );
    }

    #[test]
    fn repeated_frames_only_replay_draws_and_clears() {
        let ctx = context(Some(120));
        let mut state = GlState::default();
        let first = state.apply_all(&gl21_clear_commands(&ctx, 800, 600));
        let second = state.apply_all(&gl21_clear_commands(&ctx, 800, 600));
        assert_eq!(first.len(), 6);
        assert_eq!(second.len(), 2);
        assert!(matches!(second[0], GlCommand::Clear(_)));
        assert!(matches!(second[1], GlCommand::DrawRangeElements { .. }));
    }
}
