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

#[test]
fn pi_launches_publish_transport_and_endpoint_correlation() {
    let cfg: HarnessesConfig = serde_json::from_str(
        r#"{
          "pi-pty":{"harness":"pi","transport":"pty"},
          "pi-rpc":{"harness":"pi","transport":"pi-rpc"}
        }"#,
    )
    .unwrap();
    let scratch = tempfile::tempdir().unwrap();

    for (bundle, kind) in [
        ("pi-pty", TransportKind::Pty),
        ("pi-rpc", TransportKind::PiRpc),
    ] {
        let endpoint = format!("{bundle}-endpoint");
        let mut resolved =
            crate::harness::resolve_with(&cfg, bundle, None, scratch.path()).unwrap();
        let prepared = transport_for_kind(kind)
            .prepare_launch(&mut resolved, endpoint.clone())
            .unwrap();
        let env = prepared
            .rpc
            .as_ref()
            .map(|rpc| &rpc.extra_env)
            .unwrap_or(&prepared.pty.env);
        let value = |key: &str| {
            env.iter()
                .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
        };
        assert_eq!(value("MOSAICO_OBSERVED_HARNESS"), Some("pi"));
        assert_eq!(value("MOSAICO_TRANSPORT"), Some(kind.as_str()));
        assert_eq!(value("MOSAICO_ENDPOINT_ID"), Some(endpoint.as_str()));
    }
}
