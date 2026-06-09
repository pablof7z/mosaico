//! The per-session engine (M1 §5, §7).
//!
//! Runs in the detached background process forked by `session-start`. It:
//!   - publishes the agent's `kind:0` profile once,
//!   - heartbeats presence on an interval,
//!   - drains observed tool activity, distills it, publishes Activity + Status,
//!   - subscribes to the project + mentions-to-me, updating the peer directory
//!     and routing mentions into the per-session inbox,
//!   - watches the host PID and stops cleanly (idle status) when it dies or on
//!     SIGTERM (the `session-end` path).

use crate::codec::{Codec, Kind1Codec};
use crate::distill;
use crate::domain::{Activity, AgentRef, DomainEvent, Mention, Presence, Profile, Status};
use crate::state::{InboxRow, Store};
use crate::transport::Transport;
use crate::util::now_secs;
use anyhow::Result;
use nostr_sdk::prelude::Event;
use std::path::PathBuf;
use std::time::Duration;

pub struct EngineParams {
    pub agent_slug: String,
    pub agent_pubkey: String,
    pub keys: nostr_sdk::prelude::Keys,
    pub project: String,
    pub session_id: String,
    pub host: String,
    /// Project-relative working directory (§8e), advertised on presence/status.
    pub rel_cwd: String,
    /// The human owner pubkey(s) — p-tagged on our profile + presence, and used
    /// to discover foreign agents claiming the same owner (ACL pending set).
    pub owners: Vec<String>,
    pub relays: Vec<String>,
    pub watch_pid: Option<i32>,
    pub store_path: PathBuf,
    pub heartbeat: Duration,
    /// How often the engine polls turn state to decide whether to distill.
    pub obs_interval: Duration,
    pub status_ttl: Duration,
    /// Delay from turn-start to the first activity distillation (default 30s) —
    /// short turns that finish before this never cost an LLM call.
    pub turn_first: Duration,
    /// Interval between subsequent distillations while a turn keeps running
    /// (default 5m), to refresh intent on long turns without spamming the LLM.
    pub turn_repeat: Duration,
}

/// Targets for an incoming mention addressed to me: a specific session if pinned
/// (and it is one of mine), else all my alive sessions for this agent.
pub fn compute_targets(target_session: Option<&str>, my_alive_sessions: &[String]) -> Vec<String> {
    match target_session {
        Some(ts) => {
            if my_alive_sessions.iter().any(|s| s == ts) {
                vec![ts.to_string()]
            } else {
                Vec::new()
            }
        }
        None => my_alive_sessions.to_vec(),
    }
}

// ── daemon-hosted session task (the relocated engine) ────────────────────────

