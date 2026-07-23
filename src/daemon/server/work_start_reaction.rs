//! Agent-signed, best-effort acknowledgement that work on a kind:9 has begun.

use super::*;

const EMOJI: &str = "⚙️";

/// Consume this session's durable handoffs and publish one reaction per event.
/// The claim is completed before the asynchronous write so a failed reaction can
/// never duplicate work or hold up a harness hook.
pub(crate) fn publish_for_started_turn(state: &Arc<DaemonState>, rec: &crate::state::Session) {
    publish_claims(
        state,
        rec,
        state.with_store(|store| store.take_work_start_claims(&rec.pubkey, now_secs())),
    );
}

/// Publish only the handoffs in the exact prompt that started this managed turn.
pub(crate) fn publish_for_started_events(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    event_ids: &[String],
) {
    publish_claims(
        state,
        rec,
        state.with_store(|store| {
            store.take_work_start_claims_for_events(&rec.pubkey, event_ids, now_secs())
        }),
    );
}

fn publish_claims(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    claims: anyhow::Result<Vec<crate::state::work_start::WorkStartClaim>>,
) {
    let claims = match claims {
        Ok(claims) => claims,
        Err(error) => {
            tracing::warn!(pubkey = %rec.pubkey, %error, "work-start reaction claim failed");
            return;
        }
    };
    if claims.is_empty() {
        return;
    }
    let keys = match state.session_signing_keys(&rec.pubkey) {
        Ok(keys) => keys,
        Err(error) => {
            tracing::warn!(pubkey = %rec.pubkey, %error, "work-start reaction skipped: signer unavailable");
            return;
        }
    };

    let reactor = state.session_instance(rec).agent_ref();
    let provider = state.provider.clone();
    tokio::spawn(async move {
        for claim in claims {
            let reaction = crate::domain::Reaction {
                reactor: reactor.clone(),
                channel: claim.channel_h,
                target_event_id: claim.event_id,
                emoji: EMOJI.into(),
            };
            if let Err(error) = provider.publish_reaction_checked(&reaction, &keys).await {
                tracing::warn!(
                    event_id = %reaction.target_event_id,
                    channel = %reaction.channel,
                    %error,
                    "work-start reaction publish failed"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_start_reaction_is_session_signed_and_targets_the_kind9() {
        let reaction = crate::domain::Reaction {
            reactor: crate::domain::AgentRef::new("agent-key", "builder"),
            channel: "room".into(),
            target_event_id: "kind9-id".into(),
            emoji: EMOJI.into(),
        };
        assert_eq!(reaction.reactor.pubkey, "agent-key");
        assert_eq!(reaction.target_event_id, "kind9-id");
        assert_eq!(reaction.emoji, "⚙️");
    }
}
