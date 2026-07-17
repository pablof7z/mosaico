//! Controlled ACP child coverage for positive delivery/liveness and leak reaping.

use super::*;
use crate::rpc_harness::{Callbacks, Dialect, RpcHandle, SpawnConfig};
use crate::session_host::transport::{EndpointRef, SessionTransport};

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

fn recording_cfg(capture: &std::path::Path, dialect: Dialect) -> SpawnConfig {
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
        dialect,
        callbacks: Callbacks::allow_all(cwd),
    }
}

#[tokio::test]
async fn retained_rpc_transports_are_live_deliver_and_kill() {
    let scratch = tempfile::tempdir().unwrap();
    for (kind, dialect, method) in [
        (TransportKind::Acp, Dialect::Acp, "session/prompt"),
        (TransportKind::AppServer, Dialect::AppServer, "turn/start"),
    ] {
        let capture = scratch.path().join(format!("{}.json", kind.as_str()));
        let (handle, updates) = RpcHandle::spawn(recording_cfg(&capture, dialect))
            .await
            .expect("spawn controlled RPC child");
        let endpoint_id = format!("{}-delivery-test-{}", kind.as_str(), std::process::id());
        register_child(
            &endpoint_id,
            handle,
            "native-delivery-test".into(),
            scratch.path().to_path_buf(),
            updates,
        );
        let endpoint = EndpointRef { kind, endpoint_id };
        let transport = RpcTransport::new(kind);

        assert!(transport.is_live(&endpoint));
        let other_kind = match kind {
            TransportKind::Acp => TransportKind::AppServer,
            TransportKind::AppServer => TransportKind::Acp,
            TransportKind::Pty => unreachable!(),
        };
        let wrong_transport = RpcTransport::new(other_kind);
        let wrong_endpoint = EndpointRef {
            kind: other_kind,
            endpoint_id: endpoint.endpoint_id.clone(),
        };
        assert!(!wrong_transport.is_live(&wrong_endpoint));
        assert!(wrong_transport
            .deliver(&wrong_endpoint, "must not cross dialects", true)
            .await
            .is_err());
        assert!(wrong_transport.kill(&wrong_endpoint).await.is_err());
        assert!(transport.is_live(&endpoint));
        transport
            .deliver(&endpoint, "positive RPC delivery", true)
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
        .expect("controlled RPC child did not receive delivery");
        let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
        assert_eq!(request["method"], method);
        assert_eq!(
            request["params"]["sessionId"]
                .as_str()
                .or_else(|| request["params"]["threadId"].as_str()),
            Some("native-delivery-test")
        );
        transport.kill(&endpoint).await.unwrap();
        assert!(!transport.is_live(&endpoint));
    }
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
    assert!(!RpcTransport::new(TransportKind::Acp).is_live(&ep));
}
