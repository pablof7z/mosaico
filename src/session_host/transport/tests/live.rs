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
    LaunchSpec {
        slug: "opencode-acp".into(),
        bundle: "opencode-acp".into(),
        profile: None,
        root: "live".into(),
        abs_path: cwd.to_string_lossy().into_owned(),
        group: None,
        ephemeral: true,
        base_command: vec!["opencode".into()],
        pubkey,
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
    let TransportImpl::Acp(acp) = transport else {
        panic!("expected ACP transport for an acp bundle");
    };
    let endpoint = acp
        .launch(&live_spec(&home, "11".repeat(32)))
        .await
        .expect("launch opencode acp");
    let ep = EndpointRef {
        kind: endpoint.kind,
        endpoint_id: endpoint.endpoint_id.clone(),
    };
    assert_eq!(endpoint.kind, TransportKind::Acp);
    assert!(endpoint.watch_pid.is_some());
    assert!(acp.is_live(&ep));
    acp.kill(&ep).await.unwrap();
    std::env::remove_var("MOSAICO_HOME");
}

#[tokio::test]
#[ignore]
async fn live_acp_agent_receives_delivered_prompt() {
    let Some(home) = live_home("deliver") else {
        return;
    };
    let TransportImpl::Acp(acp) = select_transport("opencode-acp").unwrap() else {
        panic!("expected ACP transport for an acp bundle");
    };
    let endpoint = acp
        .launch(&live_spec(&home, "22".repeat(32)))
        .await
        .expect("launch opencode acp");
    let ep = EndpointRef {
        kind: endpoint.kind,
        endpoint_id: endpoint.endpoint_id.clone(),
    };
    acp.deliver(&ep, "Reply with the single word: pong", true)
        .await
        .expect("deliver prompt to acp child");
    let mut got = String::new();
    for _ in 0..120 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        got = crate::session_host::transport::acp::transcript_snapshot(&endpoint.endpoint_id)
            .unwrap_or_default();
        if !got.trim().is_empty() {
            break;
        }
    }
    acp.kill(&ep).await.unwrap();
    std::env::remove_var("MOSAICO_HOME");
    assert!(!got.trim().is_empty());
}
