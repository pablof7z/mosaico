use super::*;
use std::io::Write as _;

#[test]
fn extracts_native_id_from_json_lines() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("headless.log");
    let mut file = std::fs::File::create(&log).unwrap();
    writeln!(file, "non-json warning").unwrap();
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "type": "event",
            "payload": {
                "session": { "id": "native-session-1" }
            }
        })
    )
    .unwrap();

    assert_eq!(
        extract_native_session_id(&log).as_deref(),
        Some("native-session-1")
    );
}

#[test]
fn extracts_opencode_session_id_from_ndjson() {
    // Real `opencode run --format json` NDJSON: every line carries `sessionID`
    // (capital ID) at top level; `id`/`messageID` are part/message ids we must
    // NOT mistake for the session.
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("opencode.log");
    let mut file = std::fs::File::create(&log).unwrap();
    writeln!(
        file,
        r#"{{"type":"step_start","sessionID":"ses_0bf752c68ffeZIy7EBgv55kExz","part":{{"id":"prt_f408ae004001","messageID":"msg_f408ad41d001","sessionID":"ses_0bf752c68ffeZIy7EBgv55kExz","type":"step-start"}}}}"#
    )
    .unwrap();

    assert_eq!(
        extract_native_session_id(&log).as_deref(),
        Some("ses_0bf752c68ffeZIy7EBgv55kExz")
    );
}
