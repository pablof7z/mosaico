use super::*;

#[test]
fn canonical_strings_and_locators_round_trip() {
    assert_eq!(TransportKind::Pty.as_str(), "pty");
    assert_eq!(TransportKind::Acp.as_str(), "acp");
    assert_eq!(TransportKind::AppServer.as_str(), "app-server");
    assert_eq!(TransportKind::Pty.locator_kind(), crate::state::LOCATOR_PTY);
    assert_eq!(TransportKind::Acp.locator_kind(), crate::state::LOCATOR_ACP);
    assert_eq!(
        TransportKind::AppServer.locator_kind(),
        crate::state::LOCATOR_APP_SERVER
    );
    assert_eq!(
        TransportKind::from_locator_kind(crate::state::LOCATOR_ACP),
        Some(TransportKind::Acp)
    );
    assert_eq!(serde_json::to_value(TransportKind::Acp).unwrap(), "acp");
    assert_eq!(
        serde_json::to_value(TransportKind::AppServer).unwrap(),
        "app-server"
    );
    assert_eq!(
        serde_json::from_value::<TransportKind>(serde_json::json!("pty")).unwrap(),
        TransportKind::Pty
    );
}
