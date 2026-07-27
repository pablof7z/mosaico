use super::{Check, CheckStatus};

/// Background receipts are historical process-local evidence. They can warn an
/// operator, but only the active checked publish/readback probe establishes
/// current write health.
pub(super) fn inspect(value: &serde_json::Value, checks: &mut Vec<Check>) {
    checks.push(observer_health(value));
    if let Some(failure) = value.get("last_failure").filter(|value| !value.is_null()) {
        checks.push(Check::new(
            "write.background_failure",
            CheckStatus::Warning,
            evidence_summary("observed failure", failure),
        ));
    }
    if let Some(gap) = value.get("last_gap").filter(|value| !value.is_null()) {
        checks.push(Check::new(
            "write.background_gap",
            CheckStatus::Warning,
            evidence_summary("observation gap", gap),
        ));
    }
    if value
        .get("last_failure")
        .is_none_or(serde_json::Value::is_null)
        && value.get("last_gap").is_none_or(serde_json::Value::is_null)
    {
        let summary = value
            .get("last_success")
            .filter(|value| !value.is_null())
            .map(|success| evidence_summary("terminal success", success))
            .unwrap_or_else(|| "no background receipt evidence since daemon start".into());
        checks.push(Check::new("write.background", CheckStatus::Ok, summary));
    }
}

fn observer_health(value: &serde_json::Value) -> Check {
    let pending = value["pending"].as_u64().unwrap_or_default();
    let capacity = value["capacity"].as_u64().unwrap_or_default();
    let workers = value["workers"].as_u64().unwrap_or_default();
    let configured = value["configured_workers"].as_u64().unwrap_or_default();
    let admission_open = value["admission_open"].as_bool().unwrap_or(false);
    let reason = if !admission_open {
        Some("background write admission is closed".to_string())
    } else if capacity == 0 || pending >= capacity {
        Some(format!(
            "background write admission is full ({pending}/{capacity})"
        ))
    } else if configured == 0 || workers < configured {
        Some(format!(
            "background receipt workers unavailable ({workers}/{configured})"
        ))
    } else {
        None
    };
    match reason {
        Some(reason) => Check::new("write.observer", CheckStatus::Error, reason),
        None => Check::new(
            "write.observer",
            CheckStatus::Ok,
            format!(
                "background write admission available ({pending}/{capacity}); workers {workers}/{configured}"
            ),
        ),
    }
}

fn evidence_summary(prefix: &str, value: &serde_json::Value) -> String {
    let operation = value["operation"].as_str().unwrap_or("unknown-operation");
    let source_ref = value["source_ref"].as_str().unwrap_or("unknown-source");
    let target = value["target"].as_str().unwrap_or("unknown-target");
    let status = value["status"].as_str().unwrap_or("unknown-status");
    let detail = value["detail"].as_str().unwrap_or("no detail");
    format!("{prefix}: {operation} {source_ref} {target} [{status}]: {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_failure_is_warning_with_exact_correlated_detail() {
        let detail = "fault=latched durability=absent reopen=required: Previous I/O error occurred";
        let value = serde_json::json!({
            "last_success": null,
            "last_failure": {
                "operation": "status",
                "source_ref": "abc",
                "target": "0:wss://relay.example.com",
                "status": "failed",
                "detail": detail,
                "observed_at": 7
            },
            "last_gap": null
            ,"pending": 0
            ,"capacity": 128
            ,"admission_open": true
            ,"workers": 4
            ,"configured_workers": 4
        });
        let mut checks = Vec::new();
        inspect(&value, &mut checks);

        assert_eq!(checks.len(), 2);
        assert_eq!(checks[1].status, CheckStatus::Warning);
        assert!(checks[1].summary.contains(detail));
        assert!(!checks[1].summary.contains("store_degraded"));
        assert!(!checks[1].summary.contains("member"));
        assert!(!checks[1].summary.contains("admin"));
    }

    #[test]
    fn healthy_terminal_evidence_has_no_warning() {
        let value = serde_json::json!({
            "last_success": {
                "operation": "profile",
                "source_ref": "def",
                "target": "all",
                "status": "acked",
                "detail": "all receipt streams acknowledged",
                "observed_at": 8
            },
            "last_failure": null,
            "last_gap": null,
            "pending": 0,
            "capacity": 128,
            "admission_open": true,
            "workers": 4,
            "configured_workers": 4
        });
        let mut checks = Vec::new();
        inspect(&value, &mut checks);
        assert!(checks.iter().all(|check| check.status == CheckStatus::Ok));
    }

    #[test]
    fn historical_warning_does_not_override_verified_current_health() {
        let verified = serde_json::json!({
            "status": "verified",
            "summary": "ACK (abcdef)"
        });
        let mut checks = vec![
            super::super::probe_check("relay.publish", &verified, "repair"),
            super::super::probe_check("relay.readback", &verified, "repair"),
        ];
        inspect(
            &serde_json::json!({
                "last_failure": {
                    "operation": "status",
                    "source_ref": "abc",
                    "target": "0:wss://relay.example.com",
                    "status": "failed",
                    "detail": "Previous I/O error occurred",
                    "observed_at": 7
                },
                "last_success": null,
                "last_gap": null,
                "pending": 0,
                "capacity": 128,
                "admission_open": true,
                "workers": 4,
                "configured_workers": 4
            }),
            &mut checks,
        );

        assert_eq!(checks[0].status, CheckStatus::Ok);
        assert_eq!(checks[1].status, CheckStatus::Ok);
        assert_eq!(checks[3].status, CheckStatus::Warning);
        assert!(!checks
            .iter()
            .any(|check| check.status == CheckStatus::Error));
    }

    #[test]
    fn current_capacity_failure_is_error_and_recovery_returns_to_ok() {
        let mut value = serde_json::json!({
            "last_success": null,
            "last_failure": null,
            "last_gap": {
                "operation": "status",
                "source_ref": "full",
                "target": "admission",
                "status": "capacity_full",
                "detail": "insufficient capacity",
                "observed_at": 9
            },
            "pending": 2,
            "capacity": 2,
            "admission_open": true,
            "workers": 1,
            "configured_workers": 1
        });
        let mut checks = Vec::new();
        inspect(&value, &mut checks);
        assert_eq!(checks[0].name, "write.observer");
        assert_eq!(checks[0].status, CheckStatus::Error);
        assert!(
            checks
                .iter()
                .any(|check| check.status == CheckStatus::Error),
            "current capacity failure must make the doctor report unhealthy"
        );

        value["pending"] = serde_json::json!(0);
        let mut recovered = Vec::new();
        inspect(&value, &mut recovered);
        assert_eq!(recovered[0].status, CheckStatus::Ok);
        assert_eq!(
            recovered[1].status,
            CheckStatus::Warning,
            "historical capacity evidence remains a warning after current recovery"
        );
    }

    #[test]
    fn closed_admission_and_missing_workers_are_current_errors() {
        let mut value = serde_json::json!({
            "pending": 0,
            "capacity": 128,
            "admission_open": false,
            "workers": 4,
            "configured_workers": 4,
            "last_success": null,
            "last_failure": null,
            "last_gap": null
        });
        let mut closed = Vec::new();
        inspect(&value, &mut closed);
        assert_eq!(closed[0].status, CheckStatus::Error);
        assert!(closed[0].summary.contains("closed"));

        value["admission_open"] = serde_json::json!(true);
        value["workers"] = serde_json::json!(3);
        let mut missing = Vec::new();
        inspect(&value, &mut missing);
        assert_eq!(missing[0].status, CheckStatus::Error);
        assert!(missing[0].summary.contains("3/4"));
    }
}
