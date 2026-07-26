use super::*;

impl Nip29Materializer {
    // ── relay_channels (kind:39000) ──────────────────────────────────────────

    /// Materialise kind:39000 group metadata into `relay_channels`. The group id
    /// is the event's `d` tag; `parent` (empty for top-level root channels)
    /// distinguishes a session/task channel from a root channel.
    pub fn materialize_channel(store: &Store, event: &Event) {
        let Some(channel_h) = super::super::nostr_tag(event, "d") else {
            return;
        };
        let name = super::super::nostr_tag(event, "name").unwrap_or("");
        let about = super::super::nostr_tag(event, "about").unwrap_or("");
        let parent = super::super::nostr_tag(event, "parent").unwrap_or("");
        if let Err(e) =
            store.upsert_channel(channel_h, name, about, parent, event.created_at.as_secs())
        {
            tracing::error!(
                channel = channel_h,
                error = %e,
                "materialize_channel: relay_channels upsert failed — relay truth diverged from cache"
            );
        }
    }

    // ── relay_channel_members (kind:39001 admins / 39002 members) ─────────────

    /// Materialise kind:39001 — replace the admin rows for the channel, preserving
    /// member rows.
    pub fn materialize_admins(store: &Store, event: &Event) {
        let Some(channel_h) = super::super::nostr_tag(event, "d") else {
            return;
        };
        let admins = collect_p_pubkeys(event);
        if let Err(e) = store.replace_channel_admins(channel_h, &admins, event.created_at.as_secs())
        {
            tracing::error!(
                channel = channel_h,
                error = %e,
                "materialize_admins: replace_channel_admins failed — relay truth diverged from cache"
            );
        }
    }

    /// Materialise kind:39002 — replace the member rows for the channel, preserving
    /// admin rows.
    pub fn materialize_members(store: &Store, event: &Event) {
        let Some(channel_h) = super::super::nostr_tag(event, "d") else {
            return;
        };
        let members = collect_p_pubkeys(event);
        if let Err(e) =
            store.replace_channel_members(channel_h, &members, event.created_at.as_secs())
        {
            tracing::error!(
                channel = channel_h,
                error = %e,
                "materialize_members: replace_channel_members failed — relay truth diverged from cache"
            );
        }
    }
}
