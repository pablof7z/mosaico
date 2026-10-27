use super::*;

pub(in crate::daemon::server) mod durable_agent;

/// Resolve the calling agent's OWN session for a membership mutation, in
/// `Strict` scope: the PTY/session anchor identifies the exact session,
/// and a miss fails loud rather than binding an arbitrary sibling. `join`/
/// `leave` are per-session mutations, so picking "some session in this
/// channel" would be wrong.
pub(in crate::daemon::server) fn resolve_caller(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
    verb: &str,
) -> Result<crate::state::Session> {
    resolve_session_inner(
        state,
        &CallerAnchor::from_params(params),
        ResolveScope::Strict,
    )
    .with_context(|| format!("{verb} must be run from within a mosaico agent session"))
}

/// A channel argument must be a full absolute path (`#workspace/child`) — never
/// a bare name or opaque id. Checked before resolving so
/// the error names the actual problem instead of falling through to NotFound.
fn require_full_path(reference: &str) -> Result<&str> {
    let reference = reference.trim();
    if reference.is_empty() {
        anyhow::bail!("channel must not be empty");
    }
    absolute::require_full_path("channel", reference)?;
    Ok(reference)
}

pub(in crate::daemon::server) fn resolve_target_channel(
    state: &Arc<DaemonState>,
    reference: &str,
) -> Result<String> {
    let reference = require_full_path(reference)?;
    match state.with_store(|s| absolute::resolve_absolute_channel_ref(s, reference)) {
        super::ChannelResolution::Unique(h) => Ok(h),
        super::ChannelResolution::NotFound => {
            anyhow::bail!(
                "{}",
                state.with_store(|s| absolute::describe_missing_channel(s, reference))
            )
        }
    }
}

pub(in crate::daemon::server) async fn ensure_joinable(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    channel_h: &str,
) -> Result<()> {
    let channel_ref = state
        .with_store(|store| super::channel_resolve::channel_reference_for(store, channel_h))?;
    let _lane = state.standing_sync.lock().await;
    let is_member = state.with_store(|s| match s.is_channel_member(channel_h, &rec.pubkey) {
        Ok(present) => present,
        Err(e) => {
            tracing::error!(
                channel = channel_h,
                pubkey = %rec.pubkey,
                error = %e,
                "ensure_joinable: is_channel_member probe failed — treating as not a member"
            );
            false
        }
    });
    if !is_member {
        // Auto-add the agent via the management key — joining should be
        // transparent; an agent targeting a channel it isn't yet a member of
        // simply gets added silently rather than hitting an access error.
        let added = state
            .provider()
            .grant_member_published(channel_h, &rec.pubkey, &state.backend_keys())
            .await;
        added.require_published(format!(
            "joining agent {} to channel {}",
            rec.agent_slug, channel_ref
        ))?;
    }

    let recorded = super::managed_lifecycle::commit_confirmed_admission(
        state,
        &rec.pubkey,
        channel_h,
        rec.runtime_generation,
        rec.lifecycle_epoch,
    )
    .await?;
    if !recorded {
        anyhow::bail!("session changed while channel membership was being confirmed");
    }
    Ok(())
}

pub(in crate::daemon::server) async fn rpc_channel_join(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_join params")?;
    let rec = resolve_caller(state, params, "channel join")?;
    let channel = resolve_target_channel(state, &p.channel)?;
    let already_joined =
        state.with_store(|store| store.has_session_route(&rec.pubkey, &channel))?;
    ensure_joinable(state, &rec, &channel).await?;
    sync_subscriptions(state).await?;
    let history_notice = if already_joined {
        None
    } else {
        state.with_store(|store| {
            crate::turn_context::history::prejoin_notice(store, &rec, &channel, now_secs())
        })?
    };
    let channel =
        state.with_store(|store| super::channel_resolve::channel_reference_for(store, &channel))?;
    Ok(serde_json::json!({
        "channel": channel,
        "history_notice": history_notice,
    }))
}

