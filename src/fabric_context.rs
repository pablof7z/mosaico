use crate::state::{InboxRow, Session, Store};
use std::path::Path;

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
    pub(crate) local_host: &'a str,
    pub(crate) edge_home: Option<&'a Path>,
    pub(crate) forced_messages: &'a [FabricMessageSeed],
    pub(crate) warnings: &'a [String],
    pub(crate) force: bool,
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
