use super::*;

#[test]
fn tmux_spawn_gets_slow_response_budget() {
    assert!(method_policy("tmux_spawn").response_timeout > Duration::from_secs(20));
    assert_eq!(
        method_policy("ping").response_timeout,
        DEFAULT_RESPONSE_IO_TIMEOUT
    );
}

#[test]
fn tmux_spawn_slow_budget_does_not_make_it_retryable_after_delivery() {
    let policy = method_policy("tmux_spawn");
    assert!(policy.response_timeout > Duration::from_secs(20));
    assert!(!policy.retry_after_delivery);
}

#[test]
fn non_idempotent_method_is_not_retried_after_request_may_have_delivered() {
    let mut attempts = 0;
    let mut spawns = 0;
    let err = call_with_attempt(
        "tmux_send",
        &serde_json::json!({}),
        |_, _| {
            attempts += 1;
            Err(TryCallFailure::after_request(anyhow!("timed out")))
        },
        || {
            spawns += 1;
            Ok(())
        },
    )
    .expect_err("non-idempotent ambiguous calls must fail");

    assert_eq!(attempts, 1);
    assert_eq!(spawns, 0);
    let msg = err.to_string();
    assert!(msg.contains("may have been processed"), "{msg}");
    assert!(msg.contains("Not retrying automatically"), "{msg}");
}

#[test]
fn idempotent_method_can_retry_after_request_may_have_delivered() {
    let mut attempts = 0;
    let mut spawns = 0;
    let value = call_with_attempt(
        "who",
        &serde_json::json!({}),
        |_, _| {
            attempts += 1;
            if attempts == 1 {
                Err(TryCallFailure::after_request(anyhow!("timed out")))
            } else {
                Ok(Outcome::Ok(serde_json::json!({"ok": true})))
            }
        },
        || {
            spawns += 1;
            Ok(())
        },
    )
    .expect("idempotent ambiguous call may retry");

    assert_eq!(attempts, 2);
    assert_eq!(spawns, 1);
    assert_eq!(value, serde_json::json!({"ok": true}));
}

#[test]
fn pre_request_failure_can_retry_non_idempotent_method() {
    let mut attempts = 0;
    let mut spawns = 0;
    let value = call_with_attempt(
        "tmux_send",
        &serde_json::json!({}),
        |_, _| {
            attempts += 1;
            if attempts == 1 {
                Err(TryCallFailure::before(anyhow!("connection refused")))
            } else {
                Ok(Outcome::Ok(serde_json::json!({"sent": true})))
            }
        },
        || {
            spawns += 1;
            Ok(())
        },
    )
    .expect("pre-request failure can retry");

    assert_eq!(attempts, 2);
    assert_eq!(spawns, 1);
    assert_eq!(value, serde_json::json!({"sent": true}));
}

#[test]
fn no_spawn_call_returns_pre_request_failure_without_retry() {
    let mut attempts = 0;
    let err = call_no_spawn_with_attempt("statusline", &serde_json::json!({}), |_, _| {
        attempts += 1;
        Err(TryCallFailure::before(anyhow!("connection refused")))
    })
    .expect_err("no-spawn call should surface the connection failure");

    assert_eq!(attempts, 1);
    assert!(err.to_string().contains("connection refused"));
}
