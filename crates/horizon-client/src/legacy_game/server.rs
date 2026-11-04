//! PR 12/13: an authoritative integrated server, LAN link and multiplayer
//! state.
//!
//! This is a Horizon-internal GL2 server, not a Minecraft Java protocol
//! implementation. The world, players, day time, saves, edits, chat commands
//! and a shared container live on a 20 Hz server thread; each window renders
//! snapshots and sends actor-scoped commands. `--join` carries the same
//! commands over the newline-delimited JSON LAN protocol in `net.rs`.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, RecvTimeoutError, Sender, channel},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use horizon_client_core::lang;
use horizon_client_render::gfx21::facade::LegacySettings;
use serde::{Deserialize, Serialize};

use super::MAX_RENDER_DISTANCE;
use super::net::{NetClient, NetServer};
use super::player::{Input, Player, REACH};
use super::save::{SaveState, load, save, save_path};
use super::scene::ChunkMesh;
use super::world::{CHUNK_SIZE, World};

const SERVER_TICK_MS: u64 = 50;
const CONTAINER_SLOTS: usize = 27;
const CHAT_HISTORY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkMode {
    Offline,
    Integrated,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerAction {
    Take,
    Put,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientCommand {
    Input(Input),
    Look { dx: f32, dy: f32, sensitivity: f32 },
    Break,
    Place,
    SetHotbar(u8),
    Pause(bool),
    SetDistance(u32),
    Chat(String),
    Container { slot: u8, action: ContainerAction },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerCommand {
    Client {
        name: String,
        command: ClientCommand,
    },
    Join {
        name: String,
    },
    Leave {
        name: String,
    },
    Ping(u64),
    Save,
    Reset {
        seed: u64,
    },
    Load,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerEvent {
    Snapshot(ServerSnapshot),
    Welcome { name: String },
    Pong(u64),
    Chat { name: String, text: String },
    Log(String),
    Error(String),
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerPlayer {
    pub name: String,
    pub player: Player,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerSnapshot {
    pub player: Player,
    pub players: Vec<ServerPlayer>,
    pub day_time: u32,
    pub seed: u64,
    pub edits: Vec<(i32, i32, i32, u8)>,
    pub chunks: Vec<ChunkMesh>,
    pub container: [u8; CONTAINER_SLOTS],
    pub chat_lines: Vec<String>,
    pub tick: u64,
    pub tps: f32,
    pub paused: bool,
    pub population: usize,
    pub names: Vec<String>,
}

struct PlayerState {
    player: Player,
    input: Input,
    paused: bool,
    distance: u32,
}

struct IntegratedState {
    world: World,
    players: HashMap<String, PlayerState>,
    host_name: String,
    container: [u8; CONTAINER_SLOTS],
    chat_lines: Vec<String>,
    tick: u64,
    tps: f32,
    started: Instant,
    default_distance: u32,
    save_path: PathBuf,
    mesh_cache: HashMap<(i32, i32), Vec<f32>>,
    dirty: HashSet<(i32, i32)>,
}

impl IntegratedState {
    fn new(settings: &LegacySettings) -> Self {
        let save_path = save_path(&settings.world);
        let world = World::new(settings.seed);
        let default_distance = settings.render_distance.clamp(1, MAX_RENDER_DISTANCE);
        let mut state = Self {
            world,
            players: HashMap::new(),
            host_name: settings.name.clone(),
            container: [0; CONTAINER_SLOTS],
            chat_lines: Vec::new(),
            tick: 0,
            tps: 0.0,
            started: Instant::now(),
            default_distance,
            save_path,
            mesh_cache: HashMap::new(),
            dirty: HashSet::new(),
        };
        if state.join(&settings.name).is_some() {
            // The join notice is not emitted before the first public snapshot.
            state.chat_lines.clear();
        }
        if let Some(saved) = load(&state.save_path) {
            state.world.seed = saved.seed;
            state.world.chunks.clear();
            state.world.edits.clear();
            state.world.apply_edits(&saved.edits);
            state.world.day_time = saved.day_time;
            if let Some(host) = state.players.get_mut(&state.host_name) {
                host.player = saved.player;
            }
        }
        state
    }

    fn save(&self) -> Result<(), String> {
        let host = &self.players[&self.host_name].player;
        let mut state = SaveState::new(self.world.seed, host);
        state.day_time = self.world.day_time;
        state.edits = self.world.saved_edits();
        save(&self.save_path, &state)
    }

    fn load_saved(&mut self) -> Result<(), String> {
        let Some(saved) = load(&self.save_path) else {
            return Err("no save".to_owned());
        };
        self.world = World::new(saved.seed);
        self.world.apply_edits(&saved.edits);
        self.world.day_time = saved.day_time;
        if let Some(host) = self.players.get_mut(&self.host_name) {
            host.player = saved.player;
        }
        for (name, state) in self.players.iter_mut() {
            if name != &self.host_name {
                state.player = Player::spawn(&mut self.world);
            }
        }
        self.mesh_cache.clear();
        self.dirty.clear();
        Ok(())
    }

    fn join(&mut self, name: &str) -> Option<ServerEvent> {
        if self.players.contains_key(name) {
            return None;
        }
        let mut player = Player::spawn(&mut self.world);
        let hash = name.bytes().fold(37_u32, |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(byte as u32)
        });
        player.position[0] += (hash % 7) as f32 - 3.0;
        player.position[2] += ((hash / 7) % 7) as f32 - 3.0;
        self.players.insert(
            name.to_owned(),
            PlayerState {
                player,
                input: Input::default(),
                paused: false,
                distance: self.default_distance,
            },
        );
        let server = lang::get("horizon.legacyGl.game.serverName");
        Some(self.chat(
            &server,
            lang::tr("horizon.legacyGl.game.playerJoined", &[&name]),
        ))
    }

    fn leave(&mut self, name: &str) -> Option<ServerEvent> {
        self.players.remove(name)?;
        let server = lang::get("horizon.legacyGl.game.serverName");
        Some(self.chat(
            &server,
            lang::tr("horizon.legacyGl.game.playerLeft", &[&name]),
        ))
    }

    fn chat(&mut self, name: &str, text: String) -> ServerEvent {
        self.push_chat(format!("{name}: {text}"));
        ServerEvent::Chat {
            name: name.to_owned(),
            text,
        }
    }

    fn push_chat(&mut self, line: String) {
        self.chat_lines.push(line);
        if self.chat_lines.len() > CHAT_HISTORY {
            self.chat_lines.remove(0);
        }
    }

    fn log(&mut self, text: String) -> ServerEvent {
        self.push_chat(text.clone());
        ServerEvent::Log(text)
    }

    fn command<F: FnMut(ServerEvent)>(&mut self, actor: &str, text: &str, emit: &mut F) {
        let trimmed = text.trim();
        if !trimmed.starts_with('/') {
            emit(self.chat(actor, trimmed.to_owned()));
            return;
        }
        let parts: Vec<&str> = trimmed[1..].split_whitespace().collect();
        let command = parts.first().copied().unwrap_or("");
        let server = lang::get("horizon.legacyGl.game.serverName");
        let mut names: Vec<String> = self.players.keys().cloned().collect();
        names.sort();
        let joined = names.join(", ");
        let response = match command {
            "help" => lang::get("horizon.legacyGl.game.commandHelp"),
            "seed" => lang::tr("horizon.legacyGl.game.commandSeed", &[&self.world.seed]),
            "time" => lang::tr("horizon.legacyGl.game.commandTime", &[&self.world.day_time]),
            "list" => lang::tr("horizon.legacyGl.game.commandList", &[&joined]),
            "position" => {
                let position = self
                    .players
                    .get(actor)
                    .map(|state| state.player.position)
                    .unwrap_or([0.0; 3]);
                lang::tr(
                    "horizon.legacyGl.game.commandPosition",
                    &[&position[0], &position[1], &position[2]],
                )
            }
            "spawn" => {
                if let Some(state) = self.players.get_mut(actor) {
                    state.player = Player::spawn(&mut self.world);
                }
                lang::get("horizon.legacyGl.game.commandSpawn")
            }
            "heal" => {
                if let Some(state) = self.players.get_mut(actor) {
                    state.player.health = 20;
                    state.player.hunger = 20;
                }
                lang::get("horizon.legacyGl.game.commandHeal")
            }
            "give" => {
                let slot = self
                    .players
                    .get_mut(actor)
                    .map(|state| {
                        let slot = state.player.selected;
                        state.player.counts[slot as usize] =
                            state.player.counts[slot as usize].saturating_add(1);
                        slot
                    })
                    .unwrap_or(0);
                lang::tr("horizon.legacyGl.game.commandGive", &[&slot])
            }
            "container" => {
                let filled = self.container.iter().filter(|item| **item != 0).count();
                lang::tr("horizon.legacyGl.game.commandContainer", &[&filled])
            }
            "day" | "night" => {
                self.world.day_time = if command == "day" { 1000 } else { 13_000 };
                lang::get(if command == "day" {
                    "horizon.legacyGl.game.commandDay"
                } else {
                    "horizon.legacyGl.game.commandNight"
                })
            }
            "tp" => {
                let parsed: Vec<Option<f32>> = parts
                    .iter()
                    .skip(1)
                    .take(3)
                    .map(|part| part.parse::<f32>().ok())
                    .collect();
                if parsed.len() == 3 && parsed.iter().all(Option::is_some) {
                    let x = parsed[0].unwrap();
                    let y = parsed[1].unwrap();
                    let z = parsed[2].unwrap();
                    if let Some(state) = self.players.get_mut(actor) {
                        state.player.position = [x, y, z];
                        state.player.velocity = [0.0; 3];
                    }
                    lang::get("horizon.legacyGl.game.commandTp")
                } else {
                    lang::tr(
                        "horizon.legacyGl.game.commandUsage",
                        &[&"/tp X Y Z".to_owned()],
                    )
                }
            }
            "save" => match self.save() {
                Ok(()) => lang::get("horizon.legacyGl.game.commandSaved"),
                Err(err) => lang::tr("horizon.legacyGl.game.commandSaveError", &[&err]),
            },
            other => lang::tr("horizon.legacyGl.game.commandUnknown", &[&other]),
        };
        emit(self.chat(&server, response));
    }

    fn mark_dirty(&mut self, x: i32, z: i32) {
        let (cx, cz) = World::chunk_pos(x, z);
        for dz in -1..=1 {
            for dx in -1..=1 {
                self.dirty.insert((cx + dx, cz + dz));
            }
        }
    }

    fn sync_chunks(&mut self) {
        let requests: Vec<([f32; 3], u32)> = self
            .players
            .values()
            .map(|state| (state.player.position, state.distance))
            .collect();
        let mut keys = HashSet::new();
        for (position, distance) in requests {
            for key in self
                .world
                .stream_around(position[0], position[2], distance as i32)
            {
                keys.insert(key);
            }
        }
        self.mesh_cache.retain(|key, _| keys.contains(key));
        for key in keys {
            if self.mesh_cache.contains_key(&key) && !self.dirty.remove(&key) {
                continue;
            }
            let mesh = self.world.mesh_chunk(key.0, key.1);
            self.mesh_cache.insert(key, mesh);
        }
    }

    fn snapshot(&mut self) -> ServerSnapshot {
        self.sync_chunks();
        let mut names: Vec<String> = self.players.keys().cloned().collect();
        names.sort();
        let players = names
            .iter()
            .map(|name| ServerPlayer {
                name: name.clone(),
                player: self.players[name].player.clone(),
            })
            .collect::<Vec<_>>();
        let player = self.players[&self.host_name].player.clone();
        let paused = self.players[&self.host_name].paused;
        let mut chunk_keys: Vec<_> = self.mesh_cache.keys().copied().collect();
        chunk_keys.sort_unstable();
        let chunks = chunk_keys
            .into_iter()
            .map(|(cx, cz)| ChunkMesh {
                key: (cx, cz),
                origin: [(cx * CHUNK_SIZE) as f32, 0.0, (cz * CHUNK_SIZE) as f32],
                vertices: self.mesh_cache[&(cx, cz)].clone(),
            })
            .collect();
        ServerSnapshot {
            player,
            players: players.clone(),
            day_time: self.world.day_time,
            seed: self.world.seed,
            edits: self.world.saved_edits(),
            chunks,
            container: self.container,
            chat_lines: self.chat_lines.clone(),
            tick: self.tick,
            tps: self.tps,
            paused,
            population: players.len(),
            names,
        }
    }

    fn step(&mut self) {
        self.tick += 1;
        self.tps = self.tick as f32 / self.started.elapsed().as_secs_f32().max(0.001);
        self.world.day_time = (self.world.day_time + 1) % 24_000;
        for state in self.players.values_mut() {
            if state.paused {
                continue;
            }
            let input = state.input.clone();
            state.player.update(&mut self.world, &input, 1.0 / 20.0);
        }
        self.sync_chunks();
    }

    fn apply_client<F: FnMut(ServerEvent)>(
        &mut self,
        name: &str,
        command: ClientCommand,
        emit: &mut F,
    ) {
        if let ClientCommand::Chat(text) = command {
            self.command(name, &text, emit);
            return;
        }
        if !self.players.contains_key(name) {
            if let Some(event) = self.join(name) {
                emit(event);
            }
        }
        let place = matches!(&command, ClientCommand::Place);
        let mut dirty_points = Vec::new();
        let mut needs_sync = false;
        {
            let world = &mut self.world;
            let container = &mut self.container;
            let Some(state) = self.players.get_mut(name) else {
                return;
            };
            match command {
                ClientCommand::Input(input) => state.input = input,
                ClientCommand::Look {
                    dx,
                    dy,
                    sensitivity,
                } => state.player.look(dx, dy, sensitivity),
                ClientCommand::Break | ClientCommand::Place => {
                    let hit = state.player.raycast(world, REACH);
                    let changed = if place {
                        state.player.place_block(world)
                    } else {
                        state.player.break_block(world)
                    };
                    if changed {
                        if let Some((pos, normal)) = hit {
                            dirty_points.push((pos[0], pos[2]));
                            dirty_points.push((pos[0] + normal[0], pos[2] + normal[2]));
                        }
                    }
                }
                ClientCommand::SetHotbar(slot) => state.player.set_selection(slot),
                ClientCommand::Pause(paused) => {
                    state.paused = paused;
                    if paused {
                        state.input = Input::default();
                    }
                }
                ClientCommand::SetDistance(distance) => {
                    state.distance = distance.clamp(1, MAX_RENDER_DISTANCE);
                    needs_sync = true;
                }
                ClientCommand::Container { slot, action } => {
                    let index = (slot as usize).min(CONTAINER_SLOTS - 1);
                    let selected = state.player.selected as usize;
                    match action {
                        ContainerAction::Take => {
                            let item = container[index];
                            if item != 0
                                && (state.player.slots[selected] == item
                                    || state.player.counts[selected] == 0)
                            {
                                if state.player.counts[selected] == 0 {
                                    state.player.slots[selected] = item;
                                }
                                state.player.counts[selected] =
                                    state.player.counts[selected].saturating_add(1);
                                container[index] = 0;
                            }
                        }
                        ContainerAction::Put => {
                            let item = state.player.slots[selected];
                            if item != 0
                                && state.player.counts[selected] > 0
                                && container[index] == 0
                            {
                                state.player.counts[selected] -= 1;
                                container[index] = item;
                            }
                        }
                    }
                }
                ClientCommand::Chat(_) => unreachable!(),
            }
        }
        for (x, z) in dirty_points {
            self.mark_dirty(x, z);
        }
        if needs_sync {
            self.sync_chunks();
        }
    }

    fn apply<F: FnMut(ServerEvent)>(&mut self, command: ServerCommand, emit: &mut F) {
        match command {
            ServerCommand::Client { name, command } => {
                self.apply_client(&name, command, emit);
            }
            ServerCommand::Join { name } => {
                if let Some(event) = self.join(&name) {
                    emit(event);
                }
            }
            ServerCommand::Leave { name } => {
                if let Some(event) = self.leave(&name) {
                    emit(event);
                }
            }
            ServerCommand::Ping(id) => emit(ServerEvent::Pong(id)),
            ServerCommand::Save => match self.save() {
                Ok(()) => emit(self.log(lang::get("horizon.legacyGl.game.commandSaved"))),
                Err(err) => emit(ServerEvent::Error(err)),
            },
            ServerCommand::Reset { seed } => {
                self.world = World::new(seed);
                for state in self.players.values_mut() {
                    state.player = Player::spawn(&mut self.world);
                }
                self.mesh_cache.clear();
                self.dirty.clear();
                emit(self.log(lang::get("horizon.legacyGl.game.commandReset")));
            }
            ServerCommand::Load => match self.load_saved() {
                Ok(()) => emit(self.log(lang::get("horizon.legacyGl.game.commandLoaded"))),
                Err(err) => emit(ServerEvent::Error(err)),
            },
            ServerCommand::Stop => {}
        }
    }
}

pub struct ServerHandle {
    commands: Sender<ServerCommand>,
    thread: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl ServerHandle {
    pub fn start(
        settings: &LegacySettings,
        broadcast: Option<Sender<ServerEvent>>,
    ) -> Result<(Self, Receiver<ServerEvent>), String> {
        let (command_tx, command_rx) = channel::<ServerCommand>();
        let (event_tx, event_rx) = channel::<ServerEvent>();
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = running.clone();
        let settings = settings.clone();
        let thread = thread::Builder::new()
            .name("horizon-legacy-integrated".to_owned())
            .spawn(move || {
                let mut emit = |event: ServerEvent| {
                    let _ = event_tx.send(event.clone());
                    if let Some(tx) = broadcast.as_ref() {
                        let _ = tx.send(event);
                    }
                };
                let mut state = IntegratedState::new(&settings);
                emit(ServerEvent::Snapshot(state.snapshot()));
                let mut next_tick = Instant::now() + Duration::from_millis(SERVER_TICK_MS);
                loop {
                    if !thread_running.load(Ordering::Relaxed) {
                        break;
                    }
                    let wait = next_tick.saturating_duration_since(Instant::now());
                    match command_rx.recv_timeout(wait) {
                        Ok(command) => {
                            let stop = matches!(command, ServerCommand::Stop);
                            state.apply(command, &mut emit);
                            if stop {
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    if Instant::now() >= next_tick {
                        state.step();
                        next_tick += Duration::from_millis(SERVER_TICK_MS);
                        if state.tick % 2 == 0 {
                            emit(ServerEvent::Snapshot(state.snapshot()));
                        }
                    }
                }
                if let Err(err) = state.save() {
                    emit(ServerEvent::Error(err));
                }
                emit(ServerEvent::Stopped);
            })
            .map_err(|err| err.to_string())?;
        Ok((
            Self {
                commands: command_tx,
                thread: Some(thread),
                running,
            },
            event_rx,
        ))
    }

    pub fn command_sender(&self) -> Sender<ServerCommand> {
        self.commands.clone()
    }

    pub fn send(&self, command: ServerCommand) -> bool {
        self.commands.send(command).is_ok()
    }

    pub fn shutdown(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.commands.send(ServerCommand::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub struct Link {
    mode: LinkMode,
    server: Option<ServerHandle>,
    net_server: Option<NetServer>,
    client: Option<NetClient>,
    events: Option<Receiver<ServerEvent>>,
    name: String,
    pub tick: u64,
    pub tps: f32,
    pub players: usize,
    pub latency_ms: u32,
    pub names: Vec<String>,
    error: String,
    chat_lines: Vec<String>,
    last_ping: Instant,
    ping_id: u64,
    pending_ping: Option<(u64, Instant)>,
}

impl Link {
    pub fn open(settings: &LegacySettings) -> Self {
        if !settings.join.is_empty() {
            return match NetClient::connect(&settings.join, &settings.name) {
                Ok((client, events)) => Self {
                    mode: LinkMode::Remote,
                    server: None,
                    net_server: None,
                    client: Some(client),
                    events: Some(events),
                    name: settings.name.clone(),
                    tick: 0,
                    tps: 0.0,
                    players: 0,
                    latency_ms: 0,
                    names: Vec::new(),
                    error: String::new(),
                    chat_lines: Vec::new(),
                    last_ping: Instant::now(),
                    ping_id: 1,
                    pending_ping: None,
                },
                Err(err) => Self::failed(LinkMode::Remote, &settings.name, err),
            };
        }
        if settings.integrated {
            let (broadcast_tx, broadcast_rx) = channel::<ServerEvent>();
            let server_broadcast = settings.lan_port.map(|_| broadcast_tx.clone());
            let (mut server, events) = match ServerHandle::start(settings, server_broadcast) {
                Ok(pair) => pair,
                Err(err) => {
                    drop(broadcast_rx);
                    return Self::failed(LinkMode::Integrated, &settings.name, err);
                }
            };
            let mut net_server = None;
            if let Some(port) = settings.lan_port {
                match NetServer::start(port, server.command_sender(), broadcast_rx) {
                    Ok(net) => net_server = Some(net),
                    Err(err) => {
                        server.shutdown();
                        drop(broadcast_tx);
                        return Self::failed(LinkMode::Integrated, &settings.name, err);
                    }
                }
            } else {
                drop(broadcast_rx);
            }
            drop(broadcast_tx);
            return Self {
                mode: LinkMode::Integrated,
                server: Some(server),
                net_server,
                client: None,
                events: Some(events),
                name: settings.name.clone(),
                tick: 0,
                tps: 0.0,
                players: 1,
                latency_ms: 0,
                names: vec![settings.name.clone()],
                error: String::new(),
                chat_lines: Vec::new(),
                last_ping: Instant::now(),
                ping_id: 1,
                pending_ping: None,
            };
        }
        Self::offline(&settings.name)
    }

    fn offline(name: &str) -> Self {
        Self {
            mode: LinkMode::Offline,
            server: None,
            net_server: None,
            client: None,
            events: None,
            name: name.to_owned(),
            tick: 0,
            tps: 0.0,
            players: 0,
            latency_ms: 0,
            names: Vec::new(),
            error: String::new(),
            chat_lines: Vec::new(),
            last_ping: Instant::now(),
            ping_id: 1,
            pending_ping: None,
        }
    }

    fn failed(mode: LinkMode, name: &str, error: String) -> Self {
        let mut link = Self::offline(name);
        link.mode = mode;
        link.error = error;
        link
    }

    pub fn mode(&self) -> LinkMode {
        self.mode
    }

    pub fn is_offline(&self) -> bool {
        self.mode == LinkMode::Offline
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn chat_lines(&self) -> &[String] {
        &self.chat_lines
    }

    pub fn chat(&self) -> &str {
        self.chat_lines.last().map(String::as_str).unwrap_or("")
    }

    pub fn error(&self) -> &str {
        &self.error
    }

    pub fn lan_addr(&self) -> Option<SocketAddr> {
        self.net_server.as_ref().map(|net| net.addr)
    }

    pub fn local_chat(&mut self, line: String) {
        self.push_chat(line);
    }

    fn push_chat(&mut self, line: String) {
        self.chat_lines.push(line);
        if self.chat_lines.len() > CHAT_HISTORY {
            self.chat_lines.remove(0);
        }
    }

    pub fn send(&mut self, command: ServerCommand) -> bool {
        if let Some(server) = self.server.as_ref() {
            return server.send(command);
        }
        if let Some(client) = self.client.as_ref() {
            return client.send(command);
        }
        false
    }

    pub fn client(&mut self, command: ClientCommand) -> bool {
        self.send(ServerCommand::Client {
            name: self.name.clone(),
            command,
        })
    }

    pub fn tick(&mut self) {
        if self.mode == LinkMode::Remote
            && self.pending_ping.is_none()
            && self.last_ping.elapsed() >= Duration::from_secs(1)
        {
            let id = self.ping_id;
            self.ping_id += 1;
            self.pending_ping = Some((id, Instant::now()));
            self.last_ping = Instant::now();
            self.send(ServerCommand::Ping(id));
        }
    }

    pub fn poll(&mut self) -> Vec<ServerEvent> {
        let mut collected = Vec::new();
        if let Some(receiver) = self.events.as_ref() {
            while let Ok(event) = receiver.try_recv() {
                collected.push(event);
            }
        }
        for event in &collected {
            self.observe(event);
        }
        collected
    }

    fn observe(&mut self, event: &ServerEvent) {
        match event {
            ServerEvent::Snapshot(snapshot) => {
                self.tick = snapshot.tick;
                self.tps = snapshot.tps;
                self.players = snapshot.population;
                self.names = snapshot.names.clone();
                self.chat_lines = snapshot.chat_lines.clone();
                self.error.clear();
            }
            ServerEvent::Welcome { name } => {
                self.name = name.clone();
            }
            ServerEvent::Pong(id) => {
                if let Some((pending, sent)) = self.pending_ping {
                    if pending == *id {
                        self.latency_ms = sent.elapsed().as_millis().min(u32::MAX as u128) as u32;
                        self.pending_ping = None;
                    }
                }
            }
            ServerEvent::Chat { name, text } => {
                self.push_chat(format!("{name}: {text}"));
            }
            ServerEvent::Log(text) => self.push_chat(text.clone()),
            ServerEvent::Error(err) => {
                self.error = err.clone();
                self.push_chat(err.clone());
            }
            ServerEvent::Stopped => {}
        }
    }

    pub fn shutdown(&mut self) {
        if self.mode == LinkMode::Remote {
            self.send(ServerCommand::Leave {
                name: self.name.clone(),
            });
        }
        if let Some(mut client) = self.client.take() {
            client.shutdown();
        }
        if let Some(mut net_server) = self.net_server.take() {
            net_server.shutdown();
        }
        if let Some(mut server) = self.server.take() {
            server.shutdown();
        }
        self.events.take();
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_world(tag: &str) -> String {
        std::env::temp_dir()
            .join(format!("horizon-legacy-{tag}-{}.json", std::process::id()))
            .to_string_lossy()
            .into_owned()
    }

    fn settings_for(tag: &str) -> LegacySettings {
        let mut settings = LegacySettings::reference();
        settings.game = true;
        settings.frames = 2;
        settings.render_distance = 1;
        settings.name = "Host".to_owned();
        settings.world = temp_world(tag);
        settings
    }

    #[test]
    fn integrated_server_steps_and_answers_a_ping() {
        let settings = settings_for("server13");
        let (server, events) = ServerHandle::start(&settings, None).unwrap();
        let mut saw_snapshot = false;
        let mut saw_pong = false;
        let deadline = Instant::now() + Duration::from_secs(2);
        server.send(ServerCommand::Ping(17));
        while Instant::now() < deadline && !(saw_snapshot && saw_pong) {
            if let Ok(event) = events.recv_timeout(Duration::from_millis(50)) {
                match event {
                    ServerEvent::Snapshot(snapshot) => {
                        assert!(!snapshot.chunks.is_empty());
                        assert_eq!(snapshot.population, 1);
                        assert_eq!(snapshot.players.len(), 1);
                        saw_snapshot = true;
                    }
                    ServerEvent::Pong(17) => saw_pong = true,
                    _ => {}
                }
            }
        }
        assert!(saw_snapshot, "the integrated server must stream a snapshot");
        assert!(saw_pong, "the integrated server must answer a ping");
        drop(server);
        let _ = std::fs::remove_file(settings.world);
    }

    #[test]
    fn chat_commands_and_container_are_authoritative() {
        let settings = settings_for("commands13");
        let mut state = IntegratedState::new(&settings);
        let mut events = Vec::new();
        {
            let mut emit = |event: ServerEvent| events.push(event);
            state.apply(
                ServerCommand::Client {
                    name: settings.name.clone(),
                    command: ClientCommand::Chat("/seed".to_owned()),
                },
                &mut emit,
            );
            state.apply(
                ServerCommand::Client {
                    name: settings.name.clone(),
                    command: ClientCommand::Container {
                        slot: 0,
                        action: ContainerAction::Put,
                    },
                },
                &mut emit,
            );
            assert_ne!(state.container[0], 0);
            state.apply(
                ServerCommand::Client {
                    name: settings.name.clone(),
                    command: ClientCommand::Container {
                        slot: 0,
                        action: ContainerAction::Take,
                    },
                },
                &mut emit,
            );
        }
        assert_eq!(state.container[0], 0);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ServerEvent::Chat { .. })),
            "chat commands must produce a server message"
        );
        let snapshot = state.snapshot();
        assert_eq!(snapshot.population, 1);
        assert_eq!(snapshot.names, vec!["Host"]);
        assert!(!snapshot.chat_lines.is_empty());
        let _ = std::fs::remove_file(settings.world);
    }

    #[test]
    fn integrated_and_remote_links_exchange_a_ping() {
        let mut host_settings = settings_for("host13");
        host_settings.integrated = true;
        host_settings.lan_port = Some(0);
        let mut host = Link::open(&host_settings);
        assert_eq!(host.mode(), LinkMode::Integrated);
        let addr = host.lan_addr().expect("LAN port must be bound");

        let mut guest_settings = settings_for("guest13");
        guest_settings.name = "Guest".to_owned();
        guest_settings.join = format!("127.0.0.1:{}", addr.port());
        let mut guest = Link::open(&guest_settings);
        assert_eq!(guest.mode(), LinkMode::Remote);
        assert!(
            guest.error().is_empty(),
            "connect failed: {}",
            guest.error()
        );

        guest.send(ServerCommand::Ping(23));
        let mut saw_pong = false;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !saw_pong {
            guest.tick();
            for event in guest.poll() {
                if matches!(event, ServerEvent::Pong(23)) {
                    saw_pong = true;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_pong, "LAN Pong must reach the remote link");
        guest.shutdown();
        host.shutdown();
        let _ = std::fs::remove_file(host_settings.world);
        let _ = std::fs::remove_file(guest_settings.world);
    }
}
