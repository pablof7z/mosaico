use super::*;

fn snapshot(state: SessionState, title: &str) -> PresenceSnapshot {
    PresenceSnapshot {
        host: "laptop".into(),
        slug: "coder".into(),
        rel_cwd: ".".into(),
        dispatch_event: None,
        projection: projection(state, title),
    }
}

fn projection(state: SessionState, title: &str) -> PresenceProjection {
    PresenceProjection {
        channels: BTreeSet::from(["room".into()]),
        state,
        state_since: 5,
        title: title.into(),
    }
}
fn seeded(generation: u64, state: SessionState) -> StatusReconciler {
    let mut policy = StatusReconciler::new(90, 30);
    let out = policy.open("pk1", generation, snapshot(state, "Task"), 0);
    assert_eq!(out.effects.len(), 1);
    policy
}

fn published(effects: &[StatusEffect]) -> Option<(&Status, PublishReason)> {
    effects.iter().find_map(|effect| match effect {
        StatusEffect::Publish { status, reason } => Some((status, *reason)),
        StatusEffect::Expire { .. } => None,
    })
}

#[test]
fn same_generation_start_is_idempotent() {
    let mut policy = seeded(1, SessionState::Working);
    assert!(policy
        .open("pk1", 1, snapshot(SessionState::Idle, "Changed"), 1)
        .effects
        .is_empty());
}

#[test]
fn higher_generation_reopens_closed_presence() {
    let mut policy = seeded(1, SessionState::Working);
    let closed = policy.close("pk1", 1, 20);
    assert_eq!(
        published(&closed.effects).unwrap().0.state,
        SessionState::Offline
    );
    assert_eq!(published(&closed.effects).unwrap().0.state_since, 20);

    let opened = policy.open("pk1", 2, snapshot(SessionState::Idle, "Resumed"), 21);
    let (status, reason) = published(&opened.effects).unwrap();
    assert_eq!(reason, PublishReason::Opened);
    assert_eq!(status.state, SessionState::Idle);
    assert_eq!(status.title, "Resumed");
}

#[test]
fn stale_generation_cannot_mutate_or_close_current_presence() {
    let mut policy = seeded(1, SessionState::Working);
    policy.close("pk1", 1, 10);
    policy.open("pk1", 2, snapshot(SessionState::Idle, "Current"), 11);

    assert!(policy
        .reconcile("pk1", 1, projection(SessionState::Working, "Stale"), 40)
        .effects
        .is_empty());
    assert!(policy.renew("pk1", 1, 40).effects.is_empty());
    assert!(policy.close("pk1", 1, 40).effects.is_empty());
    assert!(policy
        .open("pk1", 1, snapshot(SessionState::Working, "Older"), 40)
        .effects
        .is_empty());

    let renewed = policy.renew("pk1", 2, 40);
    let (status, reason) = published(&renewed.effects).unwrap();
    assert_eq!(reason, PublishReason::Renewed);
    assert_eq!(status.state, SessionState::Idle);
    assert_eq!(status.title, "Current");
}

#[test]
fn semantic_reconcile_is_deduped() {
    let mut policy = seeded(1, SessionState::Working);
    assert!(policy
        .reconcile("pk1", 1, projection(SessionState::Working, "Task"), 10)
        .effects
        .is_empty());
    let changed = policy.reconcile("pk1", 1, projection(SessionState::Idle, "Task"), 10);
    let (status, reason) = published(&changed.effects).unwrap();
    assert_eq!(reason, PublishReason::Changed);
    assert_eq!(status.state, SessionState::Idle);
}

#[test]
fn renewal_rearms_without_semantic_change() {
    let mut policy = seeded(1, SessionState::Working);
    let renewed = policy.renew("pk1", 1, 30);
    let (status, reason) = published(&renewed.effects).unwrap();
    assert_eq!(reason, PublishReason::Renewed);
    assert_eq!(status.expires_at, Some(120));
    assert_eq!(status.state_since, 5);
    assert!(policy.renew("pk1", 1, 45).effects.is_empty());
}

#[test]
fn revoke_expires_only_the_owned_generation() {
    let mut policy = seeded(2, SessionState::Idle);
    assert!(policy.revoke("pk1", 1, 123).effects.is_empty());
    let revoked = policy.revoke("pk1", 2, 123);
    let StatusEffect::Expire { status } = &revoked.effects[0] else {
        panic!("expected explicit expiration")
    };
    assert_eq!(status.expires_at, Some(123));
    assert_eq!(status.state, SessionState::Offline);
}
