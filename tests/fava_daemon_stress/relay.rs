use std::collections::BTreeSet;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use nostr::event::Event;
use nostr::message::{ClientMessage, RelayMessage};
use tungstenite::{accept, Error, Message};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RelaySnapshot {
    pub(super) requests: usize,
    pub(super) deliveries: usize,
    pub(super) closes: usize,
    pub(super) active_requests: usize,
    pub(super) connections: usize,
}

#[derive(Default)]
struct State {
    requests: AtomicUsize,
    deliveries: AtomicUsize,
    closes: AtomicUsize,
    connections: AtomicUsize,
    next_connection: AtomicU64,
    active: Mutex<BTreeSet<(u64, String)>>,
    request_frames: Mutex<Vec<String>>,
}

pub(super) struct CountingRelay {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    state: Arc<State>,
    server: Option<JoinHandle<()>>,
    clients: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl CountingRelay {
    pub(super) fn start(events: impl IntoIterator<Item = Event>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind counted relay");
        let address = listener.local_addr().expect("counted relay address");
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(State::default());
        let events = Arc::new(events.into_iter().collect::<Vec<_>>());
        let clients = Arc::new(Mutex::new(Vec::new()));
        let server = {
            let stop = stop.clone();
            let state = state.clone();
            let events = events.clone();
            let clients = clients.clone();
            std::thread::spawn(move || {
                while let Ok((stream, _)) = listener.accept() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let stop = stop.clone();
                    let state = state.clone();
                    let events = events.clone();
                    clients.lock().unwrap().push(std::thread::spawn(move || {
                        serve(stream, stop, state, events);
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
            deliveries: self.state.deliveries.load(Ordering::Acquire),
            closes: self.state.closes.load(Ordering::Acquire),
            active_requests: self.state.active.lock().unwrap().len(),
            connections: self.state.connections.load(Ordering::Acquire),
        }
    }

    pub(super) fn request_frames(&self) -> Vec<String> {
        self.state.request_frames.lock().unwrap().clone()
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

fn serve(stream: TcpStream, stop: Arc<AtomicBool>, state: Arc<State>, events: Arc<Vec<Event>>) {
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
            Message::Text(text) => {
                handle_text(connection, text.as_str(), &state, &events, &mut websocket)
            }
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
    events: &[Event],
    websocket: &mut tungstenite::WebSocket<TcpStream>,
) {
    let Ok(frame) = ClientMessage::from_json(text) else {
        return;
    };
    match frame {
        ClientMessage::Req {
            subscription_id,
            filters,
        } => {
            let sub_id = subscription_id.into_owned();
            let filters = filters
                .into_iter()
                .map(|filter| filter.into_owned())
                .collect::<Vec<_>>();
            state.requests.fetch_add(1, Ordering::AcqRel);
            state.request_frames.lock().unwrap().push(text.to_string());
            state
                .active
                .lock()
                .unwrap()
                .insert((connection, sub_id.to_string()));
            for event in events.iter().filter(|event| {
                filters
                    .iter()
                    .any(|filter| filter.match_event(event, Default::default()))
            }) {
                let delivery = RelayMessage::event(sub_id.clone(), event.clone()).as_json();
                if websocket.send(Message::text(delivery)).is_err() {
                    return;
                }
                state.deliveries.fetch_add(1, Ordering::AcqRel);
            }
            let _ = websocket.send(Message::text(RelayMessage::eose(sub_id).as_json()));
        }
        ClientMessage::Close(sub_id) => {
            state.closes.fetch_add(1, Ordering::AcqRel);
            state
                .active
                .lock()
                .unwrap()
                .remove(&(connection, sub_id.to_string()));
        }
        ClientMessage::Event(_)
        | ClientMessage::Auth(_)
        | ClientMessage::NegOpen { .. }
        | ClientMessage::NegMsg { .. }
        | ClientMessage::NegClose { .. }
        | ClientMessage::Count { .. } => {}
    }
}
