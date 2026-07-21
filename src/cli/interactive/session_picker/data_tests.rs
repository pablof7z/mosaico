use super::*;

fn workspace() -> serde_json::Value {
    serde_json::json!([{
        "id": "root", "name": "mosaico", "path": "/repo",
        "channels": [{"id": "root", "name": "mosaico"}]
    }])
}

#[test]
fn parses_grouped_workspace_and_attach_endpoint() {
    let value = serde_json::json!({
        "sessions": [{
            "pubkey": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "npub": "npub1publicselector",
            "handle": "opal-codex",
            "agent": "codex",
            "workspaces": workspace(),
            "title": "shipping the picker",
            "activity": "running tests",
            "state": "working",
            "last_seen": 12,
            "host": "laptop",
            "harness": "codex",
            "endpoint": {"id": "pty-1", "kind": "pty", "live": true, "attachable": true, "cwd": "/repo"}
        }]
    });

    let rows = rows_from_value(&value);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].pubkey,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(rows[0].npub, "npub1publicselector");
    assert_eq!(rows[0].handle, "opal-codex");
    assert_eq!(rows[0].title, "shipping the picker");
    assert_eq!(rows[0].state, SessionState::Working);
    assert!(rows[0].fuzzy_score("npub1public").is_some());
    assert!(rows[0].fuzzy_score("repo").is_some());
    assert_eq!(rows[0].workspaces[0].name, "mosaico");
    assert_eq!(rows[0].pty_id.as_deref(), Some("pty-1"));
    assert!(rows[0].endpoint_live);
    assert!(!rows[0].can_take_over());
}

#[test]
fn parses_unhosted_takeover_and_open_turn_state() {
    let value = serde_json::json!({
        "sessions": [{
            "pubkey": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "npub": "npub1publicselector",
            "handle": "echo-codex",
            "agent": "codex",
            "workspaces": workspace(),
            "state": "working",
            "transport": "process",
            "takeover": {"turn_open": true, "turn_count": 7}
        }]
    });

    let rows = rows_from_value(&value);

    assert!(rows[0].can_take_over());
    assert!(rows[0].turn_open);
    assert_eq!(rows[0].turn_count, 7);
}

#[test]
fn parses_stopped_resumable_session_as_searchable_history() {
    let value = serde_json::json!({
        "sessions": [{
            "pubkey": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "npub": "npub1stoppedselector",
            "handle": "juno-codex",
            "agent": "codex",
            "workspaces": workspace(),
            "title": "finished picker work",
            "activity": "tests passed",
            "state": "offline",
            "running": false,
            "resumable": true,
            "last_seen": 40
        }]
    });

    let rows = rows_from_value(&value);

    assert_eq!(rows.len(), 1);
    assert!(!rows[0].running);
    assert!(rows[0].resumable);
    assert_eq!(rows[0].state, SessionState::Offline);
    assert!(rows[0].fuzzy_score("juno").is_some());
}

#[test]
fn malformed_takeover_contract_is_rejected() {
    let value = serde_json::json!({
        "sessions": [{
            "pubkey": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "npub": "npub1publicselector",
            "handle": "echo-codex",
            "agent": "codex",
            "workspaces": workspace(),
            "takeover": {"turn_open": true}
        }]
    });

    assert!(rows_from_value(&value).is_empty());
}

#[test]
fn parses_live_unbound_endpoint_for_attach() {
    let value = serde_json::json!({
        "sessions": [{
            "pubkey": "",
            "npub": "",
            "handle": "codex",
            "agent": "codex",
            "workspaces": workspace(),
            "title": "codex --yolo",
            "activity": "/repo",
            "state": "suspended",
            "last_seen": 0,
            "host": "laptop",
            "harness": "codex",
            "bound": false,
            "endpoint": {"id": "pty-orphan", "kind": "pty", "live": true, "attachable": true, "cwd": "/repo"}
        }]
    });

    let rows = rows_from_value(&value);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].handle, "codex");
    assert_eq!(rows[0].pty_id.as_deref(), Some("pty-orphan"));
    assert!(rows[0].endpoint_live);
}

#[test]
fn acp_transport_rows_are_not_attachable_or_takeover_candidates() {
    let value = serde_json::json!({
        "sessions": [{
            "pubkey": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "npub": "npub1acpselector",
            "handle": "delta-claude",
            "agent": "claude-code",
            "workspaces": workspace(),
            "title": "claude --yolo",
            "activity": "/repo",
            "state": "working",
            "last_seen": 5,
            "host": "laptop",
            "harness": "claude-code",
            "transport": "acp",
            "endpoint": {"id": "acp-1", "kind": "acp", "live": true, "attachable": false}
        }]
    });

    let rows = rows_from_value(&value);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].transport, "acp");
    assert!(!rows[0].attachable(), "ACP rows must not be attachable");
    assert!(!rows[0].can_take_over());
    assert!(rows[0].pty_id.is_none());
}

#[test]
fn equal_state_sessions_sort_by_identity_not_last_seen() {
    let sessions = |alpha_seen, zulu_seen| {
        serde_json::json!({
            "sessions": [
                {
                    "pubkey": "zzzz",
                    "npub": "npub1zulu",
                    "handle": "Zulu-codex",
                    "agent": "codex",
                    "state": "idle",
                    "last_seen": zulu_seen
                },
                {
                    "pubkey": "aaaa",
                    "npub": "npub1alpha",
                    "handle": "alpha-codex",
                    "agent": "codex",
                    "state": "idle",
                    "last_seen": alpha_seen
                }
            ]
        })
    };

    let first = rows_from_value(&sessions(1, 99));
    let refreshed = rows_from_value(&sessions(100, 2));

    assert_eq!(first[0].handle, "alpha-codex");
    assert_eq!(refreshed[0].handle, "alpha-codex");
}
