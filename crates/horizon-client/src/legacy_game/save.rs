//! Single-player save state. The world is regenerated from its seed and the
//! block edits are replayed, so a save is small and deterministic.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::player::Player;

pub const SAVE_VERSION: u32 = 1;
pub const DEFAULT_SAVE: &str = "horizon-legacy-gl.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SaveState {
    pub version: u32,
    pub seed: u64,
    pub day_time: u32,
    pub player: Player,
    pub edits: Vec<(i32, i32, i32, u8)>,
}

impl SaveState {
    pub fn new(seed: u64, player: &Player) -> Self {
        Self {
            version: SAVE_VERSION,
            seed,
            day_time: 1000,
            player: player.clone(),
            edits: Vec::new(),
        }
    }
}

pub fn save_path(explicit: &str) -> PathBuf {
    if explicit.is_empty() {
        PathBuf::from(DEFAULT_SAVE)
    } else {
        PathBuf::from(explicit)
    }
}

pub fn load(path: &Path) -> Option<SaveState> {
    let text = std::fs::read_to_string(path).ok()?;
    let state: SaveState = serde_json::from_str(&text).ok()?;
    (state.version == SAVE_VERSION).then_some(state)
}

pub fn save(path: &Path, state: &SaveState) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|err| err.to_string())?;
    std::fs::write(path, text).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_game::world::World;

    #[test]
    fn save_round_trip_keeps_the_player_and_edits() {
        let mut world = World::new(1234);
        let player = Player::spawn(&mut world);
        let mut state = SaveState::new(1234, &player);
        state.day_time = 6000;
        state.edits.push((1, 2, 3, 7));
        let path = std::env::temp_dir().join(format!("legacy-save-{}.json", std::process::id()));
        save(&path, &state).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, state);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unknown_versions_are_ignored() {
        let path =
            std::env::temp_dir().join(format!("legacy-save-bad-{}.json", std::process::id()));
        std::fs::write(&path, "{\"version\":999}").unwrap();
        assert!(load(&path).is_none());
        let _ = std::fs::remove_file(path);
    }
}
