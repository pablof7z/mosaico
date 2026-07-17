//! Daemon-hosted per-session engine: publishes identity/status and watches host
//! liveness. Status effects flow
//! exclusively through [`crate::reconcile::status`] and NMP's durable write plane.

use crate::domain::{DomainEvent, Profile};
use crate::fabric::provider::Nip29Provider;
use crate::state::{Session, Store};
use crate::status_seam::{drive, DriveMeta};
use crate::util::now_secs;
use anyhow::Result;
use nostr_sdk::prelude::Keys;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

mod session_status;

pub struct EngineParams {
    /// The session's read-side identity: selected pubkey, slug, and display name.
    /// Every live publish derives its wire identity from this.
    pub identity: crate::identity::SessionIdentity,
    /// The keypair selected for this session: derived or durable-agent config.
    pub keys: Keys,
    pub channel: String,
    /// Top-level workspace channel containing `channel`.
    pub workspace: String,
    pub runtime_generation: u64,
    pub host: String,
    /// Channel-relative working directory (§8e), advertised on presence/status.
    pub rel_cwd: String,
    pub dispatch_event: Option<String>,
    /// The human owner pubkey(s) — p-tagged on our profile + presence.
    pub owners: Vec<String>,
    pub relays: Vec<String>,
    pub watch_pid: Option<i32>,
    pub store_path: PathBuf,
    pub heartbeat: Duration,
    /// How often the engine polls session state for turn and channel changes.
    pub obs_interval: Duration,
}

impl EngineParams {
    /// Keys used to sign this session's live events.
    fn signing_keys(&self) -> Keys {
        self.keys.clone()
    }
}