/// Run the per-session engine INSIDE the daemon, using the SHARED relay
/// connection and the SHARED store (single writer). Unlike `run_session`, this
/// does NOT open its own store/transport and does NOT subscribe or demux: the
/// daemon owns one union subscription and demuxes incoming events centrally,
/// routing mentions to the right agent's inbox. This task only:
///   - publishes profile + presence once (signed with the agent's own keys),
///   - heartbeats presence (and refreshes a live status' TTL),
///   - distills turn activity → Activity + Status,
///   - watches the host pid and exits cleanly (idle presence/status) on pid
///     death or on `cancel` (the `session-end` path).
///
/// Store access goes through the shared `Arc<Mutex<Store>>`; the guard is held
/// only across the synchronous rusqlite calls, NEVER across `.await`.
pub async fn run_session_in_daemon(
    p: EngineParams,
    transport: std::sync::Arc<Transport>,
    store: std::sync::Arc<std::sync::Mutex<Store>>,
    cancel: std::sync::Arc<tokio::sync::Notify>,
) -> Result<()> {
    let codec = Kind1Codec;
    let me = p.agent_pubkey.clone();
    let keys = p.keys.clone();
    let aref = AgentRef::new(me.clone(), p.agent_slug.clone());
    let ttl = p.status_ttl.as_secs();
    let owners = p.owners.clone();

    macro_rules! st {
        ($f:expr) => {{
            let g = store.lock().expect("store mutex poisoned");
            #[allow(clippy::redundant_closure_call)]
            ($f)(&*g)
        }};
    }

    let publish_de = |ev: DomainEvent| {
        let transport = transport.clone();
        let codec = &codec;
        let keys = keys.clone();
        async move {
            if let Ok(b) = codec.encode(&ev) {
                let _ = transport.publish_signed(b, &keys).await;
            }
        }
    };
    let presence = |expires_at| {
        DomainEvent::Presence(Presence {
            agent: aref.clone(),
            project: p.project.clone(),
            session_id: p.session_id.clone(),
            host: p.host.clone(),
            rel_cwd: p.rel_cwd.clone(),
            audience: owners.clone(),
            expires_at,
        })
    };
    let status_de = |text: &str| {
        DomainEvent::Status(Status {
            agent: aref.clone(),
            project: p.project.clone(),
            text: text.to_string(),
            rel_cwd: p.rel_cwd.clone(),
            expires_at: Some(now_secs() + ttl),
        })
    };

    // Identity card + immediate liveness.
    publish_de(DomainEvent::Profile(Profile {
        agent: aref.clone(),
        host: p.host.clone(),
        owners: owners.clone(),
    }))
    .await;
    publish_de(presence(now_secs() + ttl)).await;
    publish_de(status_de("")).await;
    st!(|s: &Store| {
        s.set_agent_status(&me, &p.project, "", now_secs()).ok();
        s.touch_session(&p.session_id, now_secs()).ok();
    });

    let turn_first = p.turn_first.as_secs();
    let turn_repeat = p.turn_repeat.as_secs();
    let mut cur_turn_start: u64 = 0;
    let mut last_distill: u64 = 0;
    let mut cur_line: Option<String> = None;

    let mut hb = tokio::time::interval(p.heartbeat);
    let mut obs = tokio::time::interval(p.obs_interval);

    loop {
        tokio::select! {
            _ = hb.tick() => {
                if let Some(pid) = p.watch_pid {
                    if !pid_alive(pid) { break; }
                }
                st!(|s: &Store| { s.touch_session(&p.session_id, now_secs()).ok(); });
                publish_de(presence(now_secs() + ttl)).await;
                if let Some(line) = cur_line.clone() {
                    publish_de(status_de(&line)).await;
                    st!(|s: &Store| { s.set_agent_status(&me, &p.project, &line, now_secs()).ok(); });
                }
            }
            _ = obs.tick() => {
                let (working, turn_started_at) = st!(|s: &Store| s.get_turn_state(&p.session_id).unwrap_or((false, 0)));
                let now = now_secs();
                if working {
                    if turn_started_at != cur_turn_start {
                        cur_turn_start = turn_started_at;
                        last_distill = 0;
                    }
                    let due = if last_distill == 0 {
                        now.saturating_sub(cur_turn_start) >= turn_first
                    } else {
                        now.saturating_sub(last_distill) >= turn_repeat
                    };
                    if due {
                        let ctx = st!(|s: &Store| s.get_session_transcript(&p.session_id).ok().flatten())
                            .and_then(|path| crate::transcript::read_recent(std::path::Path::new(&path), 14, 2500));
                        if let Some(ctx) = ctx {
                            if let Some(line) = distill::distill_activity(&ctx).await {
                                publish_de(DomainEvent::Activity(Activity {
                                    agent: aref.clone(),
                                    project: p.project.clone(),
                                    text: format!("{line} #{}", p.project),
                                })).await;
                                publish_de(status_de(&line)).await;
                                st!(|s: &Store| { s.set_agent_status(&me, &p.project, &line, now).ok(); });
                                cur_line = Some(line);
                            }
                        }
                        last_distill = now;
                    }
                } else if cur_line.is_some() {
                    publish_de(status_de("")).await;
                    st!(|s: &Store| { s.set_agent_status(&me, &p.project, "", now).ok(); });
                    cur_line = None;
                    cur_turn_start = 0;
                    last_distill = 0;
                } else {
                    cur_turn_start = 0;
                    last_distill = 0;
                }
            }
            _ = cancel.notified() => { break; }
        }
    }

    // Clean exit: expire presence, go idle, mark the session dead.
    publish_de(presence(now_secs())).await;
    publish_de(status_de("")).await;
    st!(|s: &Store| { s.mark_session_dead(&p.session_id).ok(); });
    Ok(())
}

/// Route a mention addressed to agent `me` into the per-session inbox(es) of
/// `me`'s alive sessions, deduped per-agent across sessions. Returns true if any
/// row was newly enqueued (so the daemon can wake `wait-for-mention` waiters).
///
/// Multi-agent and multi-project safe: only sessions whose `agent_pubkey == me`
/// and `project == m.project` are considered, so a mention to agent A never
/// lands in agent B's inbox, and `codex@project-a` never wakes a `codex`
/// session in `project-b` on the same machine.
pub fn route_mention_into(store: &Store, me: &str, m: &Mention, event: &Event) -> bool {
    route_mention_into_with_id(store, me, m, &event.id.to_hex())
}

