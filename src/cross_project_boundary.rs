use crate::config::{BoundaryAction, CrossProjectBoundary};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoundaryNotice {
    pub(crate) action: BoundaryAction,
    pub(crate) owner_workspace: String,
    pub(crate) resolved_path: PathBuf,
    pub(crate) message: String,
}

pub(crate) fn classify(
    policy: CrossProjectBoundary,
    access: FileAccess,
    current_workspace: &str,
    cwd: &Path,
    requested_path: &Path,
    bindings: impl IntoIterator<Item = (String, String)>,
) -> Option<BoundaryNotice> {
    if current_workspace.is_empty() || requested_path.as_os_str().is_empty() {
        return None;
    }
    let target = resolve_lexically(cwd, requested_path)?;
    if target.starts_with("/tmp") {
        return None;
    }

    let matches = bindings
        .into_iter()
        .filter_map(|(workspace, root)| {
            let root = normalize_absolute(Path::new(&root))?;
            target.starts_with(&root).then_some((workspace, root))
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return None;
    }
    let (owner_workspace, _) = &matches[0];
    if owner_workspace == current_workspace {
        return None;
    }

    let action = match access {
        FileAccess::Read => policy.read,
        FileAccess::Write => policy.write,
    };
    if action == BoundaryAction::Allow {
        return None;
    }
    let operation = match access {
        FileAccess::Read => "read",
        FileAccess::Write => "write",
    };
    let owner = channel_name(owner_workspace);
    let current = channel_name(current_workspace);
    let disposition = match action {
        BoundaryAction::Warn => format!("This {operation} is allowed"),
        BoundaryAction::Deny => format!("This {operation} was denied"),
        BoundaryAction::Allow => unreachable!("allow returns before notice construction"),
    };
    let message = format!(
        "{disposition}: {} belongs to Mosaico workspace {owner}, not this session's {current}. \
         Coordinate in {owner} or dispatch an agent there instead of working in another \
         project's workspace directly.",
        target.display()
    );
    Some(BoundaryNotice {
        action,
        owner_workspace: owner_workspace.clone(),
        resolved_path: target,
        message,
    })
}

fn channel_name(value: &str) -> String {
    format!("/{}", value.trim_start_matches('/'))
}

fn resolve_lexically(cwd: &Path, requested: &Path) -> Option<PathBuf> {
    if requested.is_absolute() {
        normalize_absolute(requested)
    } else if cwd.is_absolute() {
        normalize_absolute(&cwd.join(requested))
    } else {
        None
    }
}

fn normalize_absolute(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bindings() -> Vec<(String, String)> {
        vec![
            ("alpha".into(), "/work/alpha".into()),
            ("beta".into(), "/work/beta".into()),
        ]
    }

    #[test]
    fn defaults_warn_cross_workspace_reads_and_deny_writes() {
        let policy = CrossProjectBoundary::default();
        let read = classify(
            policy,
            FileAccess::Read,
            "alpha",
            Path::new("/work/alpha"),
            Path::new("../beta/README.md"),
            bindings(),
        )
        .unwrap();
        let write = classify(
            policy,
            FileAccess::Write,
            "alpha",
            Path::new("/work/alpha"),
            Path::new("/work/beta/src/lib.rs"),
            bindings(),
        )
        .unwrap();

        assert_eq!(read.action, BoundaryAction::Warn);
        assert_eq!(write.action, BoundaryAction::Deny);
        assert_eq!(read.resolved_path, Path::new("/work/beta/README.md"));
        assert!(write.message.contains("Mosaico workspace /beta"));
        assert!(write.message.contains("dispatch an agent there"));
    }

    #[test]
    fn current_unknown_tmp_and_unscoped_paths_are_allowed() {
        let policy = CrossProjectBoundary::default();
        for (current, path) in [
            ("alpha", "/work/alpha/src/lib.rs"),
            ("alpha", "/home/alice/notes.txt"),
            ("alpha", "/tmp/other-worktree/src/lib.rs"),
            ("", "/work/beta/src/lib.rs"),
        ] {
            assert_eq!(
                classify(
                    policy,
                    FileAccess::Write,
                    current,
                    Path::new("/work/alpha"),
                    Path::new(path),
                    bindings(),
                ),
                None,
                "{path}"
            );
        }
    }

    #[test]
    fn ambiguous_workspace_mappings_are_allowed() {
        let duplicated = vec![
            ("beta".into(), "/work/beta".into()),
            ("nested".into(), "/work/beta/subproject".into()),
        ];
        assert_eq!(
            classify(
                CrossProjectBoundary::default(),
                FileAccess::Write,
                "alpha",
                Path::new("/work/alpha"),
                Path::new("/work/beta/subproject/file"),
                duplicated,
            ),
            None
        );
    }

    #[test]
    fn configured_allow_never_produces_a_notice() {
        let policy = CrossProjectBoundary {
            read: BoundaryAction::Allow,
            write: BoundaryAction::Allow,
        };
        assert_eq!(
            classify(
                policy,
                FileAccess::Write,
                "alpha",
                Path::new("/work/alpha"),
                Path::new("/work/beta/file"),
                bindings(),
            ),
            None
        );
    }

    #[test]
    fn classifier_is_lexical_and_does_not_chase_symlinks() {
        assert_eq!(
            classify(
                CrossProjectBoundary::default(),
                FileAccess::Write,
                "alpha",
                Path::new("/work/alpha"),
                Path::new("/work/alpha/link-to-beta/file"),
                bindings(),
            ),
            None
        );
    }
}
