use super::*;

fn obs() -> SessionObservation {
    SessionObservation {
        agent_slug: "claude".into(),
        agent_pubkey: "pk".into(),
        project: "proj".into(),
        host: "laptop".into(),
        rel_cwd: String::new(),
        harness: Harness::ClaudeCode,
        harness_session_id: None,
        resume_id: None,
        pty_session: None,
        watch_pid: None,
        observed_at: 100,
    }
}

fn snap(busy: bool, last_seen: u64, lifecycle: Lifecycle) -> SessionSnapshot {
    SessionSnapshot {
        source: SnapshotSource::Local,
        session_id: SessionId::from("s1"),
        agent_pubkey: "pk".into(),
        agent_slug: "claude".into(),
        project: "proj".into(),
        host: "laptop".into(),
        rel_cwd: String::new(),
        title: "fixing auth".into(),
        title_source: TitleSource::Distill,
        activity: "editing handler".into(),
        busy,
        phase: "working".into(),
        turn_id: 1,
        turn_started_at: 0,
        last_distill_at: 0,
        last_seen,
        resume_id: String::new(),
        state_version: 3,
        lifecycle,
        first_seen: 50,
        updated_at: 90,
    }
}

#[test]
fn alias_hit_is_existing() {
    let d = resolve_identity(&obs(), Some(SessionId::from("canon")), &[]);
    assert_eq!(d, IdentityDecision::Existing(SessionId::from("canon")));
}

#[test]
fn resume_match_reattaches() {
    let mut o = obs();
    o.resume_id = Some("ses_x".into());
    let live = vec![LiveLocator {
        session_id: SessionId::from("canon"),
        harness_session_id: None,
        resume_id: Some("ses_x".into()),
        pty_session: Some("%5".into()),
        watch_pid: Some(10),
    }];
    assert_eq!(
        resolve_identity(&o, None, &live),
        IdentityDecision::Reattach(SessionId::from("canon"))
    );
}

#[test]
fn same_endpoint_different_session_supersedes() {
    let mut o = obs();
    o.pty_session = Some("%5".into());
    o.harness_session_id = Some("new".into());
    let live = vec![LiveLocator {
        session_id: SessionId::from("old"),
        harness_session_id: Some("old".into()),
        resume_id: None,
        pty_session: Some("%5".into()),
        watch_pid: None,
    }];
    assert_eq!(
        resolve_identity(&o, None, &live),
        IdentityDecision::Supersede {
            old: SessionId::from("old")
        }
    );
}

#[test]
fn no_signal_mints() {
    assert_eq!(resolve_identity(&obs(), None, &[]), IdentityDecision::Mint);
}

#[test]
fn derive_live_busy() {
    let d = derive_status(&snap(true, 1000, Lifecycle::Active), 1000);
    assert!(d.liveness.is_live());
    assert!(d.busy);
    assert_eq!(d.activity, "editing handler");
}

#[test]
fn derive_idle_blanks_activity() {
    let d = derive_status(&snap(false, 1000, Lifecycle::Active), 1000);
    assert!(!d.busy);
    assert_eq!(d.activity, "");
    assert_eq!(d.title, "fixing auth");
}

#[test]
fn derive_stale_when_past_ttl() {
    let d = derive_status(&snap(true, 0, Lifecycle::Active), STATUS_TTL_SECS + 1);
    assert_eq!(d.liveness, Liveness::Stale);
}

#[test]
fn derive_ended_is_never_live() {
    let d = derive_status(&snap(true, 1000, Lifecycle::Ended), 1000);
    assert_eq!(d.liveness, Liveness::Stale);
    assert!(!d.busy);
}

#[test]
fn room_no_group_override_mints_when_per_session_rooms_enabled() {
    assert_eq!(
        decide_session_room(None, "my-repo", true),
        RoomDecision::Mint {
            parent: "my-repo".into()
        }
    );
}

#[test]
fn room_empty_group_override_mints_when_per_session_rooms_enabled() {
    assert_eq!(
        decide_session_room(Some(""), "my-repo", true),
        RoomDecision::Mint {
            parent: "my-repo".into()
        }
    );
}

#[test]
fn room_no_group_override_uses_project_when_per_session_rooms_disabled() {
    assert_eq!(
        decide_session_room(None, "my-repo", false),
        RoomDecision::UseExisting {
            group: "my-repo".into()
        }
    );
    assert_eq!(
        decide_session_room(Some(""), "my-repo", false),
        RoomDecision::UseExisting {
            group: "my-repo".into()
        }
    );
}

#[test]
fn room_with_group_override_uses_existing_regardless_of_flag() {
    for flag in [true, false] {
        assert_eq!(
            decide_session_room(Some("issue-514"), "my-repo", flag),
            RoomDecision::UseExisting {
                group: "issue-514".into()
            }
        );
    }
}
