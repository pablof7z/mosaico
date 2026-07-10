use std::collections::BTreeMap;

/// A copy-pasteable channel reference for diagnostics and ambiguity reruns:
/// relative name path when unique, or `@<id-prefix>` when only the id can
/// distinguish the channel.
pub(in crate::daemon::server) fn channel_reference_for(
    store: &crate::state::Store,
    channel_h: &str,
) -> String {
    let root = super::root_channel(store, channel_h);
    if root == channel_h {
        return channel_h.to_string();
    }

    let paths = subtree_paths(store, &root);
    let Some((id, segs)) = paths.iter().find(|(id, _)| id == channel_h) else {
        return channel_id_reference(channel_h);
    };
    channel_reference_from_paths(&paths, id, segs)
}

pub(super) fn channel_reference_from_paths(
    paths: &[(String, Vec<String>)],
    id: &str,
    segs: &[String],
) -> String {
    let path = segs.join("/");
    let path_unique = paths.iter().filter(|(_, s)| s.join("/") == path).count() == 1;
    if path_unique && !path.is_empty() {
        path
    } else {
        channel_id_reference(id)
    }
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

/// True when `segs` ends with `want` (both already lowercased), i.e. `want` is a
/// path suffix of `segs`. `["epic999","planning"]` ends with `["planning"]`.
pub(super) fn path_ends_with(segs: &[String], want: &[String]) -> bool {
    segs.len() >= want.len() && segs[segs.len() - want.len()..] == *want
}
