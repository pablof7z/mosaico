use crate::state::{InboxRow, Session, Store};

pub(crate) mod assemble;
mod build;
pub(crate) mod capture;
mod human_render;
mod messages;
mod model;
mod people;
mod refs;
mod render;
#[cfg(test)]
mod tests;

pub(crate) use capture::{
    capture_inputs, MembersInput, MessagesInput, MetaInput, PresenceInput, ViewInputs,
};
pub(crate) use messages::{is_backend_pubkey, p_tag_pubkeys};
pub(crate) use model::FabricView;

use build::build_view;
use human_render::render_human_view;
use render::render_view;

/// Stringify an already-derived [`FabricView`] into the exact `<tenex-edge>`
/// snapshot agents see. The Trellis reconciler derives the view from declared
/// inputs and hands it here, so the "how it is produced" changes while the
/// rendered bytes do not.
pub(crate) fn render_view_text(view: &FabricView) -> String {
    render_view(view)
}

pub(crate) struct FabricContextInput<'a> {
    pub(crate) session: Option<&'a Session>,
    pub(crate) scope: &'a str,
    pub(crate) cursor: u64,
    pub(crate) now: u64,
    pub(crate) self_slug: &'a str,
    pub(crate) self_pubkey: &'a str,
    /// This daemon's management/backend pubkey — its OWN local identity, NOT relay
    /// data — excluded from rendered member rosters so the daemon key never shows
    /// up as a channel member. Sourced from `DaemonState::backend_pubkey()`, never
    /// from a fetched kind:0 (which is absent on a cold cache right after a reset,
    /// the exact case where the mgmt key leaked into `who`). Empty `""` when the
    /// backend identity is unknown or the roster is not rendered (delta turns).
    pub(crate) backend_pubkey: &'a str,
    pub(crate) local_host: &'a str,
    pub(crate) forced_messages: &'a [FabricMessageSeed],
    pub(crate) warnings: &'a [String],
    pub(crate) force: bool,
}

fn missing_channel_warning(channel: &str) -> String {
    format!(
        "Fabric channel {channel:?} is unavailable: no relay-backed channel metadata \
         exists locally, so it is not rendered as an active channel."
    )
}

#[derive(Clone)]
pub(crate) struct FabricMessageSeed {
    pub(crate) id: String,
    pub(crate) channel: String,
    pub(crate) from_pubkey: String,
    pub(crate) body: String,
    pub(crate) created_at: u64,
    pub(crate) mention: bool,
}

pub(crate) fn render_fabric_context(
    store: &Store,
    input: FabricContextInput<'_>,
) -> Option<String> {
    let force = input.force;
    let view = build_view(store, input);
    if !force
        && view.channels.is_empty()
        && view.agents.is_empty()
        && view.important.is_empty()
        && view.warnings.is_empty()
    {
        return None;
    }
    Some(render_view(&view))
}

pub(crate) fn render_fabric_context_human(
    store: &Store,
    input: FabricContextInput<'_>,
    color: bool,
) -> Option<String> {
    let force = input.force;
    let view = build_view(store, input);
    if !force
        && view.channels.is_empty()
        && view.agents.is_empty()
        && view.important.is_empty()
        && view.warnings.is_empty()
    {
        return None;
    }
    Some(render_human_view(&view, color))
}

/// `--all-workspaces`: the same fabric renderer as a single-scope `who`, one
/// workspace block per root channel in `roots`. No single caller session
/// exists across workspaces, so each block is built session-less (no self row, no
/// chatter — `build_view` only pulls messages when a session is present).
pub(crate) fn render_fabric_all_workspaces(
    store: &Store,
    roots: &[String],
    now: u64,
    local_host: &str,
    backend_pubkey: &str,
) -> String {
    let mut out = String::new();
    for root in roots {
        let view = build_view(store, root_input(root, now, local_host, backend_pubkey));
        out.push_str(&render_view(&view));
        out.push('\n');
    }
    out
}

/// Human-rendered counterpart of [`render_fabric_all_workspaces`].
pub(crate) fn render_fabric_all_workspaces_human(
    store: &Store,
    roots: &[String],
    now: u64,
    local_host: &str,
    backend_pubkey: &str,
    color: bool,
) -> String {
    let mut out = String::new();
    for root in roots {
        let view = build_view(store, root_input(root, now, local_host, backend_pubkey));
        out.push_str(&render_human_view(&view, color));
    }
    out
}

fn root_input<'a>(
    root: &'a str,
    now: u64,
    local_host: &'a str,
    backend_pubkey: &'a str,
) -> FabricContextInput<'a> {
    FabricContextInput {
        session: None,
        scope: root,
        cursor: 0,
        now,
        self_slug: "",
        self_pubkey: "",
        backend_pubkey,
        local_host,
        forced_messages: &[],
        warnings: &[],
        force: true,
    }
}

pub(crate) fn inbox_seed(row: &InboxRow) -> FabricMessageSeed {
    FabricMessageSeed {
        id: row.event_id.clone(),
        channel: row.channel_h.clone(),
        from_pubkey: row.from_pubkey.clone(),
        body: row.body.clone(),
        created_at: row.created_at,
        mention: true,
    }
}
