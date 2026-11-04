//! PR 11/12/13: an interactive GL2 game state with an authoritative
//! integrated server, LAN link and multiplayer state.
//!
//! The offline path keeps world streaming, physics, editing, HUD, menus and
//! saves in pure Rust. `--integrated` moves authority to a 20 Hz server thread;
//! `--join` connects to another GL2 world over the Horizon-internal LAN
//! protocol. This is not Minecraft Java protocol parity.

pub mod net;
pub mod player;
pub mod save;
pub mod scene;
pub mod server;
pub mod text;
pub mod ui;
pub mod world;

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Instant;

use horizon_client_render::gfx21::facade::LegacySettings;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

use self::player::{Input, Player};
use self::save::{SaveState, load, save, save_path};
use self::scene::RenderScene;
use self::server::{
    ClientCommand, ContainerAction, Link, LinkMode, ServerCommand, ServerEvent, ServerPlayer,
    ServerSnapshot,
};
use self::ui::{Screen, UiAction, UiFrame, UiSnapshot};
use self::world::World;

pub const MAX_RENDER_DISTANCE: u32 = 8;

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
    link: Link,
    remote_players: Vec<ServerPlayer>,
    container: [u8; 27],
    chat_input: String,
}

impl Game {
    pub fn new(settings: &LegacySettings) -> Self {
        let link = Link::open(settings);
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
            link,
            remote_players: Vec::new(),
            container: [0; 27],
            chat_input: String::new(),
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
        if !self.link.is_offline() {
            return;
        }
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
        if !self.link.is_offline() {
            self.link.tick();
            self.link.client(ClientCommand::Input(self.input.clone()));
            self.link
                .client(ClientCommand::Pause(self.screen != Screen::Playing));
            for event in self.link.poll() {
                self.apply_event(event);
            }
            return;
        }
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

    pub fn key_down(&mut self, key: KeyCode, text: Option<&str>) -> bool {
        if self.screen == Screen::Chat {
            match key {
                KeyCode::Escape => self.screen = Screen::Playing,
                KeyCode::Enter => {
                    self.submit_chat();
                    self.screen = Screen::Playing;
                }
                KeyCode::Backspace => {
                    self.chat_input.pop();
                }
                _ => {
                    if let Some(text) = text {
                        self.append_chat_text(text);
                    }
                }
            }
            return false;
        }
        match key {
            KeyCode::Escape => match self.screen {
                Screen::Playing => self.screen = Screen::Pause,
                Screen::Pause | Screen::Inventory | Screen::Options | Screen::Chat => {
                    self.screen = Screen::Playing
                }
                Screen::Title => return true,
            },
            KeyCode::KeyT => {
                self.screen = Screen::Chat;
                self.chat_input.clear();
            }
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
            KeyCode::Digit1 => self.select(0),
            KeyCode::Digit2 => self.select(1),
            KeyCode::Digit3 => self.select(2),
            KeyCode::Digit4 => self.select(3),
            KeyCode::Digit5 => self.select(4),
            KeyCode::Digit6 => self.select(5),
            KeyCode::Digit7 => self.select(6),
            KeyCode::Digit8 => self.select(7),
            KeyCode::Digit9 => self.select(8),
            _ => {}
        }
        false
    }

    pub fn append_chat_text(&mut self, text: &str) {
        if self.screen != Screen::Chat {
            return;
        }
        for ch in text.chars() {
            if !ch.is_control() && self.chat_input.len() < 64 {
                self.chat_input.push(ch);
            }
        }
    }

    fn submit_chat(&mut self) {
        let text = self.chat_input.trim().to_owned();
        if text.is_empty() {
            return;
        }
        if self.link.is_offline() {
            self.link
                .local_chat(format!("{}: {text}", self.link.name()));
        } else {
            self.link.client(ClientCommand::Chat(text));
        }
        self.chat_input.clear();
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

    fn select(&mut self, slot: u8) {
        self.player.set_selection(slot);
        if !self.link.is_offline() {
            self.link.client(ClientCommand::SetHotbar(slot));
        }
    }

    pub fn mouse_delta(&mut self, dx: f32, dy: f32) {
        if self.screen != Screen::Playing {
            return;
        }
        if self.link.is_offline() {
            self.player.look(dx, dy, self.settings.sensitivity);
        } else {
            self.link.client(ClientCommand::Look {
                dx,
                dy,
                sensitivity: self.settings.sensitivity,
            });
        }
    }

    pub fn mouse_wheel(&mut self, delta: f32) {
        if self.screen != Screen::Playing {
            return;
        }
        self.player
            .cycle_selection(if delta > 0.0 { -1 } else { 1 });
        if !self.link.is_offline() {
            self.link
                .client(ClientCommand::SetHotbar(self.player.selected));
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
            if let Some(action) = self.ui.action_at(x, y) {
                if let UiAction::Container(slot) = action {
                    let action = if button == MouseButton::Left {
                        ContainerAction::Take
                    } else {
                        ContainerAction::Put
                    };
                    self.container_action(slot, action);
                    return false;
                }
                if button == MouseButton::Left {
                    return self.handle_action(action);
                }
            }
            return false;
        }
        if !self.link.is_offline() {
            match button {
                MouseButton::Left => {
                    self.link.client(ClientCommand::Break);
                }
                MouseButton::Right => {
                    self.link.client(ClientCommand::Place);
                }
                _ => {}
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

    fn container_action(&mut self, slot: u8, action: ContainerAction) {
        if !self.link.is_offline() {
            self.link.client(ClientCommand::Container { slot, action });
            return;
        }
        let index = (slot as usize).min(self.container.len() - 1);
        let selected = self.player.selected as usize;
        match action {
            ContainerAction::Take => {
                let item = self.container[index];
                if item != 0
                    && (self.player.slots[selected] == item || self.player.counts[selected] == 0)
                {
                    if self.player.counts[selected] == 0 {
                        self.player.slots[selected] = item;
                    }
                    self.player.counts[selected] = self.player.counts[selected].saturating_add(1);
                    self.container[index] = 0;
                }
            }
            ContainerAction::Put => {
                let item = self.player.slots[selected];
                if item != 0 && self.player.counts[selected] > 0 && self.container[index] == 0 {
                    self.player.counts[selected] -= 1;
                    self.container[index] = item;
                }
            }
        }
    }

    pub fn handle_action(&mut self, action: UiAction) -> bool {
        match action {
            UiAction::NewWorld => {
                if self.link.is_offline() {
                    self.world = World::new(self.settings.seed);
                    self.player = Player::spawn(&mut self.world);
                    self.chunk_cache.clear();
                    self.dirty.clear();
                    self.sync_chunks();
                } else {
                    self.link.send(ServerCommand::Reset {
                        seed: self.settings.seed,
                    });
                    self.chunk_cache.clear();
                    self.dirty.clear();
                }
                self.screen = Screen::Playing;
            }
            UiAction::LoadWorld => {
                if self.link.is_offline() {
                    if let Some(state) = load(&self.save_path) {
                        self.world = World::new(state.seed);
                        self.world.apply_edits(&state.edits);
                        self.world.day_time = state.day_time;
                        self.player = state.player;
                    }
                    self.chunk_cache.clear();
                    self.dirty.clear();
                    self.sync_chunks();
                } else {
                    self.link.send(ServerCommand::Load);
                    self.chunk_cache.clear();
                    self.dirty.clear();
                }
                self.screen = Screen::Playing;
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
                self.link
                    .client(ClientCommand::SetDistance(self.settings.render_distance));
            }
            UiAction::DecreaseDistance => {
                self.settings.render_distance =
                    self.settings.render_distance.saturating_sub(1).max(1);
                self.dirty.clear();
                self.sync_chunks();
                self.link
                    .client(ClientCommand::SetDistance(self.settings.render_distance));
            }
            UiAction::IncreaseFov => self.settings.fov = (self.settings.fov + 5.0).min(110.0),
            UiAction::DecreaseFov => self.settings.fov = (self.settings.fov - 5.0).max(30.0),
            UiAction::Container(_) => {}
        }
        false
    }

    pub fn save(&mut self) -> Result<(), String> {
        if !self.link.is_offline() {
            self.link.send(ServerCommand::Save);
            return Ok(());
        }
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
            network_mode: self.link.mode(),
            server_tick: self.link.tick,
            server_tps: self.link.tps,
            server_players: self.link.players,
            latency_ms: self.link.latency_ms,
            network_error: self.link.error().to_owned(),
            chat: self.link.chat().to_owned(),
            chat_input: self.chat_input.clone(),
            chat_lines: self.link.chat_lines().to_vec(),
            player_names: self.link.names().to_vec(),
            container: self.container,
        }
    }

    fn apply_event(&mut self, event: ServerEvent) {
        let ServerEvent::Snapshot(snapshot) = event else {
            return;
        };
        self.apply_snapshot(snapshot);
    }

    fn apply_snapshot(&mut self, snapshot: ServerSnapshot) {
        self.world.seed = snapshot.seed;
        self.world.chunks.clear();
        self.world.edits.clear();
        self.world.apply_edits(&snapshot.edits);
        self.world.day_time = snapshot.day_time;
        let local_name = self.link.name();
        self.player = snapshot
            .players
            .iter()
            .find(|entry| entry.name.as_str() == local_name)
            .map(|entry| entry.player.clone())
            .unwrap_or(snapshot.player);
        self.remote_players = snapshot.players;
        self.container = snapshot.container;
        self.chunk_cache = snapshot
            .chunks
            .into_iter()
            .map(|chunk| (chunk.key, chunk.vertices))
            .collect();
        self.dirty.clear();
    }

    pub fn shutdown(&mut self) {
        self.link.shutdown();
    }

    pub fn network_mode(&self) -> LinkMode {
        self.link.mode()
    }

    pub fn network_error(&self) -> &str {
        self.link.error()
    }

    pub fn lan_addr(&self) -> Option<SocketAddr> {
        self.link.lan_addr()
    }

    pub fn build_scene(&mut self, width: u32, height: u32) -> Result<RenderScene, String> {
        self.resize(width, height);
        self.sync_chunks();
        RenderScene::build(self, width, height)
    }
}
