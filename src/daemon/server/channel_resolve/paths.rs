use std::collections::BTreeMap;

pub(super) fn canonical_segments(root: &str, reference: &str) -> Vec<String> {
    let mut segments = reference
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if segments.first().is_some_and(|segment| segment == root) {
        segments.remove(0);
    }
    segments
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
    format!("{root}.{}", segs.join("."))
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

/// Every channel id in `root`'s subtree, including channels below unnamed
/// session rooms. Explicit `@<prefix>` selectors must not inherit the
/// human-name path filter.
pub(super) fn subtree_ids(store: &crate::state::Store, root: &str) -> Vec<String> {
    let channels = store.list_channels().unwrap_or_default();
    let mut by_parent: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for channel in channels {
        by_parent
            .entry(channel.parent)
            .or_default()
            .push(channel.channel_h);
    }

    let mut out = vec![root.to_string()];
    let mut stack = vec![root.to_string()];
    let mut guard = 0usize;
    while let Some(id) = stack.pop() {
        guard += 1;
        if guard > 10_000 {
            break;
        }
        let Some(children) = by_parent.get(&id) else {
            continue;
        };
        for child in children {
            out.push(child.clone());
            stack.push(child.clone());
        }
    }
    out
}

/// True when `segs` ends with `want` (both already lowercased), i.e. `want` is a
/// path suffix of `segs`. `["epic999","planning"]` ends with `["planning"]`.
pub(super) fn path_ends_with(segs: &[String], want: &[String]) -> bool {
    segs.len() >= want.len() && segs[segs.len() - want.len()..] == *want
}
