use super::super::*;
use crate::state::Store;

pub(in crate::daemon::server) struct ResolvedRecipient {
    pub(in crate::daemon::server) pubkey: String,
    pub(in crate::daemon::server) target_run_id: Option<String>,
    pub(in crate::daemon::server) channel: String,
}

/// Resolve a recipient to a wire pubkey under the canonical scheme:
///   - `agent@backend-label` resolves through the backend profile cache.
///   - 64-hex / npub selects the permanent session identity directly.
///   - an exact current local handle resolves through the lease authority.
///   - an exact remote handle resolves only with session-status history.
///   - a bare local agent label may resolve through the local profile cache.
///
/// Raw session ids and prefixes are internal correlation values and are never
/// accepted as chat targets.
pub(in crate::daemon::server) fn resolve_recipient(
    store: &Store,
    my_channel: &str,
    local_host: &str,
    target: &str,
) -> Result<ResolvedRecipient> {
    use crate::idref::{parse_ref, Ref};

    let session_recipient =
        |store: &Store, session_id: String, fallback_pk: String, channel: String| {
            let pubkey = store
                .session_identity_for_session(&session_id)
                .ok()
                .flatten()
                .map(|i| i.pubkey)
                .or_else(|| {
                    store
                        .get_session(&session_id)
                        .ok()
                        .flatten()
                        .map(|s| s.agent_pubkey)
                })
                .unwrap_or(fallback_pk);
            ResolvedRecipient {
                pubkey,
                target_run_id: Some(session_id),
                channel,
            }
        };

    match parse_ref(target) {
        Ref::Agent { slug, host } => {
            let pk = store.resolve_agent_pubkey(&slug, &host)?.with_context(|| {
                format!(
                    "can't resolve {slug}@{host} (no profile seen yet — try `tenex-edge my session`)"
                )
            })?;
            Ok(ResolvedRecipient {
                pubkey: pk,
                target_run_id: None,
                channel: my_channel.to_string(),
            })
        }
        Ref::Pubkey(raw) => {
            let pubkey = nostr_sdk::prelude::PublicKey::parse(&raw)
                .map(|pk| pk.to_hex())
                .unwrap_or(raw);
            Ok(ResolvedRecipient {
                pubkey,
                target_run_id: None,
                channel: my_channel.to_string(),
            })
        }
        Ref::Token(tok) => {
            if let Some(pubkey) = store.pubkey_for_handle(&tok)? {
                if let Some(session) = store.session_for_pubkey(&pubkey)? {
                    return Ok(session_recipient(
                        store,
                        session.session_id,
                        pubkey,
                        session.channel_h,
                    ));
                }
            }
            if let Some(pubkey) = store.resolve_profile_handle_pubkey(&tok)? {
                return Ok(ResolvedRecipient {
                    pubkey,
                    target_run_id: None,
                    channel: my_channel.to_string(),
                });
            }
            // Bare local agent label: profile fallback for local peers.
            if let Some(pk) = store.resolve_agent_pubkey(&tok, local_host.trim())? {
                return Ok(ResolvedRecipient {
                    pubkey: pk,
                    target_run_id: None,
                    channel: my_channel.to_string(),
                });
            }
            anyhow::bail!("can't resolve recipient {target:?} (try `tenex-edge my session`)")
        }
    }
}
