use crate::state::Store;

pub(super) fn display_name(store: &Store, channel: &str) -> String {
    store
        .get_channel(channel)
        .ok()
        .flatten()
        .and_then(|c| c.human_name().map(str::to_string))
        .unwrap_or_else(|| channel.to_string())
}

/// The session-bearing member reference: `codename-agent` (never the raw
/// internal `session_id`). Shared by the legacy (`people`) and pure
/// (`assemble`) member-row paths so they can never drift.
pub(super) fn session_ref(
    _session_id: &str,
    status_slug: &str,
    _profile_agent_slug: &str,
) -> String {
    status_slug.to_string()
}

/// The raw profile host for a pubkey (empty when unknown). Kept separate from
/// [`pubkey_ref`] so fallback member rendering can stay identical in both the
/// legacy and pure derivations.
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
    let slug = profile
        .as_ref()
        .map(|p| p.slug.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::util::pubkey_short(pubkey));
    let host = profile
        .as_ref()
        .map(|p| p.host.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| local_host.to_string());
    if profile.as_ref().is_some_and(|p| !p.agent_slug.is_empty()) {
        slug
    } else {
        crate::idref::agent_ref_from(&slug, &host, local_host)
    }
}