/// Like [`route_mention_into`], but takes the mention's event id directly instead
/// of a decoded `Event`. Used by the local-delivery path in `send_message`, where
/// the daemon publishes the event and routes it to a hosted sibling session
/// without waiting for (and without relying on) a relay echo. The published
/// `EventId` is identical to what the relay would echo, so the inbox PK
/// `(mention_event_id, target_session)` keeps delivery idempotent across both
/// paths.
pub fn route_mention_into_with_id(store: &Store, me: &str, m: &Mention, eid: &str) -> bool {
    // Already delivered to this agent in some session? Don't re-enqueue it in a
    // new session (mentions persist on the relay as stored kind:1 events).
    // Per-agent dedup applies ONLY to agent-wide (untargeted) mentions, so an
    // already-seen agent-wide mention does not resurface in every later session.
    // SESSION-TARGETED mentions bypass per-agent dedup: a reply between sibling
    // sessions of the same agent (same pubkey) must reach its target session even
    // if another sibling already marked the event seen. Idempotency for the
    // targeted case is carried by the inbox PK `(mention_event_id, target_session)`
    // (`enqueue_mention` is INSERT OR IGNORE; delivered rows are never deleted).
    if m.target_session.is_none() && store.is_mention_seen(me, eid).unwrap_or(false) {
        return false;
    }
    let alive: Vec<String> = store
        .list_alive_sessions()
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.agent_pubkey == me && s.project == m.project)
        .map(|s| s.session_id)
        .collect();
    let targets = compute_targets(m.target_session.as_deref(), &alive);
    let mut routed = false;
    for t in targets {
        let newly = store
            .enqueue_mention(&InboxRow {
                mention_event_id: eid.to_string(),
                target_session: t,
                from_pubkey: m.from.pubkey.clone(),
                from_slug: m.from.slug.clone(),
                project: m.project.clone(),
                body: m.body.clone(),
                created_at: now_secs(),
            })
            .unwrap_or(false);
        routed = routed || newly;
    }
    routed
}

