use super::channel_resolve::{resolve_channel_ref, root_channel, ChannelResolution};
use super::resolution::work_root_for;
use super::*;
use crate::domain::{AgentRef, ChatMessage};
use crate::fabric::provider::chat::OutboundChatRecord;
use anyhow::{Context, Result};
use nostr::Event;

mod list_agents;
mod parse;
mod reaction;
mod sessions;

use parse::parse_command;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ManagementCommand {
    Add { spec: String },
    ListAgents,
    ListSessions { all_channels: bool },
    Kill { selector: String },
    Archive { channel_ref: String },
}

pub(super) fn is_management_command_for_backend(state: &Arc<DaemonState>, event: &Event) -> bool {
    if event.kind.as_u16() != crate::fabric::nip29::wire::KIND_CHAT {
        return false;
    }
    let Some(backend_pk) = state.backend_pubkey() else {
        return false;
    };
    event.pubkey.to_hex() != backend_pk
        && p_tags(event).iter().any(|pk| pk == &backend_pk)
        && parse::is_command_shaped(&event.content) // #375: prose is not a command
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
        tracing::debug!(event_id = %short(&event_id), "mgmt command already claimed");
        return;
    }

    let reply = match accept_command(state, event, &channel_h).await {
        Ok(command) => {
            // The command is now parsed, authorized, and durably claimed. React
            // before executing it so a potentially slow add/kill/archive gives
            // the requester immediate, truthful acknowledgement from mgmt.
            reaction::publish_thumbs_up(state, event, &channel_h).await;
            match execute_command(state, command, &channel_h).await {
                Ok(reply) => reply,
                Err(e) => format!("mgmt error: {e:#}"),
            }
        }
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

async fn accept_command(
    state: &Arc<DaemonState>,
    event: &Event,
    channel_h: &str,
) -> Result<ManagementCommand> {
    let command = parse_command(&event.content)?;
    let signer = event.pubkey.to_hex();
    if !is_admin(state, channel_h, &signer).await? {
        anyhow::bail!(
            "signer {} is not an admin of channel {}",
            crate::util::pubkey_short(&signer),
            channel_h
        );
    }
    Ok(command)
}

async fn execute_command(
    state: &Arc<DaemonState>,
    command: ManagementCommand,
    channel_h: &str,
) -> Result<String> {
    match command {
        ManagementCommand::Add { spec } => add_agent(state, channel_h, &spec).await,
        ManagementCommand::ListAgents => list_agents::list_agents(state),
        ManagementCommand::ListSessions { all_channels } => {
            sessions::list_sessions(state, (!all_channels).then_some(channel_h))
        }
        ManagementCommand::Kill { selector } => kill_session(state, &selector).await,
        ManagementCommand::Archive { channel_ref } => {
            archive_named_channel(state, channel_h, &channel_ref).await
        }
    }
}

async fn add_agent(state: &Arc<DaemonState>, channel_h: &str, spec: &str) -> Result<String> {
    let work_root = state.with_store(|s| work_root_for(s, channel_h))?;
    let out = super::invite_rpc::invite_agent(state, channel_h, &work_root, spec, None).await?;
    let who = out
        .get("online_agent")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!("@{s}"))
        .unwrap_or_else(|| spec.to_string());
    Ok(format!(
        "mgmt ok: added {who} to {}.",
        channel_label(state, channel_h)
    ))
}

async fn kill_session(state: &Arc<DaemonState>, selector: &str) -> Result<String> {
    let rec = state
        .with_store(|s| super::resolution::resolve_public_session(s, selector))?
        .with_context(|| {
            format!("no local session matching {selector:?}; use its npub or current handle")
        })?;
    if !rec.is_running() {
        anyhow::bail!("session {selector:?} is not running");
    }
    let public_label = state
        .with_store(|s| s.handle_for_pubkey(&rec.pubkey))?
        .or_else(|| crate::idref::npub(&rec.pubkey))
        .unwrap_or_else(|| rec.pubkey.clone());
    let stop_note = stop_local_process(state, &rec).await;
    let _ = super::session_end::end_runtime_generation(
        state,
        &rec.pubkey,
        rec.runtime_generation,
        crate::state::StopReason::OperatorKill,
    )
    .await?;
    match stop_note {
        Ok(note) => Ok(format!("mgmt ok: killed @{public_label}{note}")),
        Err(e) => Ok(format!(
            "mgmt ok: ended @{public_label}, but process stop failed: {e:#}",
        )),
    }
}

async fn archive_named_channel(
    state: &Arc<DaemonState>,
    command_channel: &str,
    channel_ref: &str,
) -> Result<String> {
    let target = state.with_store(|s| {
        let root = root_channel(s, command_channel)?;
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

async fn is_admin(state: &Arc<DaemonState>, channel_h: &str, signer: &str) -> Result<bool> {
    if state.with_store(|s| s.is_channel_admin(channel_h, signer).unwrap_or(false)) {
        return Ok(true);
    }
    Ok(state
        .provider
        .fetch_group_roles(channel_h)
        .await?
        .get(signer)
        .map(String::as_str)
        == Some("admin"))
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
        from: AgentRef::new(pubkey, format!("{} (mosaico)", state.host)),
        channel: channel_h.to_string(),
        body: body.to_string(),
        mentioned_pubkeys: vec![requester.to_string()],
    };
    let record = OutboundChatRecord {
        channel_h: channel_h.to_string(),
        direction: "outbound",
    };
    state
        .provider
        .publish_chat_checked(&chat, &keys, &record)
        .await?;
    Ok(())
}

async fn stop_local_process(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
) -> Result<String> {
    let note = super::session_termination::terminate_explicit(state, rec).await?;
    Ok(if note.is_empty() {
        String::new()
    } else {
        format!(" {note}")
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
