//! The per-session engine (M1 §5, §7).
//!
//! Runs as a daemon-hosted task. It is a STATELESS driver over the canonical
//! `session_state` aggregate (the single source of truth): it holds no cached
//! title/activity and never builds a `Status`. It:
//!   - publishes the agent's `kind:0` profile once,
//!   - refreshes liveness each heartbeat (`heartbeat_session` bumps `last_seen`,
//!     whose freshness the kind:30315 NIP-40 expiration encodes) — no version
//!     bump, no outbox; the relay re-arm is the drainer/heartbeat-publisher's job,
//!   - drives turn transitions (`start_turn`/`seed_title_if_empty`/`end_turn`) on
//!     the rising/falling edges of `turn_state`, and schedules background
//!     distillation that applies via `apply_distill_result` under a versioned
//!     guard (a stale distill structurally no-ops),
//!   - broadcasts a social Activity note when a distill changes the title,
//!   - watches the host PID and finishes the session (`end_session`, title
//!     retained) when it dies or on `cancel` (the `session-end` path). Status
//!     publication is entirely the outbox drainer's responsibility.

use crate::distill;
use crate::domain::{Activity, AgentRef, DomainEvent, Profile};
use crate::fabric::provider::Kind1Nip29Provider;
use crate::state::Store;
use crate::util::now_secs;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;

pub struct EngineParams {
    pub agent_slug: String,
    pub agent_pubkey: String,
    pub keys: nostr_sdk::prelude::Keys,
    /// Stage 3 (Issue #2): derived per-session keypair. When `Some`, live
    /// events emitted by this session (kind:0 profile, kind:30315 status,
    /// kind:1 messages) are signed with this key so the session pubkey is the
    /// on-wire author identity. Falls back to `keys` (durable agent key) when
    /// `None` (operator nsec absent / derivation skipped).
    pub session_keys: Option<nostr_sdk::prelude::Keys>,
    pub project: String,
    pub session_id: String,
    pub host: String,
    /// Project-relative working directory (§8e), advertised on presence/status.
    pub rel_cwd: String,
    /// The human owner pubkey(s) — p-tagged on our profile + presence.
    pub owners: Vec<String>,
    pub relays: Vec<String>,
    pub watch_pid: Option<i32>,
    pub store_path: PathBuf,
    pub heartbeat: Duration,
    /// How often the engine polls turn state to decide whether to distill.
    pub obs_interval: Duration,
    pub status_ttl: Duration,
    /// Delay from turn-start to the first title distillation (default 30s) —
    /// short turns that finish before this never cost an LLM call. The title is
    /// re-distilled at each new turn (new user message) with the current title
    /// fed back, so it stays stable unless the work substantively changes.
    pub turn_first: Duration,
    /// Safety re-distillation interval WITHIN a single long-running turn that has
    /// no new user message (default 0 = disabled). Cheap thanks to nudge-to-keep.
    pub turn_repeat: Duration,
}

// ── daemon-hosted session task (the relocated engine) ────────────────────────

