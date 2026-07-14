//! Resolve `channel add --session` by permanent npub/hex identity or by the
//! exact current leased handle. Raw session ids are internal and never accepted.

use crate::daemon::server::DaemonState;
use anyhow::Result;
use std::sync::Arc;

pub(super) struct RemoteSession {
    pub(super) pubkey: String,
    pub(super) slug: String,
    pub(super) backend: String,
}

pub(super) fn remote_session(state: &Arc<DaemonState>, selector: &str) -> Result<RemoteSession> {
    let selector = selector.trim().trim_start_matches('@');
    let matches = state.with_store(|s| remote_sessions(s, selector))?;
    one_remote_session(selector, matches)
}

fn remote_sessions(store: &crate::state::Store, selector: &str) -> Result<Vec<RemoteSession>> {
    if let Some(pubkey) = crate::idref::normalize_pubkey(selector) {
        let Some(profile) = store.get_profile(&pubkey)? else {
            return Ok(Vec::new());
        };
        if profile.is_backend
            || profile.agent_slug.is_empty()
            || profile.name == profile.agent_slug
            || profile.slug == profile.agent_slug
        {
            return Ok(Vec::new());
        }
        return Ok(vec![RemoteSession {
            pubkey,
            slug: profile.slug,
            backend: profile.host,
        }]);
    }

    let Some(pubkey) = store.resolve_profile_handle_pubkey(selector)? else {
        return Ok(Vec::new());
    };
    let Some(profile) = store.get_profile(&pubkey)? else {
        return Ok(Vec::new());
    };
    if profile.name == profile.agent_slug || profile.slug == profile.agent_slug {
        return Ok(Vec::new());
    }
    Ok(vec![RemoteSession {
        pubkey,
        slug: profile.slug,
        backend: profile.host,
    }])
}

fn one_remote_session(selector: &str, matches: Vec<RemoteSession>) -> Result<RemoteSession> {
    match matches.as_slice() {
        [one] => Ok(RemoteSession {
            pubkey: one.pubkey.clone(),
            slug: one.slug.clone(),
            backend: one.backend.clone(),
        }),
        [] => anyhow::bail!("no session matching {selector:?}; use its npub or current handle"),
        _ => anyhow::bail!("session selector is ambiguous; use the full npub"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Status, Store};
    use nostr_sdk::prelude::{Keys, ToBech32};

    fn profile(store: &Store, pubkey: &str, handle: &str) {
        store
            .upsert_profile_with_agent_slug(pubkey, handle, handle, "codex", "remote", false, 1)
            .unwrap();
    }

    #[test]
    fn historical_remote_npub_resolves_without_status() {
        let store = Store::open_memory().unwrap();
        let pubkey = Keys::generate().public_key();
        profile(&store, &pubkey.to_hex(), "old-codex");

        let matches = remote_sessions(&store, &pubkey.to_bech32().unwrap()).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pubkey, pubkey.to_hex());
        assert_eq!(matches[0].backend, "remote");
    }

    #[test]
    fn expired_remote_handle_remains_resolvable() {
        let store = Store::open_memory().unwrap();
        let pubkey = Keys::generate().public_key().to_hex();
        profile(&store, &pubkey, "old-codex");
        store
            .upsert_status(&Status {
                pubkey: pubkey.clone(),
                channel_h: "root".into(),
                slug: "old-codex".into(),
                title: String::new(),
                activity: String::new(),
                busy: false,
                last_seen: 1,
                updated_at: 1,
                expiration: 1,
            })
            .unwrap();

        let matches = remote_sessions(&store, "old-codex").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pubkey, pubkey);
    }

    #[test]
    fn backend_npub_is_not_a_resumable_session() {
        let store = Store::open_memory().unwrap();
        let pubkey = Keys::generate().public_key();
        store
            .upsert_profile(&pubkey.to_hex(), "remote", "remote", "remote", true, 1)
            .unwrap();

        assert!(remote_sessions(&store, &pubkey.to_bech32().unwrap())
            .unwrap()
            .is_empty());
    }
}
