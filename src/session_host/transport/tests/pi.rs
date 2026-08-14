use super::*;

#[test]
fn transport_kind_and_configured_bundle_remain_distinct() {
    let kind = TransportKind::parse("pi-rpc").unwrap();
    assert_eq!(kind.as_str(), "pi-rpc");
    assert_eq!(kind.locator_kind(), crate::state::LOCATOR_PI_RPC);

    let cfg: HarnessesConfig =
        serde_json::from_str(r#"{"pi-rpc":{"harness":"pi","transport":"pi-rpc"}}"#).unwrap();
    assert_eq!(
        select_transport_with(&cfg, "pi-rpc").unwrap().kind(),
        TransportKind::PiRpc
    );
}
