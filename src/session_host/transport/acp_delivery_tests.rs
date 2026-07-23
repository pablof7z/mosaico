//! Controlled ACP child coverage for delivery and active-turn steering.

use super::*;
use crate::rpc_harness::{Callbacks, Dialect, RpcHandle, SpawnConfig};
use crate::session_host::transport::{DeliveryCompletion, EndpointRef, SessionTransport};

pub(super) fn recording_cfg(capture: &std::path::Path, dialect: Dialect) -> SpawnConfig {
    let cwd = std::env::temp_dir();
    SpawnConfig {
        program: "sh".into(),
        args: vec![
            "-c".into(),
            r#"IFS= read -r line || exit 1
printf '%s\n' "$line" > "$1.tmp"
mv "$1.tmp" "$1"
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}'
printf '%s\n' '{"jsonrpc":"2.0","method":"turn/completed","params":{}}'
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
        let pid = i32::try_from(handle.pid.expect("controlled child pid")).unwrap();
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
        let completion = transport
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
        let DeliveryCompletion::Managed(completion) = completion else {
            panic!("RPC delivery must return daemon-owned turn completion");
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), completion)
            .await
            .expect("managed RPC turn did not complete")
            .expect("managed completion sender dropped")
            .expect("managed RPC turn failed");
        transport.kill(&endpoint).await.unwrap();
        assert!(!transport.is_live(&endpoint));
        assert!(
            !crate::liveness::pid_alive(pid),
            "kill returned before RPC child {pid} exited"
        );
    }
}

#[tokio::test]
async fn submitted_app_server_delivery_steers_an_active_turn() {
    let scratch = tempfile::tempdir().unwrap();
    let capture = scratch.path().join("steer.json");
    let cfg = recording_cfg(&capture, Dialect::AppServer);
    let (handle, updates) = RpcHandle::spawn(cfg).await.unwrap();
    let endpoint_id = format!("active-app-server-{}", std::process::id());
    register_child(
        &endpoint_id,
        handle,
        "thread-active".into(),
        scratch.path().to_path_buf(),
        updates,
    );
    {
        let registry = registry().lock().unwrap();
        let child = registry.get(&endpoint_id).unwrap();
        child
            .runtime
            .lock()
            .unwrap()
            .note_update("turn/started", &serde_json::json!({"turnId":"turn-active"}));
    }

    let transport = RpcTransport::new(TransportKind::AppServer);
    let endpoint = EndpointRef {
        kind: TransportKind::AppServer,
        endpoint_id: endpoint_id.clone(),
    };
    let completion = transport
        .deliver(&endpoint, "mid-turn human message", true)
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
    .expect("active app-server did not receive steer delivery");
    let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
    assert_eq!(request["method"], "turn/steer");
    assert_eq!(request["params"]["expectedTurnId"], "turn-active");
    let DeliveryCompletion::ManagedSteer(accepted) = completion else {
        panic!("active app-server steer must report its acknowledgement");
    };
    accepted
        .await
        .expect("app-server steer confirmation sender dropped")
        .expect("app-server steer was not accepted");

    transport.kill(&endpoint).await.unwrap();
}
