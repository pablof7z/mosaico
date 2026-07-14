use super::super::*;
use super::reg;

#[test]
fn canonical_id_stable_across_external_id_rotation() {
    let s = Store::open_memory().unwrap();
    let canonical = s
        .register_session(&reg("claude-code", "ext-A", "h1"))
        .unwrap();
    // A rotated harness id repointed onto the same canonical session.
    s.put_alias("claude-code", "resume", "ext-B", &canonical, 1500)
        .unwrap();
    // Mutating by EITHER external id must resolve to the canonical row.
    s.set_working("ext-A", true, 2000).unwrap();
    assert!(s.get_session("ext-B").unwrap().unwrap().working);
    assert_eq!(
        s.get_session("ext-A").unwrap().unwrap().session_id,
        canonical
    );
}

#[test]
fn register_is_idempotent_per_external_id() {
    let s = Store::open_memory().unwrap();
    let a = s.register_session(&reg("codex", "x1", "h1")).unwrap();
    let b = s.register_session(&reg("codex", "x1", "h1")).unwrap();
    assert_eq!(a, b);
    assert_eq!(s.list_alive_sessions().unwrap().len(), 1);
}

/// "Born-right" registration: `rpc_session_start` resolves the canonical id,
/// selects the ordinal signer, then writes the row with the ordinal pubkey. The
/// id is STABLE across the resolve/mint step, and re-asserting with the same
/// ordinal pubkey keeps it — so an ordinal never collapses back to the base and
/// a p-tagged mention reaches exactly one session. Regression for the mention
/// fan-out.
#[test]
fn born_right_id_is_stable_and_ordinal_pubkey_persists() {
    let s = Store::open_memory().unwrap();
    // First start: resolve/mint the id, then write the row with the ORDINAL key.
    let sid = s
        .resolve_or_mint_session_id("claude-code", "harness_session", "x1", 1000)
        .unwrap();
    let mut r = reg("claude-code", "x1", "h1");
    r.agent_pubkey = "pk-ordinal-1".into();
    s.upsert_session_row(&sid, &r).unwrap();
    assert_eq!(
        s.get_session(&sid).unwrap().unwrap().agent_pubkey,
        "pk-ordinal-1"
    );

    // Re-assert: same external id → SAME canonical id, and the signer re-selects
    // the same ordinal, so the row keeps its ordinal pubkey.
    let again = s
        .resolve_or_mint_session_id("claude-code", "harness_session", "x1", 2000)
        .unwrap();
    assert_eq!(again, sid, "same external id → same canonical session");
    s.upsert_session_row(&sid, &r).unwrap();
    assert_eq!(
        s.get_session(&sid).unwrap().unwrap().agent_pubkey,
        "pk-ordinal-1",
        "re-assert must keep the ordinal pubkey, never collapse to the base"
    );
}

#[test]
fn mark_dead_resolves_external_id() {
    let s = Store::open_memory().unwrap();
    s.register_session(&reg("opencode", "o1", "h1")).unwrap();
    s.mark_dead("o1").unwrap();
    assert!(!s.get_session("o1").unwrap().unwrap().alive);
    assert!(s.list_alive_sessions().unwrap().is_empty());
}

/// Defect #6: clearing a dead endpoint alias MUST also mark the row
/// dead. Otherwise the session stays ALIVE guarded only by `child_pid`, and a
/// later reconcile — seeing a recycled PID as alive — false-revives a ghost.
#[test]
fn retire_dead_endpoint_clears_aliases_and_marks_row_dead() {
    let s = Store::open_memory().unwrap();
    let sid = s.register_session(&reg("claude-code", "c1", "h1")).unwrap();
    s.put_alias("claude-code", "pty_session", "endpoint-1", &sid, 1000)
        .unwrap();
    assert!(s.get_session(&sid).unwrap().unwrap().alive);

    s.retire_dead_endpoint(&sid).unwrap();

    // The row is retired, not left ALIVE for a recycled-PID false-revive.
    assert!(
        !s.get_session(&sid).unwrap().unwrap().alive,
        "clearing the endpoint alias must mark the session dead (defect #6)"
    );
    assert!(s.list_alive_sessions().unwrap().is_empty());
    // The endpoint alias is gone.
    let kinds: Vec<String> = s
        .aliases_for_session(&sid)
        .unwrap()
        .into_iter()
        .map(|a| a.external_id_kind)
        .collect();
    assert!(!kinds.iter().any(|k| k == "pty_session"));
}

#[test]
fn explicit_chat_marker_resolves_external_id_and_stays_first_publish() {
    let s = Store::open_memory().unwrap();
    let sid = s.register_session(&reg("codex", "x1", "h1")).unwrap();

    assert_eq!(
        s.get_session(&sid)
            .unwrap()
            .unwrap()
            .explicit_chat_published_at,
        0
    );

    s.mark_session_explicit_chat_published("x1", 1200).unwrap();
    s.mark_session_explicit_chat_published(&sid, 1300).unwrap();

    assert_eq!(
        s.get_session(&sid)
            .unwrap()
            .unwrap()
            .explicit_chat_published_at,
        1200
    );
}
