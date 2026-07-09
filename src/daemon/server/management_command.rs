use super::channel_resolve::{resolve_channel_ref, root_channel, ChannelResolution};
use super::resolution::work_root_for;
use super::*;
use crate::domain::{AgentRef, ChatMessage};
use crate::fabric::provider::chat::OutboundChatRecord;
use anyhow::{Context, Result};
use nostr_sdk::prelude::Event;

mod list_agents;
mod parse;
mod sessions;

use parse::parse_command;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ManagementCommand {
    Add { spec: String },
    ListAgents,
    ListSessions { all_channels: bool },
    Kill { session_id: String },
    Archive { channel_ref: String },
}

pub(super) fn is_management_command_for_backend(state: &Arc<DaemonState>, event: &Event) -> bool {
    if event.kind.as_u16() != crate::fabric::nip29::wire::KIND_CHAT {
        return false;
    }
    let Some(backend_pk) = state.backend_pubkey() else {
        return false;
    };
    event.pubkey.to_hex() != backend_pk && p_tags(event).iter().any(|pk| pk == &backend_pk)
}

pub(super) async fn handle_management_command(state: &Arc<DaemonState>, event: &Event) {
    let event_id = event.id.to_hex();
    let signer = event.pubkey.to_hex();
    let Some(channel_h) = first_tag(event, "h").map(str::to_string) else {
        tracing::warn!(
            event_id = %short(&event_id),
            "management command ignored: missing h tag"
        );
        return;
    };
    let claimed = match state.with_store(|s| {
        s.claim_management_command(&event_id, &signer, &channel_h, &event.content, now_secs())
    }) {
        Ok(claimed) => claimed,
        Err(e) => {
            tracing::error!(
                event_id = %short(&event_id),
                error = %e,
                "management command claim failed"
            );
            return;
        }
    };
    if !claimed {
        tracing::debug!(
            event_id = %short(&event_id),
            "management command already complete or in flight"
        );
        return;
    }

    let reply = match execute_claimed(state, event, &channel_h).await {
        Ok(reply) => reply,
        Err(e) => format!("mgmt error: {e:#}"),
    };
    if let Err(e) = state.with_store(|s| s.complete_management_command(&event_id, now_secs())) {
        tracing::error!(
            event_id = %short(&event_id),
            error = %e,
            "management command completion mark failed"
        );
    }
    if let Err(e) = publish_reply(state, &channel_h, &signer, &reply).await {
        tracing::warn!(
            event_id = %short(&event_id),
            error = %format!("{e:#}"),
            "management command reply publish failed"
        );
    }
}

async fn execute_claimed(
    state: &Arc<DaemonState>,
    event: &Event,
    channel_h: &str,
) -> Result<String> {
    let command = parse_command(&event.content)?;
    let signer = event.pubkey.to_hex();
    if !is_admin(state, channel_h, &signer).await {
        anyhow::bail!(
            "signer {} is not an admin of channel {}",
            crate::util::pubkey_short(&signer),
            channel_h
        );
    }
    match command {
        ManagementCommand::Add { spec } => add_agent(state, channel_h, &spec).await,
        ManagementCommand::ListAgents => list_agents::list_agents(state),
        ManagementCommand::ListSessions { all_channels } => {
            sessions::list_sessions(state, (!all_channels).then_some(channel_h))
        }
        ManagementCommand::Kill { session_id } => kill_session(state, &session_id).await,
        ManagementCommand::Archive { channel_ref } => {
            archive_named_channel(state, channel_h, &channel_ref).await
        }
    }
}

