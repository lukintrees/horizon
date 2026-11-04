//! Pure-Rust cache of GL 2.1 fixed-function state (PR 3, stage 1).
//!
//! The real `glEnable`/`glViewport`/… calls arrive with the GL loader in a
//! later PR. Keeping the state machine here lets the pass builders emit a
//! `Vec<GlCommand>` that is easy to unit-test, diff and replay in CI without a
//! window. Redundant state changes are dropped by [`GlState::apply`].

/// GL enum values the legacy passes need. Kept as plain `u32` so this module
/// has no bindings dependency.
pub const GL_COLOR_BUFFER_BIT: u32 = 0x0000_4000;
pub const GL_DEPTH_BUFFER_BIT: u32 = 0x0000_0100;
pub const GL_TRIANGLES: u32 = 0x0004;
pub const GL_DEPTH_TEST: u32 = 0x0B71;
pub const GL_BLEND: u32 = 0x0BE2;
pub const GL_CULL_FACE: u32 = 0x0B44;
pub const GL_SCISSOR_TEST: u32 = 0x0C11;
pub const GL_LEQUAL: u32 = 0x0203;
pub const GL_GEQUAL: u32 = 0x0206;
pub const GL_SRC_ALPHA: u32 = 0x0302;
pub const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
pub const GL_TEXTURE_2D: u32 = 0x0DE1;
pub const GL_FRAMEBUFFER: u32 = 0x8D40;
pub const GL_UNSIGNED_SHORT: u32 = 0x1403;

/// A single legacy GL operation. Draws and clears are always emitted; state
/// setters can be filtered through [`GlState::apply`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlCommand {
    Viewport(u32, u32, u32, u32),
    ClearColor([u8; 4]),
    Clear(u32),
    Enable(u32),
    Disable(u32),
    DepthFunc(u32),
    BlendFunc(u32, u32),
    UseProgram(u32),
    BindTexture(u32, u32),
    BindFramebuffer(u32, u32),
    DrawRangeElements {
        mode: u32,
        start: u32,
        end: u32,
        count: u32,
        ty: u32,
        offset: u32,
    },
}

/// Last known state. `apply` returns `Some(command)` only when the command
/// would change it (or when the command is not a state setter).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlState {
    pub viewport: [u32; 4],
    pub clear_color: [u8; 4],
    pub depth_test: bool,
    pub blend: bool,
    pub cull_face: bool,
    pub scissor_test: bool,
    pub depth_func: u32,
    pub blend_src: u32,
    pub blend_dst: u32,
    pub program: u32,
    pub texture_unit: u32,
    pub bound_texture: u32,
    pub framebuffer: u32,
}

impl Default for GlState {
    fn default() -> Self {
        Self {
            viewport: [0, 0, 0, 0],
            clear_color: [0, 0, 0, 0],
            depth_test: false,
            blend: false,
            cull_face: false,
            scissor_test: false,
            depth_func: GL_LEQUAL,
            blend_src: GL_SRC_ALPHA,
            blend_dst: GL_ONE_MINUS_SRC_ALPHA,
            program: 0,
            texture_unit: 0,
            bound_texture: 0,
            framebuffer: 0,
        }
    }
}

impl GlState {
    pub fn apply(&mut self, command: &GlCommand) -> Option<GlCommand> {
        let changed = match command {
            GlCommand::Viewport(x, y, w, h) => {
                let next = [*x, *y, *w, *h];
                let changed = self.viewport != next;
                self.viewport = next;
                changed
            }
            GlCommand::ClearColor(rgba) => {
                let changed = self.clear_color != *rgba;
                self.clear_color = *rgba;
                changed
            }
            GlCommand::Enable(cap) => {
                let changed = match *cap {
                    GL_DEPTH_TEST => !self.depth_test,
                    GL_BLEND => !self.blend,
                    GL_CULL_FACE => !self.cull_face,
                    GL_SCISSOR_TEST => !self.scissor_test,
                    _ => true,
                };
                match *cap {
                    GL_DEPTH_TEST => self.depth_test = true,
                    GL_BLEND => self.blend = true,
                    GL_CULL_FACE => self.cull_face = true,
                    GL_SCISSOR_TEST => self.scissor_test = true,
                    _ => {}
                }
                changed
            }
            GlCommand::Disable(cap) => {
                let changed = match *cap {
                    GL_DEPTH_TEST => self.depth_test,
                    GL_BLEND => self.blend,
                    GL_CULL_FACE => self.cull_face,
                    GL_SCISSOR_TEST => self.scissor_test,
                    _ => true,
                };
                match *cap {
                    GL_DEPTH_TEST => self.depth_test = false,
                    GL_BLEND => self.blend = false,
                    GL_CULL_FACE => self.cull_face = false,
                    GL_SCISSOR_TEST => self.scissor_test = false,
                    _ => {}
                }
                changed
            }
            GlCommand::DepthFunc(func) => {
                let changed = self.depth_func != *func;
                self.depth_func = *func;
                changed
            }
            GlCommand::BlendFunc(src, dst) => {
                let changed = self.blend_src != *src || self.blend_dst != *dst;
                self.blend_src = *src;
                self.blend_dst = *dst;
                changed
            }
            GlCommand::UseProgram(program) => {
                let changed = self.program != *program;
                self.program = *program;
                changed
            }
            GlCommand::BindTexture(unit, texture) => {
                let changed = self.texture_unit != *unit || self.bound_texture != *texture;
                self.texture_unit = *unit;
                self.bound_texture = *texture;
                changed
            }
            GlCommand::BindFramebuffer(_target, fbo) => {
                let changed = self.framebuffer != *fbo;
                self.framebuffer = *fbo;
                changed
            }
            GlCommand::Clear(_) | GlCommand::DrawRangeElements { .. } => true,
        };
        changed.then(|| command.clone())
    }

    pub fn apply_all(&mut self, commands: &[GlCommand]) -> Vec<GlCommand> {
        commands.iter().filter_map(|c| self.apply(c)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redundant_state_changes_are_dropped() {
        let mut state = GlState::default();
        let commands = vec![
            GlCommand::Viewport(0, 0, 800, 600),
            GlCommand::Viewport(0, 0, 800, 600),
            GlCommand::Enable(GL_DEPTH_TEST),
            GlCommand::Enable(GL_DEPTH_TEST),
            GlCommand::Disable(GL_BLEND),
            GlCommand::Clear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT),
            GlCommand::Clear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT),
        ];
        let emitted = state.apply_all(&commands);
        // Disable(GL_BLEND) is already the default, so only four commands
        // survive the first frame's state diff.
        assert_eq!(emitted.len(), 4);
        assert_eq!(emitted[0], GlCommand::Viewport(0, 0, 800, 600));
        assert_eq!(emitted[1], GlCommand::Enable(GL_DEPTH_TEST));
        assert!(state.depth_test);
        assert!(!state.blend);
        assert!(emitted.contains(&GlCommand::Clear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)));
    }

    #[test]
    fn defaults_follow_the_gl21_plan() {
        let state = GlState::default();
        assert_eq!(state.depth_func, GL_LEQUAL);
        assert_eq!(state.blend_src, GL_SRC_ALPHA);
        assert_eq!(state.blend_dst, GL_ONE_MINUS_SRC_ALPHA);
        assert_eq!(state.program, 0);
    }
}
