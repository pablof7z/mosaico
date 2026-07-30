use crate::session_state::SessionState;
use crate::state::Store;
use anyhow::Result;
use serde::Serialize;

#[cfg(test)]
#[path = "coaching/tests.rs"]
mod tests;

const COORDINATION_GUIDE: &str = "~/.agents/skills/mosaico/references/coordination-guide.md";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct CoachingNotice {
    level: &'static str,
    code: &'static str,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tagged_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    typed_label: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    candidates: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matched_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matched_agent_state: Option<&'static str>,
}

pub(super) fn redundant_prefix(label: String) -> CoachingNotice {
    CoachingNotice {
        level: "notice",
        code: "redundant_tag_prefix",
        summary: format!(
            "NOTICE: Removed a redundant leading agent label. \
             `--tag {label}` already adds the @{label} mention; \
             do not include it at the start of `--message`."
        ),
        tagged_agent: Some(label),
        typed_label: None,
        candidates: Vec::new(),
        matched_agent: None,
        matched_agent_state: None,
    }
}

pub(super) fn ack_like(message: &str) -> Option<CoachingNotice> {
    is_ack_like(message).then(|| CoachingNotice {
        level: "info",
        code: "ack_like_chat",
        summary: format!(
            "INFO: Was this simply an ACK? Next time use \
             `mosaico channel react <message-id> \"👍\"`. Read `{COORDINATION_GUIDE}`."
        ),
        tagged_agent: None,
        typed_label: None,
        candidates: Vec::new(),
        matched_agent: None,
        matched_agent_state: None,
    })
}

pub(super) fn untagged_agent_prefix(
    store: &Store,
    message: &str,
    channel: &str,
    self_pubkey: &str,
    backend_pubkey: &str,
    now: u64,
) -> Result<Option<CoachingNotice>> {
    let Some(typed_label) = leading_label(message) else {
        return Ok(None);
    };
    let candidates = participant_candidates(store, channel, self_pubkey, backend_pubkey, now)?;
    let exact = matching_candidates(&typed_label, &candidates, true);
    let matched = if exact.is_empty() {
        matching_candidates(&typed_label, &candidates, false)
    } else {
        exact
    };
    if matched.is_empty() {
        return Ok(None);
    }
    if matched.len() > 1 {
        let names = matched
            .iter()
            .map(|candidate| candidate.handle.clone())
            .collect::<Vec<_>>();
        let shown = names
            .iter()
            .map(|name| format!("@{name}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Ok(Some(CoachingNotice {
            level: "warn",
            code: "untagged_agent_prefix_ambiguous",
            summary: format!(
                "WARN: This message published but won't tag anyone. \
                 \"{typed_label}:\" matches multiple agents: {shown}. No recipient was inferred."
            ),
            tagged_agent: None,
            typed_label: Some(typed_label),
            candidates: names,
            matched_agent: None,
            matched_agent_state: None,
        }));
    }

    let candidate = &matched[0];
    let idle = candidate.state == SessionState::Idle;
    let idle_note = idle.then(|| {
        format!(
            " @{} was idle when this message was sent. Ambient chat will not start its turn, \
             so tag it explicitly if it should see this now.",
            candidate.handle
        )
    });
    Ok(Some(CoachingNotice {
        level: "warn",
        code: "untagged_agent_prefix",
        summary: format!(
            "WARN: This message published but won't tag anyone. \
             \"{typed_label}:\" looks like @{}. It remains ambient channel context \
             and will be picked up whenever agents next run.{}",
            candidate.handle,
            idle_note.as_deref().unwrap_or_default()
        ),
        tagged_agent: None,
        typed_label: Some(typed_label),
        candidates: vec![candidate.handle.clone()],
        matched_agent: Some(candidate.handle.clone()),
        matched_agent_state: idle.then_some("idle"),
    }))
}

fn is_ack_like(message: &str) -> bool {
    let normalized = message
        .trim()
        .trim_end_matches(['.', '!', ','])
        .trim()
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "ack"
            | "acknowledged"
            | "got it"
            | "gotcha"
            | "ok"
            | "okay"
            | "sounds good"
            | "thank you"
            | "thanks"
            | "will do"
            | "👍"
            | "✅"
            | "👀"
    )
}

fn leading_label(message: &str) -> Option<String> {
    let trimmed = message.trim_start();
    let (label, rest) = trimmed.split_once(':')?;
    if label.is_empty()
        || !label
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        || !rest.starts_with(char::is_whitespace)
    {
        return None;
    }
    Some(label.to_string())
}

#[derive(Clone, Debug)]
struct Candidate {
    handle: String,
    state: SessionState,
}

fn participant_candidates(
    store: &Store,
    channel: &str,
    self_pubkey: &str,
    backend_pubkey: &str,
    now: u64,
) -> Result<Vec<Candidate>> {
    let mut candidates = Vec::new();
    for member in store.list_channel_members(channel)? {
        if member.pubkey == self_pubkey || member.pubkey == backend_pubkey {
            continue;
        }
        let profile = store.get_profile(&member.pubkey)?;
        if profile.as_ref().is_some_and(|profile| profile.is_backend) {
            continue;
        }
        let session = store.get_session(&member.pubkey)?;
        let status = store.get_status(&member.pubkey, channel)?;
        if session.is_none() && status.is_none() {
            continue;
        }
        if session.is_none()
            && profile
                .as_ref()
                .is_some_and(|profile| profile.agent_slug.trim().is_empty() && !profile.is_backend)
        {
            continue;
        }
        let handle = if let Some(session) = session.as_ref() {
            store
                .session_identity(&session.pubkey)?
                .map(|identity| identity.display_slug())
        } else {
            status
                .as_ref()
                .map(|status| status.slug.trim().to_string())
                .filter(|slug| !slug.is_empty())
                .or_else(|| {
                    profile
                        .as_ref()
                        .map(|profile| profile.slug.trim().to_string())
                        .filter(|slug| !slug.is_empty())
                })
        };
        let Some(handle) = handle else {
            continue;
        };
        let state = match (session.as_ref(), status.as_ref()) {
            (Some(session), published) => {
                crate::session_presence::local(store, session, published).state
            }
            (None, Some(status)) => crate::session_presence::remote(status, now).state,
            (None, None) => SessionState::Offline,
        };
        candidates.push(Candidate { handle, state });
    }
    candidates.sort_by_key(|candidate| candidate.handle.to_ascii_lowercase());
    candidates.dedup_by(|left, right| left.handle.eq_ignore_ascii_case(&right.handle));
    Ok(candidates)
}

fn matching_candidates<'a>(
    typed: &str,
    candidates: &'a [Candidate],
    exact: bool,
) -> Vec<&'a Candidate> {
    candidates
        .iter()
        .filter(|candidate| {
            if exact {
                candidate.handle.eq_ignore_ascii_case(typed)
            } else {
                candidate
                    .handle
                    .to_ascii_lowercase()
                    .starts_with(&typed.to_ascii_lowercase())
            }
        })
        .collect()
}
