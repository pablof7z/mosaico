use super::*;

enum TargetChannel {
    Unique(String),
    Ambiguous(serde_json::Value),
}

fn env_session(params: &serde_json::Value, verb: &str) -> Result<String> {
    params
        .get("env_session")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .with_context(|| {
            format!(
                "{verb} must be run from within a tenex-edge agent session \
                 (TENEX_EDGE_SESSION is not set)"
            )
        })
}

fn resolve_target_channel(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    reference: &str,
) -> Result<TargetChannel> {
    if reference.trim().is_empty() {
        anyhow::bail!("channel h must not be empty");
    }
    let root = state.with_store(|s| super::project_root(s, &rec.channel_h));
    match state.with_store(|s| super::resolve_channel_ref(s, &root, reference)) {
        super::ChannelResolution::Unique(h) => Ok(TargetChannel::Unique(h)),
        super::ChannelResolution::Ambiguous(refs) => Ok(TargetChannel::Ambiguous(
            serde_json::json!({ "ambiguous": refs, "reference": reference }),
        )),
        super::ChannelResolution::NotFound => {
            anyhow::bail!("no channel matching {reference:?} in this project")
        }
    }
}

async fn ensure_joinable(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    channel_h: &str,
) -> Result<()> {
    refresh_project_members_cache(state, channel_h).await;
    let is_member = state.with_store(|s| {
        match s.is_channel_member(channel_h, &rec.agent_pubkey) {
            Ok(present) => present,
            Err(e) => {
                tracing::error!(
                    channel = channel_h,
                    pubkey = %rec.agent_pubkey,
                    error = %e,
                    "ensure_joinable: is_channel_member probe failed — treating as not a member"
                );
                false
            }
        }
    });
    if !is_member {
        anyhow::bail!(
            "agent {} is not a member of channel {:?}",
            rec.agent_slug,
            channel_h
        );
    }

    // A store error here MUST fail the switch — never read as "no occupant",
    // which would let a second instance of the same agent silently barge into a
    // channel another instance is already working in.
    let occupied = state.with_store(|s| -> Result<Option<crate::state::Session>> {
        for other in s
            .list_alive_sessions()
            .context("ensure_joinable: listing live sessions for occupancy check")?
        {
            if other.session_id != rec.session_id
                && other.agent_pubkey == rec.agent_pubkey
                && s.is_session_joined_channel(&other.session_id, channel_h)
                    .context("ensure_joinable: checking channel occupancy")?
            {
                return Ok(Some(other));
            }
        }
        Ok(None)
    })?;
    if occupied.is_some() {
        anyhow::bail!(
            "Another instance of you is already active in #{channel_h}, so you cannot join it. \
Send it a message instead: tenex-edge chat write --channel {channel_h} --message \"...\" \
— it will arrive in the context of the instance working there."
        );
    }
    Ok(())
}

pub(in crate::daemon::server) async fn rpc_channels_join(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_join params")?;
    let env_session = env_session(params, "channels join")?;
    let rec = resolve_session(state, None, Some(&env_session), None, None, None)?;
    let channel = match resolve_target_channel(state, &rec, &p.channel)? {
        TargetChannel::Unique(h) => h,
        TargetChannel::Ambiguous(v) => return Ok(v),
    };
    ensure_joinable(state, &rec, &channel).await?;
    ensure_subscription(state, &channel).await?;
    state.with_store(|s| {
        s.join_session_channel(&rec.session_id, &channel, now_secs())
            .ok();
    });
    Ok(serde_json::json!({
        "session_id": rec.session_id,
        "channel": channel,
        "active_channel": rec.channel_h,
    }))
}

