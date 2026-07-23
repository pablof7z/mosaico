//! Detect sustained root-channel coordination and offer an advisory move.

use super::*;
use crate::channel_nudge::{
    detect_root_conversation, is_substantive_message, render_nudge, ConversationEvidence,
    ConversationMessage, ConversationReaction, ParticipantSnapshot, CONVERSATION_WINDOW_SECS,
};
use std::collections::{BTreeMap, HashSet};

mod accept;
pub(in crate::daemon::server) use accept::rpc_accept;

const MESSAGE_SCAN_LIMIT: u32 = 200;

pub(super) fn maybe_nudge(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    now: u64,
) -> Option<String> {
    maybe_nudge_with_roll(state, rec, now, random_roll())
}

fn maybe_nudge_with_roll(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    now: u64,
    roll: u64,
) -> Option<String> {
    let evidence = match current_evidence(state, &rec.channel_h, now) {
        Ok(Some(evidence)) => evidence,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(
                pubkey = %rec.pubkey,
                channel = %rec.channel_h,
                error = %format!("{error:#}"),
                "channel topology nudge evaluation failed open"
            );
            return None;
        }
    };
    let offer = state
        .runtime
        .channel_nudges
        .lock()
        .expect("channel nudge mutex poisoned")
        .consider(&rec.pubkey, evidence, now, roll)?;
    Some(render_nudge(&offer))
}

pub(super) fn clear_offer(state: &Arc<DaemonState>, pubkey: &str) {
    state
        .runtime
        .channel_nudges
        .lock()
        .expect("channel nudge mutex poisoned")
        .clear_offer(pubkey);
}

pub(super) fn current_offer(
    state: &Arc<DaemonState>,
    pubkey: &str,
    now: u64,
) -> Option<crate::channel_nudge::MoveOffer> {
    state
        .runtime
        .channel_nudges
        .lock()
        .expect("channel nudge mutex poisoned")
        .current_offer(pubkey, now)
}

pub(super) fn current_evidence(
    state: &Arc<DaemonState>,
    parent: &str,
    now: u64,
) -> Result<Option<ConversationEvidence>> {
    let whitelisted = state
        .whitelisted_pubkeys()
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let host = state.host.clone();
    let (is_root, participants, messages, reactions) = state.with_store(|store| -> Result<_> {
        let is_root = super::root_channel(store, parent)? == parent;
        if !is_root {
            return Ok((false, Vec::new(), Vec::new(), Vec::new()));
        }
        let admins = store
            .list_channel_members(parent)?
            .into_iter()
            .filter(|member| member.role == "admin")
            .map(|member| member.pubkey)
            .collect::<HashSet<_>>();
        let mut participants = BTreeMap::<String, ParticipantSnapshot>::new();

        for session in store.list_running_sessions()? {
            let joined = session.channel_h == parent
                || store
                    .has_session_route(&session.pubkey, parent)
                    .unwrap_or(false);
            if !joined || admins.contains(&session.pubkey) || whitelisted.contains(&session.pubkey)
            {
                continue;
            }
            let profile = store.get_profile(&session.pubkey)?;
            if profile.as_ref().is_some_and(|profile| profile.is_backend) {
                continue;
            }
            let label = profile
                .as_ref()
                .map(|profile| profile.slug.trim())
                .filter(|label| !label.is_empty())
                .unwrap_or(&session.agent_slug)
                .to_string();
            participants.insert(
                session.pubkey.clone(),
                ParticipantSnapshot {
                    pubkey: session.pubkey.clone(),
                    label,
                    host: host.clone(),
                    runtime_generation: Some(session.runtime_generation),
                    live: true,
                    busy: session.is_working(),
                },
            );
        }

        for status in store.live_status_for_channel(parent, now)? {
            if participants.contains_key(&status.pubkey)
                || admins.contains(&status.pubkey)
                || whitelisted.contains(&status.pubkey)
            {
                continue;
            }
            let Some(profile) = store.get_profile(&status.pubkey)? else {
                continue;
            };
            if profile.is_backend || profile.agent_slug.trim().is_empty() {
                continue;
            }
            let observed = crate::session_presence::remote(&status, now).state;
            if !observed.is_live() {
                continue;
            }
            let label = if profile.slug.trim().is_empty() {
                status.slug
            } else {
                profile.slug
            };
            participants.insert(
                status.pubkey.clone(),
                ParticipantSnapshot {
                    pubkey: status.pubkey,
                    label,
                    host: profile.host,
                    runtime_generation: None,
                    live: true,
                    busy: observed.is_working(),
                },
            );
        }

        let messages = store
            .recent_chat_messages_for_channel(
                parent,
                now.saturating_sub(CONVERSATION_WINDOW_SECS),
                MESSAGE_SCAN_LIMIT,
            )?
            .into_iter()
            .filter(|message| message.sync_state == "accepted")
            .map(|message| ConversationMessage {
                message_id: message.message_id,
                author_pubkey: message.author_pubkey,
                created_at: message.created_at,
                substantive: is_substantive_message(&message.body),
            })
            .collect();
        let reactions = store
            .recent_reactions_for_channel(
                parent,
                now.saturating_sub(CONVERSATION_WINDOW_SECS),
                MESSAGE_SCAN_LIMIT,
            )?
            .into_iter()
            .map(|reaction| ConversationReaction {
                reactor_pubkey: reaction.reactor_pubkey,
                target_message_id: reaction.target_message_id,
            })
            .collect();
        Ok((
            true,
            participants.into_values().collect(),
            messages,
            reactions,
        ))
    })?;
    Ok(detect_root_conversation(
        parent,
        is_root,
        &messages,
        &reactions,
        &participants,
    ))
}

fn random_roll() -> u64 {
    let hex = nostr_sdk::prelude::Keys::generate().public_key().to_hex();
    u64::from_str_radix(&hex[..16], 16).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "channel_move/tests.rs"]
mod tests;
