//! PR 11: an interactive, offline single-player GL2 game state.
//!
//! This is the first stage of the full `game.rs` port requested after the
//! additive/`full_pr8` work. It keeps original world streaming, physics,
//! editing, HUD, menus and saves in pure Rust; the GL2 backend only receives
//! vertices and a text texture. Vanilla networking/integrated-server parity is
//! not claimed yet.

pub mod player;
pub mod save;
pub mod scene;
pub mod text;
pub mod ui;
pub mod world;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use horizon_client_render::gfx21::facade::LegacySettings;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

use self::player::{Input, Player};
use self::save::{SaveState, load, save, save_path};
use self::scene::RenderScene;
use self::ui::{Screen, UiAction, UiFrame, UiSnapshot};
use self::world::World;

pub const MAX_RENDER_DISTANCE: u32 = 8;

#[derive(Debug)]
pub struct Game {
    pub settings: LegacySettings,
    pub world: World,
    pub player: Player,
    pub screen: Screen,
    pub input: Input,
    pub width: f32,
    pub height: f32,
    pub fps: f32,
    pub draws: usize,
    pub chunk_cache: HashMap<(i32, i32), Vec<f32>>,
    pub dirty: HashSet<(i32, i32)>,
    pub ui: UiFrame,
    pub save_path: PathBuf,
    pub last_frame: Instant,
    tick_accum: f32,
}

