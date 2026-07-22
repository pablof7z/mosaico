//! Pure policy and ephemeral state for advisory channel-topology nudges.
//!
//! The daemon adapter owns store reads and channel mutations. This module keeps
//! the interaction heuristic, BUSY-only lottery, offer lifetime, and prompt
//! wording deterministic and directly testable.

use std::collections::{BTreeMap, HashMap};

pub(crate) const CONVERSATION_WINDOW_SECS: u64 = 5 * 60;
pub(crate) const LOTTERY_COOLDOWN_SECS: u64 = 10;
pub(crate) const OFFER_TTL_SECS: u64 = 2 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParticipantSnapshot {
    pub pubkey: String,
    pub label: String,
    pub host: String,
    pub runtime_generation: Option<u64>,
    pub live: bool,
    pub busy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConversationMessage {
    pub message_id: String,
    pub author_pubkey: String,
    pub created_at: u64,
    pub substantive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConversationReaction {
    pub reactor_pubkey: String,
    pub target_message_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConversationEvidence {
    pub parent: String,
    pub cohort: Vec<ParticipantSnapshot>,
    pub busy_pubkeys: Vec<String>,
    pub audience_count: usize,
    pub engaged_count: usize,
    pub message_count: usize,
    pub alternations: usize,
    pub started_at: u64,
    pub ended_at: u64,
    pub last_message_id: String,
}

impl ConversationEvidence {
    #[cfg(test)]
    pub(crate) fn cohort_pubkeys(&self) -> Vec<&str> {
        self.cohort.iter().map(|p| p.pubkey.as_str()).collect()
    }

    pub(crate) fn caller_is_busy(&self, pubkey: &str) -> bool {
        self.busy_pubkeys
            .iter()
            .any(|candidate| candidate == pubkey)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MoveOffer {
    pub evidence: ConversationEvidence,
    pub offered_at: u64,
    pub expires_at: u64,
}

#[derive(Default)]
struct SessionNudgeState {
    last_lottery_at: Option<u64>,
    offer: Option<MoveOffer>,
}

#[derive(Default)]
pub(crate) struct ChannelNudgeState {
    sessions: HashMap<String, SessionNudgeState>,
}

impl ChannelNudgeState {
    pub(crate) fn consider(
        &mut self,
        caller_pubkey: &str,
        evidence: ConversationEvidence,
        now: u64,
        roll: u64,
    ) -> Option<MoveOffer> {
        if !evidence.caller_is_busy(caller_pubkey) || evidence.busy_pubkeys.is_empty() {
            return None;
        }
        let session = self.sessions.entry(caller_pubkey.to_string()).or_default();
        if session
            .offer
            .as_ref()
            .is_some_and(|offer| offer.expires_at >= now)
        {
            return None;
        }
        session.offer = None;
        if session
            .last_lottery_at
            .is_some_and(|last| now.saturating_sub(last) < LOTTERY_COOLDOWN_SECS)
        {
            return None;
        }
        session.last_lottery_at = Some(now);
        if !lottery_wins(roll, evidence.busy_pubkeys.len()) {
            return None;
        }
        let offer = MoveOffer {
            evidence,
            offered_at: now,
            expires_at: now.saturating_add(OFFER_TTL_SECS),
        };
        session.offer = Some(offer.clone());
        Some(offer)
    }

    pub(crate) fn current_offer(&mut self, caller_pubkey: &str, now: u64) -> Option<MoveOffer> {
        let session = self.sessions.get_mut(caller_pubkey)?;
        if session
            .offer
            .as_ref()
            .is_some_and(|offer| offer.expires_at < now)
        {
            session.offer = None;
        }
        session.offer.clone()
    }

    pub(crate) fn clear_offer(&mut self, caller_pubkey: &str) {
        if let Some(session) = self.sessions.get_mut(caller_pubkey) {
            session.offer = None;
        }
    }
}

pub(crate) fn detect_root_conversation(
    parent: &str,
    is_root: bool,
    messages: &[ConversationMessage],
    reactions: &[ConversationReaction],
    participants: &[ParticipantSnapshot],
) -> Option<ConversationEvidence> {
    if !is_root {
        return None;
    }
    let participants = participants
        .iter()
        .filter(|participant| participant.live)
        .map(|participant| (participant.pubkey.as_str(), participant))
        .collect::<HashMap<_, _>>();
    let mut relevant = messages
        .iter()
        .filter(|message| {
            message.substantive && participants.contains_key(message.author_pubkey.as_str())
        })
        .collect::<Vec<_>>();
    relevant.sort_by_key(|message| (message.created_at, message.message_id.as_str()));

    let mut counts = BTreeMap::<&str, usize>::new();
    for message in &relevant {
        *counts.entry(&message.author_pubkey).or_default() += 1;
    }
    if counts.len() < 2 || counts.values().filter(|count| **count >= 2).count() < 2 {
        return None;
    }

    let cohort = counts
        .keys()
        .filter_map(|pubkey| participants.get(*pubkey).copied())
        .cloned()
        .collect::<Vec<_>>();
    let relevant_message_ids = relevant
        .iter()
        .map(|message| message.message_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut engaged_pubkeys = counts
        .keys()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    for reaction in reactions {
        if participants.contains_key(reaction.reactor_pubkey.as_str())
            && relevant_message_ids.contains(reaction.target_message_id.as_str())
        {
            engaged_pubkeys.insert(reaction.reactor_pubkey.as_str());
        }
    }
    let audience_count = participants.len().max(cohort.len());
    let engaged_count = engaged_pubkeys.len().max(cohort.len());
    let narrow_share = audience_count >= engaged_count.saturating_add(3)
        && engaged_count.saturating_mul(2) <= audience_count;
    let minimum_messages = if narrow_share { 4 } else { 6 };
    if relevant.len() < minimum_messages {
        return None;
    }

    let alternations = relevant
        .windows(2)
        .filter(|pair| pair[0].author_pubkey != pair[1].author_pubkey)
        .count();
    if alternations < 3 {
        return None;
    }
    let started_at = relevant.first()?.created_at;
    let ended_at = relevant.last()?.created_at;
    if ended_at.saturating_sub(started_at) < 20 && relevant.len() < 8 {
        return None;
    }

    let busy_pubkeys = cohort
        .iter()
        .filter(|participant| participant.busy)
        .map(|participant| participant.pubkey.clone())
        .collect();
    Some(ConversationEvidence {
        parent: parent.to_string(),
        cohort,
        busy_pubkeys,
        audience_count,
        engaged_count,
        message_count: relevant.len(),
        alternations,
        started_at,
        ended_at,
        last_message_id: relevant.last()?.message_id.clone(),
    })
}

pub(crate) fn is_substantive_message(body: &str) -> bool {
    let normalized = body
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation())
        .to_ascii_lowercase();
    if normalized.is_empty() || body.trim_start().starts_with("Moving this to #") {
        return false;
    }
    !matches!(
        normalized.as_str(),
        "ack" | "ok" | "okay" | "thanks" | "thank you" | "got it" | "done" | "yes" | "no"
    ) && normalized.chars().count() >= 8
}

pub(crate) fn render_nudge(offer: &MoveOffer) -> String {
    let evidence = &offer.evidence;
    let peers = evidence.cohort.len().saturating_sub(1);
    let peer_word = if peers == 1 { "agent" } else { "agents" };
    format!(
        "<channel-topology-nudge>\n\
You are communicating with {peers} other {peer_word} in #{}. If this is ongoing work with a natural home, consider a focused child channel.\n\
Run `mosaico --yes-lets-move <new-channel-name> <about>` to create or reuse it, adding all {} participating agents plus human users and admins.\n\
</channel-topology-nudge>",
        evidence.parent,
        evidence.cohort.len(),
    )
}

fn lottery_wins(roll: u64, busy_count: usize) -> bool {
    // Each BUSY participant samples independently. A 1/n² chance makes both
    // aggregate nudges and same-window collisions fall as more loops are live.
    let denominator = (busy_count as u128)
        .saturating_mul(busy_count as u128)
        .max(1);
    (roll as u128) <= (u64::MAX as u128) / denominator
}

#[cfg(test)]
#[path = "channel_nudge/tests.rs"]
mod tests;
