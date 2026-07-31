//! Global, full-path channel resolution.
//!
//! Every "channel" argument across the CLI (send/read/join/leave/archive/
//! edit/create/add) requires a full absolute path (`#root/child`). Resolution
//! is GLOBAL — no caller-scoped root, opaque-id selector, or relative/suffix
//! matching.

use super::super::*;
use super::paths::{absolute_path_segments, subtree_paths};
use crate::channel_ref::CHANNEL_PATH_PREFIX;

/// Every channel argument is full-path-only. `label` names the argument in the
/// error so the caller sees which one it was (`channel`, `--parent-channel`…).
pub(in crate::daemon::server) fn require_full_path(label: &str, reference: &str) -> Result<()> {
    if !reference.starts_with(CHANNEL_PATH_PREFIX) {
        anyhow::bail!(
            "{label} must be a full path starting with \"{CHANNEL_PATH_PREFIX}\", \
             e.g. #workspace/child (got {reference:?})"
        );
    }
    Ok(())
}

/// Outcome of resolving a channel reference.
pub(in crate::daemon::server) enum ChannelResolution {
    /// Exactly one channel matched → its opaque `channel_h`.
    Unique(String),
    /// Nothing matched.
    NotFound,
}

/// Walk `parent` links up from `channel` to the top-level channel root.
pub(in crate::daemon::server) fn root_channel(
    store: &crate::state::Store,
    channel: &str,
) -> Result<String> {
    crate::daemon::workspace_path::WorkspacePathResolver::new(store).root_for_channel(channel)
}

/// The top-level channel whose durable public root slug matches `slug`, if any.
pub(in crate::daemon::server) fn root_channel_by_slug(
    store: &crate::state::Store,
    slug: &str,
) -> Option<String> {
    store
        .list_root_channels()
        .unwrap_or_default()
        .into_iter()
        .find(|channel| channel.channel_h == slug)
        .map(|c| c.channel_h)
}

/// Resolve a full absolute channel path GLOBALLY. `#root[/child...]` segment 0
/// must name an existing top-level root; each further segment is an exact
/// (case-insensitive) name
///     lookup under the previous segment. Any miss is `NotFound` — there is
///     no fuzzy/suffix matching once a full path is required.
pub(in crate::daemon::server) fn resolve_absolute_channel_ref(
    store: &crate::state::Store,
    reference: &str,
) -> ChannelResolution {
    let reference = reference.trim();
    if reference.is_empty() {
        return ChannelResolution::NotFound;
    }
    let Some(segments) = absolute_path_segments(reference) else {
        return ChannelResolution::NotFound;
    };
    let Some(workspace_h) = root_channel_by_slug(store, &segments[0]) else {
        return ChannelResolution::NotFound;
    };
    if segments.len() == 1 {
        return ChannelResolution::Unique(workspace_h);
    }
    let channels = store.list_channels().unwrap_or_default();
    let mut by_parent: std::collections::HashMap<&str, Vec<&crate::state::Channel>> =
        std::collections::HashMap::new();
    for c in &channels {
        by_parent.entry(c.parent.as_str()).or_default().push(c);
    }
    let mut parent = workspace_h.as_str();
    for seg in &segments[1..] {
        let Some(next) = by_parent
            .get(parent)
            .and_then(|children| children.iter().find(|c| c.name.eq_ignore_ascii_case(seg)))
        else {
            return ChannelResolution::NotFound;
        };
        parent = next.channel_h.as_str();
    }
    ChannelResolution::Unique(parent.to_string())
}

/// Human-facing "here's what actually exists" message for a channel path that
/// didn't resolve. Lists the requested workspace's channels, and separately
/// lists any OTHER path segment that itself happens to be a distinct
/// top-level workspace — the caller may have meant to address that workspace
/// directly instead of nesting under the first one.
pub(in crate::daemon::server) fn describe_missing_channel(
    store: &crate::state::Store,
    reference: &str,
) -> String {
    let Some(segments) = absolute_path_segments(reference) else {
        return format!(
            "{reference:?} is not a valid channel path; use a full path such as #workspace/child"
        );
    };
    let mut out = format!("no channel matching {reference:?}.");
    match root_channel_by_slug(store, &segments[0]) {
        None => {
            let known = workspace_slugs(store);
            if known.is_empty() {
                out.push_str(" No workspaces exist yet.");
            } else {
                out.push_str(&format!(
                    " No workspace named {:?}. Known workspaces: {}",
                    segments[0],
                    known.join(", ")
                ));
            }
        }
        Some(workspace_h) => {
            out.push_str(&format!(
                "\nChannels in {CHANNEL_PATH_PREFIX}{}:\n{}",
                segments[0],
                render_workspace_channels(store, &segments[0], &workspace_h)
            ));
            let mut shown = std::collections::HashSet::new();
            shown.insert(segments[0].to_lowercase());
            for seg in &segments[1..] {
                if !shown.insert(seg.to_lowercase()) {
                    continue;
                }
                if let Some(other_h) = root_channel_by_slug(store, seg) {
                    out.push_str(&format!(
                        "\n{CHANNEL_PATH_PREFIX}{seg} is also a separate workspace. Channels in {CHANNEL_PATH_PREFIX}{seg}:\n{}",
                        render_workspace_channels(store, seg, &other_h)
                    ));
                }
            }
        }
    }
    out
}

fn workspace_slugs(store: &crate::state::Store) -> Vec<String> {
    let mut names: Vec<String> = store
        .list_root_channels()
        .unwrap_or_default()
        .into_iter()
        .map(|c| crate::channel_ref::format_channel_ref(&c.channel_h, &[]))
        .collect();
    names.sort();
    names
}

fn render_workspace_channels(
    store: &crate::state::Store,
    workspace_slug: &str,
    workspace_h: &str,
) -> String {
    let mut lines = vec![format!("  {CHANNEL_PATH_PREFIX}{workspace_slug}")];
    let mut paths = subtree_paths(store, workspace_h);
    paths.sort_by(|a, b| a.1.cmp(&b.1));
    for (_, segs) in paths {
        lines.push(format!(
            "  {CHANNEL_PATH_PREFIX}{workspace_slug}/{}",
            segs.join("/")
        ));
    }
    lines.join("\n")
}