pub(in crate::daemon::server) async fn rpc_channels_leave(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_leave params")?;
    let env_session = env_session(params, "channels leave")?;
    let rec = resolve_session(state, None, Some(&env_session), None, None, None)?;
    let channel = match resolve_target_channel(state, &rec, &p.channel)? {
        TargetChannel::Unique(h) => h,
        TargetChannel::Ambiguous(v) => return Ok(v),
    };
    if channel == rec.channel_h {
        anyhow::bail!("cannot leave the active channel; switch to another channel first");
    }
    let left = state.with_store(|s| {
        s.leave_session_channel(&rec.session_id, &channel)
            .unwrap_or(false)
    });
    Ok(serde_json::json!({
        "session_id": rec.session_id,
        "channel": channel,
        "left": left,
    }))
}

pub(in crate::daemon::server) async fn rpc_channels_switch(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_switch params")?;
    let env_session = env_session(params, "channels switch")?;
    let rec = resolve_session(state, None, Some(&env_session), None, None, None)?;
    let new_channel = match resolve_target_channel(state, &rec, &p.channel)? {
        TargetChannel::Unique(h) => h,
        TargetChannel::Ambiguous(v) => return Ok(v),
    };
    ensure_joinable(state, &rec, &new_channel).await?;
    ensure_subscription(state, &new_channel).await?;
    let prev_channel = rec.channel_h.clone();
    set_active_session_channel(
        state,
        &rec.session_id,
        &rec.agent_pubkey,
        &new_channel,
        true,
    )?;
    Ok(serde_json::json!({
        "session_id": rec.session_id,
        "prev_channel": prev_channel,
        "channel": new_channel,
    }))
}

/// Set the active publishing channel. When `leave_previous` is true this is the
/// user-facing `channels switch` semantics: leave the previous active channel
/// and join the new one. `channels create` uses `false` so creating a room can
/// move focus into it without dropping the parent from passive context.
pub(in crate::daemon::server) fn set_active_session_channel(
    state: &Arc<DaemonState>,
    session_id: &str,
    agent_pubkey: &str,
    new_channel: &str,
    leave_previous: bool,
) -> Result<()> {
    // Every write here is part of one logical "this session now publishes into
    // `new_channel`" move. A swallowed error would leave the session and its
    // identity pointing at different channels — mentions delivered to a channel
    // the agent isn't actually focused on. Fail loud so the switch/create RPC
    // reports the failure instead of silently half-applying it.
    state.with_store(|s| -> Result<()> {
        // No store transaction is available here, so do the fallible reads/checks
        // FIRST and perform the mutations only after they all pass: a failure must
        // not leave the session/identity half-moved (left/joined/repointed but the
        // identity row stale, or vice versa).
        //
        // A live session that is switching channels MUST have a bound identity
        // row. `Ok(None)` means the identity vanished out from under an active
        // session — a real invariant break, not a benign miss — so bail BEFORE any
        // mutation rather than skip the identity repoint.
        let prev_to_leave = if leave_previous {
            s.get_session(session_id)
                .context("set_active_session_channel: reading current session")?
                .map(|r| r.channel_h)
                .filter(|h| h != new_channel)
        } else {
            None
        };
        let mut idn = s
            .get_identity(agent_pubkey)
            .context("set_active_session_channel: loading identity")?
            .with_context(|| {
                format!(
                    "set_active_session_channel: no identity row for live session \
                     {session_id} (agent {agent_pubkey}); refusing to silently skip the \
                     identity channel move"
                )
            })?;
        idn.channel_h = new_channel.to_string();
        idn.session_id = session_id.to_string();
        idn.alive = true;

        // Mutations — every fallible precondition above has already passed.
        if let Some(prev) = prev_to_leave {
            s.leave_session_channel(session_id, &prev)
                .context("set_active_session_channel: leaving previous channel")?;
        }
        s.join_session_channel(session_id, new_channel, now_secs())
            .context("set_active_session_channel: joining new channel")?;
        s.set_session_channel(session_id, new_channel)
            .context("set_active_session_channel: repointing active channel")?;
        s.upsert_identity(&idn)
            .context("set_active_session_channel: persisting identity channel move")?;
        Ok(())
    })?;
    state.outbox_notify.notify_waiters();
    Ok(())
}