fn pid_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_pinned_session_only_if_mine() {
        let mine = vec!["s1".to_string(), "s2".to_string()];
        assert_eq!(compute_targets(Some("s2"), &mine), vec!["s2"]);
        assert!(compute_targets(Some("not-mine"), &mine).is_empty());
    }

    #[test]
    fn targets_agent_level_fans_out_to_all_my_sessions() {
        let mine = vec!["s1".to_string(), "s2".to_string()];
        assert_eq!(compute_targets(None, &mine), mine);
    }

    #[test]
    fn current_pid_is_alive() {
        assert!(pid_alive(std::process::id() as i32));
    }

    // ── helpers for routing/dedup tests ───────────────────────────────────
    use crate::state::SessionRecord;
    use nostr_sdk::prelude::Keys;

    fn alive_session(id: &str, pubkey: &str) -> SessionRecord {
        alive_session_in_project(id, pubkey, "proj")
    }

    fn alive_session_in_project(id: &str, pubkey: &str, project: &str) -> SessionRecord {
        SessionRecord {
            session_id: id.into(),
            agent_slug: "claude".into(),
            agent_pubkey: pubkey.into(),
            project: project.into(),
            host: "laptop".into(),
            child_pid: None,
            watch_pid: None,
            created_at: 1000,
            alive: true,
            rel_cwd: String::new(),
        }
    }

    /// Build a real signed kind:1 Mention event from `from_keys` to `to_pubkey`.
    fn signed_mention(from_keys: &Keys, to_pubkey: &str, target_session: Option<&str>) -> (Mention, Event) {
        let m = Mention {
            from: AgentRef::new(from_keys.public_key().to_hex(), "claude"),
            to_pubkey: to_pubkey.to_string(),
            project: "proj".to_string(),
            body: "hi sibling".to_string(),
            target_session: target_session.map(str::to_string),
        };
        let event = Kind1Codec
            .encode(&DomainEvent::Mention(m.clone()))
            .unwrap()
            .sign_with_keys(from_keys)
            .unwrap();
        (m, event)
    }

    /// Bug A (sibling session delivery): a claude session A sends to a DIFFERENT
    /// claude session B that shares the same pubkey. The mention must land in B's
    /// inbox and NOT in A's. (Both sessions are alive.)
    #[test]
    fn sibling_session_mention_lands_in_target_not_sender() {
        let s = Store::open_memory().unwrap();
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        s.upsert_session(&alive_session("sess-A", &pubkey)).unwrap();
        s.upsert_session(&alive_session("sess-B", &pubkey)).unwrap();

        let (m, event) = signed_mention(&keys, &pubkey, Some("sess-B"));
        let routed = route_mention_into(&s, &pubkey, &m, &event);
        assert!(routed, "sibling-targeted mention should be newly routed");

        assert_eq!(s.drain_inbox("sess-B").unwrap().len(), 1, "B must receive it");
        assert!(s.drain_inbox("sess-A").unwrap().is_empty(), "A (sender) must NOT receive it");
    }

    /// Bug B (per-(pubkey,session) dedup): a session-targeted mention must still be
    /// delivered to its target session even if a SIBLING session of the same agent
    /// already "saw" (per-agent dedup-marked) that event. Per-agent dedup must NOT
    /// block session-targeted delivery.
    #[test]
    fn session_targeted_mention_not_blocked_by_sibling_seen() {
        let s = Store::open_memory().unwrap();
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        s.upsert_session(&alive_session("sess-A", &pubkey)).unwrap();
        s.upsert_session(&alive_session("sess-B", &pubkey)).unwrap();

        let (m, event) = signed_mention(&keys, &pubkey, Some("sess-B"));
        // Sibling A marks the event seen per-agent (e.g. it drained an agent-wide
        // copy in its own turn). This must NOT block the session-targeted delivery.
        s.mark_mention_seen(&pubkey, &event.id.to_hex(), now_secs()).unwrap();

        let routed = route_mention_into(&s, &pubkey, &m, &event);
        assert!(routed, "session-targeted mention must bypass per-agent dedup");
        assert_eq!(s.drain_inbox("sess-B").unwrap().len(), 1, "B must still receive it");
    }

    /// Bug A (local delivery): `send_message` routes the just-published event to a
    /// hosted sibling session via `route_mention_into_with_id`, using the SAME
    /// event id it published. Delivery must reach the target sibling, not the
    /// sender, and a later relay echo of the same id must NOT double-deliver
    /// (idempotent on the inbox PK).
    #[test]
    fn local_delivery_by_event_id_is_idempotent_and_targets_sibling() {
        let s = Store::open_memory().unwrap();
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        s.upsert_session(&alive_session("sess-A", &pubkey)).unwrap();
        s.upsert_session(&alive_session("sess-B", &pubkey)).unwrap();

        let (m, event) = signed_mention(&keys, &pubkey, Some("sess-B"));
        let eid = event.id.to_hex();

        // Local delivery (send_message path).
        assert!(route_mention_into_with_id(&s, &pubkey, &m, &eid));
        // A later relay echo of the SAME event id (handle_incoming path).
        assert!(!route_mention_into_with_id(&s, &pubkey, &m, &eid), "echo must not double-deliver");

        assert_eq!(s.drain_inbox("sess-B").unwrap().len(), 1, "exactly one delivery to B");
        assert!(s.drain_inbox("sess-A").unwrap().is_empty(), "sender A must not receive");
    }

    #[test]
    fn local_delivery_only_routes_to_sessions_in_mentions_project() {
        let s = Store::open_memory().unwrap();
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        s.upsert_session(&alive_session_in_project("sess-current", &pubkey, "current"))
            .unwrap();
        s.upsert_session(&alive_session_in_project("sess-other", &pubkey, "other"))
            .unwrap();

        let mut m = signed_mention(&keys, &pubkey, None).0;
        m.project = "current".to_string();

        assert!(route_mention_into_with_id(&s, &pubkey, &m, "event-project-current"));
        assert_eq!(s.drain_inbox("sess-current").unwrap().len(), 1);
        assert!(s.drain_inbox("sess-other").unwrap().is_empty());
    }

    /// Preserve: an AGENT-WIDE (untargeted) mention is still deduped per-agent so it
    /// does not resurface in every session once seen.
    #[test]
    fn agent_wide_mention_still_deduped_per_agent() {
        let s = Store::open_memory().unwrap();
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        s.upsert_session(&alive_session("sess-A", &pubkey)).unwrap();

        let (m, event) = signed_mention(&keys, &pubkey, None);
        s.mark_mention_seen(&pubkey, &event.id.to_hex(), now_secs()).unwrap();

        let routed = route_mention_into(&s, &pubkey, &m, &event);
        assert!(!routed, "agent-wide mention already seen must not re-route");
        assert!(s.drain_inbox("sess-A").unwrap().is_empty());
    }
}
