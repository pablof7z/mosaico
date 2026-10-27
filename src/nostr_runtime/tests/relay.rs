use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use nostr::prelude::{ClientMessage, Event, RelayMessage};
use tungstenite::{accept, Error, Message};

#[derive(Clone)]
pub(super) enum AckMode {
    Accept,
    Reject(String),
    Silent,
    Disconnect,
}

/// One subscription frame the relay received, in arrival order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SubscriptionFrame {
    Req(String),
    Close(String),
}

pub(super) struct TestRelay {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    connections: Arc<AtomicUsize>,
    subscriptions: Arc<Mutex<Vec<SubscriptionFrame>>>,
    server: Option<JoinHandle<()>>,
    clients: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl TestRelay {
    pub(super) fn new(mode: AckMode, events: impl IntoIterator<Item = Event>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test relay");
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(AtomicUsize::new(0));
        let clients = Arc::new(Mutex::new(Vec::new()));
        let subscriptions = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(events.into_iter().collect::<Vec<_>>());
        let server = {
            let stop = stop.clone();
            let connections = connections.clone();
            let clients = clients.clone();
            let subscriptions = subscriptions.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let mode = mode.clone();
                            let events = events.clone();
                            let stop = stop.clone();
                            let connections = connections.clone();
                            let subscriptions = subscriptions.clone();
                            clients.lock().unwrap().push(std::thread::spawn(move || {
                                serve(stream, mode, events, stop, connections, subscriptions)
                            }));
                        }
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept test relay: {error}"),
                    }
                }
            })
        };
        Self {
            address,
            stop,
            connections,
            subscriptions,
            server: Some(server),
            clients,
        }
    }

    pub(super) fn url(&self) -> String {
        format!("ws://{}", self.address)
    }

    pub(super) fn connections(&self) -> usize {
        self.connections.load(Ordering::Acquire)
    }

    pub(super) fn subscription_frames(&self) -> Vec<SubscriptionFrame> {
        self.subscriptions.lock().unwrap().clone()
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(server) = self.server.take() {
            server.join().unwrap();
        }
        for client in self.clients.lock().unwrap().drain(..) {
            client.join().unwrap();
        }
    }
}

fn serve(
    stream: TcpStream,
    mode: AckMode,
    events: Arc<Vec<Event>>,
    stop: Arc<AtomicBool>,
    connections: Arc<AtomicUsize>,
    subscriptions: Arc<Mutex<Vec<SubscriptionFrame>>>,
) {
    stream
        .set_read_timeout(Some(Duration::from_millis(40)))
        .ok();
    let Ok(mut socket) = accept(stream) else {
        return;
    };
    connections.fetch_add(1, Ordering::AcqRel);
    while !stop.load(Ordering::Acquire) {
        let text = match socket.read() {
            Ok(Message::Text(text)) => text,
            Ok(Message::Ping(bytes)) => {
                let _ = socket.send(Message::Pong(bytes));
                continue;
            }
            Err(Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                continue;
            }
            _ => break,
        };
        let Ok(message) = ClientMessage::from_json(text.as_str()) else {
            continue;
        };
        match message {
            ClientMessage::Req {
                subscription_id,
                filters,
            } => {
                let id = subscription_id.into_owned();
                subscriptions
                    .lock()
                    .unwrap()
                    .push(SubscriptionFrame::Req(id.to_string()));
                let filters = filters
                    .into_iter()
                    .map(|f| f.into_owned())
                    .collect::<Vec<_>>();
                for event in events.iter().filter(|event| {
                    filters
                        .iter()
                        .any(|filter| filter.match_event(event, Default::default()))
                }) {
                    send(&mut socket, RelayMessage::event(id.clone(), event.clone()));
                }
                send(&mut socket, RelayMessage::eose(id));
            }
            ClientMessage::Event(event) => match &mode {
                AckMode::Accept => send(&mut socket, RelayMessage::ok(event.id, true, "accepted")),
                AckMode::Reject(message) => send(
                    &mut socket,
                    RelayMessage::ok(event.id, false, message.clone()),
                ),
                AckMode::Silent => {}
                AckMode::Disconnect => break,
            },
            ClientMessage::Close(id) => subscriptions
                .lock()
                .unwrap()
                .push(SubscriptionFrame::Close(id.to_string())),
            ClientMessage::Auth(_)
            | ClientMessage::NegOpen { .. }
            | ClientMessage::NegMsg { .. }
            | ClientMessage::NegClose { .. }
            | ClientMessage::Count { .. } => {}
        }
    }
    connections.fetch_sub(1, Ordering::AcqRel);
}

fn send(socket: &mut tungstenite::WebSocket<TcpStream>, message: RelayMessage<'_>) {
    let _ = socket.send(Message::text(message.as_json()));
}
