use super::Nip29Provider;
use crate::domain::DomainEvent;
use crate::fabric::nip29::wire::Nip29WireCodec;
use std::time::Duration;

const PROFILE_FETCH_TIMEOUT: Duration = Duration::from_secs(4);

impl Nip29Provider {
    /// Resolve one exact-author kind:0 through a bounded NMP read.
    ///
    /// The returned value is decoded directly from NMP's Row. It is deliberately
    /// not inserted into Mosaico's retained views: only a live observation owns
    /// those views, and Row identity/source provenance remains at the NMP read
    /// boundary because the product profile domain has no place for it.
    pub(crate) async fn fetch_profile(&self, pubkey: &str) -> Option<crate::domain::Profile> {
        nostr::PublicKey::from_hex(pubkey).ok()?;
        let filter = crate::nmp_host::read::filter(&[0], &[pubkey.to_string()], &[]).ok()?;
        let read = self
            .nmp
            .fetch_profiles(filter, 1, PROFILE_FETCH_TIMEOUT)
            .await
            .ok()?;
        read.rows
            .into_iter()
            .filter_map(|row| {
                let DomainEvent::Profile(profile) = Nip29WireCodec.decode_event(&row.event)? else {
                    return None;
                };
                Some(profile)
            })
            .find(|profile| profile.agent.pubkey == pubkey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AgentRef, Profile};
    use crate::state::Store;
    use nostr::Keys;
    use std::sync::{Arc, Mutex};

    fn provider() -> (Nip29Provider, Arc<crate::nmp_host::NmpHost>) {
        let backend = Keys::generate();
        let nmp = Arc::new(
            crate::nmp_host::NmpHost::open(&[], None, None, &backend).expect("in-memory NMP host"),
        );
        let mut store = Store::open_memory().expect("in-memory state store");
        store.bind_nmp_views_and_feed(nmp.views_handle(), nmp.clone());
        let store = Arc::new(Mutex::new(store));
        let management_nsec = backend.secret_key().to_secret_hex();
        (
            Nip29Provider::new(nmp.clone(), store, Some(management_nsec), None, Vec::new()),
            nmp,
        )
    }

    #[tokio::test]
    async fn bounded_profile_fetch_decodes_the_nmp_row_without_mutating_the_view() {
        let (provider, nmp) = provider();
        let profile_keys = Keys::generate();
        let owner = Keys::generate().public_key().to_hex();
        let profile = Profile::agent(
            AgentRef::new(profile_keys.public_key().to_hex(), "willow-echo-042-codex"),
            "codex",
            "laptop",
            vec![owner.clone()],
        )
        .with_workspace("mosaico");
        let event = provider
            .encode(&DomainEvent::Profile(profile))
            .expect("profile draft")
            .sign_with_keys(&profile_keys)
            .expect("signed profile");
        nmp.script_read_settled_events(vec![event.clone()]);

        let fetched = provider
            .fetch_profile(&profile_keys.public_key().to_hex())
            .await
            .expect("bounded read returns profile");

        assert_eq!(fetched.agent.pubkey, profile_keys.public_key().to_hex());
        assert_eq!(fetched.agent.slug, "willow-echo-042-codex");
        assert_eq!(fetched.agent_slug, "codex");
        assert_eq!(fetched.host, "laptop");
        assert_eq!(fetched.owners, [owner]);
        assert_eq!(fetched.workspace, "mosaico");
        assert!(provider
            .with_store(|store| store
                .get_profile(&profile_keys.public_key().to_hex())
                .unwrap())
            .is_none());
    }
}
