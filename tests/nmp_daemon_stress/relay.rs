use std::collections::BTreeSet;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tungstenite::{accept, Error, Message};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RelaySnapshot {
    pub(super) requests: usize,
    pub(super) closes: usize,
    pub(super) active_requests: usize,
    pub(super) connections: usize,
}

#[derive(Default)]
struct State {
    requests: AtomicUsize,
    closes: AtomicUsize,
    connections: AtomicUsize,
    next_connection: AtomicU64,
    active: Mutex<BTreeSet<(u64, String)>>,
}

pub(super) struct CountingRelay {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    state: Arc<State>,
    server: Option<JoinHandle<()>>,
    clients: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl CountingRelay {
    pub(super) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind counted relay");
        let address = listener.local_addr().expect("counted relay address");
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(State::default());
        let clients = Arc::new(Mutex::new(Vec::new()));
        let server = {
            let stop = stop.clone();
            let state = state.clone();
            let clients = clients.clone();
            std::thread::spawn(move || {
                while let Ok((stream, _)) = listener.accept() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let stop = stop.clone();
                    let state = state.clone();
                    clients.lock().unwrap().push(std::thread::spawn(move || {
                        serve(stream, stop, state);
                    }));
                }
            })
        };
        Self {
            address,
            stop,
            state,
            server: Some(server),
            clients,
        }
    }

    pub(super) fn url(&self) -> String {
        format!("ws://{}", self.address)
    }

    pub(super) fn snapshot(&self) -> RelaySnapshot {
        RelaySnapshot {
            requests: self.state.requests.load(Ordering::Acquire),
            closes: self.state.closes.load(Ordering::Acquire),
            active_requests: self.state.active.lock().unwrap().len(),
            connections: self.state.connections.load(Ordering::Acquire),
        }
    }
}

impl Drop for CountingRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(server) = self.server.take() {
            server.join().expect("counted relay server joins");
        }
        for client in self.clients.lock().unwrap().drain(..) {
            client.join().expect("counted relay client joins");
        }
    }
}

fn serve(stream: TcpStream, stop: Arc<AtomicBool>, state: Arc<State>) {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set counted relay read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .expect("set counted relay write timeout");
    let Ok(mut websocket) = accept(stream) else {
        return; // NIP-11 HTTP discovery reaches the same ephemeral listener.
    };
    let connection = state.next_connection.fetch_add(1, Ordering::AcqRel) + 1;
    state.connections.fetch_add(1, Ordering::AcqRel);
    while !stop.load(Ordering::Acquire) {
        let message = match websocket.read() {
            Ok(message) => message,
            Err(Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(Error::ConnectionClosed | Error::AlreadyClosed) => break,
            Err(_) => break,
        };
        match message {
            Message::Ping(bytes) => {
                if websocket.send(Message::Pong(bytes)).is_err() {
                    break;
                }
            }
            Message::Text(text) => handle_text(connection, text.as_str(), &state, &mut websocket),
            Message::Close(_) => break,
            _ => {}
        }
    }
    state
        .active
        .lock()
        .unwrap()
        .retain(|(owner, _)| *owner != connection);
    state.connections.fetch_sub(1, Ordering::AcqRel);
}

fn handle_text(
    connection: u64,
    text: &str,
    state: &State,
    websocket: &mut tungstenite::WebSocket<TcpStream>,
) {
    let Ok(frame) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let Some(parts) = frame.as_array() else {
        return;
    };
    let Some(kind) = parts.first().and_then(serde_json::Value::as_str) else {
        return;
    };
    let Some(sub_id) = parts.get(1).and_then(serde_json::Value::as_str) else {
        return;
    };
    match kind {
        "REQ" => {
            state.requests.fetch_add(1, Ordering::AcqRel);
            state
                .active
                .lock()
                .unwrap()
                .insert((connection, sub_id.to_string()));
            let eose = serde_json::json!(["EOSE", sub_id]).to_string();
            let _ = websocket.send(Message::Text(eose.into()));
        }
        "CLOSE" => {
            state.closes.fetch_add(1, Ordering::AcqRel);
            state
                .active
                .lock()
                .unwrap()
                .remove(&(connection, sub_id.to_string()));
        }
        _ => {}
    }
}
