use super::*;

/// Passively admit an exact live session. A session that stopped or left the
/// parent is a terminal skip: this operation must never resume it later.
pub(super) async fn admit_running_target(
    state: &Arc<DaemonState>,
    op: &crate::fabric::nip29::orchestration::AddAgentsOp,
    target: &crate::fabric::nip29::orchestration::AddTarget,
) -> bool {
    let session_pubkey = target.session_pubkey.as_deref().unwrap_or_default();
    let rec = state
        .with_store(|store| -> Result<Option<crate::state::Session>> {
            let Some(rec) = store.get_session(session_pubkey)? else {
                return Ok(None);
            };
            let in_parent = store
                .has_session_route(&rec.pubkey, &op.parent)
                .unwrap_or(false);
            Ok((rec.is_running() && in_parent).then_some(rec))
        })
        .unwrap_or_else(|error| {
            tracing::error!(
                session_pubkey,
                error = %format!("{error:#}"),
                "running-only orchestration lookup failed"
            );
            None
        });
    let Some(rec) = rec else {
        tracing::info!(
            session_pubkey,
            child = %op.child_h,
            "running-only orchestration skipped a stopped or relocated session"
        );
        return true;
    };
    match super::super::channel_membership_rpc::ensure_joinable(state, &rec, &op.child_h).await {
        Ok(()) => {
            if let Err(error) = sync_subscriptions(state).await {
                tracing::error!(
                    pubkey = %rec.pubkey,
                    child = %op.child_h,
                    error = %format!("{error:#}"),
                    "running-only orchestration subscription sync failed"
                );
                return false;
            }
            tracing::info!(
                pubkey = %rec.pubkey,
                child = %op.child_h,
                "orchestration: live session admitted passively"
            );
            true
        }
        Err(error) => {
            tracing::error!(
                pubkey = %rec.pubkey,
                child = %op.child_h,
                error = %format!("{error:#}"),
                "running-only orchestration admission failed"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RegisterSession, StopReason};

    const SESSION: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    #[tokio::test]
    async fn stopped_session_is_a_terminal_skip_and_is_never_restarted() {
        let state = DaemonState::new_for_test().await;
        state.with_store(|store| {
            store.upsert_channel("root", "root", "", "", 1).unwrap();
            store
                .upsert_channel("child", "child", "", "root", 1)
                .unwrap();
            let generation = store
                .reserve_hook_session_for_test(&RegisterSession {
                    pubkey: SESSION.into(),
                    observed_harness: "codex".into(),
                    agent_slug: "agent".into(),
                    launch_channel_h: "root".into(),
                    work_root: "root".into(),
                    child_pid: None,
                    now: 1,
                })
                .unwrap();
            store
                .mark_runtime_stopped_if_generation(SESSION, generation, StopReason::Crash, 2)
                .unwrap();
        });
        let target = crate::fabric::nip29::orchestration::AddTarget {
            backend_pubkey: state.backend_pubkey().unwrap(),
            slug: "agent".into(),
            session_pubkey: Some(SESSION.into()),
        };
        let op = crate::fabric::nip29::orchestration::AddAgentsOp {
            parent: "root".into(),
            child_h: "child".into(),
            adds: vec![target.clone()],
            running_only: true,
        };

        assert!(admit_running_target(&state, &op, &target).await);
        state.with_store(|store| {
            let session = store.get_session(SESSION).unwrap().unwrap();
            assert!(!session.is_running());
            assert!(!store.has_session_route(SESSION, "child").unwrap());
        });
    }
}
