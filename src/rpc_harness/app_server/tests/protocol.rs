use super::super::outcome::MAX_ERROR_CHARS;
use super::super::turn_protocol::{parse_completed, parse_started, ObservedTurn, TurnBaseline};
use super::super::TurnOutcome;

fn completed_params(status: &str) -> serde_json::Value {
    serde_json::json!({
        "threadId": "thread-1",
        "turn": {
            "id": "turn-1",
            "items": [],
            "status": status
        }
    })
}

#[test]
fn current_terminal_statuses_are_typed_and_errors_are_bounded() {
    let completed = parse_completed("thread-1", "turn-1", completed_params("completed")).unwrap();
    assert!(matches!(
        completed,
        ObservedTurn::Terminal(TurnOutcome::Completed { .. })
    ));

    let mut failed = completed_params("failed");
    failed["turn"]["error"] = serde_json::json!({
        "message": format!("  unsupported\nmodel {}", "x".repeat(600)),
        "additionalDetails": "  provider\trefused  "
    });
    let failed = parse_completed("thread-1", "turn-1", failed).unwrap();
    let ObservedTurn::Terminal(TurnOutcome::Failed {
        error: Some(error), ..
    }) = failed
    else {
        panic!("expected typed failed outcome")
    };
    assert!(error.message.starts_with("unsupported model "));
    assert_eq!(error.message.chars().count(), MAX_ERROR_CHARS);
    assert_eq!(
        error.additional_details.as_deref(),
        Some("provider refused")
    );

    let interrupted =
        parse_completed("thread-1", "turn-1", completed_params("interrupted")).unwrap();
    assert!(matches!(
        interrupted,
        ObservedTurn::Terminal(TurnOutcome::Interrupted { .. })
    ));
}

#[test]
fn in_progress_completion_and_unknown_status_are_never_success() {
    assert!(matches!(
        parse_completed("thread-1", "turn-1", completed_params("inProgress")).unwrap(),
        ObservedTurn::InProgress
    ));
    let error = parse_completed("thread-1", "turn-1", completed_params("legacySuccess"))
        .err()
        .expect("unknown status must fail current-schema decoding");
    assert!(error.to_string().contains("current app-server schema"));

    let baseline = TurnBaseline::from(["turn-1".to_string()]);
    let error = parse_started(
        "thread-1",
        &baseline,
        serde_json::json!({
            "turn": {"id":"turn-1","items":[],"status":"inProgress"}
        }),
    )
    .err()
    .expect("turn/start cannot return a pre-existing turn");
    assert!(error.to_string().contains("pre-existing turn"));
}
