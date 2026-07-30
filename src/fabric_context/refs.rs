use crate::state::Store;

/// The raw profile host for a pubkey (empty when unknown). Kept separate from
/// [`pubkey_ref`] because capture stores the host independently from the
/// rendered public reference.
pub(super) fn profile_host(store: &Store, pubkey: &str) -> String {
    store
        .get_profile(pubkey)
        .ok()
        .flatten()
        .map(|p| p.host)
        .unwrap_or_default()
}

pub(crate) fn pubkey_ref(store: &Store, pubkey: &str, local_host: &str) -> String {
    let profile = store.get_profile(pubkey).ok().flatten();
    let session = store.get_session(pubkey).ok().flatten();
    let slug = profile
        .as_ref()
        .map(|p| p.slug.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| session.as_ref().map(|row| row.agent_slug.clone()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::util::pubkey_short(pubkey));
    let host = profile
        .as_ref()
        .map(|p| p.host.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| session.as_ref().map(|_| local_host.to_string()))
        .unwrap_or_else(|| local_host.to_string());
    if profile.as_ref().is_some_and(|p| !p.agent_slug.is_empty()) {
        slug
    } else {
        crate::idref::agent_ref_from(&slug, &host, local_host)
    }
}
