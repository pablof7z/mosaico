use crate::state::Store;

const MAX_CHANNEL_REF_DEPTH: usize = 32;

/// Full, agent-facing channel path for reply instructions, e.g.
/// `workspace.channel.qa`. Falls back to raw h-tags only when metadata is
/// missing or a node is unnamed.
pub(crate) fn full_channel_ref(store: &Store, channel_h: &str) -> String {
    let mut parts = Vec::new();
    let mut cur = channel_h.to_string();
    for _ in 0..MAX_CHANNEL_REF_DEPTH {
        let Some(channel) = store.get_channel(&cur).ok().flatten() else {
            if parts.is_empty() {
                return channel_h.to_string();
            }
            parts.push(cur);
            break;
        };
        parts.push(
            channel
                .human_name()
                .map(str::to_string)
                .unwrap_or_else(|| channel.channel_h.clone()),
        );
        if channel.parent.is_empty() {
            break;
        }
        cur = channel.parent;
    }
    parts.reverse();
    parts.join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_channel_ref_walks_to_workspace_root() {
        let store = Store::open_memory().unwrap();
        store
            .upsert_channel("root-h", "workspace", "", "", 1)
            .unwrap();
        store
            .upsert_channel("child-h", "channel", "", "root-h", 2)
            .unwrap();
        store
            .upsert_channel("qa-h", "qa", "", "child-h", 3)
            .unwrap();

        assert_eq!(full_channel_ref(&store, "qa-h"), "workspace.channel.qa");
    }

    #[test]
    fn full_channel_ref_falls_back_to_unknown_h() {
        let store = Store::open_memory().unwrap();

        assert_eq!(full_channel_ref(&store, "opaque"), "opaque");
    }
}
