use super::*;

mod active_channel;

pub(in crate::daemon::server) use active_channel::set_active_session_channel;

pub(in crate::daemon::server) enum TargetChannel {
    Unique(String),
    Ambiguous(serde_json::Value),
}

/// Resolve the calling agent's OWN session for a membership mutation, in
/// `Strict` scope: the PTY/session anchor identifies the exact session,
/// and a miss fails loud rather than binding an arbitrary sibling. `join`/
/// `leave`/`switch` are per-session mutations, so picking "some session in this
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

/// Resolve `reference` in the caller's channel subtree, returning the channel
/// root alongside the resolution so callers can decide what to do with a miss.
fn resolve_ref_in_channel(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    reference: &str,
) -> Result<(String, super::ChannelResolution)> {
    if reference.trim().is_empty() {
        anyhow::bail!("channel h must not be empty");
    }
    let root = state.with_store(|s| super::root_channel(s, &rec.channel_h))?;
    let resolution = state.with_store(|s| super::resolve_channel_ref(s, &root, reference));
    Ok((root, resolution))
}

fn ambiguous(refs: Vec<String>, reference: &str) -> TargetChannel {
    TargetChannel::Ambiguous(serde_json::json!({ "ambiguous": refs, "reference": reference }))
}

pub(in crate::daemon::server) fn resolve_target_channel(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    reference: &str,
) -> Result<TargetChannel> {
    match resolve_ref_in_channel(state, rec, reference)? {
        (_, super::ChannelResolution::Unique(h)) => Ok(TargetChannel::Unique(h)),
        (_, super::ChannelResolution::Ambiguous(refs)) => Ok(ambiguous(refs, reference)),
        (_, super::ChannelResolution::NotFound) => {
            anyhow::bail!("no channel matching {reference:?} in this channel")
        }
    }
}

/// Like [`resolve_target_channel`] but with `mkdir -p` semantics: when the
/// reference names a channel-relative path that does not exist yet, create the
/// whole missing ancestor chain (not just the leaf) and target the leaf. Used by
/// `join`/`switch`, which are intent-to-be-there gestures; `leave`/`archive` keep
/// the non-creating [`resolve_target_channel`].
pub(in crate::daemon::server) async fn resolve_or_create_target_channel(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    reference: &str,
) -> Result<TargetChannel> {
    match resolve_ref_in_channel(state, rec, reference)? {
        (_, super::ChannelResolution::Unique(h)) => Ok(TargetChannel::Unique(h)),
        (_, super::ChannelResolution::Ambiguous(refs)) => Ok(ambiguous(refs, reference)),
        (root, super::ChannelResolution::NotFound) => Ok(TargetChannel::Unique(
            super::resolve_channel_path(state, &root, reference, true).await?,
        )),
    }
}

pub(in crate::daemon::server) async fn ensure_joinable(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    channel_h: &str,
) -> Result<()> {
    let _lane = state.standing_sync.lock().await;
    refresh_channel_members_cache(state, channel_h).await;
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
        // Auto-add the agent via the management key — join/switch should be
        // transparent; an agent targeting a channel it isn't yet a member of
        // simply gets added silently rather than hitting an access error.
        let added = state
            .provider
            .grant_member_confirmed(channel_h, &rec.pubkey)
            .await;
        if !added.is_confirmed() {
            anyhow::bail!(
                "agent {} is not a member of channel {:?} and could not be confirmed as added \
                 (is the management key an admin of that channel?)",
                rec.agent_slug,
                channel_h
            );
        }
        refresh_channel_members_cache(state, channel_h).await;
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
    let channel = match resolve_or_create_target_channel(state, &rec, &p.channel).await? {
        TargetChannel::Unique(h) => h,
        TargetChannel::Ambiguous(v) => return Ok(v),
    };
    ensure_joinable(state, &rec, &channel).await?;
    super::presence::reconcile_generation(
        state,
        &rec.pubkey,
        rec.runtime_generation,
        "channel_joined",
    )
    .await;
    sync_subscriptions(state).await?;
    Ok(serde_json::json!({
        "channel": channel,
        "active_channel": rec.channel_h,
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
    let channel = match resolve_target_channel(state, &rec, &p.channel)? {
        TargetChannel::Unique(h) => h,
        TargetChannel::Ambiguous(v) => return Ok(v),
    };
    if channel == rec.channel_h {
        anyhow::bail!("cannot leave the active channel; switch to another channel first");
    }
    let was_joined =
        state.with_store(|s| s.has_session_route(&rec.pubkey, &channel).unwrap_or(false));
    let left = if was_joined {
        let _lane = state.standing_sync.lock().await;
        let removed = state
            .provider
            .remove_member_confirmed(&channel, &rec.pubkey)
            .await;
        if !removed.is_confirmed() {
            anyhow::bail!(
                "agent {} could not be confirmed as removed from channel {:?}",
                rec.agent_slug,
                channel
            );
        }
        state.with_store(|s| {
            s.revoke_route_and_mark_absent(&rec.pubkey, &channel, now_secs())
                .unwrap_or(false)
        })
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
        )
        .await;
        subscriptions::reconcile_subs_logged(state, "channel_leave").await;
    }
    Ok(serde_json::json!({
        "channel": channel,
        "left": left,
    }))
}

pub(in crate::daemon::server) async fn rpc_channel_switch(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_switch params")?;
    let rec = resolve_caller(state, params, "channel switch")?;
    let new_channel = match resolve_or_create_target_channel(state, &rec, &p.channel).await? {
        TargetChannel::Unique(h) => h,
        TargetChannel::Ambiguous(v) => return Ok(v),
    };
    ensure_joinable(state, &rec, &new_channel).await?;
    sync_subscriptions(state).await?;
    let prev_channel = rec.channel_h.clone();
    set_active_session_channel(state, &rec.pubkey, &new_channel)?;
    if prev_channel != new_channel {
        let _lane = state.standing_sync.lock().await;
        let removed = state
            .provider
            .remove_member_confirmed(&prev_channel, &rec.pubkey)
            .await;
        if !removed.is_confirmed() {
            tracing::warn!(
                agent = %rec.agent_slug,
                prev_channel,
                "channel_switch: previous membership removal was not confirmed"
            );
        } else {
            state.with_store(|store| {
                store.revoke_route_and_mark_absent(&rec.pubkey, &prev_channel, now_secs())
            })?;
        }
        // Reconcile so the left channel observation closes when no owner remains.
        subscriptions::reconcile_subs_logged(state, "channel_switch").await;
    }
    super::presence::reconcile_generation(
        state,
        &rec.pubkey,
        rec.runtime_generation,
        "channel_switched",
    )
    .await;
    Ok(serde_json::json!({
        "prev_channel": prev_channel,
        "channel": new_channel,
    }))
}