pub(in crate::daemon::server) async fn rpc_channel_leave(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_leave params")?;
    let rec = resolve_caller(state, params, "channel leave")?;
    let channel = resolve_target_channel(state, &p.channel)?;
    let channel_ref =
        state.with_store(|store| super::channel_resolve::channel_reference_for(store, &channel))?;
    // Phase one: drop the route but keep the relay-confirmed standing. From
    // here every presence reconcile excludes the channel, and the pair is the
    // durable "relay removal is due" state standing cleanup retries.
    let leaving = {
        let _lane = state.standing_sync.lock().await;
        state.with_store(|store| {
            let member = is_member(store, &rec.pubkey, &channel)?;
            Ok::<_, anyhow::Error>(
                store
                    .begin_route_leave(&rec.pubkey, &channel)?
                    .map(|route| (route, member)),
            )
        })?
    };
    let left = if let Some((route, was_member)) = leaving {
        // Not under the standing lane: a wedged relay must not stall every
        // other admission for the publisher's whole retry budget.
        super::presence::publish_generation_now(
            state,
            &rec.pubkey,
            rec.runtime_generation,
            "channel_leave",
        )
        .await;
        let _lane = state.standing_sync.lock().await;
        // Standing cleanup may already have completed the removal.
        let compensated =
            was_member && !state.with_store(|store| is_member(store, &rec.pubkey, &channel))?;
        let removed = if !compensated {
            state
                .provider()
                .remove_member_published(&channel, &rec.pubkey, &state.backend_keys())
                .await
        } else {
            crate::fabric::group_management::GroupMutationOutcome::Published
        };
        if !removed.is_published() {
            state.with_store(|store| {
                store.restore_route_after_failed_leave(&rec.pubkey, &channel, &route)
            })?;
            drop(_lane);
            // The session is still a member: republish the presence the
            // leave withdrew.
            super::presence::reconcile_generation(
                state,
                &rec.pubkey,
                rec.runtime_generation,
                "channel_leave_failed",
                None,
            )
            .await;
        }
        removed.require_published(format!(
            "leaving agent {} from channel {}",
            rec.agent_slug, channel_ref
        ))?;
        state.with_store(|store| store.finish_route_leave(&rec.pubkey, &channel, now_secs()))?;
        true
    } else {
        false
    };
    // Teardown: with no other owner, dropping the channel emits a REAL NIP-01 CLOSE.
    if left {
        super::presence::reconcile_generation(
            state,
            &rec.pubkey,
            rec.runtime_generation,
            "channel_left",
            Some(
                crate::fabric::provider::ConfirmedGroupScope::from_confirmed_removal(
                    &channel,
                    &rec.pubkey,
                ),
            ),
        )
        .await;
        subscriptions::reconcile_subs_logged(state, "channel_leave").await;
    }
    Ok(serde_json::json!({ "channel": channel_ref, "left": left }))
}

fn is_member(store: &crate::state::Store, pubkey: &str, channel: &str) -> Result<bool> {
    Ok(store
        .get_session_standing(pubkey, channel)?
        .is_some_and(|standing| standing.state == crate::state::StandingState::Member))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{TestGroup, TestGroupDelivery};

    #[tokio::test]
    async fn joining_a_missing_channel_never_creates_it() {
        let state = DaemonState::new_for_test().await;
        state.with_store(|store| {
            store.install_test_group_delivery(TestGroupDelivery::new([
                TestGroup::new("root-h").metadata("project", "", "", 1)
            ]))
        });
        let before = state.with_store(|store| store.list_channels().unwrap());

        let error = match resolve_target_channel(&state, "#project/does-not-exist") {
            Ok(_) => panic!("a missing channel must not resolve"),
            Err(error) => error.to_string(),
        };

        assert!(error.contains("#project/does-not-exist"), "{error}");
        let after = state.with_store(|store| store.list_channels().unwrap());
        assert_eq!(after, before, "join resolution must be existing-only");
    }
}
