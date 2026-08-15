use super::*;

#[test]
fn pi_rpc_transport_kind_is_canonical() {
    let kind = TransportKind::parse("pi-rpc").unwrap();
    assert_eq!(kind.as_str(), "pi-rpc");
    assert_eq!(kind.locator_kind(), crate::state::LOCATOR_PI_RPC);
}

#[test]
fn pi_launches_publish_transport_and_endpoint_correlation() {
    let scratch = tempfile::tempdir().unwrap();

    for (transport, kind) in [
        (Transport::Pty, TransportKind::Pty),
        (Transport::PiRpc, TransportKind::PiRpc),
    ] {
        let endpoint = format!("{}-endpoint", kind.as_str());
        let mut resolved = crate::harness::resolve_with(
            &crate::harness::PresetsConfig::default(),
            crate::session::Harness::Pi,
            transport,
            None,
            None,
            scratch.path(),
        )
        .unwrap();
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
