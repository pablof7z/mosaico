use super::*;

fn snapshot(state: SessionState, title: &str) -> PresenceSnapshot {
    PresenceSnapshot {
        host: "laptop".into(),
        workspace: "mosaico".into(),
        slug: "coder".into(),
        rel_cwd: ".".into(),
        dispatch_event: None,
        projection: projection(state, title),
    }
}

fn projection(state: SessionState, title: &str) -> PresenceProjection {
    PresenceProjection {
        channels: BTreeSet::from(["room".into()]),
        branch: "feat/context".into(),
        state,
        state_since: 5,
        title: title.into(),
    }
}

fn unscoped_projection() -> PresenceProjection {
    PresenceProjection {
        channels: BTreeSet::new(),
        branch: "feat/context".into(),
        state: SessionState::Idle,
        state_since: 5,
        title: "Waiting".into(),
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

#[test]
fn an_unscoped_owner_publishes_only_after_its_first_join() {
    let mut policy = StatusReconciler::new(90, 30);
    let mut initial = snapshot(SessionState::Idle, "Waiting");
    initial.projection = unscoped_projection();
    assert!(policy.open("pk1", 1, initial, 0).effects.is_empty());
    assert!(policy.renew("pk1", 1, 30).effects.is_empty());

    let joined = policy.reconcile("pk1", 1, projection(SessionState::Idle, "Joined"), 31);
    let (status, reason) = published(&joined.effects).unwrap();
    assert_eq!(reason, PublishReason::Changed);
    assert_eq!(status.channels, vec!["room"]);

    let left = policy.reconcile("pk1", 1, unscoped_projection(), 32);
    let StatusEffect::Expire { status } = &left.effects[0] else {
        panic!("last leave must expire the prior channel presence");
    };
    assert_eq!(status.channels, vec!["room"]);
    assert_eq!(status.expires_at, Some(32));
}

#[test]
fn semantic_reconcile_refreshes_the_published_branch() {
    let mut policy = seeded(1, SessionState::Working);
    let mut changed = projection(SessionState::Working, "Task");
    changed.branch = "feat/new-branch".into();

    let out = policy.reconcile("pk1", 1, changed, 10);
    assert_eq!(published(&out.effects).unwrap().0.branch, "feat/new-branch");
}
