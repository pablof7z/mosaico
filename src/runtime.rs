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

use crate::distill;
use crate::domain::{Activity, AgentRef, DomainEvent, Mention, Presence, Profile, Status};
use crate::fabric::provider::Kind1Nip29Provider;
use crate::state::{InboxRow, Store};
use crate::util::{now_secs, SessionId};
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
    provider: std::sync::Arc<Kind1Nip29Provider>,
    store: std::sync::Arc<std::sync::Mutex<Store>>,
    cancel: std::sync::Arc<tokio::sync::Notify>,
) -> Result<()> {
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
        let provider = provider.clone();
        let keys = keys.clone();
        async move {
            let _ = provider.publish(&ev, &keys).await;
        }
    };
    let presence = |expires_at| {
        DomainEvent::Presence(Presence {
            agent: aref.clone(),
            project: p.project.clone(),
            session_id: SessionId::from(p.session_id.clone()),
            host: p.host.clone(),
            rel_cwd: p.rel_cwd.clone(),
            audience: owners.clone(),
            expires_at,
        })
    };
    let status_de = |text: &str, activity: &str, active: bool| {
        DomainEvent::Status(Status {
            agent: aref.clone(),
            project: p.project.clone(),
            session_id: Some(SessionId::from(p.session_id.clone())),
            text: text.to_string(),
            activity: activity.to_string(),
            active,
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

    // The session TITLE persists across idle turns and feeds back into the
    // distiller. On a daemon restart for an existing session, recover the prior
    // title from the store so re-distillation nudges-to-keep it.
    let turn_first = p.turn_first.as_secs();
    let turn_repeat = p.turn_repeat.as_secs();
    let mut cur_turn_start: u64 = 0;
    let mut last_distill: u64 = 0;
    let mut prev_working = false;
    let mut cur_title: Option<String> = st!(|s: &Store| {
        s.get_agent_status(&me, &p.project, Some(&p.session_id))
            .ok()
            .flatten()
            .map(|(t, _a, _)| t)
    })
    .filter(|t| !t.trim().is_empty());
    // Live "doing now" line. Ephemeral: NOT recovered across a daemon restart
    // (it describes the current step, which is stale by then) and cleared on idle.
    let mut cur_activity: Option<String> = None;

    // Publish initial status: retain any recovered title, but go idle (no live
    // activity) until a turn starts.
    let init_title = cur_title.clone().unwrap_or_default();
    publish_de(status_de(&init_title, "", false)).await;
    st!(|s: &Store| {
        s.set_agent_status(
            &me,
            &p.project,
            Some(&p.session_id),
            &init_title,
            "",
            false,
            now_secs(),
        )
        .ok();
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
                st!(|s: &Store| { s.touch_session(&p.session_id, now_secs()).ok(); });
                publish_de(presence(now_secs() + ttl)).await;
                // Refresh the title's TTL with the live active flag so an idle
                // session keeps its title alive on the relay (until it exits).
                // The live activity is only meaningful mid-turn; clear it when idle.
                if let Some(title) = cur_title.clone() {
                    let (working, _) = st!(|s: &Store| s.get_turn_state(&p.session_id).unwrap_or((false, 0)));
                    let activity = if working { cur_activity.clone().unwrap_or_default() } else { String::new() };
                    publish_de(status_de(&title, &activity, working)).await;
                    st!(|s: &Store| { s.set_agent_status(&me, &p.project, Some(&p.session_id), &title, &activity, working, now_secs()).ok(); });
                }
            }
            _ = obs.tick() => {
                let (working, turn_started_at) = st!(|s: &Store| s.get_turn_state(&p.session_id).unwrap_or((false, 0)));
                let now = now_secs();
                if working {
                    // Rising edge / new user message: mark active immediately
                    // (retaining the current title) and arm a re-distillation. The
                    // prior turn's live activity is now stale → clear it.
                    if turn_started_at != cur_turn_start {
                        cur_turn_start = turn_started_at;
                        last_distill = 0;
                        cur_activity = None;
                        let title = cur_title.clone().unwrap_or_default();
                        publish_de(status_de(&title, "", true)).await;
                        st!(|s: &Store| { s.set_agent_status(&me, &p.project, Some(&p.session_id), &title, "", true, now).ok(); });
                    }
                    // Distill the title + live activity shortly after turn start,
                    // then optionally re-check on long turns (turn_repeat == 0 off).
                    let due = if last_distill == 0 {
                        now.saturating_sub(cur_turn_start) >= turn_first
                    } else {
                        turn_repeat > 0 && now.saturating_sub(last_distill) >= turn_repeat
                    };
                    if due {
                        let ctx = st!(|s: &Store| s.get_session_transcript(&p.session_id).ok().flatten())
                            .and_then(|path| crate::transcript::read_recent(std::path::Path::new(&path), 14, 2500));
                        if let Some(ctx) = ctx {
                            // One model call yields BOTH the stable title and the
                            // live activity line (see distill::distill_session).
                            if let Some(labels) = distill::distill_session(&ctx, cur_title.as_deref()).await {
                                // Only announce an Activity note when the title actually changes.
                                let changed = cur_title.as_deref() != Some(labels.title.as_str());
                                cur_title = Some(labels.title.clone());
                                cur_activity = (!labels.activity.is_empty()).then(|| labels.activity.clone());
                                if changed {
                                    publish_de(DomainEvent::Activity(Activity {
                                        agent: aref.clone(),
                                        project: p.project.clone(),
                                        text: format!("{} #{}", labels.title, p.project),
                                    })).await;
                                }
                                publish_de(status_de(&labels.title, &labels.activity, true)).await;
                                st!(|s: &Store| { s.set_agent_status(&me, &p.project, Some(&p.session_id), &labels.title, &labels.activity, true, now).ok(); });
                            }
                        }
                        last_distill = now;
                    }
                } else if prev_working {
                    // Falling edge: turn ended → go idle but KEEP the title; the
                    // live activity is dropped (only the persistent title survives).
                    cur_activity = None;
                    let title = cur_title.clone().unwrap_or_default();
                    publish_de(status_de(&title, "", false)).await;
                    st!(|s: &Store| { s.set_agent_status(&me, &p.project, Some(&p.session_id), &title, "", false, now).ok(); });
                    cur_turn_start = 0;
                    last_distill = 0;
                }
                prev_working = working;
            }
            _ = cancel.notified() => { break; }
        }
    }

    // Clean exit: expire presence, clear the title (the ONLY place a live title
    // is dropped), and mark the session dead.
    publish_de(presence(now_secs())).await;
    publish_de(status_de("", "", false)).await;
    st!(|s: &Store| {
        s.set_agent_status(
            &me,
            &p.project,
            Some(&p.session_id),
            "",
            "",
            false,
            now_secs(),
        )
        .ok();
        s.mark_session_dead(&p.session_id).ok();
    });
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
    // Use the event's own timestamp as the send time so the envelope Date reflects
    // when the sender published, not when we fetched/routed it.
    route_mention_into_with_id(store, me, m, &event.id.to_hex(), event.created_at.as_secs())
}

/// Like [`route_mention_into`], but takes the mention's event id directly instead
/// of a decoded `Event`. Used by the local-delivery path in `send_message`, where
/// the daemon publishes the event and routes it to a hosted sibling session
/// without waiting for (and without relying on) a relay echo. The published
/// `EventId` is identical to what the relay would echo, so the inbox PK
/// `(mention_event_id, target_session)` keeps delivery idempotent across both
/// paths.
pub fn route_mention_into_with_id(
    store: &Store,
    me: &str,
    m: &Mention,
    eid: &str,
    sent_at: u64,
) -> bool {
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
    let targets = compute_targets(m.target_session.as_ref().map(|s| s.as_str()), &alive);
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
                created_at: sent_at,
                from_session: m
                    .from_session
                    .as_ref()
                    .map(|s| s.as_str().to_owned())
                    .unwrap_or_default(),
                subject: m.meta.subject.clone(),
                branch: m.meta.branch.clone(),
                commit: m.meta.commit.clone(),
                dirty: m.meta.dirty,
                host: m.meta.host.clone(),
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
    fn signed_mention(
        from_keys: &Keys,
        to_pubkey: &str,
        target_session: Option<&str>,
    ) -> (Mention, Event) {
        let m = Mention {
            from: AgentRef::new(from_keys.public_key().to_hex(), "claude"),
            to_pubkey: to_pubkey.to_string(),
            project: "proj".to_string(),
            body: "hi sibling".to_string(),
            target_session: target_session.map(crate::util::SessionId::from),
            from_session: None,
            meta: crate::domain::MentionMeta::default(),
        };
        use crate::codec::Codec as _;
        let event = crate::codec::Kind1Codec
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

        assert_eq!(
            s.drain_inbox("sess-B").unwrap().len(),
            1,
            "B must receive it"
        );
        assert!(
            s.drain_inbox("sess-A").unwrap().is_empty(),
            "A (sender) must NOT receive it"
        );
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
        s.mark_mention_seen(&pubkey, &event.id.to_hex(), now_secs())
            .unwrap();

        let routed = route_mention_into(&s, &pubkey, &m, &event);
        assert!(
            routed,
            "session-targeted mention must bypass per-agent dedup"
        );
        assert_eq!(
            s.drain_inbox("sess-B").unwrap().len(),
            1,
            "B must still receive it"
        );
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
        assert!(route_mention_into_with_id(&s, &pubkey, &m, &eid, 12345));
        // A later relay echo of the SAME event id (handle_incoming path).
        assert!(
            !route_mention_into_with_id(&s, &pubkey, &m, &eid, 12345),
            "echo must not double-deliver"
        );

        assert_eq!(
            s.drain_inbox("sess-B").unwrap().len(),
            1,
            "exactly one delivery to B"
        );
        assert!(
            s.drain_inbox("sess-A").unwrap().is_empty(),
            "sender A must not receive"
        );
    }

    #[test]
    fn local_delivery_only_routes_to_sessions_in_mentions_project() {
        let s = Store::open_memory().unwrap();
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        s.upsert_session(&alive_session_in_project(
            "sess-current",
            &pubkey,
            "current",
        ))
        .unwrap();
        s.upsert_session(&alive_session_in_project("sess-other", &pubkey, "other"))
            .unwrap();

        let mut m = signed_mention(&keys, &pubkey, None).0;
        m.project = "current".to_string();

        assert!(route_mention_into_with_id(
            &s,
            &pubkey,
            &m,
            "event-project-current",
            12345
        ));
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
        s.mark_mention_seen(&pubkey, &event.id.to_hex(), now_secs())
            .unwrap();

        let routed = route_mention_into(&s, &pubkey, &m, &event);
        assert!(!routed, "agent-wide mention already seen must not re-route");
        assert!(s.drain_inbox("sess-A").unwrap().is_empty());
    }

    // ── freeze tests (Phase-0 regression oracle) ─────────────────────────────

    /// FREEZE A1: TARGETED mention reaches ONLY the named session.
    /// Two alive sessions (same agent, same project): a mention targeting sess-B
    /// must land ONLY in sess-B. sess-A (sibling) must not receive it.
    #[test]
    fn freeze_targeted_mention_routes_only_to_named_session() {
        let s = Store::open_memory().unwrap();
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        s.upsert_session(&alive_session("sess-A", &pubkey)).unwrap();
        s.upsert_session(&alive_session("sess-B", &pubkey)).unwrap();

        let (m, event) = signed_mention(&keys, &pubkey, Some("sess-B"));
        let routed = route_mention_into(&s, &pubkey, &m, &event);

        assert!(
            routed,
            "FREEZE: targeted mention to sess-B must be newly routed"
        );
        assert_eq!(
            s.drain_inbox("sess-B").unwrap().len(),
            1,
            "FREEZE: sess-B must receive exactly one row"
        );
        assert!(
            s.drain_inbox("sess-A").unwrap().is_empty(),
            "FREEZE: sess-A (sibling) must NOT receive a targeted mention for sess-B"
        );
    }

    /// FREEZE A2: UNTARGETED mention fans out to ALL alive sessions of the recipient
    /// agent+project, and NOT to sessions of other agents or other projects.
    ///
    /// Scenario: three sessions alive —
    ///   sess-1 (pk1, proj)
    ///   sess-2 (pk1, proj)   ← both should receive
    ///   sess-other (pk2, proj) ← different agent: must not receive
    ///   sess-other-proj (pk1, other-proj) ← different project: must not receive
    #[test]
    fn freeze_untargeted_mention_fans_out_to_all_alive_sessions_of_recipient_agent_project() {
        let s = Store::open_memory().unwrap();
        let keys1 = Keys::generate();
        let pk1 = keys1.public_key().to_hex();
        let keys2 = Keys::generate();
        let pk2 = keys2.public_key().to_hex();

        s.upsert_session(&alive_session_in_project("sess-1", &pk1, "proj"))
            .unwrap();
        s.upsert_session(&alive_session_in_project("sess-2", &pk1, "proj"))
            .unwrap();
        s.upsert_session(&alive_session_in_project("sess-other-agent", &pk2, "proj"))
            .unwrap();
        s.upsert_session(&alive_session_in_project(
            "sess-other-proj",
            &pk1,
            "other-proj",
        ))
        .unwrap();

        // Untargeted mention addressed to pk1/proj.
        let (m, event) = signed_mention(&keys2, &pk1, None);
        let routed = route_mention_into(&s, &pk1, &m, &event);

        assert!(routed, "FREEZE: untargeted mention must be newly routed");
        assert_eq!(
            s.drain_inbox("sess-1").unwrap().len(),
            1,
            "FREEZE: sess-1 (pk1/proj) must receive untargeted mention"
        );
        assert_eq!(
            s.drain_inbox("sess-2").unwrap().len(),
            1,
            "FREEZE: sess-2 (pk1/proj) must receive untargeted mention"
        );
        assert!(
            s.drain_inbox("sess-other-agent").unwrap().is_empty(),
            "FREEZE: different-agent session must NOT receive mention to pk1"
        );
        assert!(
            s.drain_inbox("sess-other-proj").unwrap().is_empty(),
            "FREEZE: same-agent but different-project session must NOT receive mention"
        );
    }

    /// FREEZE A3: re-routing the SAME event id is idempotent (inbox PK guarantee).
    ///
    /// For UNTARGETED mentions: the per-agent seen-mark deduplicates. But this test
    /// exercises idempotency at the inbox-PK level WITHOUT marking seen — to prove
    /// the `INSERT OR IGNORE` constraint is the safety net for every code path.
    ///
    /// After the first route_mention_into_with_id: returns true (newly routed).
    /// After the second call with same eid (without marking seen): returns false
    /// (inbox PK `(eid, target_session)` already exists — INSERT OR IGNORE fires).
    /// Drain yields exactly one row per session.
    #[test]
    fn freeze_routing_same_event_id_twice_is_idempotent_no_double_delivery() {
        let s = Store::open_memory().unwrap();
        let keys = Keys::generate();
        let pk = keys.public_key().to_hex();
        s.upsert_session(&alive_session_in_project("sess-1", &pk, "proj"))
            .unwrap();
        s.upsert_session(&alive_session_in_project("sess-2", &pk, "proj"))
            .unwrap();

        let (m, event) = signed_mention(&keys, &pk, None);
        let eid = event.id.to_hex();

        // First route: both sessions get the mention.
        let first = route_mention_into_with_id(&s, &pk, &m, &eid, 12345);
        assert!(first, "FREEZE: first route must be newly enqueued");

        // Second route (same eid, same sessions, no mark_mention_seen in between):
        // inbox PK (eid, sess-1) and (eid, sess-2) already exist → both INSERT OR
        // IGNORE fire → nothing new, returns false.
        let second = route_mention_into_with_id(&s, &pk, &m, &eid, 12345);
        assert!(
            !second,
            "FREEZE: second route of same eid must be idempotent (no new rows)"
        );

        // Each session has exactly one undelivered row — no duplicates.
        assert_eq!(
            s.drain_inbox("sess-1").unwrap().len(),
            1,
            "FREEZE: sess-1 must have exactly one delivery (no duplicate)"
        );
        assert_eq!(
            s.drain_inbox("sess-2").unwrap().len(),
            1,
            "FREEZE: sess-2 must have exactly one delivery (no duplicate)"
        );
    }

    /// FREEZE A4: TARGETED mention to a session id that is NOT among my alive
    /// sessions results in zero deliveries and route returns false.
    #[test]
    fn freeze_targeted_mention_to_unknown_session_delivers_nothing() {
        let s = Store::open_memory().unwrap();
        let keys = Keys::generate();
        let pk = keys.public_key().to_hex();
        // Only sess-A is alive; the mention targets a nonexistent session.
        s.upsert_session(&alive_session("sess-A", &pk)).unwrap();

        let (m, _event) = signed_mention(&keys, &pk, Some("nonexistent-session"));
        let routed = route_mention_into_with_id(&s, &pk, &m, "eid-unknown", 12345);

        assert!(
            !routed,
            "FREEZE: mention targeting unknown session must not route"
        );
        assert!(
            s.drain_inbox("sess-A").unwrap().is_empty(),
            "FREEZE: sess-A must not receive a mention targeting a different session id"
        );
    }
}
