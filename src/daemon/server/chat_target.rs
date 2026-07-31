use super::*;

#[derive(Debug)]
pub(in crate::daemon::server) struct ChatTarget {
    pub channel_h: String,
    pub explicit: bool,
}

/// Resolve `--channel`/inferred destination for `channel send`/`channel read`.
///
/// An explicit reference must be a full absolute path (`#workspace/child`) —
/// no bare names or opaque ids, and no exception for a launch channel.
/// Resolution is GLOBAL (not scoped to
/// the caller's own workspace). A reference that doesn't resolve is rejected
/// with a list of what actually exists — nothing is ever created here. A
/// reference that DOES resolve but the calling session hasn't joined is also
/// rejected: sending/reading requires having joined first. (A session that
/// has exactly one joined channel, including one whose relay
/// metadata hasn't materialized yet and so isn't otherwise addressable, should
/// simply omit `--channel` — see below.)
///
/// With no explicit reference, the destination is inferred from the
/// session's already-joined channels (none -> error; exactly one -> that one;
/// several -> ambiguous, re-run explicitly).
pub(in crate::daemon::server) fn resolve_chat_target(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    explicit: Option<&str>,
    command: &str,
) -> Result<ChatTarget> {
    if let Some(reference) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        absolute::require_full_path("--channel", reference)?;
        let channel_h = state.with_store(|s| resolve_chat_channel_ref(s, reference))?;
        let joined = state.with_store(|s| {
            s.has_session_route(&rec.pubkey, &channel_h)
                .unwrap_or(false)
        });
        if !joined {
            anyhow::bail!(
                "this session hasn't joined channel {reference:?}; run \
                 `mosaico channel join {reference}` first"
            );
        }
        return Ok(ChatTarget {
            channel_h,
            explicit: true,
        });
    }

    let joined = state.with_store(|s| s.list_session_routes(&rec.pubkey))?;
    match joined.as_slice() {
        [] => anyhow::bail!(
            "{command} requires a channel because this session has not joined any channels"
        ),
        [(channel_h, _)] => Ok(ChatTarget {
            channel_h: channel_h.clone(),
            explicit: false,
        }),
        _ => {
            let refs = state.with_store(|s| {
                joined
                    .iter()
                    .map(|(h, _)| super::channel_resolve::channel_reference_for(s, h))
                    .collect::<Result<Vec<_>>>()
            })?;
            anyhow::bail!(
                "{} is ambiguous because this session is joined to {} channels. \
Pass one explicitly:\n{}",
                command,
                joined.len(),
                refs.iter()
                    .map(|r| format!("  mosaico {command} --channel {r}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }
}

fn resolve_chat_channel_ref(store: &crate::state::Store, reference: &str) -> Result<String> {
    match absolute::resolve_absolute_channel_ref(store, reference) {
        super::ChannelResolution::Unique(h) => Ok(h),
        super::ChannelResolution::NotFound => {
            anyhow::bail!("{}", absolute::describe_missing_channel(store, reference))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::channel_resolve::channel_reference_for;
    use super::*;
    use crate::state::{Session, Store};

    fn session(_channel_h: &str) -> Session {
        Session {
            pubkey: "pk".to_string(),
            runtime_generation: 1,
            agent_slug: "codex".to_string(),
            work_root: "root".to_string(),
            readiness_parent: "root".to_string(),
            observed_harness: "codex".to_string(),
            claimed_harness: String::new(),
            admitted_bundle: String::new(),
            admitted_transport: String::new(),
            endpoint_provenance: "hook".to_string(),
            child_pid: None,
            runtime_state: crate::state::RuntimeState::Running,
            presentation_state: crate::state::PresentationState::Headed,
            work_state: crate::state::WorkState::Idle,
            recovery_state: crate::state::RecoveryState::Pending,
            lifecycle_epoch: 1,
            attachment_epoch: 1,
            idle_since: 0,
            idle_deadline: 0,
            stopped_at: 0,
            stop_reason: None,
            turn_count: 0,
            busy_seconds: 0,
            created_at: 1,
            last_seen: 1,
            turn_started_at: 0,
            seen_cursor: 0,
            title: String::new(),
            state_changed_at: 1,
        }
    }

    #[test]
    fn explicit_chat_target_resolves_absolute_path_when_joined() {
        let store = Store::open_memory().unwrap();
        store.upsert_channel("root", "general", "", "", 1).unwrap();
        store
            .upsert_channel("abcd1234", "planning", "", "root", 1)
            .unwrap();
        store.grant_session_route("pk", "abcd1234", 1).unwrap();

        assert_eq!(
            resolve_chat_channel_ref(&store, "#root/planning").unwrap(),
            "abcd1234"
        );
    }

    #[tokio::test]
    async fn explicit_chat_target_rejects_a_relative_reference() {
        let state = DaemonState::new_for_test().await;
        let rec = session("root");
        let err = resolve_chat_target(&state, &rec, Some("planning"), "channel send")
            .expect_err("a relative reference must be rejected");
        assert!(
            err.to_string().contains("must be a full path"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn explicit_chat_target_rejects_an_unjoined_but_existing_channel() {
        let state = DaemonState::new_for_test().await;
        let rec = session("root");
        state
            .with_store(|s| s.upsert_channel("root", "root", "", "", 1))
            .unwrap();
        state
            .with_store(|s| s.upsert_channel("other", "other", "", "", 1))
            .unwrap();

        let err = resolve_chat_target(&state, &rec, Some("#other"), "channel send")
            .expect_err("an existing but un-joined channel must be rejected");
        assert!(
            err.to_string().contains("hasn't joined"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn explicit_chat_target_rejects_a_missing_path_with_suggestions() {
        let state = DaemonState::new_for_test().await;
        let rec = session("root");
        state
            .with_store(|s| s.upsert_channel("workspace", "general", "", "", 1))
            .unwrap();
        state
            .with_store(|s| s.upsert_channel("h-alpha", "alpha", "", "workspace", 1))
            .unwrap();

        let err = resolve_chat_target(&state, &rec, Some("#workspace/test/hello"), "channel send")
            .expect_err("a missing path must be rejected, not auto-created");
        let message = err.to_string();
        assert!(message.contains("no channel matching"), "{message}");
        assert!(message.contains("#workspace/alpha"), "{message}");
        assert!(
            state
                .with_store(|s| s.get_channel("test"))
                .unwrap()
                .is_none(),
            "a rejected path must never be silently created"
        );
    }

    #[tokio::test]
    async fn a_bare_raw_id_is_rejected_even_when_it_is_the_callers_own_channel() {
        // No exception for "it's my own channel": --channel is always a full
        // path, with zero leniency.
        let state = DaemonState::new_for_test().await;
        let rec = session("pending-channel-id");

        let err = resolve_chat_target(&state, &rec, Some("pending-channel-id"), "channel send")
            .expect_err("a bare raw id must be rejected");
        assert!(
            err.to_string().contains("must be a full path"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn omitting_channel_rejects_a_session_with_no_memberships() {
        let state = DaemonState::new_for_test().await;
        let rec = session("pending-channel-id");

        let error = resolve_chat_target(&state, &rec, None, "channel send")
            .expect_err("zero memberships have no implicit destination");
        assert!(error.to_string().contains("has not joined any channels"));
    }

    #[test]
    fn multi_join_without_explicit_channel_errors_with_reruns() {
        let store = Store::open_memory().unwrap();
        store.upsert_channel("root", "root", "", "", 1).unwrap();
        store.upsert_channel("other", "other", "", "", 1).unwrap();
        store
            .reserve_hook_session_for_test(&crate::state::RegisterSession {
                pubkey: "pk".to_string(),
                observed_harness: "codex".to_string(),
                agent_slug: "codex".to_string(),
                launch_channel_h: "root".to_string(),
                work_root: "root".to_string(),
                child_pid: None,
                now: 1,
            })
            .unwrap();
        store.grant_session_route("pk", "root", 1).unwrap();
        store.grant_session_route("pk", "other", 2).unwrap();

        let joined = store.list_session_routes("pk").unwrap();
        assert_eq!(joined.len(), 2);
        let refs = joined
            .iter()
            .map(|(h, _)| channel_reference_for(&store, h))
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert!(refs.contains(&"#root".to_string()));
        assert!(refs.contains(&"#other".to_string()));
    }

    #[test]
    fn multi_join_rerun_refs_use_relative_channel_paths() {
        let store = Store::open_memory().unwrap();
        store.upsert_channel("root", "root", "", "", 1).unwrap();
        store
            .upsert_channel("h-epic", "epic", "", "root", 1)
            .unwrap();
        store
            .upsert_channel("h-plan", "planning", "", "h-epic", 1)
            .unwrap();

        assert_eq!(
            channel_reference_for(&store, "h-plan").unwrap(),
            "#root/epic/planning"
        );
    }
}
