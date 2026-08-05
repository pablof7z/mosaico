use super::*;

fn snapshot(stuck: serde_json::Value, stuck_total: u64) -> serde_json::Value {
    serde_json::json!({
        "entries": 4,
        "outstanding": 2,
        "stuck": stuck,
        "stuck_total": stuck_total,
    })
}

#[test]
fn an_empty_queue_is_healthy_and_says_so_in_counts() {
    let mut checks = Vec::new();
    inspect(&snapshot(serde_json::json!([]), 0), &mut checks);
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].name, "write.queue");
    assert_eq!(checks[0].status, CheckStatus::Ok);
    assert!(checks[0].summary.contains("2 write(s) in flight"));
    assert!(checks[0].summary.contains("4 retained"));
}

/// A write nothing will move is a warning, not an error: a person decides
/// whether to attach the signer or drop the obligation, and the daemon is
/// otherwise healthy.
#[test]
fn a_stuck_write_is_a_warning_naming_the_exact_reason() {
    let mut checks = Vec::new();
    inspect(
        &snapshot(
            serde_json::json!([{
                "event_id": "abcdef0123456789",
                "accepted_at": 1_700_000_000u64,
                "reason": "no signer is attached for npubkeyhex",
            }]),
            1,
        ),
        &mut checks,
    );
    assert_eq!(checks[0].status, CheckStatus::Warning);
    assert!(
        checks[0].summary.contains("abcdef"),
        "{}",
        checks[0].summary
    );
    assert!(
        checks[0].summary.contains("no signer is attached"),
        "{}",
        checks[0].summary
    );
    assert!(checks[0].repair.is_some());
}

#[test]
fn a_truncated_list_says_how_many_it_left_out() {
    let mut checks = Vec::new();
    inspect(
        &snapshot(
            serde_json::json!([{
                "event_id": "abcdef0123456789",
                "accepted_at": 1u64,
                "reason": "refused at acceptance: Tombstoned",
            }]),
            9,
        ),
        &mut checks,
    );
    assert!(
        checks[0].summary.contains("and 8 more"),
        "{}",
        checks[0].summary
    );
}

/// An unreadable queue is not an empty queue, and must never be reported as
/// one: the difference is between "you owe nothing" and "we cannot tell".
#[test]
fn an_unreadable_queue_is_an_error_and_never_a_clean_bill() {
    let mut checks = Vec::new();
    inspect(
        &serde_json::json!({
            "entries": 0,
            "outstanding": 0,
            "stuck": [],
            "stuck_total": 0,
            "unreadable": "engine already shut down",
        }),
        &mut checks,
    );
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].status, CheckStatus::Error);
    assert!(checks[0].summary.contains("engine already shut down"));
}
