//! Plain in-process Nostr relay for tests that prove real NMP delivery of
//! kind:0 (and other self-authenticating events) without an external binary.
//!
//! kind:0 is self-authenticating and the profile feed reads with
//! `CacheMode::Agnostic`, so a plain relay that serves seeded events on REQ,
//! sends EOSE, and then forwards live injections to open subscriptions is
//! enough to prove a retained feed's drain receives and resolves rows through
//! the real NMP engine.

#![cfg(test)]

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nostr::{ClientMessage, Event, EventId, Filter, JsonUtil, RelayMessage, SubscriptionId};
use tungstenite::{Error as WebSocketError, Message};

struct Sub {
    filters: Vec<Filter>,
    tx: mpsc::Sender<Event>,
}

struct State {
    events: BTreeMap<EventId, Event>,
    subs: Vec<(SubscriptionId, Sub)>,
}

/// Plain relay serving seeded + injected events to open subscriptions.
pub(crate) struct PlainRelay {
    url: String,
    state: Arc<Mutex<State>>,
    shutdown: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl PlainRelay {
    /// Spawn the relay seeded with `events`. Each is served to any REQ whose
    /// filters match it; `inject` adds more after spawn.
    pub(crate) fn spawn(events: impl IntoIterator<Item = Event>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind plain relay");
        listener
            .set_nonblocking(true)
            .expect("make plain listener cancellable");
        let port = listener.local_addr().expect("plain relay address").port();
        let url = format!("ws://127.0.0.1:{port}");
        let state = Arc::new(Mutex::new(State {
            events: events.into_iter().map(|e| (e.id, e)).collect(),
            subs: Vec::new(),
        }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let state_cl = Arc::clone(&state);
        let shut_cl = Arc::clone(&shutdown);
        let join = thread::spawn(move || run(listener, state_cl, shut_cl));
        Self {
            url,
            state,
            shutdown,
            join: Mutex::new(Some(join)),
        }
    }

    pub(crate) fn url(&self) -> String {
        self.url.clone()
    }

    /// Insert an event after spawn and forward it to every open subscription
    /// whose filters match it, proving a retained feed's live drain path.
    pub(crate) fn inject(&self, event: Event) {
        let matching = {
            let mut state = self.state.lock().expect("plain relay state poisoned");
            state.events.insert(event.id, event.clone());
            state
                .subs
                .iter()
                .filter(|(_, sub)| {
                    sub.filters
                        .iter()
                        .any(|filter| filter.match_event(&event, Default::default()))
                })
                .map(|(id, sub)| (id.clone(), sub.tx.clone()))
                .collect::<Vec<_>>()
        };
        for (_, tx) in matching {
            let _ = tx.send(event.clone());
        }
    }

    pub(crate) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(join) = self.join.lock().expect("plain relay join poisoned").take() {
            if let Err(panic) = join.join() {
                if !thread::panicking() {
                    std::panic::resume_unwind(panic);
                }
            }
        }
    }
}

impl Drop for PlainRelay {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(listener: TcpListener, state: Arc<Mutex<State>>, shutdown: Arc<AtomicBool>) {
    let mut workers = Vec::new();
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let state = Arc::clone(&state);
                let shutdown = Arc::clone(&shutdown);
                workers.push(thread::spawn(move || serve(stream, &state, &shutdown)));
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) if shutdown.load(Ordering::Acquire) => break,
            Err(error) => panic!("accept plain relay connection: {error}"),
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
}

fn serve(stream: std::net::TcpStream, state: &Arc<Mutex<State>>, shutdown: &Arc<AtomicBool>) {
    let _ = stream.set_nodelay(true);
    let mut ws = match tungstenite::accept(stream) {
        Ok(ws) => ws,
        Err(_) => return,
    };
    ws.get_mut()
        .set_read_timeout(Some(Duration::from_millis(50)))
        .expect("set plain relay read timeout");

    let (tx, rx) = mpsc::channel::<Event>();
    let mut sub_id: Option<SubscriptionId> = None;

    while !shutdown.load(Ordering::Acquire) {
        while let Ok(event) = rx.try_recv() {
            if let Some(id) = sub_id.as_ref() {
                send(&mut ws, RelayMessage::event(id.clone(), event));
            }
        }
        let text = match ws.read() {
            Ok(Message::Text(text)) => text.as_str().to_string(),
            Ok(Message::Close(_))
            | Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => break,
            Ok(_) => continue,
            Err(WebSocketError::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                continue
            }
            Err(_) if shutdown.load(Ordering::Acquire) => break,
            Err(error) => panic!("read plain relay frame: {error}"),
        };
        let message = match ClientMessage::from_json(text) {
            Ok(message) => message,
            Err(_) => continue,
        };
        match message {
            ClientMessage::Req {
                subscription_id,
                filters,
            } => {
                let id = subscription_id.into_owned();
                let filters = filters
                    .into_iter()
                    .map(|f| f.into_owned())
                    .collect::<Vec<_>>();
                let matching = state
                    .lock()
                    .expect("plain relay state poisoned")
                    .events
                    .values()
                    .filter(|event| {
                        filters
                            .iter()
                            .any(|f| f.match_event(event, Default::default()))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if let Some(id) = sub_id.as_ref() {
                    state
                        .lock()
                        .expect("plain relay state poisoned")
                        .subs
                        .retain(|(sid, _)| sid != id);
                }
                state
                    .lock()
                    .expect("plain relay state poisoned")
                    .subs
                    .push((
                        id.clone(),
                        Sub {
                            filters,
                            tx: tx.clone(),
                        },
                    ));
                sub_id = Some(id.clone());
                for event in matching {
                    send(&mut ws, RelayMessage::event(id.clone(), event));
                }
                send(&mut ws, RelayMessage::eose(id));
            }
            ClientMessage::Close(_) => {
                if let Some(id) = sub_id.take() {
                    state
                        .lock()
                        .expect("plain relay state poisoned")
                        .subs
                        .retain(|(sid, _)| sid != &id);
                }
                break;
            }
            // The feed never publishes through this relay; ignore writes and
            // negotiation messages.
            ClientMessage::Event(_)
            | ClientMessage::Auth(_)
            | ClientMessage::NegOpen { .. }
            | ClientMessage::NegMsg { .. }
            | ClientMessage::NegClose { .. }
            | ClientMessage::Count { .. } => {}
        }
    }
}

fn send(ws: &mut tungstenite::WebSocket<std::net::TcpStream>, message: RelayMessage<'_>) {
    let _ = ws.send(Message::text(message.as_json()));
}
