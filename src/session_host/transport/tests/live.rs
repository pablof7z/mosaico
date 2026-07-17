//! Opt-in live ACP checks requiring a configured `opencode` runtime.

use super::*;

fn live_home(label: &str) -> Option<std::path::PathBuf> {
    if std::env::var("MOSAICO_RPC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live test (set MOSAICO_RPC_LIVE=1)");
        return None;
    }
    let home = std::env::temp_dir().join(format!("acp-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("MOSAICO_HOME", &home);
    std::fs::write(
        home.join("harnesses.json"),
        r#"{ "opencode-acp": { "harness": "opencode", "transport": "acp" } }"#,
    )
    .unwrap();
    Some(home)
}

fn live_spec(home: &std::path::Path, pubkey: String) -> LaunchSpec {
    let cwd = home.join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let cfg = crate::harness::HarnessesConfig::load().unwrap();
    let mut resolved =
        crate::harness::resolve_with(&cfg, "opencode-acp", None, &home.join("profile")).unwrap();
    let prepared = RpcTransport::new(TransportKind::Acp)
        .prepare_launch(&mut resolved, "live-endpoint".into())
        .unwrap();
    LaunchSpec {
        slug: "opencode-acp".into(),
        native_agent: None,
        root: "live".into(),
        abs_path: cwd.to_string_lossy().into_owned(),
        group: None,
        ephemeral: true,
        session_name: None,
        base_command: vec!["opencode".into()],
        pubkey,
        agent_nsec: "test-agent-nsec".into(),
        prepared,
    }
}

#[tokio::test]
#[ignore]
async fn live_launch_dispatch_spawns_opencode_acp() {
    let Some(home) = live_home("launch") else {
        return;
    };
    let transport = select_transport("opencode-acp").unwrap();
    assert_eq!(transport.kind(), TransportKind::Acp);
    let endpoint = transport
        .launch(&live_spec(&home, "11".repeat(32)))
        .await
        .expect("launch opencode acp");
    let ep = EndpointRef {
        kind: endpoint.kind,
        endpoint_id: endpoint.endpoint_id.clone(),
    };
    assert_eq!(endpoint.kind, TransportKind::Acp);
    assert!(endpoint.watch_pid.is_some());
    assert!(transport.is_live(&ep));
    transport.kill(&ep).await.unwrap();
    std::env::remove_var("MOSAICO_HOME");
}

#[tokio::test]
#[ignore]
async fn live_acp_agent_receives_delivered_prompt() {
    let Some(home) = live_home("deliver") else {
        return;
    };
    let transport = select_transport("opencode-acp").unwrap();
    let endpoint = transport
        .launch(&live_spec(&home, "22".repeat(32)))
        .await
        .expect("launch opencode acp");
    let ep = EndpointRef {
        kind: endpoint.kind,
        endpoint_id: endpoint.endpoint_id.clone(),
    };
    transport
        .deliver(&ep, "Reply with the single word: pong", true)
        .await
        .expect("deliver prompt to acp child");
    transport.kill(&ep).await.unwrap();
    std::env::remove_var("MOSAICO_HOME");
}
