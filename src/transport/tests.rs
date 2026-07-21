use super::*;

// ── assert_relay_accepted unit tests ────────────────────────────────────────

fn output_with(success: &[&str], failed: &[(&str, &str)]) -> Output<EventId> {
    let mut out: Output<EventId> = Output {
        val: EventId::all_zeros(),
        success: Default::default(),
        failed: Default::default(),
    };
    for url in success {
        out.success.insert(RelayUrl::parse(url).unwrap());
    }
    for (url, reason) in failed {
        out.failed
            .insert(RelayUrl::parse(url).unwrap(), reason.to_string());
    }
    out
}

#[test]
fn accepted_when_any_relay_succeeds() {
    let out = output_with(&["wss://ok.relay"], &[("wss://bad.relay", "blocked")]);
    assert!(assert_relay_accepted(&out).is_ok());
}

#[test]
fn duplicate_response_counts_as_already_accepted() {
    let out = output_with(&[], &[("wss://ok.relay", "duplicate: already have event")]);
    assert!(assert_relay_accepted(&out).is_ok());
}

#[test]
fn rejected_surfaces_relay_reason() {
    let out = output_with(&[], &[("wss://nip29.relay", "blocked: unknown member")]);
    let err = assert_relay_accepted(&out).unwrap_err().to_string();
    assert!(err.contains("blocked: unknown member"), "got: {err}");
}

#[test]
fn no_accept_no_reason_reports_timeout() {
    // Every relay silent: send_event resolved Ok but no OK,true ever arrived.
    let out = output_with(&[], &[]);
    let err = assert_relay_accepted(&out).unwrap_err().to_string();
    assert!(err.contains("no relay accepted"), "got: {err}");
}

#[test]
fn doctor_probe_marker_is_not_a_nonexistent_group() {
    let marker = "mosaico-doctor-test";
    let event = doctor_probe_builder(marker)
        .unwrap()
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let tags = serde_json::to_value(event.tags).unwrap();
    let tags = tags.as_array().unwrap();

    assert!(tags
        .iter()
        .any(|tag| tag == &serde_json::json!(["t", marker])));
    assert!(!tags
        .iter()
        .any(|tag| tag == &serde_json::json!(["h", marker])));
}
