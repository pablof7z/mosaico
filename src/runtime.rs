//! Daemon-hosted per-session engine: publishes identity/status and watches host
//! liveness. Status effects flow
//! exclusively through [`crate::reconcile::status`] and NMP's durable write plane.

use crate::domain::{DomainEvent, Profile};
use crate::fabric::provider::Nip29Provider;
use crate::presence_publisher::{drive, DriveMeta};
use crate::state::{Session, Store};
use crate::util::now_secs;
use anyhow::Result;
use nostr::Keys;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

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
    /// Periodic renewal of the signed remote-observer presence lease.
    pub presence_lease_interval: Duration,
    /// Host-process liveness sampling. This never classifies semantic state.
    pub process_probe_interval: Duration,
}

impl EngineParams {
    /// Keys used to sign this session's live events.
    fn signing_keys(&self) -> Keys {
        self.keys.clone()
    }
}

fn publishes_presence(channel: &str) -> bool {
    !channel.is_empty()
}

// ── daemon-hosted session task (the relocated engine) ────────────────────────

/// Run the per-session engine INSIDE the daemon, using the SHARED relay
/// connection and the SHARED store (single writer). The daemon owns one union
/// subscription and demuxes incoming events centrally; this task only:
///   - publishes the profile once (signed with the agent's keys),
///   - renews the signed kind:30315 presence lease,
///   - watches the host pid and stops the runtime (title retained) on pid
///     death or `cancel` (the `session-end` path).
///
/// Store access goes through the shared `Arc<Mutex<Store>>`; the guard is held
/// only across the synchronous rusqlite calls, NEVER across `.await`.
pub(crate) async fn run_session_in_daemon(
    p: EngineParams,
    provider: std::sync::Arc<Nip29Provider>,
    store: std::sync::Arc<Mutex<Store>>,
    cancel: std::sync::Arc<tokio::sync::Notify>,
    status: std::sync::Arc<Mutex<crate::reconcile::StatusReconciler>>,
    presence_publisher: crate::presence_publisher::PresencePublisher,
) -> Result<()> {
    let owners = p.owners.clone();
    let signing_keys = p.signing_keys();
    let aref = p.identity.agent_ref();
    let publishes_presence = publishes_presence(&p.channel);

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

    // Load the session row, distinguishing a genuine "no such session" (None)
    // from a store error that must not masquerade as an idle session.
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

    macro_rules! drive_status {
        ($trigger:expr, $f:expr) => {
            drive(
                &status,
                &presence_publisher,
                &signing_keys,
                DriveMeta { trigger: $trigger },
                $f,
            )
        };
    }

    if let Err(e) = st!(|s: &Store| s.touch_session(&aref.pubkey, now_secs())) {
        tracing::error!(session = %aref.pubkey, error = %e, "touch_session failed — liveness not bumped at startup");
    }
    if publishes_presence {
        if let Some(session) = load_session("startup-status") {
            let now = now_secs();
            let projection = st!(|s: &Store| crate::session_presence::publication(s, &session));
            drive_status!("session_started", |r| {
                r.open(
                    &aref.pubkey,
                    p.runtime_generation,
                    crate::reconcile::PresenceSnapshot {
                        host: p.host.clone(),
                        slug: aref.slug.clone(),
                        rel_cwd: p.rel_cwd.clone(),
                        dispatch_event: p.dispatch_event.clone(),
                        projection,
                    },
                    now,
                )
            });
        }
    }

    let mut lease = tokio::time::interval(p.presence_lease_interval);
    let mut process_probe = tokio::time::interval(p.process_probe_interval);

    loop {
        tokio::select! {
            _ = lease.tick() => {
                if publishes_presence {
                    let now = now_secs();
                    drive_status!("presence_lease_renewal", |r| {
                        r.renew(&aref.pubkey, p.runtime_generation, now)
                    });
                }
            }
            _ = process_probe.tick() => {
                if let Some(pid) = p.watch_pid {
                    if !crate::liveness::pid_alive(pid) { break; }
                }
                let now = now_secs();
                if let Err(e) = st!(|s: &Store| s.touch_session(&aref.pubkey, now)) {
                    tracing::error!(session = %aref.pubkey, error = %e, "process observation could not be recorded");
                }
            }
            _ = cancel.notified() => { break; }
        }
    }

    let end_now = now_secs();
    if publishes_presence {
        drive_status!("session_ended", |r| {
            r.close(&aref.pubkey, p.runtime_generation, end_now)
        });
    }

    if let Err(e) = st!(|s: &Store| { s.touch_session(&aref.pubkey, end_now) }) {
        tracing::error!(pubkey = %aref.pubkey, error = %e, "final liveness touch failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::publishes_presence;

    #[test]
    fn current_pid_is_alive() {
        assert!(crate::liveness::pid_alive(std::process::id() as i32));
    }

    #[test]
    fn unscoped_sessions_do_not_publish_channel_presence() {
        assert!(!publishes_presence(""));
        assert!(publishes_presence("workspace"));
    }
}
