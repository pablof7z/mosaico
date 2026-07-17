use crate::session::Harness;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Detect installed native harnesses from current host state. This is live
/// capability discovery, never a persisted config snapshot.
pub fn detect() -> Result<Vec<Harness>> {
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .context("HOME is required to detect installed harnesses")?;
    Ok(detect_with(&home, std::env::var_os("PATH").as_deref()))
}

fn detect_with(home: &Path, path: Option<&std::ffi::OsStr>) -> Vec<Harness> {
    let candidates = [
        (Harness::ClaudeCode, ".claude", "claude"),
        (Harness::Codex, ".codex", "codex"),
        (Harness::Opencode, ".config/opencode", "opencode"),
        (Harness::Grok, ".grok", "grok"),
    ];
    candidates
        .into_iter()
        .filter(|(_, dir, bin)| home.join(dir).exists() || bin_on_path(path, bin))
        .map(|(harness, _, _)| harness)
        .collect()
}

fn bin_on_path(path: Option<&std::ffi::OsStr>, bin: &str) -> bool {
    path.into_iter()
        .flat_map(std::env::split_paths)
        .any(|dir| dir.join(bin).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_home_directories_and_path_binaries_in_stable_order() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".codex")).unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::fs::write(bin.join("opencode"), "").unwrap();

        assert_eq!(
            detect_with(root.path(), Some(bin.as_os_str())),
            [Harness::Codex, Harness::Opencode]
        );
    }
}
