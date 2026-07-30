use super::*;
use crate::session_host::transport::{
    DeliveryCompletion, EndpointRef, SessionEndpoint, SessionTransport, TransportKind,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

struct MismatchedEndpointTransport {
    killed: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl SessionTransport for MismatchedEndpointTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Pty
    }

    async fn launch(&self, spec: &LaunchSpec) -> Result<SessionEndpoint> {
        Ok(SessionEndpoint {
            kind: TransportKind::Acp,
            endpoint_id: "opened-but-invalid".into(),
            watch_pid: None,
            native_id: None,
            meta: crate::pty::LaunchMetadata {
                id: "opened-but-invalid".into(),
                socket: String::new(),
                supervisor_pid: 0,
                instance_token: String::new(),
                child_pid: None,
                agent: spec.slug.clone(),
                root: spec.root.clone(),
                cwd: spec.abs_path.clone(),
                ephemeral: spec.ephemeral,
                command: spec.base_command.clone(),
            },
        })
    }

    async fn resume(&self, spec: &LaunchSpec, _resume: &ResumeSpec) -> Result<SessionEndpoint> {
        self.launch(spec).await
    }

    async fn deliver(
        &self,
        _endpoint: &EndpointRef,
        _text: &str,
        _submit: bool,
    ) -> Result<DeliveryCompletion> {
        unreachable!("rollback test never delivers")
    }

    fn is_live(&self, _endpoint: &EndpointRef) -> bool {
        true
    }

    async fn kill(&self, _endpoint: &EndpointRef) -> Result<()> {
        self.killed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn prepare(
    base: &[&str],
    resume: ResumeMechanism,
    conversation: ConversationOpen<'_>,
    extra_args: &[&str],
) -> (Vec<String>, crate::session_host::transport::PreparedLaunch) {
    let command: Vec<String> = base.iter().map(|arg| (*arg).to_string()).collect();
    let rpc_argv = command.clone();
    let prepared = crate::session_host::transport::PreparedLaunch {
        rpc: Some(crate::session_host::transport::RpcLaunchSpec {
            driver: crate::harness::driver::lookup(
                crate::session::Harness::Codex,
                crate::harness::Transport::AppServer,
            )
            .unwrap(),
            argv: rpc_argv,
            extra_env: Vec::new(),
            harness: crate::session::Harness::Codex,
        }),
        ..Default::default()
    };
    prepare_commands(
        command,
        prepared,
        resume,
        conversation,
        &extra_args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>(),
        "test-agent",
    )
    .unwrap()
}

#[test]
fn fresh_extra_args_reach_pty_and_rpc_commands() {
    let (command, prepared) = prepare(
        &["codex", "app-server"],
        ResumeMechanism::None,
        ConversationOpen::Fresh,
        &["--yolo", "--model=x"],
    );

    let expected = ["codex", "app-server", "--yolo", "--model=x"];
    assert_eq!(command, expected);
    assert_eq!(prepared.rpc.unwrap().argv, expected);
}

#[test]
fn claude_resume_appends_the_native_id_after_bundle_args() {
    let (command, prepared) = prepare(
        &["claude", "--dangerously-skip-permissions"],
        ResumeMechanism::AppendFlag("--resume"),
        ConversationOpen::Resume {
            native_id: "02ff0867-a7bb-4254-a36e-37081ccc3b51",
        },
        &["--model", "sonnet"],
    );

    assert_eq!(
        command,
        [
            "claude",
            "--dangerously-skip-permissions",
            "--resume",
            "02ff0867-a7bb-4254-a36e-37081ccc3b51",
            "--model",
            "sonnet",
        ]
    );
    assert_eq!(
        prepared.rpc.unwrap().argv,
        [
            "claude",
            "--dangerously-skip-permissions",
            "--model",
            "sonnet",
        ]
    );
}

#[test]
fn goose_resume_appends_both_required_flags() {
    let (command, _) = prepare(
        &["goose", "session"],
        ResumeMechanism::AppendFlags(&["--resume", "--session-id"]),
        ConversationOpen::Resume {
            native_id: "20260721_9",
        },
        &["--debug"],
    );

    assert_eq!(
        command,
        [
            "goose",
            "session",
            "--resume",
            "--session-id",
            "20260721_9",
            "--debug",
        ]
    );
}

#[test]
fn codex_resume_inserts_the_subcommand_before_bundle_args() {
    let (command, _) = prepare(
        &["codex", "--profile", "writer"],
        ResumeMechanism::Subcommand("resume"),
        ConversationOpen::Resume {
            native_id: "019f7f5c-575d-7640-958d-e7428d4d77b0",
        },
        &["--yolo"],
    );

    assert_eq!(
        command,
        [
            "codex",
            "resume",
            "019f7f5c-575d-7640-958d-e7428d4d77b0",
            "--profile",
            "writer",
            "--yolo",
        ]
    );
}

#[tokio::test]
async fn bootstrap_failure_kills_the_endpoint_and_releases_the_reservation() {
    let state = DaemonState::new_for_test().await;
    let identity = crate::identity::AgentIdentity::per_session("codex", "codex-pty");
    let reservation = admission::reserve_fresh(
        &state,
        &identity,
        "codex",
        "codex-pty",
        "pty",
        "root",
        Some("root"),
        None,
    )
    .unwrap();
    let pubkey = reservation.pubkey.clone();
    let killed = Arc::new(AtomicBool::new(false));
    let source = ResolvedSource {
        transport: crate::session_host::transport::TransportImpl::for_test(
            MismatchedEndpointTransport {
                killed: killed.clone(),
            },
        ),
        command: vec!["fake-codex".into()],
        harness: crate::session::Harness::Codex,
        resume: ResumeMechanism::None,
        bundle: "codex-pty".into(),
        native_agent: None,
        identity,
        prepared_launch: Default::default(),
    };
    let channels = vec!["root".to_string()];

    let error = open(
        &state,
        HostedOpenRequest {
            source,
            reservation,
            conversation: ConversationOpen::Fresh,
            placement: HostedPlacement {
                root: "root",
                abs_path: "/tmp",
                group: Some("root"),
                channels: &channels,
            },
            presentation: HostedPresentation {
                ephemeral: false,
                session_name: None,
                dispatch_event: None,
            },
            extra_args: &[],
        },
    )
    .await
    .err()
    .expect("mismatched endpoint kind must reject registration");

    assert!(
        error.to_string().contains("registering hosted session"),
        "{error:#}"
    );
    assert!(killed.load(Ordering::SeqCst));
    let session = state
        .with_store(|store| store.get_session(&pubkey))
        .unwrap()
        .unwrap();
    assert!(!session.is_running());
}
