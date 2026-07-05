use super::*;

fn enqueue_fact() -> InputFact {
    InputFact::OutboxEnqueueApplied {
        local_id: 7,
        event_id: "ev7".into(),
        event_hash: "sha256:event".into(),
        source_surface: "status".into(),
        source_ref: "status/s1#tx:1".into(),
        at: 100,
    }
}

#[test]
fn enqueue_tracks_pending_outbox_row() {
    let mut r = OutboxReconciler::new();
    let out = r.drive(enqueue_fact()).unwrap();
    r.assert_oracle().unwrap();

    assert_eq!(out.effects, vec![OutboxEffect::None]);
    let rows = r.state_rows();
    assert_eq!(rows[0].local_id, 7);
    assert_eq!(rows[0].state, "pending");
    assert_eq!(rows[0].source_ref, "status/s1#tx:1");
}

#[test]
fn relay_acceptance_marks_published_from_graph() {
    let mut r = OutboxReconciler::new();
    r.drive(enqueue_fact()).unwrap();

    let out = r
        .drive(InputFact::RelayPublishAccepted {
            local_id: 7,
            event_id: "ev7".into(),
            accepted: true,
            error: None,
            at: 120,
        })
        .unwrap();

    assert_eq!(
        out.effects,
        vec![OutboxEffect::MarkPublished { local_id: 7 }]
    );
    assert_eq!(r.state_rows()[0].state, "published");
}

#[test]
fn relay_failure_keeps_pending_and_bumps_retry() {
    let mut r = OutboxReconciler::new();
    r.drive(enqueue_fact()).unwrap();

    let out = r
        .drive(InputFact::RelayPublishAccepted {
            local_id: 7,
            event_id: "ev7".into(),
            accepted: false,
            error: Some("relay rejected".into()),
            at: 120,
        })
        .unwrap();

    assert_eq!(
        out.effects,
        vec![OutboxEffect::MarkFailed {
            local_id: 7,
            error: "relay rejected".into(),
        }]
    );
    let row = &r.state_rows()[0];
    assert_eq!(row.state, "pending");
    assert_eq!(row.retries, 1);
    assert_eq!(row.last_error.as_deref(), Some("relay rejected"));
}

#[test]
fn preview_does_not_mutate_outbox_graph() {
    let mut r = OutboxReconciler::new();
    let preview = r.preview_fact(&enqueue_fact()).unwrap().unwrap();

    assert_eq!(r.revision(), 0);
    assert!(r.state_rows().is_empty());
    assert_eq!(preview.result.resource_plan.commands().len(), 1);
}
