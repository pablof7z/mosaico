use crate::state::{InboxRow, Session, Store};

pub(crate) mod assemble;
pub(crate) mod capture;
mod human_render;
mod member_delta;
mod messages;
mod model;
mod reactions;
pub(crate) mod refs;
mod render;
#[cfg(test)]
mod tests;
mod tree;

pub(crate) use capture::{capture_inputs, ViewInputs};
pub(crate) use member_delta::{inject_member_deltas, inject_member_snapshot};
pub(crate) use messages::is_backend_pubkey;
pub(crate) use model::FabricView;

use human_render::{render_human_view, render_human_views};
use render::render_view;

/// Stringify an already-derived [`FabricView`] into the exact `<mosaico>`
/// snapshot agents see.
pub(crate) fn render_view_text(view: &FabricView) -> String {
    render_view(view)
}

fn derive_view(store: &Store, input: FabricContextInput<'_>) -> anyhow::Result<FabricView> {
    Ok(derive_view_with_inputs(store, input)?.0)
}

/// [`derive_view`] that also hands back the frozen capture.
fn derive_view_with_inputs(
    store: &Store,
    input: FabricContextInput<'_>,
) -> anyhow::Result<(FabricView, ViewInputs)> {
    let cursor = input.cursor;
    let now = input.now;
    let inputs = capture_inputs(store, &input)?;
    let view = assemble::assemble_view(&inputs, cursor, now);
    Ok((view, inputs))
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
    /// A native extension's active daemon delivery lease is a real return path
    /// even though it is not a process-hosting transport locator.
    pub(crate) extension_delivery_live: bool,
    pub(crate) force: bool,
}

fn missing_channel_warning(channel: &str) -> String {
    format!(
        "Fabric channel {channel:?} is unavailable: no relay-backed channel metadata \
         exists locally, so it is not rendered as a joined channel."
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
    pub(crate) attachment_dir: String,
}

#[cfg(test)]
pub(crate) fn render_fabric_context(
    store: &Store,
    input: FabricContextInput<'_>,
) -> Option<String> {
    let force = input.force;
    let view = derive_view(store, input).expect("fabric test fixture has valid channel ancestry");
    if !force && view.is_empty() {
        return None;
    }
    Some(render_view(&view))
}

#[cfg(test)]
pub(crate) fn render_fabric_context_human(
    store: &Store,
    input: FabricContextInput<'_>,
    color: bool,
) -> anyhow::Result<Option<String>> {
    let force = input.force;
    let view = derive_view(store, input)?;
    if !force && view.is_empty() {
        return Ok(None);
    }
    Ok(Some(render_human_view(&view, color)))
}

/// Operator view for one requested root. Agent snapshots still receive the
/// complete topology policy; `who --workspace` uses this narrow projection and
/// appends the other roots as compact summaries itself.
pub(crate) fn render_fabric_context_human_scoped(
    store: &Store,
    input: FabricContextInput<'_>,
    color: bool,
) -> anyhow::Result<Option<String>> {
    let force = input.force;
    let (mut view, inputs) = derive_view_with_inputs(store, input)?;
    if let Some(workspaces) = &mut view.workspaces {
        workspaces.retain(|workspace| workspace.name == inputs.meta.current_workspace);
    }
    if !force && view.is_empty() {
        return Ok(None);
    }
    Ok(Some(render_human_view(&view, color)))
}

/// Test-only agent XML entry point for a full all-workspaces snapshot.
#[cfg(test)]
pub(crate) fn render_fabric_all_workspaces(
    store: &Store,
    roots: &[String],
    now: u64,
    local_host: &str,
    backend_pubkey: &str,
) -> String {
    let scope = roots.first().map(String::as_str).unwrap_or_default();
    let view = derive_view(store, root_input(scope, now, local_host, backend_pubkey))
        .expect("fabric test fixtures have valid channel ancestry");
    render_view(&view)
}

/// Human-rendered counterpart of [`render_fabric_all_workspaces`].
pub(crate) fn render_fabric_all_workspaces_human(
    store: &Store,
    roots: &[String],
    now: u64,
    local_host: &str,
    backend_pubkey: &str,
    color: bool,
) -> anyhow::Result<String> {
    let scope = roots.first().map(String::as_str).unwrap_or_default();
    let view = derive_view(store, root_input(scope, now, local_host, backend_pubkey))?;
    Ok(render_human_views(&[view], color))
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
        extension_delivery_live: false,
        force: true,
    }
}

/// The full agent-facing snapshot from the current retained NMP views.
pub(crate) fn render_full_session_state(
    store: &Store,
    session: &Session,
    self_slug: &str,
    backend_pubkey: &str,
    local_host: &str,
    now: u64,
    extension_delivery_live: bool,
) -> anyhow::Result<String> {
    let (view, _) = derive_view_with_inputs(
        store,
        FabricContextInput {
            session: Some(session),
            scope: "",
            cursor: 0,
            now,
            self_slug,
            self_pubkey: &session.pubkey,
            backend_pubkey,
            local_host,
            forced_messages: &[],
            warnings: &[],
            extension_delivery_live,
            force: true,
        },
    )?;
    Ok(render_view_text(&view))
}

pub(crate) fn inbox_seed(row: &InboxRow) -> FabricMessageSeed {
    FabricMessageSeed {
        id: row.event_id.clone(),
        channel: row.channel_h.clone(),
        from_pubkey: row.from_pubkey.clone(),
        body: row.body.clone(),
        created_at: row.created_at,
        mention: true,
        attachment_dir: row.attachment_dir.clone(),
    }
}
