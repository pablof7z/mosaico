//! Controlled ACP child coverage for positive delivery/liveness and leak reaping.

use super::*;
use crate::rpc_harness::{Callbacks, Dialect, RpcHandle, SpawnConfig};

fn short_lived_cfg() -> SpawnConfig {
    let cwd = std::env::temp_dir();
    SpawnConfig {
        // Exits ~immediately, closing stdout -> reader EOF -> exit signal.
        program: "sh".into(),
        args: vec!["-c".into(), "exit 0".into()],
        cwd: cwd.clone(),
        env: vec![],
        env_remove: vec![],
        dialect: Dialect::Acp,
        callbacks: Callbacks::allow_all(cwd),
    }
}

fn recording_cfg(capture: &std::path::Path) -> SpawnConfig {
    let cwd = std::env::temp_dir();
    SpawnConfig {
        program: "sh".into(),
        args: vec![
            "-c".into(),
            r#"IFS= read -r line || exit 1
printf '%s\n' "$line" > "$1"
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}'
while IFS= read -r line; do :; done"#
                .into(),
            "mosaico-acp-fixture".into(),
            capture.to_string_lossy().into_owned(),
        ],
        cwd: cwd.clone(),
        env: vec![],
        env_remove: vec![],
        dialect: Dialect::Acp,
        callbacks: Callbacks::allow_all(cwd),
    }
}

#[tokio::test]
async fn registered_acp_child_is_live_and_receives_delivery() {
    let scratch = tempfile::tempdir().unwrap();
    let capture = scratch.path().join("request.json");
    let (handle, updates) = RpcHandle::spawn(recording_cfg(&capture))
        .await
        .expect("spawn controlled ACP child");
    let endpoint_id = format!("acp-delivery-test-{}", std::process::id());
    register_child(
        &endpoint_id,
        handle,
        "native-delivery-test".into(),
        scratch.path().to_path_buf(),
        updates,
    );
    let endpoint = EndpointRef {
        kind: TransportKind::Acp,
        endpoint_id,
    };

    assert!(AcpTransport.is_live(&endpoint));
    AcpTransport
        .deliver(&endpoint, "positive ACP delivery", true)
        .await
        .unwrap();

    let request = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Ok(bytes) = std::fs::read(&capture) {
                break bytes;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("controlled ACP child did not receive delivery");
    let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
    assert_eq!(request["method"], "session/prompt");
    assert_eq!(request["params"]["sessionId"], "native-delivery-test");
    assert_eq!(
        request["params"]["prompt"][0]["text"],
        "positive ACP delivery"
    );
    AcpTransport.kill(&endpoint).await.unwrap();
    assert!(!AcpTransport.is_live(&endpoint));
}

#[tokio::test]
async fn self_exiting_child_is_reaped_from_registry() {
    let (handle, updates) = RpcHandle::spawn(short_lived_cfg())
        .await
        .expect("spawn short-lived child");
    let endpoint_id = format!("acp-test-{}", std::process::id());
    register_child(
        &endpoint_id,
        handle,
        "native-test".into(),
        std::env::temp_dir(),
        updates,
    );

    // The reaper should drop the entry once the child exits. Poll briefly.
    let mut reaped = false;
    for _ in 0..100 {
        if registry().lock().unwrap().get(&endpoint_id).is_none() {
            reaped = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        reaped,
        "self-exiting child left a leaked registry entry for {endpoint_id}"
    );

    let ep = EndpointRef {
        kind: TransportKind::Acp,
        endpoint_id,
    };
    assert!(
        !AcpTransport.is_live(&ep),
        "reaped endpoint must not be live"
    );
}
