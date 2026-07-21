//! Controlled ACP child coverage for positive delivery/liveness and leak reaping.

use super::acp_delivery_tests::recording_cfg;
use super::*;
use crate::rpc_harness::{Callbacks, Dialect, RpcHandle, SpawnConfig};
use crate::session_host::transport::{
    EndpointRef, PreparedLaunch, ResumeSpec, RpcLaunchSpec, SessionTransport,
};

use super::registry::remove_after_exit_confirmation;

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

fn failing_app_server_spec(cwd: &std::path::Path, descendant_pid: &std::path::Path) -> LaunchSpec {
    let driver = crate::harness::driver::lookup(
        crate::session::Harness::Codex,
        crate::harness::Transport::AppServer,
    )
    .unwrap();
    LaunchSpec {
        slug: "failed-handshake".into(),
        native_agent: None,
        root: "test".into(),
        abs_path: cwd.to_string_lossy().into_owned(),
        group: None,
        ephemeral: false,
        session_name: None,
        base_command: vec![],
        pubkey: "11".repeat(32),
        agent_nsec: "test-nsec".into(),
        prepared: PreparedLaunch {
            pty: Default::default(),
            rpc: Some(RpcLaunchSpec {
                driver,
                argv: vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    "sleep 60 & echo $! > \"$1\"; IFS= read -r line; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-1,\"message\":\"fixture rejection\"}}'; wait".into(),
                    "mosaico-failed-handshake".into(),
                    descendant_pid.to_string_lossy().into_owned(),
                ],
                extra_env: vec![],
                harness: crate::session::Harness::Codex,
            }),
        },
    }
}

async fn assert_failed_handshake_reaps_descendant(resume: bool) {
    let scratch = tempfile::tempdir().unwrap();
    let descendant_pid = scratch.path().join("descendant.pid");
    let spec = failing_app_server_spec(scratch.path(), &descendant_pid);
    let transport = RpcTransport::new(TransportKind::AppServer);
    let result = if resume {
        transport
            .resume(
                &spec,
                &ResumeSpec {
                    native_id: "existing-thread".into(),
                },
            )
            .await
            .map(|_| ())
    } else {
        transport.open(&spec).await.map(|_| ())
    };
    assert!(result.is_err(), "fixture handshake unexpectedly succeeded");
    let descendant = std::fs::read_to_string(descendant_pid)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    assert!(!crate::liveness::pid_alive(descendant));
}

#[tokio::test]
async fn fresh_handshake_failure_reaps_the_unregistered_process_group() {
    assert_failed_handshake_reaps_descendant(false).await;
}

#[tokio::test]
async fn resume_handshake_failure_reaps_the_unregistered_process_group() {
    assert_failed_handshake_reaps_descendant(true).await;
}

#[tokio::test]
async fn failed_exit_confirmation_preserves_registry_ownership() {
    let scratch = tempfile::tempdir().unwrap();
    let capture = scratch.path().join("failed-confirmation.json");
    let (handle, updates) = RpcHandle::spawn(recording_cfg(&capture, Dialect::Acp))
        .await
        .expect("spawn controlled RPC child");
    let endpoint_id = format!("acp-failed-confirmation-{}", std::process::id());
    register_child(
        &endpoint_id,
        handle,
        "native-failed-confirmation".into(),
        scratch.path().to_path_buf(),
        updates,
    );
    let endpoint = EndpointRef {
        kind: TransportKind::Acp,
        endpoint_id: endpoint_id.clone(),
    };
    let transport = RpcTransport::new(TransportKind::Acp);

    let forced = std::io::Error::other("forced kill confirmation failure");
    assert!(remove_after_exit_confirmation(&endpoint_id, Err(forced)).is_err());
    assert!(
        transport.is_live(&endpoint),
        "failed exit confirmation must retain registry ownership"
    );

    transport.kill(&endpoint).await.unwrap();
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
