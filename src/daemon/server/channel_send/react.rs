use super::super::*;
use crate::domain::{Reaction, MAX_REACTION_EMOJI_BYTES};
use anyhow::{bail, Context, Result};

#[derive(serde::Deserialize, Default)]
struct ChannelReactParams {
    id: String,
    emoji: String,
}

/// React to a specific channel message with an emoji (NIP-25 kind:7), signed by
/// the caller's per-session key. A reaction is a NON-disruptive acknowledgement:
/// it never enqueues an inbox row and never rings a doorbell, so it cannot wake an
/// idle agent or inject mid-turn. The target sees it as compact turn-start
/// awareness.
pub(in crate::daemon::server) async fn rpc_channel_react(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: ChannelReactParams =
        serde_json::from_value(params.clone()).context("parsing channel_react params")?;
    if p.id.trim().is_empty() {
        bail!("react id must not be empty");
    }
    let emoji = validate_emoji(&p.emoji)?;

    let rec = resolve_session(state, &CallerAnchor::from_params(params))?;
    let original = state
        .with_store(|s| s.get_message_by_prefix(p.id.trim()))
        .with_context(|| format!("resolving react id {:?}", p.id.trim()))?
        .with_context(|| format!("message not found for react id {:?}", p.id.trim()))?;
    // React against the target's native event id so the `e` tag references the
    // exact relay event the peer will see.
    let target_event_id = original
        .native_event_id
        .clone()
        .unwrap_or_else(|| original.message_id.clone());

    let instance = state.session_instance(&rec);
    let keys = state.session_signing_keys(&rec.pubkey)?;
    let reaction = Reaction {
        reactor: instance.agent_ref(),
        channel: original.channel_h.clone(),
        target_event_id: target_event_id.clone(),
        emoji: emoji.clone(),
    };
    // NOTE: intentionally NO enqueue_inbox and NO ring_doorbells here — a reaction
    // is passive awareness only. Do not add either without revisiting the
    // non-disruption guarantee.
    let event_id = state
        .provider
        .publish_reaction_checked(&reaction, &keys)
        .await?;

    Ok(serde_json::json!({
        "event_id": event_id,
        "channel": original.channel_h,
        "target": target_event_id,
        "emoji": emoji,
    }))
}

/// Accept a trimmed, non-empty reaction of at most [`MAX_EMOJI_BYTES`] bytes with
/// no control/whitespace characters. `+`/`-` are the NIP-25 like/dislike
/// shorthands; everything else is expected to be a single emoji.
fn validate_emoji(raw: &str) -> Result<String> {
    let emoji = raw.trim();
    if emoji.is_empty() {
        bail!("reaction emoji must not be empty");
    }
    if emoji.len() > MAX_REACTION_EMOJI_BYTES {
        bail!("reaction emoji is too long (max {MAX_REACTION_EMOJI_BYTES} bytes)");
    }
    if emoji.chars().any(|c| c.is_control() || c.is_whitespace()) {
        bail!("reaction emoji must not contain whitespace or control characters");
    }
    // Belt-and-suspenders: the detailed checks above must agree with the canonical
    // trust-boundary predicate that the wire decoder also enforces.
    debug_assert!(Reaction::emoji_is_valid(emoji));
    Ok(emoji.to_string())
}

#[cfg(test)]
mod tests {
    use super::validate_emoji;

    #[test]
    fn accepts_emoji_and_plus_minus() {
        assert_eq!(validate_emoji("👍").unwrap(), "👍");
        assert_eq!(validate_emoji(" ✅ ").unwrap(), "✅");
        assert_eq!(validate_emoji("+").unwrap(), "+");
        assert_eq!(validate_emoji("-").unwrap(), "-");
    }

    #[test]
    fn rejects_empty_whitespace_and_oversized() {
        assert!(validate_emoji("").is_err());
        assert!(validate_emoji("   ").is_err());
        assert!(validate_emoji("a b").is_err());
        assert!(validate_emoji(&"x".repeat(32)).is_err());
    }
}
