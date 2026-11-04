//! Opt-in CPU mirror of live game data, used while moving the game away from
//! wgpu-specific rendering. It does not create a GL context or draw anything.
use super::{Frame, SectionKey};
use crate::{
    legacy::{animation::SpriteAnimator, atlas, hud, scene, world::World},
    mesher::SectionMesh,
    sprites::SpriteAnimation,
};

pub(super) struct Mirror {
    world: World,
    gui: hud::PatchAtlas,
    animator: SpriteAnimator,
    reported_frame: bool,
    reported_error: bool,
}
impl Mirror {
    pub fn new(
        pixels: &[u8],
        side: u32,
        layers: u32,
        gui: (&[u8], u32, u32),
        animations: &[SpriteAnimation],
    ) -> Result<Self, String> {
        // A conservative page limit for the intended legacy hardware. This is
        // only a CPU diagnostic; an eventual GL context must query its own limit.
        let terrain = atlas::Atlas::new(side, layers, 2048, pixels).map_err(|e| e.to_string())?;
        let animator = SpriteAnimator::new(animations, &terrain);
        let gui =
            hud::Atlas::new(gui.1, gui.2, gui.0, 2048).map_err(|e| format!("GUI atlas: {e:?}"))?;
        Ok(Self {
            world: World::new(terrain, [255; 1024]),
            gui: hud::PatchAtlas::new(gui),
            animator,
            reported_frame: false,
            reported_error: false,
        })
    }

    pub fn upload(&mut self, key: SectionKey, mesh: &SectionMesh) -> Result<(), String> {
        self.world
            .upload(key, mesh)
            .map_err(|e| format!("section {key:?}: {e:?}"))
    }
    pub fn clear(&mut self) {
        self.world.clear();
    }
    pub fn remove_columns(&mut self, columns: &[(i32, i32)]) {
        self.world.remove_columns(columns.iter().copied());
    }

    /// Validate one actual game frame and keep the mirror's CPU atlases in
    /// sync. Deliberately logs only the first unsupported frame to avoid a
    /// warning every frame. This does not affect the wgpu render result.
    pub fn frame(
        &mut self,
        frame: &Frame<'_>,
        viewport: [u32; 2],
        min_y: i32,
        gamma: f32,
        fullbright: bool,
        animated: bool,
    ) {
        let result = (|| -> Result<(usize, usize, usize), String> {
            if animated {
                self.animator
                    .advance(self.world.atlas_mut(), frame.ticks, None)
                    .map_err(|e| format!("sprite animation: {e}"))?;
            }
            let keys: Vec<_> = self.world.section_keys().collect();
            let plan = scene::prepare_frame(
                &mut self.world,
                &mut self.gui,
                frame,
                viewport,
                min_y,
                &keys,
                gamma,
                fullbright,
            )
            .map_err(|e| format!("frame: {e:?}"))?;
            Ok((
                plan.terrain.len(),
                plan.entities.vertices.len(),
                plan.hud.vertices.len(),
            ))
        })();
        match result {
            Ok((terrain, entities, hud)) if !frame.title && !self.reported_frame => {
                log::info!(
                    "Legacy CPU mirror prepared a game frame: {terrain} terrain draws, {entities} entity vertices, {hud} HUD vertices (no GL rendering)"
                );
                self.reported_frame = true;
            }
            Err(error) if !self.reported_error => {
                log::warn!(
                    "Legacy CPU mirror cannot prepare a frame: {error}; wgpu continues normally"
                );
                self.reported_error = true;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_game_section_and_frame_lifecycle() {
        let mut mirror = Mirror::new(&[255; 4], 1, 1, (&[255; 4], 1, 1), &[]).unwrap();
        let key = (2, 0, 3);
        let section = SectionMesh::default();
        mirror.upload(key, &section).unwrap();
        assert_eq!(mirror.world.section_keys().collect::<Vec<_>>(), [key]);
        let hud = crate::hud::HudBatch::new([32, 32]);
        let frame = crate::legacy::scene::tests::test_frame(&hud);
        mirror.frame(&frame, [32, 32], 0, 0.5, false, true);
        assert!(mirror.reported_frame);
        assert!(!mirror.reported_error);
        mirror.remove_columns(&[(2, 3)]);
        assert!(mirror.world.is_empty());
        mirror.upload(key, &section).unwrap();
        mirror.clear();
        assert!(mirror.world.is_empty());
        assert!(Mirror::new(&[], 1, 1, (&[255; 4], 1, 1), &[]).is_err());
    }
}
