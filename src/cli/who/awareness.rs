use crate::state::Store;
use std::fmt::Write as _;

mod summary;

use summary::{
    changed_member_lines, changed_status_items, changed_subchannel_lines, channel_breadcrumb,
    channel_path_line, channel_ref, channel_summary_line, current_activity_lines, member_lines,
    other_active_channel_lines, project_line, subchannels_of,
};

const RECENT_SEMANTIC_WINDOW_SECS: u64 = 10 * 60;

/// `is_root_channel`, but a store error is logged loudly before defaulting to
/// `true` (the safe choice: suppresses the `Subchannels:` section rather than
/// fabricating one) instead of being silently swallowed by `unwrap_or(true)`.
fn is_root_channel_loud(store: &Store, scope: &str) -> bool {
    match store.is_root_channel(scope) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(channel = %scope, error = ?e, "who awareness: is_root_channel lookup failed; assuming root");
            true
        }
    }
}

/// The HOOK first-turn snapshot: the fabric orientation block prefixed with the
/// `[tenex-edge] Fabric context` label. Returns `None` when the channel is not
/// materialized — the hook simply injects nothing in that case.
pub(crate) fn render_awareness_snapshot(
    store: &Store,
    project: &str,
    now: u64,
    self_slug: &str,
    self_pubkey: &str,
    local_host: &str,
) -> Option<String> {
    let breadcrumb = channel_breadcrumb(store, project);
    if breadcrumb.is_empty() {
        return None;
    }
    let proj_label = project_line(store, &breadcrumb, now);
    let chan_label = channel_path_line(store, &breadcrumb, now);
    let body = render_snapshot_body(
        store,
        project,
        &proj_label,
        &chan_label,
        now,
        self_slug,
        self_pubkey,
        local_host,
    );
    Some(format!("[tenex-edge] Fabric context\n\n{body}"))
}

/// The `who` command's fabric view: ALWAYS renders (no materialization guard) and
/// carries no `[tenex-edge]` context label — `who` leads with the project name.
/// When the channel has no kind:39000 record yet (e.g. a root project), the scope
/// id — which for a root IS its human slug — stands in directly as both the
/// project and channel label (never re-derived through the work-title fallback,
/// which would otherwise show a session's title in place of the project name).
pub(crate) fn render_fabric_view(
    store: &Store,
    project: &str,
    now: u64,
    self_slug: &str,
    self_pubkey: &str,
    local_host: &str,
) -> String {
    let breadcrumb = channel_breadcrumb(store, project);
    let (proj_label, chan_label) = if breadcrumb.is_empty() {
        (project.to_string(), project.to_string())
    } else {
        (
            project_line(store, &breadcrumb, now),
            channel_path_line(store, &breadcrumb, now),
        )
    };
    render_snapshot_body(
        store,
        project,
        &proj_label,
        &chan_label,
        now,
        self_slug,
        self_pubkey,
        local_host,
    )
}

/// Shared body for both the hook snapshot and the `who` fabric view: the
/// Project/Channel header (labels pre-computed by the caller), members,
/// subchannels, and other-active channels.
#[allow(clippy::too_many_arguments)]
fn render_snapshot_body(
    store: &Store,
    project: &str,
    proj_label: &str,
    chan_label: &str,
    now: u64,
    self_slug: &str,
    self_pubkey: &str,
    local_host: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Project: {proj_label}");
    let _ = writeln!(out, "Channel: {chan_label}");

    let members = member_lines(store, project, now, self_slug, self_pubkey, local_host);
    write_section(&mut out, "Members:", &members);

    // At the project root, the root's direct children ARE the "other channels"
    // (rendered below); a separate `Subchannels:` section would double-list them.
    // Inside a branch, `Subchannels:` shows that branch's own subtree.
    if !is_root_channel_loud(store, project) {
        let subs = subchannels_of(store, project);
        if !subs.is_empty() {
            let lines = subs
                .iter()
                .map(|(id, _, _)| channel_summary_line(store, id, now))
                .collect::<Vec<_>>();
            write_section(&mut out, "Subchannels:", &lines);
        }
    }

    let other = other_active_channel_lines(
        store,
        project,
        now.saturating_sub(RECENT_SEMANTIC_WINDOW_SECS),
        now,
    );
    write_section(&mut out, "Other active channels, last 10m:", &other);

    out.trim_end().to_string()
}