impl Game {
    pub fn new(settings: &LegacySettings) -> Self {
        let save_path = save_path(&settings.world);
        let mut world = World::new(settings.seed);
        let mut player = Player::spawn(&mut world);
        if let Some(state) = load(&save_path) {
            world.seed = state.seed;
            world.chunks.clear();
            world.edits.clear();
            world.apply_edits(&state.edits);
            world.day_time = state.day_time;
            player = state.player;
        }
        let mut game = Self {
            settings: settings.clone(),
            world,
            player,
            screen: if settings.frames == 0 {
                Screen::Title
            } else {
                Screen::Playing
            },
            input: Input::default(),
            width: 800.0,
            height: 600.0,
            fps: 0.0,
            draws: 0,
            chunk_cache: HashMap::new(),
            dirty: HashSet::new(),
            ui: UiFrame {
                rects: Vec::new(),
                texts: Vec::new(),
                buttons: Vec::new(),
            },
            save_path,
            last_frame: Instant::now(),
            tick_accum: 0.0,
        };
        game.sync_chunks();
        game
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1) as f32;
        self.height = height.max(1) as f32;
    }

    pub fn cursor_grabbed(&self) -> bool {
        self.screen == Screen::Playing
    }

    pub fn mark_dirty(&mut self, x: i32, _y: i32, z: i32) {
        let (cx, cz) = World::chunk_pos(x, z);
        for dz in -1..=1 {
            for dx in -1..=1 {
                self.dirty.insert((cx + dx, cz + dz));
            }
        }
    }

    pub fn sync_chunks(&mut self) {
        let distance = self.settings.render_distance.clamp(1, MAX_RENDER_DISTANCE) as i32;
        let keys =
            self.world
                .stream_around(self.player.position[0], self.player.position[2], distance);
        self.chunk_cache.retain(|key, _| keys.contains(key));
        for key in keys {
            if self.chunk_cache.contains_key(&key) && !self.dirty.remove(&key) {
                continue;
            }
            let mesh = self.world.mesh_chunk(key.0, key.1);
            self.chunk_cache.insert(key, mesh);
        }
    }

    pub fn update(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.1);
        self.tick_accum += dt;
        while self.tick_accum >= 1.0 / 20.0 {
            self.tick_accum -= 1.0 / 20.0;
            if self.screen == Screen::Playing {
                self.world.day_time = (self.world.day_time + 1) % 24_000;
                self.player.update(&mut self.world, &self.input, 1.0 / 20.0);
                self.sync_chunks();
            }
        }
    }

    pub fn key_down(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Escape => match self.screen {
                Screen::Playing => self.screen = Screen::Pause,
                Screen::Pause | Screen::Inventory | Screen::Options => {
                    self.screen = Screen::Playing
                }
                Screen::Title => return true,
            },
            KeyCode::KeyE => {
                self.screen = if self.screen == Screen::Inventory {
                    Screen::Playing
                } else if self.screen == Screen::Playing {
                    Screen::Inventory
                } else {
                    self.screen
                };
            }
            KeyCode::KeyW | KeyCode::ArrowUp => self.input.forward = true,
            KeyCode::KeyS | KeyCode::ArrowDown => self.input.back = true,
            KeyCode::KeyA | KeyCode::ArrowLeft => self.input.left = true,
            KeyCode::KeyD | KeyCode::ArrowRight => self.input.right = true,
            KeyCode::Space => self.input.jump = true,
            KeyCode::ShiftLeft | KeyCode::ShiftRight => self.input.sprint = true,
            KeyCode::ControlLeft | KeyCode::ControlRight => self.input.sneak = true,
            KeyCode::Digit1 => self.player.set_selection(0),
            KeyCode::Digit2 => self.player.set_selection(1),
            KeyCode::Digit3 => self.player.set_selection(2),
            KeyCode::Digit4 => self.player.set_selection(3),
            KeyCode::Digit5 => self.player.set_selection(4),
            KeyCode::Digit6 => self.player.set_selection(5),
            KeyCode::Digit7 => self.player.set_selection(6),
            KeyCode::Digit8 => self.player.set_selection(7),
            KeyCode::Digit9 => self.player.set_selection(8),
            _ => {}
        }
        false
    }

    pub fn key_up(&mut self, key: KeyCode) {
        match key {
            KeyCode::KeyW | KeyCode::ArrowUp => self.input.forward = false,
            KeyCode::KeyS | KeyCode::ArrowDown => self.input.back = false,
            KeyCode::KeyA | KeyCode::ArrowLeft => self.input.left = false,
            KeyCode::KeyD | KeyCode::ArrowRight => self.input.right = false,
            KeyCode::Space => self.input.jump = false,
            KeyCode::ShiftLeft | KeyCode::ShiftRight => self.input.sprint = false,
            KeyCode::ControlLeft | KeyCode::ControlRight => self.input.sneak = false,
            _ => {}
        }
    }

    pub fn mouse_delta(&mut self, dx: f32, dy: f32) {
        if self.screen == Screen::Playing {
            self.player.look(dx, dy, self.settings.sensitivity);
        }
    }

    pub fn mouse_wheel(&mut self, delta: f32) {
        if self.screen == Screen::Playing {
            self.player
                .cycle_selection(if delta > 0.0 { -1 } else { 1 });
        }
    }

    pub fn mouse_click(
        &mut self,
        button: MouseButton,
        state: ElementState,
        x: f32,
        y: f32,
    ) -> bool {
        if state != ElementState::Pressed {
            return false;
        }
        if self.screen != Screen::Playing {
            if button == MouseButton::Left {
                if let Some(action) = self.ui.action_at(x, y) {
                    return self.handle_action(action);
                }
            }
            return false;
        }
        let hit = self
            .player
            .raycast(&mut self.world, player::REACH)
            .map(|(pos, normal)| (pos, normal));
        let changed = if button == MouseButton::Left {
            self.player.break_block(&mut self.world)
        } else if button == MouseButton::Right {
            self.player.place_block(&mut self.world)
        } else {
            false
        };
        if changed {
            if let Some((pos, normal)) = hit {
                self.mark_dirty(pos[0], pos[1], pos[2]);
                self.mark_dirty(pos[0] + normal[0], pos[1] + normal[1], pos[2] + normal[2]);
            }
            self.sync_chunks();
        }
        false
    }

    pub fn handle_action(&mut self, action: UiAction) -> bool {
        match action {
            UiAction::NewWorld => {
                self.world = World::new(self.settings.seed);
                self.player = Player::spawn(&mut self.world);
                self.chunk_cache.clear();
                self.dirty.clear();
                self.sync_chunks();
                self.screen = Screen::Playing;
            }
            UiAction::LoadWorld => {
                if let Some(state) = load(&self.save_path) {
                    self.world = World::new(state.seed);
                    self.world.apply_edits(&state.edits);
                    self.world.day_time = state.day_time;
                    self.player = state.player;
                    self.chunk_cache.clear();
                    self.dirty.clear();
                    self.sync_chunks();
                    self.screen = Screen::Playing;
                } else {
                    self.screen = Screen::Playing;
                }
            }
            UiAction::Resume | UiAction::Back => self.screen = Screen::Playing,
            UiAction::Inventory => self.screen = Screen::Inventory,
            UiAction::Options => self.screen = Screen::Options,
            UiAction::SaveQuit => {
                let _ = self.save();
                return true;
            }
            UiAction::Quit => return true,
            UiAction::IncreaseDistance => {
                self.settings.render_distance =
                    (self.settings.render_distance + 1).min(MAX_RENDER_DISTANCE);
                self.dirty.clear();
                self.sync_chunks();
            }
            UiAction::DecreaseDistance => {
                self.settings.render_distance =
                    self.settings.render_distance.saturating_sub(1).max(1);
                self.dirty.clear();
                self.sync_chunks();
            }
            UiAction::IncreaseFov => self.settings.fov = (self.settings.fov + 5.0).min(110.0),
            UiAction::DecreaseFov => self.settings.fov = (self.settings.fov - 5.0).max(30.0),
        }
        false
    }

    pub fn save(&self) -> Result<(), String> {
        let mut state = SaveState::new(self.world.seed, &self.player);
        state.day_time = self.world.day_time;
        state.edits = self.world.saved_edits();
        save(&self.save_path, &state)
    }

    pub fn snapshot(&self, draws: usize) -> UiSnapshot {
        UiSnapshot {
            screen: self.screen,
            width: self.width,
            height: self.height,
            fps: self.fps,
            position: self.player.position,
            chunks: self.chunk_cache.len(),
            draws,
            day_time: self.world.day_time,
            seed: self.world.seed,
            health: self.player.health,
            hunger: self.player.hunger,
            selected: self.player.selected,
            slots: self.player.slots,
            counts: self.player.counts,
            render_distance: self.settings.render_distance as i32,
            fov: self.settings.fov,
            sensitivity: self.settings.sensitivity,
        }
    }

    pub fn build_scene(&mut self, width: u32, height: u32) -> Result<RenderScene, String> {
        self.resize(width, height);
        self.sync_chunks();
        RenderScene::build(self, width, height)
    }
}
