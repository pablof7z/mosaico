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
    assert!(route_mention_into_with_id(&s, &pubkey, &m, &eid));
    // A later relay echo of the SAME event id (handle_incoming path).
    assert!(
        !route_mention_into_with_id(&s, &pubkey, &m, &eid),
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
        "event-project-current"
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
