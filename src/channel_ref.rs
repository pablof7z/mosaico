use crate::state::Store;

const MAX_CHANNEL_REF_DEPTH: usize = 32;

/// Full, agent-facing channel path for reply instructions. The workspace is its
/// root channel, so descendants extend the durable root `h` directly.
pub(crate) fn full_channel_ref(store: &Store, channel_h: &str) -> String {
    let mut parts = Vec::new();
    let mut cur = channel_h.to_string();
    for _ in 0..MAX_CHANNEL_REF_DEPTH {
        let Some(channel) = store.get_channel(&cur).ok().flatten() else {
            return String::new();
        };
        if channel.parent.is_empty() {
            parts.reverse();
            return format_channel_ref(&channel.channel_h, &parts);
        }
        let Some(name) = channel.human_name() else {
            return String::new();
        };
        parts.push(name.to_string());
        cur = channel.parent;
    }
    String::new()
}

pub(crate) fn format_channel_ref(workspace: &str, descendants: &[String]) -> String {
    let mut reference = format!("/{workspace}");
    for descendant in descendants {
        reference.push('/');
        reference.push_str(descendant);
    }
    reference
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

        assert_eq!(full_channel_ref(&store, "qa-h"), "/root-h/channel/qa");
    }

    #[test]
    fn full_channel_ref_never_exposes_unknown_h() {
        let store = Store::open_memory().unwrap();

        assert_eq!(full_channel_ref(&store, "opaque"), "");
    }

    #[test]
    fn workspace_is_the_root_channel() {
        let store = Store::open_memory().unwrap();
        store
            .upsert_channel("workspace", "workspace", "", "", 1)
            .unwrap();

        assert_eq!(full_channel_ref(&store, "workspace"), "/workspace");
    }
}
