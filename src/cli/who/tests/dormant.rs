use super::*;
use crate::state::session_claims::SessionClaim;

fn claim(owner_backend_pubkey: &str, owner_host: &str) -> SessionClaim {
    SessionClaim {
        pubkey: "pk-codex1".to_string(),
        base_pubkey: "pk-codex".to_string(),
        agent_slug: "codex".to_string(),
        ordinal: 1,
        session_id: "sid-codex".to_string(),
        channel_h: "proj".to_string(),
        native_id: "native-codex".to_string(),
        harness: "codex".to_string(),
        last_active_at: 900,
        expires_at: 1_100,
        owner_backend_pubkey: owner_backend_pubkey.to_string(),
        owner_host: owner_host.to_string(),
    }
}

#[test]
fn who_snapshot_renders_active_claim_as_dormant_presence() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_session_claim(&claim("backend-laptop", "laptop"))
        .unwrap();

    let snapshot = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let row = snapshot.rows.first().expect("dormant row");
    assert!(row.dormant);
    assert_eq!(row.slug, "codex1");
    assert_eq!(row.age_secs, Some(100));

    let once = strip_ansi(&render_who_once(&snapshot));
    assert!(once.contains("codex1 (laptop) - last active 1m ago"));
    let plain = render_who_plain(&snapshot);
    assert!(plain.contains("| codex1 | laptop | — | last active 1m ago |"));
}

#[test]
fn who_snapshot_marks_remote_owned_claims_remote() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_session_claim(&claim("backend-tower", "tower"))
        .unwrap();

    let snapshot = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let row = snapshot.rows.first().expect("dormant row");
    assert!(row.dormant);
    assert!(row.remote);
    assert_eq!(row.host, "tower");

    let once = strip_ansi(&render_who_once(&snapshot));
    assert!(once.contains("codex1 (tower, remote) - last active 1m ago"));
}