async fn add_agent(state: &Arc<DaemonState>, channel_h: &str, spec: &str) -> Result<String> {
    let work_root = state.with_store(|s| work_root_for(s, channel_h));
    let out = super::invite_rpc::invite_agent(state, channel_h, &work_root, spec, None).await?;
    let pty = out
        .get("pty_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!(" pty={s}"))
        .unwrap_or_default();
    let orchestration = out
        .get("orchestration_event_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!(" orchestration={}", short(s)))
        .unwrap_or_default();
    Ok(format!(
        "mgmt ok: added {spec} to {}.{pty}{orchestration}",
        channel_label(state, channel_h)
    ))
}

async fn kill_session(state: &Arc<DaemonState>, session_id: &str) -> Result<String> {
    let rec = find_live_local_session(state, session_id)?;
    let stop_note = stop_local_process(state, &rec).await;
    let _ = super::rpc_session_end(
        state,
        &serde_json::json!({
            "session": rec.session_id,
        }),
    )
    .await?;
    match stop_note {
        Ok(note) => Ok(format!(
            "mgmt ok: killed {} {}{}",
            short(&rec.session_id),
            rec.agent_slug,
            note
        )),
        Err(e) => Ok(format!(
            "mgmt ok: ended {} {}, but process stop failed: {e:#}",
            short(&rec.session_id),
            rec.agent_slug
        )),
    }
}

async fn archive_named_channel(
    state: &Arc<DaemonState>,
    command_channel: &str,
    channel_ref: &str,
) -> Result<String> {
    let target = state.with_store(|s| {
        let root = root_channel(s, command_channel);
        match resolve_channel_ref(s, &root, channel_ref) {
            ChannelResolution::Unique(h) => Ok(h),
            ChannelResolution::Ambiguous(refs) => {
                anyhow::bail!("channel {channel_ref:?} is ambiguous: {}", refs.join(", "))
            }
            ChannelResolution::NotFound => {
                anyhow::bail!("no channel matching {channel_ref:?}")
            }
        }
    })?;
    let label = channel_label(state, &target);
    let out = super::channels_rpc::archive_channel(state, &target).await?;
    let removed = out
        .get("removed_members")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    Ok(format!(
        "mgmt ok: archived {label}; removed {removed} non-admin member(s)"
    ))
}

async fn is_admin(state: &Arc<DaemonState>, channel_h: &str, signer: &str) -> bool {
    if state.with_store(|s| s.is_channel_admin(channel_h, signer).unwrap_or(false)) {
        return true;
    }
    state
        .provider
        .fetch_group_roles(channel_h)
        .await
        .get(signer)
        .map(String::as_str)
        == Some("admin")
}

async fn publish_reply(
    state: &Arc<DaemonState>,
    channel_h: &str,
    requester: &str,
    body: &str,
) -> Result<()> {
    let keys = state.management_keys()?;
    let pubkey = keys.public_key().to_hex();
    let chat = ChatMessage {
        from: AgentRef::new(pubkey, format!("{} (tenex-edge)", state.host)),
        channel: channel_h.to_string(),
        body: body.to_string(),
        mentioned_pubkey: Some(requester.to_string()),
    };
    let record = OutboundChatRecord {
        from_session: None,
        channel_h: channel_h.to_string(),
        body: body.to_string(),
        mentioned_pubkey: Some(requester.to_string()),
        mentioned_session: None,
        created_at: None,
        direction: "outbound",
    };
    state
        .provider
        .publish_chat_checked(&chat, &keys, &record)
        .await?;
    Ok(())
}

fn find_live_local_session(
    state: &Arc<DaemonState>,
    id_or_prefix: &str,
) -> Result<crate::state::Session> {
    state.with_store(|s| {
        if let Some(rec) = s.get_session(id_or_prefix)? {
            if rec.alive {
                return Ok(rec);
            }
            anyhow::bail!("session {} is not running", rec.session_id);
        }
        let matches: Vec<_> = s
            .list_alive_sessions()?
            .into_iter()
            .filter(|rec| rec.session_id.starts_with(id_or_prefix))
            .collect();
        match matches.as_slice() {
            [one] => Ok(one.clone()),
            [] => anyhow::bail!("no running local session matching {id_or_prefix:?}"),
            _ => anyhow::bail!("session id {id_or_prefix:?} is ambiguous; use more characters"),
        }
    })
}

async fn stop_local_process(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
) -> Result<String> {
    if let Some(pty_id) = pty_session_for_session(state, &rec.session_id) {
        crate::pty::kill(&pty_id).with_context(|| format!("killing PTY session {pty_id}"))?;
        state.with_store(|s| s.clear_pty_session(&rec.session_id).ok());
        return Ok(format!(" pty={pty_id}"));
    }
    if let Some(pid) = rec.child_pid {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            Some(nix::sys::signal::Signal::SIGTERM),
        )
        .with_context(|| format!("sending SIGTERM to pid {pid}"))?;
        return Ok(format!(" pid={pid}"));
    }
    Ok(String::new())
}

fn pty_session_for_session(state: &Arc<DaemonState>, session_id: &str) -> Option<String> {
    state
        .with_store(|s| s.aliases_for_session(session_id))
        .ok()
        .and_then(|aliases| {
            aliases
                .into_iter()
                .find(|a| a.external_id_kind == "pty_session")
                .map(|a| a.external_id)
        })
}

fn channel_label(state: &Arc<DaemonState>, channel_h: &str) -> String {
    state.with_store(|s| {
        s.get_channel(channel_h)
            .ok()
            .flatten()
            .and_then(|c| c.human_name().map(str::to_string))
            .unwrap_or_else(|| channel_h.to_string())
    })
}

fn first_tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        let s = tag.as_slice();
        if s.first().map(String::as_str) == Some(name) {
            s.get(1).map(String::as_str)
        } else {
            None
        }
    })
}

fn p_tags(event: &Event) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let s = tag.as_slice();
            if s.first().map(String::as_str) == Some("p") {
                s.get(1).cloned()
            } else {
                None
            }
        })
        .collect()
}

fn short(s: &str) -> String {
    s.chars().take(12).collect()
}
