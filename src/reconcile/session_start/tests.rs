use super::*;
use crate::reconcile::InputFact;

fn request(already_running: bool, ordinal: u32) -> InputFact {
    InputFact::SessionStartRequested(crate::reconcile::SessionStartRequestFact {
        session_id: "s1".into(),
        agent: "coder".into(),
        harness: "codex".into(),
        external_id_kind: "harness_session".into(),
        external_id: "native-1".into(),
        native_id: "native-1".into(),
        work_root: "/repo".into(),
        channel_h: "room".into(),
        channel_for_upsert: if already_running {
            "old-room".into()
        } else {
            "room".into()
        },
        rel_cwd: ".".into(),
        room_parent: None,
        watch_pid: Some(42),
        tmux_pane: Some("%1".into()),
        ring_doorbell: true,
        base_pubkey: "base".into(),
        signer_pubkey: if ordinal > 0 {
            "ord".into()
        } else {
            "base".into()
        },
        signer_label: if ordinal > 0 {
            "coder1".into()
        } else {
            "coder".into()
        },
        signer_ordinal: ordinal,
        already_running,
        channel_already_subscribed: true,
        at: 100,
    })
}

#[test]
fn request_derives_execute_plan() {
    let mut r = SessionStartReconciler::new();
    let out = r.drive(request(false, 1)).unwrap();
    let cmd = out.command.unwrap();
    assert_eq!(cmd.action, SessionStartAction::Execute);
    assert_eq!(cmd.plan.row.agent_pubkey, "ord");
    assert!(cmd.plan.admit_pubkey.is_none());
    assert_eq!(
        cmd.plan
            .channel_ready
            .as_ref()
            .map(|ready| ready.signer_pubkey.as_str()),
        Some("ord")
    );
    assert!(cmd.plan.ensure_subscription);
    assert!(cmd.plan.replay_chat);
    assert!(cmd.plan.spawn.is_some());
    r.assert_oracle().unwrap();
}

#[test]
fn reassert_suppresses_effects_after_row_and_tmux() {
    let mut r = SessionStartReconciler::new();
    let out = r.drive(request(true, 0)).unwrap();
    let cmd = out.command.unwrap();
    assert_eq!(cmd.action, SessionStartAction::Reassert);
    assert_eq!(cmd.plan.row.channel_h, "old-room");
    assert!(cmd.plan.tmux_pane.is_some());
    assert!(cmd.plan.ring_doorbell);
    assert!(cmd.plan.channel_ready.is_none());
    assert!(cmd.plan.spawn.is_none());
    assert!(!cmd.plan.notify_outbox);
}

#[test]
fn started_and_failed_round_trip_after_request() {
    let mut r = SessionStartReconciler::new();
    r.drive(request(false, 0)).unwrap();
    let started = r
        .drive(InputFact::SessionStarted {
            session_id: "s1".into(),
            channel_h: Some("room".into()),
            agent_pubkey: Some("base".into()),
            pid: Some(42),
            at: 101,
        })
        .unwrap()
        .command
        .unwrap();
    assert_eq!(started.action, SessionStartAction::RecordStarted);

    let failed = r
        .drive(InputFact::SessionStartFailed(
            crate::reconcile::SessionStartFailedFact {
                session_id: "s1".into(),
                stage: "spawn".into(),
                error: "boom".into(),
                at: 102,
            },
        ))
        .unwrap()
        .command
        .unwrap();
    assert_eq!(failed.action, SessionStartAction::RecordFailed);
    assert_eq!(failed.failure_stage.as_deref(), Some("spawn"));
    assert_eq!(failed.failure_error.as_deref(), Some("boom"));
}

#[test]
fn preview_does_not_mutate_state() {
    let mut r = SessionStartReconciler::new();
    let fact = request(false, 0);
    let preview = r.preview_fact(&fact).unwrap().unwrap();
    assert_eq!(preview.result.resource_plan.commands().len(), 1);
    assert!(r.state_rows().is_empty());
    r.drive(fact).unwrap();
    assert_eq!(r.state_rows().len(), 1);
}

#[test]
fn replay_dispatch_accepts_request_and_started() {
    let mut script = trellis_testing::DataTransactionScript::new();
    script.step("request").operation(request(false, 0)).commit();
    script
        .step("started")
        .operation(InputFact::SessionStarted {
            session_id: "s1".into(),
            channel_h: Some("room".into()),
            agent_pubkey: Some("base".into()),
            pid: Some(42),
            at: 101,
        })
        .commit();

    let report = crate::reconcile::replay::replay_script(&script, false).unwrap();
    assert_eq!(report.surface, "session_start");
    assert_eq!(report.steps, 2);
    assert_eq!(report.resource_commands, 2);
}
