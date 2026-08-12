use crate::state::Store;

const MAX_CHANNEL_REF_DEPTH: usize = 32;

/// Leading sigil for every agent-facing channel path (`#root/child`).
/// Path segments after the sigil are still separated by `/`.
pub(crate) const CHANNEL_PATH_PREFIX: char = '#';

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

pub(crate) fn split_create_path(path: &str) -> anyhow::Result<(String, String)> {
    let path = path.trim();
    if !path.starts_with(CHANNEL_PATH_PREFIX) || path.ends_with('/') || path.contains("//") {
        anyhow::bail!("channel create <path> requires a full absolute path, e.g. #workspace/child");
    }
    let segments = path[1..].split('/').map(str::trim).collect::<Vec<_>>();
    if segments.iter().any(|segment| segment.is_empty()) {
        anyhow::bail!("channel create <path> requires a full absolute path, e.g. #workspace/child");
    }
    let Some((name, parents)) = segments.split_last() else {
        anyhow::bail!("channel create <path> requires a non-empty path");
    };
    if parents.is_empty() {
        anyhow::bail!(
            "channel create <path> needs a parent, e.g. #workspace/{{name}}; the workspace itself \
             comes from `channel init`, not `channel create`"
        );
    }
    Ok((
        format_channel_ref(&parents.join("/"), &[]),
        (*name).to_string(),
    ))
}

pub(crate) fn format_channel_ref(workspace: &str, descendants: &[String]) -> String {
    let workspace = workspace.trim_start_matches(CHANNEL_PATH_PREFIX);
    let mut reference = format!("{CHANNEL_PATH_PREFIX}{workspace}");
    for descendant in descendants {
        reference.push('/');
        reference.push_str(descendant);
    }
    reference
}

/// Hint shown when a channel name is missing — usually because the shell ate an
/// unquoted `#…` path as a comment.
pub(crate) const MISSING_CHANNEL_NAME_HINT: &str =
    "no channel name; did you put quotes around the name? e.g. '#channel'";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{TestGroup, TestGroupDelivery};

    #[test]
    fn full_channel_ref_walks_to_workspace_root() {
        let store = Store::open_memory().unwrap();
        store.install_test_nmp_group_delivery(TestGroupDelivery::new([
            TestGroup::new("root-h").metadata("workspace", "", "", 1),
            TestGroup::new("child-h").metadata("channel", "", "root-h", 2),
            TestGroup::new("qa-h").metadata("qa", "", "child-h", 3),
        ]));

        assert_eq!(full_channel_ref(&store, "qa-h"), "#root-h/channel/qa");
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
            .install_test_nmp_group_delivery(TestGroupDelivery::new([
                TestGroup::new("workspace").metadata("workspace", "", "", 1)
            ]));

        assert_eq!(full_channel_ref(&store, "workspace"), "#workspace");
    }

    #[test]
    fn split_create_path_requires_an_absolute_child_path() {
        assert_eq!(
            split_create_path("#workspace/epic/planning").unwrap(),
            ("#workspace/epic".into(), "planning".into())
        );
        assert!(split_create_path("workspace/child").is_err());
        assert!(split_create_path("#workspace").is_err());
        assert_eq!(
            split_create_path("#f7z.io/child").unwrap(),
            ("#f7z.io".into(), "child".into())
        );
    }
}
