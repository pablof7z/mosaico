use std::collections::BTreeMap;

pub(super) fn canonical_segments(root: &str, reference: &str) -> Option<Vec<String>> {
    let reference = reference.trim();
    if reference.is_empty()
        || reference.contains('.')
        || reference.ends_with('/')
        || reference.contains("//")
    {
        return None;
    }
    let absolute = reference.starts_with('/');
    let path = reference.strip_prefix('/').unwrap_or(reference);
    let mut segments = path
        .split('/')
        .map(str::trim)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if segments.iter().any(String::is_empty) {
        return None;
    }
    if absolute {
        if !segments.first().is_some_and(|segment| segment == root) {
            return None;
        }
        segments.remove(0);
    }
    Some(segments)
}

/// Absolute path segments, e.g. `/workspace/a/b` -> `["workspace","a","b"]`.
/// Unlike [`canonical_segments`], this never scopes to a known root: segment 0
/// IS the workspace slug the caller looks up. `None` for anything that isn't a
/// well-formed absolute path (no leading `/`, `.`, trailing `/`, `//`, or empty
/// segments).
pub(super) fn absolute_path_segments(reference: &str) -> Option<Vec<String>> {
    let reference = reference.trim();
    if !reference.starts_with('/')
        || reference.contains('.')
        || reference.ends_with('/')
        || reference.contains("//")
    {
        return None;
    }
    let segments: Vec<String> = reference[1..]
        .split('/')
        .map(|s| s.trim().to_string())
        .collect();
    if segments.is_empty() || segments.iter().any(String::is_empty) {
        return None;
    }
    Some(segments)
}

/// A copy-pasteable canonical channel path for diagnostics and ambiguity reruns.
pub(in crate::daemon::server) fn channel_reference_for(
    store: &crate::state::Store,
    channel_h: &str,
) -> anyhow::Result<String> {
    let root = super::root_channel(store, channel_h)?;
    if root == channel_h {
        return Ok(crate::channel_ref::full_channel_ref(store, channel_h));
    }
    let paths = subtree_paths(store, &root);
    let Some((_, segments)) = paths.iter().find(|(id, _)| id == channel_h) else {
        return Ok(channel_id_reference(channel_h));
    };
    Ok(canonical_channel_reference(&root, segments))
}

pub(super) fn canonical_channel_reference(root: &str, segs: &[String]) -> String {
    crate::channel_ref::format_channel_ref(root, segs)
}

fn channel_id_reference(id: &str) -> String {
    format!("@{}", &id[..id.len().min(8)])
}

/// Every channel in `root`'s subtree (excluding root) as `(channel_h, name_path)`,
/// where `name_path` is the chain of kind:39000 NAMES from root's child down to
/// the channel. Unnamed nodes (per [`Channel::human_name`] — e.g. session rooms
/// whose name defaulted to their opaque id) are not path-referenceable, so they
/// and their subtrees are skipped.
pub(super) fn subtree_paths(store: &crate::state::Store, root: &str) -> Vec<(String, Vec<String>)> {
    let channels = store.list_channels().unwrap_or_default();
    let mut by_parent: BTreeMap<String, Vec<crate::state::Channel>> = BTreeMap::new();
    for c in channels {
        by_parent.entry(c.parent.clone()).or_default().push(c);
    }
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut stack: Vec<(String, Vec<String>)> = vec![(root.to_string(), Vec::new())];
    let mut guard = 0usize;
    while let Some((id, path)) = stack.pop() {
        guard += 1;
        if guard > 10_000 {
            break;
        }
        let Some(children) = by_parent.get(&id) else {
            continue;
        };
        for c in children {
            let Some(name) = c.human_name() else {
                continue; // unnamed -> not referenceable by path; skip its subtree
            };
            let mut child_path = path.clone();
            child_path.push(name.to_lowercase());
            out.push((c.channel_h.clone(), child_path.clone()));
            stack.push((c.channel_h.clone(), child_path));
        }
    }
    out
}