pub(crate) fn render_awareness_update_since_turn(
    store: &Store,
    since: u64,
    project: &str,
    now: u64,
    exclude_pubkey: Option<&str>,
    local_host: &str,
) -> Option<String> {
    render_awareness_update(
        store,
        since,
        project,
        now,
        exclude_pubkey,
        "last turn",
        local_host,
    )
}

pub(crate) fn render_awareness_update_since_check(
    store: &Store,
    since: u64,
    project: &str,
    now: u64,
    exclude_pubkey: Option<&str>,
    local_host: &str,
) -> Option<String> {
    render_awareness_update(
        store,
        since,
        project,
        now,
        exclude_pubkey,
        "last check",
        local_host,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_awareness_update(
    store: &Store,
    since: u64,
    project: &str,
    now: u64,
    exclude_pubkey: Option<&str>,
    label: &str,
    local_host: &str,
) -> Option<String> {
    // At the project root the direct children surface as "other channels", so the
    // `Subchannels:` delta is suppressed there (empty subs → no subchannel lines).
    let subs = if is_root_channel_loud(store, project) {
        Vec::new()
    } else {
        subchannels_of(store, project)
    };
    let changed = changed_status_items(store, since, project, now, exclude_pubkey);

    let members = changed_member_lines(project, &changed, local_host);
    let subchannels = changed_subchannel_lines(store, since, project, now, &subs, &changed);
    let other = other_active_channel_lines(
        store,
        project,
        since.max(now.saturating_sub(RECENT_SEMANTIC_WINDOW_SECS)),
        now,
    );
    let activity = current_activity_lines(store, project, since, now, exclude_pubkey, local_host);

    if members.is_empty() && subchannels.is_empty() && other.is_empty() && activity.is_empty() {
        return None;
    }

    let mut out = format!("[tenex-edge] Fabric updates since your {label}");
    write_section(&mut out, "Members:", &members);
    write_section(&mut out, "Subchannels:", &subchannels);
    write_section(&mut out, "Other active channels, last 10m:", &other);
    if !activity.is_empty() {
        write_section(
            &mut out,
            &format!("Activity in {}:", channel_ref(store, project, now)),
            &activity,
        );
    }
    Some(out.trim_end().to_string())
}

/// The "New agents available" delta section: invitable agents whose keystore
/// entry was created since the viewer's last turn (`created_at` in `(since, now]`).
/// Decision D — the invitable roster is surfaced ONLY when it CHANGES, never
/// re-injected every turn. `agents` is `(slug, byline, created_at)`; pure and
/// injectable so the fs read stays in the daemon. `None` when nothing is new.
pub(crate) fn new_agent_block(
    agents: &[(String, Option<String>, u64)],
    since: u64,
    now: u64,
) -> Option<String> {
    let fresh: Vec<&(String, Option<String>, u64)> = agents
        .iter()
        .filter(|(_, _, created)| *created > since && *created <= now)
        .collect();
    if fresh.is_empty() {
        return None;
    }
    let mut out = String::from("New agents available (invite with `tenex-edge invite <slug>`):");
    for (slug, byline, _) in fresh {
        match byline.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
            Some(b) => {
                let _ = write!(out, "\n- @{slug} - {b}");
            }
            None => {
                let _ = write!(out, "\n- @{slug}");
            }
        }
    }
    Some(out)
}

fn write_section(out: &mut String, title: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let _ = writeln!(out);
    let _ = writeln!(out);
    let _ = writeln!(out, "{title}");
    for line in lines {
        let _ = writeln!(out, "- {line}");
    }
}

#[cfg(test)]
mod tests;
