use crate::state::session_claims::SessionClaim;
use crate::state::{RegisterSession, Store};

fn claim(pubkey: &str, expires_at: u64) -> SessionClaim {
    SessionClaim {
        pubkey: pubkey.to_string(),
        agent_slug: "codex".to_string(),
        channel_h: "chan".to_string(),
        harness: "codex".to_string(),
        last_active_at: 10,
        expires_at,
        owner_backend_pubkey: "backend-a".to_string(),
        owner_host: "laptop".to_string(),
    }
}

#[test]
fn active_claims_are_ttl_bounded() {
    let store = Store::open_memory().unwrap();
    store.upsert_session_claim(&claim("pk", 20)).unwrap();

    assert!(store
        .get_active_session_claim("pk", "chan", 20)
        .unwrap()
        .is_some());
    assert!(store
        .get_active_session_claim("pk", "chan", 21)
        .unwrap()
        .is_none());
}

#[test]
fn claim_ownership_treats_legacy_empty_owner_as_local() {
    let mut c = claim("pk", 20);
    assert!(c.is_owned_by_backend(Some("backend-a")));
    assert!(!c.is_owned_by_backend(Some("backend-b")));
    assert!(!c.is_owned_by_backend(None));

    c.owner_backend_pubkey.clear();
    assert!(c.is_owned_by_backend(None));
    assert!(c.is_owned_by_backend(Some("backend-b")));
}

#[test]
fn session_reassert_clears_its_claim() {
    let store = Store::open_memory().unwrap();
    store
        .reserve_session(&RegisterSession {
            pubkey: "pk".to_string(),
            harness: "codex".to_string(),
            agent_slug: "codex".to_string(),
            channel_h: "chan".to_string(),
            child_pid: Some(1),
            transcript_path: None,
            now: 10,
        })
        .unwrap();
    store.upsert_session_claim(&claim("pk", 30)).unwrap();
    store.mark_dead("pk").unwrap();

    store
        .reserve_session(&RegisterSession {
            pubkey: "pk".to_string(),
            harness: "codex".to_string(),
            agent_slug: "codex".to_string(),
            channel_h: "chan".to_string(),
            child_pid: Some(2),
            transcript_path: None,
            now: 20,
        })
        .unwrap();

    assert!(store.get_session_claim("pk", "chan").unwrap().is_none());
}
