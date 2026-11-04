//! PR 12: a small Horizon-internal LAN protocol over newline-delimited JSON.
//!
//! This is **not** the Minecraft Java protocol. It carries the same
//! `ServerCommand`/`ServerEvent` values used by the in-process integrated
//! server, so a GL2 client can join another GL2 world on the LAN. No new
//! crates are needed: std TCP plus the client's existing `serde_json`.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{Receiver, RecvTimeoutError, Sender, channel},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::server::{ServerCommand, ServerEvent};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NetMessage {
    Hello { name: String },
    Welcome { name: String },
    Command(ServerCommand),
    Event(ServerEvent),
    Bye,
}

fn write_message(stream: &mut TcpStream, message: &NetMessage) -> io::Result<()> {
    let line = serde_json::to_string(message)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn parse_message(line: &str) -> Option<NetMessage> {
    serde_json::from_str(line.trim()).ok()
}

struct Client {
    id: u64,
    name: String,
    tx: Sender<NetMessage>,
}

pub struct NetServer {
    pub addr: SocketAddr,
    running: Arc<AtomicBool>,
    commands: Option<Sender<ServerCommand>>,
    clients: Arc<Mutex<Vec<Client>>>,
    threads: Vec<JoinHandle<()>>,
}

impl NetServer {
    pub fn start(
        port: u16,
        commands: Sender<ServerCommand>,
        events: Receiver<ServerEvent>,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|err| err.to_string())?;
        let addr = listener.local_addr().map_err(|err| err.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|err| err.to_string())?;
        let running = Arc::new(AtomicBool::new(true));
        let clients = Arc::new(Mutex::new(Vec::<Client>::new()));
        let next_id = Arc::new(AtomicU64::new(1));
        let mut threads = Vec::new();

        {
            let running = running.clone();
            let clients = clients.clone();
            threads.push(
                thread::Builder::new()
                    .name("horizon-legacy-lan-broadcast".to_owned())
                    .spawn(move || {
                        while running.load(Ordering::Relaxed) {
                            match events.recv_timeout(Duration::from_millis(50)) {
                                Ok(event) => {
                                    let message = NetMessage::Event(event);
                                    if let Ok(list) = clients.lock() {
                                        for client in list.iter() {
                                            let _ = client.tx.send(message.clone());
                                        }
                                    }
                                }
                                Err(RecvTimeoutError::Timeout) => {}
                                Err(RecvTimeoutError::Disconnected) => break,
                            }
                        }
                    })
                    .map_err(|err| err.to_string())?,
            );
        }

        {
            let running = running.clone();
            let clients = clients.clone();
            let accept_commands = commands.clone();
            threads.push(
                thread::Builder::new()
                    .name("horizon-legacy-lan-accept".to_owned())
                    .spawn(move || {
                        while running.load(Ordering::Relaxed) {
                            match listener.accept() {
                                Ok((stream, peer)) => {
                                    let _ = stream.set_nodelay(true);
                                    let _ =
                                        stream.set_read_timeout(Some(Duration::from_millis(50)));
                                    let Some(writer_stream) = stream.try_clone().ok() else {
                                        continue;
                                    };
                                    let id = next_id.fetch_add(1, Ordering::Relaxed);
                                    let default_name = format!("guest-{id}-{}", peer.port());
                                    let (tx, rx) = channel::<NetMessage>();
                                    {
                                        let running = running.clone();
                                        thread::spawn(move || {
                                            let mut writer = writer_stream;
                                            while running.load(Ordering::Relaxed) {
                                                match rx.recv_timeout(Duration::from_millis(50)) {
                                                    Ok(message) => {
                                                        if write_message(&mut writer, &message)
                                                            .is_err()
                                                        {
                                                            break;
                                                        }
                                                    }
                                                    Err(RecvTimeoutError::Timeout) => {}
                                                    Err(RecvTimeoutError::Disconnected) => break,
                                                }
                                            }
                                        });
                                    }
                                    if let Ok(mut list) = clients.lock() {
                                        list.push(Client {
                                            id,
                                            name: default_name.clone(),
                                            tx: tx.clone(),
                                        });
                                    }
                                    let reader_commands = accept_commands.clone();
                                    let reader_clients = clients.clone();
                                    let reader_running = running.clone();
                                    thread::spawn(move || {
                                        let mut reader = BufReader::new(stream);
                                        let mut name = default_name;
                                        while reader_running.load(Ordering::Relaxed) {
                                            let mut line = String::new();
                                            match reader.read_line(&mut line) {
                                                Ok(0) => break,
                                                Ok(_) => {
                                                    if let Some(message) = parse_message(&line) {
                                                        match message {
                                                            NetMessage::Hello { name: hello } => {
                                                                let mut assigned = hello;
                                                                if let Ok(mut list) =
                                                                    reader_clients.lock()
                                                                {
                                                                    if list.iter().any(|client| {
                                                                        client.name == assigned
                                                                    }) {
                                                                        assigned = format!(
                                                                            "{assigned}-{}",
                                                                            peer.port()
                                                                        );
                                                                    }
                                                                    if let Some(client) = list
                                                                        .iter_mut()
                                                                        .find(|client| {
                                                                            client.id == id
                                                                        })
                                                                    {
                                                                        client.name =
                                                                            assigned.clone();
                                                                    }
                                                                }
                                                                name = assigned.clone();
                                                                let _ =
                                                                    tx.send(NetMessage::Welcome {
                                                                        name: assigned.clone(),
                                                                    });
                                                                let _ = reader_commands.send(
                                                                    ServerCommand::Join {
                                                                        name: assigned,
                                                                    },
                                                                );
                                                            }
                                                            NetMessage::Command(command) => {
                                                                let forwarded = match command {
                                                                    ServerCommand::Client {
                                                                        command,
                                                                        ..
                                                                    } => Some(
                                                                        ServerCommand::Client {
                                                                            name: name.clone(),
                                                                            command,
                                                                        },
                                                                    ),
                                                                    ServerCommand::Ping(id) => {
                                                                        Some(ServerCommand::Ping(
                                                                            id,
                                                                        ))
                                                                    }
                                                                    ServerCommand::Leave {
                                                                        ..
                                                                    } => {
                                                                        Some(ServerCommand::Leave {
                                                                            name: name.clone(),
                                                                        })
                                                                    }
                                                                    _ => None,
                                                                };
                                                                if let Some(command) = forwarded {
                                                                    let _ = reader_commands
                                                                        .send(command);
                                                                }
                                                            }
                                                            NetMessage::Welcome { .. }
                                                            | NetMessage::Event(_) => {}
                                                            NetMessage::Bye => break,
                                                        }
                                                    }
                                                }
                                                Err(err)
                                                    if matches!(
                                                        err.kind(),
                                                        io::ErrorKind::WouldBlock
                                                            | io::ErrorKind::TimedOut
                                                    ) => {}
                                                Err(_) => break,
                                            }
                                        }
                                        if let Ok(mut list) = reader_clients.lock() {
                                            list.retain(|client| client.id != id);
                                        }
                                        let _ = reader_commands.send(ServerCommand::Leave { name });
                                    });
                                }
                                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                                    thread::sleep(Duration::from_millis(10));
                                }
                                Err(_) => break,
                            }
                        }
                    })
                    .map_err(|err| err.to_string())?,
            );
        }

        Ok(Self {
            addr,
            running,
            commands: Some(commands),
            clients,
            threads,
        })
    }

    pub fn shutdown(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        self.commands.take();
        if let Ok(mut list) = self.clients.lock() {
            list.clear();
        }
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

impl Drop for NetServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub struct NetClient {
    commands: Option<Sender<ServerCommand>>,
    net_tx: Option<Sender<NetMessage>>,
    running: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl NetClient {
    pub fn connect(addr: &str, name: &str) -> Result<(Self, Receiver<ServerEvent>), String> {
        let target = addr
            .to_socket_addrs()
            .map_err(|err| err.to_string())?
            .next()
            .ok_or_else(|| "no address".to_owned())?;
        let stream = TcpStream::connect_timeout(&target, Duration::from_secs(2))
            .map_err(|err| err.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|err| err.to_string())?;
        stream.set_nodelay(true).map_err(|err| err.to_string())?;
        let writer_stream = stream.try_clone().map_err(|err| err.to_string())?;

        let (command_tx, command_rx) = channel::<ServerCommand>();
        let (event_tx, event_rx) = channel::<ServerEvent>();
        let (net_tx, net_rx) = channel::<NetMessage>();
        net_tx
            .send(NetMessage::Hello {
                name: name.to_owned(),
            })
            .map_err(|err| err.to_string())?;
        let running = Arc::new(AtomicBool::new(true));
        let mut threads = Vec::new();

        {
            let running = running.clone();
            threads.push(
                thread::Builder::new()
                    .name("horizon-legacy-lan-writer".to_owned())
                    .spawn(move || {
                        let mut writer = writer_stream;
                        while running.load(Ordering::Relaxed) {
                            match net_rx.recv_timeout(Duration::from_millis(50)) {
                                Ok(message) => {
                                    if write_message(&mut writer, &message).is_err() {
                                        break;
                                    }
                                }
                                Err(RecvTimeoutError::Timeout) => {}
                                Err(RecvTimeoutError::Disconnected) => break,
                            }
                        }
                    })
                    .map_err(|err| err.to_string())?,
            );
        }

        {
            let running = running.clone();
            let command_net_tx = net_tx.clone();
            threads.push(
                thread::Builder::new()
                    .name("horizon-legacy-lan-commands".to_owned())
                    .spawn(move || {
                        while running.load(Ordering::Relaxed) {
                            match command_rx.recv_timeout(Duration::from_millis(50)) {
                                Ok(command) => {
                                    if command_net_tx.send(NetMessage::Command(command)).is_err() {
                                        break;
                                    }
                                }
                                Err(RecvTimeoutError::Timeout) => {}
                                Err(RecvTimeoutError::Disconnected) => break,
                            }
                        }
                    })
                    .map_err(|err| err.to_string())?,
            );
        }

        {
            let running = running.clone();
            let reader_event_tx = event_tx.clone();
            threads.push(
                thread::Builder::new()
                    .name("horizon-legacy-lan-reader".to_owned())
                    .spawn(move || {
                        let mut reader = BufReader::new(stream);
                        while running.load(Ordering::Relaxed) {
                            let mut line = String::new();
                            match reader.read_line(&mut line) {
                                Ok(0) => break,
                                Ok(_) => {
                                    if let Some(message) = parse_message(&line) {
                                        match message {
                                            NetMessage::Welcome { name } => {
                                                let _ = reader_event_tx
                                                    .send(ServerEvent::Welcome { name });
                                            }
                                            NetMessage::Event(event) => {
                                                let _ = reader_event_tx.send(event);
                                            }
                                            NetMessage::Bye => break,
                                            NetMessage::Hello { .. } | NetMessage::Command(_) => {}
                                        }
                                    }
                                }
                                Err(err)
                                    if matches!(
                                        err.kind(),
                                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                                    ) => {}
                                Err(_) => break,
                            }
                        }
                    })
                    .map_err(|err| err.to_string())?,
            );
        }

        drop(event_tx);
        Ok((
            Self {
                commands: Some(command_tx),
                net_tx: Some(net_tx),
                running,
                threads,
            },
            event_rx,
        ))
    }

    pub fn send(&self, command: ServerCommand) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|tx| tx.send(command).is_ok())
    }

    pub fn shutdown(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(tx) = self.net_tx.as_ref() {
            let _ = tx.send(NetMessage::Bye);
        }
        self.net_tx.take();
        self.commands.take();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

impl Drop for NetClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::super::server::ClientCommand;
    use super::*;
    use std::time::Instant;

    #[test]
    fn messages_round_trip_as_one_json_line() {
        let message = NetMessage::Command(ServerCommand::Client {
            name: "Tester".to_owned(),
            command: ClientCommand::Chat("hello".to_owned()),
        });
        let line = serde_json::to_string(&message).unwrap();
        assert_eq!(parse_message(&line), Some(message));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn a_client_reads_a_loopback_welcome_and_event() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader_stream = stream.try_clone().unwrap();
            reader_stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(reader_stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(matches!(
                parse_message(&line),
                Some(NetMessage::Hello { .. })
            ));
            let mut writer = stream;
            write_message(
                &mut writer,
                &NetMessage::Welcome {
                    name: "loopback".to_owned(),
                },
            )
            .unwrap();
            write_message(&mut writer, &NetMessage::Event(ServerEvent::Pong(42))).unwrap();
        });

        let (mut client, events) = NetClient::connect(&addr.to_string(), "Tester").unwrap();
        let mut saw_welcome = false;
        let mut saw_pong = false;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && !(saw_welcome && saw_pong) {
            if let Ok(event) = events.recv_timeout(Duration::from_millis(50)) {
                match event {
                    ServerEvent::Welcome { .. } => saw_welcome = true,
                    ServerEvent::Pong(42) => saw_pong = true,
                    _ => {}
                }
            }
        }
        assert!(saw_welcome, "the LAN client must decode a welcome");
        assert!(saw_pong, "the LAN client must decode a server event");
        client.shutdown();
        server.join().unwrap();
    }
}
