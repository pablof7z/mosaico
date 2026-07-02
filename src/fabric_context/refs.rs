use crate::state::Store;

pub(super) fn display_name(store: &Store, channel: &str) -> String {
    store
        .get_channel(channel)
        .ok()
        .flatten()
        .and_then(|c| c.human_name().map(str::to_string))
        .unwrap_or_else(|| channel.to_string())
}

pub(super) fn pubkey_ref(store: &Store, pubkey: &str, local_host: &str) -> String {
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
    crate::idref::agent_ref_from(&slug, &host, local_host)
}
