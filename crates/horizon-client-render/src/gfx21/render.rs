//! PR 7: one additive frame plan covering all six GL2 passes.
//!
//! `FramePlan` stays byte-for-byte compatible with the PR 6 golden frame. This
//! wrapper adds only CPU-posed references for entity, sky, HUD and line work,
//! so PR 7 can be checked without rewriting the existing render pipeline.

use super::entities::EntityBatch;
use super::hud::HudBatch;
use super::lines::LineBatch;
use super::passes::{FramePlan, MAX_DRAWS};
use super::sky::SkyState;
use super::state::GlCommand;

#[derive(Clone, Debug, PartialEq)]
pub struct FullFramePlan {
    pub terrain: FramePlan,
    pub entities: EntityBatch,
    pub sky: SkyState,
    pub hud: HudBatch,
    pub lines: LineBatch,
}

impl FullFramePlan {
    pub fn reference() -> Self {
        Self {
            terrain: super::frame::reference_frame(),
            entities: EntityBatch::reference(),
            sky: SkyState::reference(),
            hud: HudBatch::reference(),
            lines: LineBatch::reference(),
        }
    }

    /// Terrain multi-draws + overlays + entities + one sky quad + HUD + lines.
    pub fn draw_call_count(&self) -> u32 {
        self.terrain.draw_call_count()
            + self.entities.draw_call_count()
            + 1
            + self.hud.draw_call_count()
            + self.lines.draw_call_count()
    }

    pub fn validate(&self) -> Result<(), String> {
        self.terrain
            .validate()
            .map_err(|err| format!("terrain: {err}"))?;
        let draws = self.draw_call_count();
        if draws > MAX_DRAWS {
            return Err(format!("draw budget: {draws} > {MAX_DRAWS}"));
        }
        Ok(())
    }

    pub fn gl_commands(&self) -> Vec<GlCommand> {
        let mut commands = self.sky.gl_commands();
        commands.extend([
            GlCommand::UseProgram(0),
            GlCommand::BindTexture(0, 0),
            GlCommand::BindTexture(1, 1),
            GlCommand::UseProgram(2),
        ]);
        commands.extend(self.entities.gl_commands());
        commands.extend(self.hud.gl_commands());
        commands.extend(self.lines.gl_commands());
        commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_full_plan_has_sixteen_draws_and_uses_every_program() {
        let plan = FullFramePlan::reference();
        assert_eq!(plan.terrain.draw_call_count(), 5);
        assert_eq!(plan.entities.draw_call_count(), 2);
        assert_eq!(plan.hud.draw_call_count(), 4);
        assert_eq!(plan.lines.draw_call_count(), 4);
        assert_eq!(plan.draw_call_count(), 16);
        plan.validate().unwrap();
        let commands = plan.gl_commands();
        for program in 0..6_u32 {
            assert!(commands.contains(&GlCommand::UseProgram(program)));
        }
    }
}
