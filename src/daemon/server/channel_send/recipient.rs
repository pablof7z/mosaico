use super::super::*;
use crate::state::Store;

pub(in crate::daemon::server) struct ResolvedRecipient {
    pub(in crate::daemon::server) pubkey: String,
    pub(in crate::daemon::server) channel: String,
}

pub(super) struct TaggedRecipient {
    pub(super) label: String,
    pub(super) pubkey: String,
    pub(super) channel: String,
}

/// Resolve a recipient to a wire pubkey under the canonical scheme:
///   - `agent@backend-label` resolves through the backend profile cache.
///   - 64-hex / npub selects the permanent session identity directly.
///   - an exact current local handle resolves through the lease authority.
///   - an exact remote handle resolves only with session-status history.
///   - a bare local agent label may resolve through the local profile cache.
///
/// Runtime locators and prefixes are never accepted as chat targets.
pub(in crate::daemon::server) fn resolve_recipient(
    store: &Store,
    my_channel: &str,
    local_host: &str,
    target: &str,
) -> Result<ResolvedRecipient> {
    use crate::idref::{parse_ref, Ref};

    match parse_ref(target) {
        Ref::Agent { slug, host } => {
            let pk = store.resolve_agent_pubkey(&slug, &host)?.with_context(|| {
                format!(
                    "can't resolve {slug}@{host} (no profile seen yet — try `mosaico my session`)"
                )
            })?;
            Ok(ResolvedRecipient {
                pubkey: pk,
                channel: my_channel.to_string(),
            })
        }
        Ref::Pubkey(raw) => {
            let pubkey = nostr::PublicKey::parse(&raw)
                .map(|pk| pk.to_hex())
                .unwrap_or(raw);
            Ok(ResolvedRecipient {
                pubkey,
                channel: my_channel.to_string(),
            })
        }
        Ref::Token(tok) => {
            if let Some(pubkey) = store.pubkey_for_handle(&tok)? {
                if let Some(identity) = store.session_identity(&pubkey)? {
                    let session = store
                        .get_session(&identity.pubkey)?
                        .context("handle points to a missing session")?;
                    return Ok(ResolvedRecipient {
                        pubkey: identity.pubkey,
                        channel: session.channel_h,
                    });
                }
            }
            if let Some(pubkey) = store.resolve_profile_handle_pubkey(&tok)? {
                return Ok(ResolvedRecipient {
                    pubkey,
                    channel: my_channel.to_string(),
                });
            }
            // Bare local agent label: profile fallback for local peers.
            if let Some(pk) = store.resolve_agent_pubkey(&tok, local_host.trim())? {
                return Ok(ResolvedRecipient {
                    pubkey: pk,
                    channel: my_channel.to_string(),
                });
            }
            anyhow::bail!("can't resolve recipient {target:?} (try `mosaico my session`)")
        }
    }
}