fn status_channels(p: &EngineParams, store: &Mutex<Store>, session: &Session) -> Vec<String> {
    let mut channels = match store.lock() {
        Ok(g) => g
            .list_session_joined_channels(&session.pubkey)
            .unwrap_or_default()
            .into_iter()
            .map(|(channel, _)| channel)
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    if !session.channel_h.is_empty() && !channels.iter().any(|c| c == &session.channel_h) {
        channels.push(session.channel_h.clone());
    }
    if channels.is_empty() && !p.channel.is_empty() {
        channels.push(p.channel.clone());
    }
    channels.sort();
    channels.dedup();
    if let Ok(g) = store.lock() {
        channels.retain(|channel| !g.is_archived_channel(channel).unwrap_or(false));
    }
    channels
}

/// A session's joined-channel set (archived excluded) — the canonical h-tag input.
fn channel_set(
    p: &EngineParams,
    store: &Mutex<Store>,
    session: &Session,
) -> std::collections::BTreeSet<String> {
    status_channels(p, store, session).into_iter().collect()
}

// ── daemon-hosted session task (the relocated engine) ────────────────────────

/// Run the per-session engine INSIDE the daemon, using the SHARED relay
/// connection and the SHARED store (single writer). The daemon owns one union
/// subscription and demuxes incoming events centrally; this task only:
///   - publishes the profile once (signed with the agent's keys),
///   - heartbeats `last_seen` and submits a re-armed kind:30315 to NMP,
///   - watches the host pid and marks the session dead (title RETAINED) on pid
///     death or `cancel` (the `session-end` path).
///
/// Store access goes through the shared `Arc<Mutex<Store>>`; the guard is held
/// only across the synchronous rusqlite calls, NEVER across `.await`.
pub async fn run_session_in_daemon(
    p: EngineParams,
    provider: std::sync::Arc<Nip29Provider>,
    store: std::sync::Arc<Mutex<Store>>,
    cancel: std::sync::Arc<tokio::sync::Notify>,
    status: std::sync::Arc<Mutex<crate::reconcile::StatusReconciler>>,
) -> Result<()> {
    let owners = p.owners.clone();
    let signing_keys = p.signing_keys();
    let aref = p.identity.agent_ref();

    macro_rules! st {
        ($f:expr) => {{
            let g = store.lock().expect("store mutex poisoned");
            #[allow(clippy::redundant_closure_call)]
            ($f)(&*g)
        }};
    }

    let publish_de = |ev: DomainEvent| {
        let provider = provider.clone();
        let keys = signing_keys.clone();
        async move {
            if let Err(e) = provider.enqueue(&ev, &keys).await {
                tracing::error!(error = %format!("{e:#}"), "run_session_in_daemon: domain-event publish failed");
            }
        }
    };

    // Load the session row, distinguishing a genuine "no such session" (None) from
    // a store error (loud): a swallowed Err here silently skips the heartbeat
    // cycle that depends on the row, masking DB corruption as an idle session.
    let load_session = |label: &str| -> Option<Session> {
        match st!(|s: &Store| s.get_session(&aref.pubkey)) {
            Ok(row) => row,
            Err(e) => {
                tracing::error!(session = %aref.pubkey, error = %e, "{label}: get_session failed — skipping this cycle");
                None
            }
        }
    };

    // Publish identity card signed with this session's own ordinal key.
    let profile = Profile::agent(
        aref.clone(),
        p.identity.slug.clone(),
        p.host.clone(),
        owners.clone(),
    )
    .with_workspace(p.workspace.clone());
    publish_de(DomainEvent::Profile(profile)).await;

    let mut prev_working = false;
    macro_rules! drive_status {
        ($trigger:expr, $f:expr) => {
            drive(
                &status,
                &provider,
                &signing_keys,
                &store,
                DriveMeta { trigger: $trigger },
                $f,
            )
            .await
        };
    }

    if let Err(e) = st!(|s: &Store| s.touch_session(&aref.pubkey, now_secs())) {
        tracing::error!(session = %aref.pubkey, error = %e, "touch_session failed — liveness not bumped at startup");
    }
    if let Some(session) = load_session("startup-status") {
        let now = now_secs();
        let chans = channel_set(&p, &store, &session);
        let automatic_delivery = session_status::automatic_delivery(&store, Some(&session));
        drive_status!("session_started", |r| {
            r.on_session_started_with_dispatch(
                &aref.pubkey,
                &p.host,
                &aref.slug,
                &p.rel_cwd,
                chans,
                session.working,
                automatic_delivery,
                &session.title,
                p.dispatch_event.clone(),
                now,
            )
        });
    }

    let mut hb = tokio::time::interval(p.heartbeat);
    let mut obs = tokio::time::interval(p.obs_interval);

    loop {
        tokio::select! {
            _ = hb.tick() => {
                if let Some(pid) = p.watch_pid {
                    if !crate::liveness::pid_alive(pid) { break; }
                }
                let now = now_secs();
                if let Err(e) = st!(|s: &Store| s.touch_session(&aref.pubkey, now)) {
                    tracing::error!(session = %aref.pubkey, error = %e, "touch_session failed — liveness not re-armed this beat");
                }
                let session = load_session("heartbeat");
                let automatic_delivery = session_status::automatic_delivery(&store, session.as_ref());
                drive_status!(
                    "tick",
                    |r| r.on_tick(&aref.pubkey, automatic_delivery, now)
                );
            }
            _ = obs.tick() => {
                let now = now_secs();
                let session = load_session("observe-tick");
                let working = session
                    .as_ref()
                    .is_some_and(|s| s.working);

                if working != prev_working {
                    drive_status!("turn_edge", |r| {
                        if working { r.on_turn_start(&aref.pubkey, now) } else { r.on_turn_end(&aref.pubkey, now) }
                    });
                }
                if let Some(chans) = session.as_ref().map(|s| channel_set(&p, &store, s)) {
                    drive_status!("channels_changed", |r| {
                        r.on_channels_changed(&aref.pubkey, chans, now)
                    });
                }
                prev_working = working;
            }
            _ = cancel.notified() => { break; }
        }
    }

    let end_now = now_secs();
    drive_status!("session_ended", |r| r
        .on_session_ended(&aref.pubkey, end_now));

    if let Err(e) = st!(|s: &Store| { s.touch_session(&aref.pubkey, end_now) }) {
        tracing::error!(pubkey = %aref.pubkey, error = %e, "final liveness touch failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn current_pid_is_alive() {
        assert!(crate::liveness::pid_alive(std::process::id() as i32));
    }
}
