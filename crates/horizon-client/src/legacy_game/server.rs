//! PR 12: an authoritative integrated server and a client link.
//!
//! This is a Horizon-internal GL2 server, not a Minecraft Java protocol
//! implementation. The world, player, day time, saves and edits live on a
//! 20 Hz server thread; the window sends commands and renders snapshots.
//! `--join` uses the same commands over a small newline-delimited JSON LAN
//! protocol instead of an in-process channel.

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

use horizon_client_render::gfx21::facade::LegacySettings;
use serde::{Deserialize, Serialize};

use super::MAX_RENDER_DISTANCE;
use super::net::{NetClient, NetServer};
use super::player::{Input, Player, REACH};
use super::save::{SaveState, load, save, save_path};
use super::scene::ChunkMesh;
use super::world::{CHUNK_SIZE, World};

const SERVER_TICK_MS: u64 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkMode {
    Offline,
    Integrated,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerCommand {
    Input(Input),
    Look { dx: f32, dy: f32, sensitivity: f32 },
    Break,
    Place,
    SetHotbar(u8),
    Pause(bool),
    Save,
    Reset { seed: u64 },
    Load,
    SetDistance(u32),
    Ping(u64),
    Chat { name: String, text: String },
    Join { name: String },
    Leave { name: String },
    Stop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerEvent {
    Snapshot(ServerSnapshot),
    Pong(u64),
    Chat { name: String, text: String },
    Log(String),
    Error(String),
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerSnapshot {
    pub player: Player,
    pub day_time: u32,
    pub seed: u64,
    pub edits: Vec<(i32, i32, i32, u8)>,
    pub chunks: Vec<ChunkMesh>,
    pub tick: u64,
    pub tps: f32,
    pub paused: bool,
    pub players: usize,
    pub names: Vec<String>,
}

struct IntegratedState {
    world: World,
    player: Player,
    input: Input,
    paused: bool,
    tick: u64,
    tps: f32,
    started: Instant,
    distance: u32,
    save_path: PathBuf,
    mesh_cache: HashMap<(i32, i32), Vec<f32>>,
    dirty: HashSet<(i32, i32)>,
    names: Vec<String>,
}

impl IntegratedState {
    fn new(settings: &LegacySettings) -> Self {
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
        Self {
            world,
            player,
            input: Input::default(),
            paused: false,
            tick: 0,
            tps: 0.0,
            started: Instant::now(),
            distance: settings.render_distance.clamp(1, MAX_RENDER_DISTANCE),
            save_path,
            mesh_cache: HashMap::new(),
            dirty: HashSet::new(),
            names: Vec::new(),
        }
    }

    fn save(&self) -> Result<(), String> {
        let mut state = SaveState::new(self.world.seed, &self.player);
        state.day_time = self.world.day_time;
        state.edits = self.world.saved_edits();
        save(&self.save_path, &state)
    }

    fn load_saved(&mut self) -> Result<(), String> {
        let Some(state) = load(&self.save_path) else {
            return Err("no save".to_owned());
        };
        self.world = World::new(state.seed);
        self.world.apply_edits(&state.edits);
        self.world.day_time = state.day_time;
        self.player = state.player;
        self.mesh_cache.clear();
        self.dirty.clear();
        Ok(())
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
        let keys = self.world.stream_around(
            self.player.position[0],
            self.player.position[2],
            self.distance as i32,
        );
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
        let mut keys: Vec<_> = self.mesh_cache.keys().copied().collect();
        keys.sort_unstable();
        let chunks = keys
            .into_iter()
            .map(|(cx, cz)| ChunkMesh {
                key: (cx, cz),
                origin: [(cx * CHUNK_SIZE) as f32, 0.0, (cz * CHUNK_SIZE) as f32],
                vertices: self.mesh_cache[&(cx, cz)].clone(),
            })
            .collect();
        ServerSnapshot {
            player: self.player.clone(),
            day_time: self.world.day_time,
            seed: self.world.seed,
            edits: self.world.saved_edits(),
            chunks,
            tick: self.tick,
            tps: self.tps,
            paused: self.paused,
            players: self.names.len() + 1,
            names: self.names.clone(),
        }
    }

    fn step(&mut self) {
        self.tick += 1;
        self.tps = self.tick as f32 / self.started.elapsed().as_secs_f32().max(0.001);
        if !self.paused {
            self.world.day_time = (self.world.day_time + 1) % 24_000;
            self.player.update(&mut self.world, &self.input, 1.0 / 20.0);
            self.sync_chunks();
        }
    }

    fn apply<F: FnMut(ServerEvent)>(&mut self, command: ServerCommand, emit: &mut F) {
        let place = matches!(&command, ServerCommand::Place);
        match command {
            ServerCommand::Input(input) => self.input = input,
            ServerCommand::Look {
                dx,
                dy,
                sensitivity,
            } => self.player.look(dx, dy, sensitivity),
            ServerCommand::Break | ServerCommand::Place => {
                let hit = self.player.raycast(&mut self.world, REACH);
                let changed = if place {
                    self.player.place_block(&mut self.world)
                } else {
                    self.player.break_block(&mut self.world)
                };
                if changed {
                    if let Some((pos, normal)) = hit {
                        self.mark_dirty(pos[0], pos[2]);
                        self.mark_dirty(pos[0] + normal[0], pos[2] + normal[2]);
                    }
                }
            }
            ServerCommand::SetHotbar(slot) => self.player.set_selection(slot),
            ServerCommand::Pause(paused) => self.paused = paused,
            ServerCommand::Save => match self.save() {
                Ok(()) => emit(ServerEvent::Log("saved".to_owned())),
                Err(err) => emit(ServerEvent::Error(err)),
            },
            ServerCommand::Reset { seed } => {
                self.world = World::new(seed);
                self.player = Player::spawn(&mut self.world);
                self.mesh_cache.clear();
                self.dirty.clear();
                emit(ServerEvent::Log("reset".to_owned()));
            }
            ServerCommand::Load => match self.load_saved() {
                Ok(()) => emit(ServerEvent::Log("loaded".to_owned())),
                Err(err) => emit(ServerEvent::Error(err)),
            },
            ServerCommand::SetDistance(distance) => {
                self.distance = distance.clamp(1, MAX_RENDER_DISTANCE);
                self.dirty.clear();
                self.sync_chunks();
            }
            ServerCommand::Ping(id) => emit(ServerEvent::Pong(id)),
            ServerCommand::Chat { name, text } => {
                emit(ServerEvent::Chat { name, text });
            }
            ServerCommand::Join { name } => {
                if !self.names.contains(&name) {
                    self.names.push(name);
                }
            }
            ServerCommand::Leave { name } => self.names.retain(|entry| entry != &name),
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
    chat: String,
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
                    chat: String::new(),
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
                names: Vec::new(),
                error: String::new(),
                chat: String::new(),
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
            chat: String::new(),
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

    pub fn error(&self) -> &str {
        &self.error
    }

    pub fn chat(&self) -> &str {
        &self.chat
    }

    pub fn lan_addr(&self) -> Option<SocketAddr> {
        self.net_server.as_ref().map(|net| net.addr)
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
                self.players = snapshot.players;
                self.names = snapshot.names.clone();
                self.error.clear();
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
                self.chat = format!("{name}: {text}");
            }
            ServerEvent::Error(err) => self.error = err.clone(),
            ServerEvent::Log(_) | ServerEvent::Stopped => {}
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

    #[test]
    fn integrated_server_steps_and_answers_a_ping() {
        let mut settings = LegacySettings::reference();
        settings.game = true;
        settings.integrated = true;
        settings.frames = 2;
        settings.render_distance = 1;
        settings.world = temp_world("server");
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
    fn integrated_and_remote_links_exchange_a_ping() {
        let mut host_settings = LegacySettings::reference();
        host_settings.game = true;
        host_settings.integrated = true;
        host_settings.lan_port = Some(0);
        host_settings.frames = 2;
        host_settings.render_distance = 1;
        host_settings.name = "Host".to_owned();
        host_settings.world = temp_world("host");
        let mut host = Link::open(&host_settings);
        assert_eq!(host.mode(), LinkMode::Integrated);
        let addr = host.lan_addr().expect("LAN port must be bound");

        let mut guest_settings = LegacySettings::reference();
        guest_settings.game = true;
        guest_settings.frames = 2;
        guest_settings.render_distance = 1;
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
    }
}