/// Run the per-session engine INSIDE the daemon, using the SHARED relay
/// connection and the SHARED store (single writer). Unlike `run_session`, this
/// does NOT open its own store/transport and does NOT subscribe or demux: the
/// daemon owns one union subscription and demuxes incoming events centrally,
/// routing mentions to the right agent's inbox. This task only:
///   - publishes profile once + an initial Status (signed with the agent's keys),
///   - heartbeats the per-session Status (refreshing the store's `last_seen`,
///     which is what tracks liveness — the event itself never expires),
///   - distills turn activity → Activity + Status,
///   - watches the host pid and exits cleanly (idle, title RETAINED) on pid
///     death or on `cancel` (the `session-end` path) — a finished session keeps
///     its title.
///
/// Store access goes through the shared `Arc<Mutex<Store>>`; the guard is held
/// only across the synchronous rusqlite calls, NEVER across `.await`.
pub async fn run_session_in_daemon(
    p: EngineParams,
    provider: std::sync::Arc<Kind1Nip29Provider>,
    store: std::sync::Arc<std::sync::Mutex<Store>>,
    cancel: std::sync::Arc<tokio::sync::Notify>,
) -> Result<()> {
    let me = p.agent_pubkey.clone();
    let keys = p.keys.clone();
    let aref = AgentRef::new(me.clone(), p.agent_slug.clone());
    let owners = p.owners.clone();

    macro_rules! st {
        ($f:expr) => {{
            let g = store.lock().expect("store mutex poisoned");
            #[allow(clippy::redundant_closure_call)]
            ($f)(&*g)
        }};
    }

    let publish_de = |ev: DomainEvent| {
        let provider = provider.clone();
        let keys = keys.clone();
        async move {
            let _ = provider.publish(&ev, &keys).await;
        }
    };

    // Identity card (the one publish the engine still owns; status publication is
    // the outbox drainer's job).
    publish_de(DomainEvent::Profile(Profile {
        agent: aref.clone(),
        host: p.host.clone(),
        owners: owners.clone(),
    }))
    .await;

    // Stage 3 (Issue #2): also publish a session-keyed kind:0 so peers can
    // resolve the session pubkey to a display name ("<codename> (<agent_slug>)")
    // via the profiles table. Signed with the session key so the event pubkey
    // equals the session pubkey. The durable kind:0 above is NOT removed —
    // both identities coexist, with the session key active for live events.
    if let Some(ref sk) = p.session_keys {
        // Canonical session display name: `codename (agent@host)`.
        let session_display = crate::idref::session_label(&p.session_id, &p.agent_slug, &p.host);
        let session_aref = AgentRef::new(sk.public_key().to_hex(), session_display);
        let _ = provider
            .publish(
                &DomainEvent::Profile(Profile {
                    agent: session_aref,
                    host: p.host.clone(),
                    owners: owners.clone(),
                }),
                sk,
            )
            .await;
    }

    // This loop is a STATELESS driver: it holds NO cur_title/cur_activity and
    // never builds a `Status` nor writes any status table. The canonical
    // `session_state` row is the single source of truth; the engine only applies
    // transitions (start_turn/seed_title_if_empty/apply_distill_result/end_turn/
    // end_session) and refreshes liveness (heartbeat_session). Publication of the
    // resulting kind:30315 is the outbox drainer's responsibility.
    let turn_first = p.turn_first.as_secs();
    let turn_repeat = p.turn_repeat.as_secs();

    // The ONLY locals are scheduling bookkeeping (not session status):
    //   - the in-flight distill task + the (turn_id, state_version) it was based on
    //     so its result is applied with the versioned guard (stale → store no-ops),
    //   - last_distill_attempt: wall-clock retry gate (success time lives in the
    //     store's last_distill_at),
    //   - cur_turn_started / prev_working: edge detection against turn_state.
    let mut distill_task: Option<
        tokio::task::JoinHandle<(Option<distill::SessionLabels>, Option<String>)>,
    > = None;
    let mut distill_task_turn_id: i64 = 0;
    let mut distill_task_base_version: i64 = 0;
    let mut last_distill_attempt: u64 = 0;
    let mut cur_turn_started: u64 = 0;
    let mut prev_working = false;

    // Assert liveness immediately (refreshes session_state.last_seen + the legacy
    // `sessions` registry used by mention routing). No version bump, no outbox.
    st!(|s: &Store| {
        s.heartbeat_session(&p.session_id, now_secs()).ok();
        s.touch_session(&p.session_id, now_secs()).ok();
    });

    let mut hb = tokio::time::interval(p.heartbeat);
    let mut obs = tokio::time::interval(p.obs_interval);

    loop {
        tokio::select! {
            _ = hb.tick() => {
                if let Some(pid) = p.watch_pid {
                    if !pid_alive(pid) { break; }
                }
                // Liveness re-arm ONLY: bump last_seen in the canonical row (the
                // freshness that the kind:30315 NIP-40 expiration encodes) and the
                // legacy `sessions` registry. The relay re-publish that re-arms the
                // expiration is the drainer/heartbeat-publisher's job — the engine
                // never builds a Status. No version bump, no outbox.
                st!(|s: &Store| {
                    s.heartbeat_session(&p.session_id, now_secs()).ok();
                    s.touch_session(&p.session_id, now_secs()).ok();
                });
            }
            _ = obs.tick() => {
                let now = now_secs();

                // ── collect a finished background distillation ────────────
                // apply_distill_result is the versioned gate: it no-ops (returns
                // None) unless the session's current (turn_id, state_version) still
                // equals the base the task captured, so a stale distill or a
                // duplicate runtime cannot flip the title.
                if distill_task.as_ref().is_some_and(|h| h.is_finished()) {
                    let (result, error) = distill_task.take().unwrap().await.ok().unwrap_or((None, None));
                    if let Some(labels) = result {
                        // Capture the pre-apply title to decide whether to broadcast
                        // a new Activity note (a social kind:1, separate from status).
                        let prev_title = st!(|s: &Store| s.local_session_snapshot(&p.session_id))
                            .ok().flatten().map(|snap| snap.title).unwrap_or_default();
                        let applied = st!(|s: &Store| s.apply_distill_result(
                            &p.session_id,
                            distill_task_turn_id,
                            distill_task_base_version,
                            &labels.title,
                            &labels.activity,
                            now,
                        )).ok().flatten();
                        if let Some(snap) = applied {
                            if !snap.title.is_empty() && snap.title != prev_title {
                                publish_de(DomainEvent::Activity(Activity {
                                    agent: aref.clone(),
                                    project: p.project.clone(),
                                    text: format!("{} #{}", snap.title, p.project),
                                })).await;
                            }
                        }
                        // On a rejected apply (stale base) nothing changes; the next
                        // due-check re-reads the fresh base and reschedules.
                    } else if let Some(err_msg) = error {
                        let now = now_secs();
                        // Append to per-session log file for post-mortem inspection.
                        let log_dir = crate::config::edge_home().join("logs");
                        if crate::config::ensure_dir(&log_dir).is_ok() {
                            use std::io::Write as _;
                            let short = crate::util::session_codename(&p.session_id);
                            let log_path = log_dir.join(format!("{short}.log"));
                            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
                                let _ = writeln!(f, "{} [distill] ERROR: {}", crate::util::format_local_datetime(now), err_msg);
                            }
                        }
                        // Store in DB so the statusline can flash it for this session.
                        st!(|s: &Store| { s.record_session_error(&p.session_id, &err_msg, now).ok(); });
                    }
                }

                let (working, turn_started_at) = st!(|s: &Store| s.get_turn_state(&p.session_id).unwrap_or((false, 0)));
                if working {
                    // ── rising edge / new user message ────────────────────
                    if turn_started_at != cur_turn_started {
                        // The turn was OPENED by rpc_turn_start (the single owner of
                        // the start_turn transition). The engine only OBSERVES it:
                        // read the post-start snapshot for turn_id/title, seed a
                        // provisional title, and (re)schedule distillation. Calling
                        // start_turn here too would double-bump turn_id/version.
                        let snap = st!(|s: &Store| s.local_session_snapshot(&p.session_id)).ok().flatten();
                        if let Some(snap) = snap {
                            cur_turn_started = snap.turn_started_at;
                            let turn_id = snap.turn_id;
                            // Seed a provisional title from the user's prompt so the
                            // TUI shows something before the LLM distiller fires.
                            // seed_title_if_empty self-guards (only when title_source
                            // is 'none' and turn_id still matches), so this is safe to
                            // attempt unconditionally when the title is empty.
                            if snap.title.trim().is_empty() {
                                let quick = st!(|s: &Store| s.get_session_transcript(&p.session_id).ok().flatten())
                                    .and_then(|path| crate::transcript::read_last_user_prompt(std::path::Path::new(&path)))
                                    .and_then(|prompt| {
                                        let t = titleize_prompt(&prompt);
                                        if t.is_empty() { None } else { Some(t) }
                                    });
                                if let Some(qt) = quick {
                                    st!(|s: &Store| s.seed_title_if_empty(&p.session_id, turn_id, &qt, now)).ok();
                                }
                            }
                        } else {
                            // No canonical row (shouldn't happen mid-session); record
                            // the raw cursor so we don't spin on it.
                            cur_turn_started = turn_started_at;
                        }
                        // Fresh turn → reset distill scheduling.
                        last_distill_attempt = 0;
                        distill_task = None;
                        distill_task_turn_id = 0;
                        distill_task_base_version = 0;
                    }

                    // ── schedule background distillation ──────────────────
                    // Timing is derived from the canonical row (turn_started_at /
                    // last_distill_at), not cached locally. due = no task running AND
                    // (first-attempt window OR retry-after-failure OR turn_repeat
                    // refresh after a success this turn).
                    if distill_task.is_none() {
                        if let Some(snap) = st!(|s: &Store| s.local_session_snapshot(&p.session_id)).ok().flatten() {
                            let succeeded_this_turn =
                                snap.turn_started_at > 0 && snap.last_distill_at >= snap.turn_started_at;
                            let due = if last_distill_attempt == 0 {
                                now.saturating_sub(snap.turn_started_at) >= turn_first
                            } else if succeeded_this_turn {
                                turn_repeat > 0 && now.saturating_sub(snap.last_distill_at) >= turn_repeat
                            } else {
                                // Last attempt failed/timed out: retry after another window.
                                now.saturating_sub(last_distill_attempt) >= turn_first
                            };
                            if due {
                                let ctx = st!(|s: &Store| s.get_session_transcript(&p.session_id).ok().flatten())
                                    .and_then(|path| crate::transcript::read_recent(std::path::Path::new(&path), 14, 2500));
                                if let Some(ctx) = ctx {
                                    let current = (!snap.title.trim().is_empty()).then(|| snap.title.clone());
                                    last_distill_attempt = now;
                                    distill_task_turn_id = snap.turn_id;
                                    distill_task_base_version = snap.state_version;
                                    distill_task = Some(tokio::spawn(async move {
                                        match tokio::time::timeout(
                                            Duration::from_secs(20),
                                            distill::distill_session(&ctx, current.as_deref()),
                                        )
                                        .await
                                        {
                                            Ok(pair) => pair,
                                            Err(_) => (None, Some("distillation timed out after 20s".to_string())),
                                        }
                                    }));
                                }
                            }
                        }
                    }
                } else if prev_working {
                    // Falling edge: the turn was CLOSED by rpc_turn_end (single owner
                    // of end_turn). The engine only resets its local distill
                    // scheduling bookkeeping — calling end_turn here would
                    // double-bump version and re-enqueue the outbox.
                    cur_turn_started = 0;
                    last_distill_attempt = 0;
                    distill_task = None;
                    distill_task_turn_id = 0;
                    distill_task_base_version = 0;
                }
                prev_working = working;
            }
            _ = cancel.notified() => { break; }
        }
    }

    // Clean exit: finish the session in the canonical aggregate (lifecycle='ended',
    // TITLE retained; the final status — with a fresh expiration — is enqueued to
    // the outbox and published by the drainer, then ages off the relay as beats
    // stop). Also mark the legacy `sessions` row dead so mention routing
    // (list_alive_sessions) drops it. The engine itself publishes no Status.
    st!(|s: &Store| {
        s.end_session(&p.session_id, now_secs()).ok();
        s.mark_session_dead(&p.session_id).ok();
    });
    Ok(())
}

/// Derive a short title from a raw user prompt: take the first non-empty line,
/// strip leading markdown sigils (#, *, -, >), and cap at 60 chars on a word
/// boundary. Returns an empty string when nothing meaningful remains.
fn titleize_prompt(prompt: &str) -> String {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_start_matches(['#', '*', '-', '>', ' ', '\t'])
        .trim();
    if line.is_empty() {
        return String::new();
    }
    if line.len() <= 60 {
        return line.to_string();
    }
    match line[..60].rfind(' ') {
        Some(i) => line[..i].to_string(),
        None => line[..60].to_string(),
    }
}

fn pid_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_pid_is_alive() {
        assert!(pid_alive(std::process::id() as i32));
    }
}

