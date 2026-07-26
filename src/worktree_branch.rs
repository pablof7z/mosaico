use crate::state::Store;
use std::path::Path;

pub(crate) fn for_root(store: &Store, work_root: &str) -> String {
    store
        .workspace_path(work_root)
        .ok()
        .flatten()
        .and_then(|path| at_path(Path::new(&path)))
        .unwrap_or_default()
}

fn at_path(workspace: &Path) -> Option<String> {
    if !workspace.is_dir() {
        return None;
    }
    branch_from_head(workspace)
}

fn branch_from_head(workspace: &Path) -> Option<String> {
    let dot_git = workspace.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let pointer = std::fs::read_to_string(dot_git).ok()?;
        let path = Path::new(pointer.trim().strip_prefix("gitdir:")?.trim());
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace.join(path)
        }
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let branch = head.trim().strip_prefix("ref: refs/heads/")?;
    (!branch.is_empty() && !branch.chars().any(char::is_control)).then(|| branch.to_string())
}
