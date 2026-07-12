//! Leak-reaper coverage (defect #1): a child that self-exits must have its
//! process-global registry entry removed (and its zombie `wait()`ed) without any
//! explicit `kill()`.

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

#[tokio::test]
async fn self_exiting_child_is_reaped_from_registry() {
    let (handle, updates) = RpcHandle::spawn(short_lived_cfg())
        .await
        .expect("spawn short-lived child");
    let endpoint_id = format!("te-acp-test-{}", std::process::id());
    AcpTransport::register_child(
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
