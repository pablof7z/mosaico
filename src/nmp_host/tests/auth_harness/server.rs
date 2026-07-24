//! Blocking WebSocket server loop for the strict AUTH harness.

use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use nostr::{ClientMessage, JsonUtil, Kind, PublicKey, RelayMessage, RelayUrl};
use tungstenite::{Error as WebSocketError, Message};

use super::protocol::{send, validate_auth_event};
use super::AuthRelayState;

pub(super) fn run_relay(
    listener: TcpListener,
    relay: RelayUrl,
    allowed: BTreeSet<PublicKey>,
    state: Arc<Mutex<AuthRelayState>>,
    shutdown: Arc<AtomicBool>,
) {
    let mut workers = Vec::new();
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let relay = relay.clone();
                let allowed = allowed.clone();
                let state = Arc::clone(&state);
                let shutdown = Arc::clone(&shutdown);
                workers.push(thread::spawn(move || {
                    serve_socket(stream, &relay, &allowed, &state, &shutdown);
                }));
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) if shutdown.load(Ordering::Acquire) => break,
            Err(error) => panic!("accept AUTH relay connection: {error}"),
        }
    }
    for worker in workers {
        worker.join().expect("join AUTH relay connection");
    }
}

fn serve_socket(
    stream: std::net::TcpStream,
    relay: &RelayUrl,
    allowed: &BTreeSet<PublicKey>,
    state: &Arc<Mutex<AuthRelayState>>,
    shutdown: &Arc<AtomicBool>,
) {
    let _ = stream.set_nodelay(true);
    let mut ws = match tungstenite::accept(stream) {
        Ok(ws) => ws,
        Err(_) => return,
    };
    ws.get_mut()
        .set_read_timeout(Some(Duration::from_millis(50)))
        .expect("set AUTH relay read timeout");
    let connection = {
        let mut state = state.lock().unwrap_or_else(|poison| poison.into_inner());
        state.connections += 1;
        state.connections
    };
    let challenge = format!("mosaico-nip42-{connection}");
    send(&mut ws, RelayMessage::auth(challenge.clone()));

    let mut authenticated = None;
    while !shutdown.load(Ordering::Acquire) {
        let text = match ws.read() {
            Ok(Message::Text(text)) => text.as_str().to_string(),
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(WebSocketError::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                continue;
            }
            Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => break,
            Err(_) if shutdown.load(Ordering::Acquire) => break,
            Err(error) => panic!("read AUTH relay frame: {error}"),
        };
        let message = ClientMessage::from_json(text).expect("parse client wire frame");
        if authenticated.is_none() {
            handle_unauthenticated(
                message,
                &mut ws,
                allowed,
                relay,
                &challenge,
                state,
                &mut authenticated,
            );
            continue;
        }
        handle_authenticated(message, &mut ws, state, authenticated.unwrap());
    }
}

fn handle_unauthenticated(
    message: ClientMessage<'_>,
    ws: &mut tungstenite::WebSocket<std::net::TcpStream>,
    allowed: &BTreeSet<PublicKey>,
    relay: &RelayUrl,
    challenge: &str,
    state: &Arc<Mutex<AuthRelayState>>,
    authenticated: &mut Option<PublicKey>,
) {
    match message {
        ClientMessage::Auth(event) => {
            let event = event.into_owned();
            match validate_auth_event(&event, allowed, relay, challenge) {
                Ok(()) => {
                    *authenticated = Some(event.pubkey);
                    state
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .observation
                        .auth_events
                        .push(event.clone());
                    send(ws, RelayMessage::ok(event.id, true, "authenticated"));
                }
                Err(reason) => {
                    state
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .observation
                        .invalid_auth
                        .push(reason.clone());
                    send(
                        ws,
                        RelayMessage::ok(event.id, false, format!("auth-required: {reason}")),
                    );
                }
            }
        }
        ClientMessage::Req {
            subscription_id, ..
        } => {
            state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .observation
                .pre_auth_reqs += 1;
            send(
                ws,
                RelayMessage::closed(
                    subscription_id.into_owned(),
                    "auth-required: authenticate before REQ",
                ),
            );
        }
        ClientMessage::Event(event) => {
            let event = event.into_owned();
            state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .observation
                .pre_auth_events += 1;
            send(
                ws,
                RelayMessage::ok(event.id, false, "auth-required: authenticate before EVENT"),
            );
        }
        ClientMessage::Close(_) | ClientMessage::NegMsg { .. } | ClientMessage::NegClose { .. } => {
        }
        ClientMessage::NegOpen { .. } | ClientMessage::Count { .. } => {
            state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .observation
                .pre_auth_reqs += 1;
        }
    }
}

fn handle_authenticated(
    message: ClientMessage<'_>,
    ws: &mut tungstenite::WebSocket<std::net::TcpStream>,
    state: &Arc<Mutex<AuthRelayState>>,
    identity: PublicKey,
) {
    match message {
        ClientMessage::Req {
            subscription_id,
            filters,
        } => {
            let id = subscription_id.into_owned();
            let filters = filters
                .into_iter()
                .map(|filter| filter.into_owned())
                .collect::<Vec<_>>();
            let matching = state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .events
                .values()
                .filter(|event| {
                    filters
                        .iter()
                        .any(|filter| filter.match_event(event, Default::default()))
                })
                .cloned()
                .collect::<Vec<_>>();
            state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .observation
                .authenticated_reqs
                .push((identity, filters));
            for event in matching {
                send(ws, RelayMessage::event(id.clone(), event));
            }
            send(ws, RelayMessage::eose(id));
        }
        ClientMessage::Event(event) => {
            let event = event.into_owned();
            assert_ne!(event.kind, Kind::Authentication);
            assert_eq!(
                event.pubkey, identity,
                "ordinary EVENT author must match its authenticated identity"
            );
            event.verify().expect("ordinary relay EVENT must verify");
            {
                let mut state = state.lock().unwrap_or_else(|poison| poison.into_inner());
                state.observation.ordinary_events.push(event.clone());
                state.events.insert(event.id, event.clone());
            }
            send(ws, RelayMessage::ok(event.id, true, "saved"));
        }
        ClientMessage::Auth(event) => {
            let event = event.into_owned();
            send(
                ws,
                RelayMessage::ok(event.id, false, "auth-required: already authenticated"),
            );
        }
        ClientMessage::Count {
            subscription_id, ..
        } => send(
            ws,
            RelayMessage::closed(subscription_id.into_owned(), "restricted: unsupported"),
        ),
        ClientMessage::Close(_)
        | ClientMessage::NegOpen { .. }
        | ClientMessage::NegMsg { .. }
        | ClientMessage::NegClose { .. } => {}
    }
}
