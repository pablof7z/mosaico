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
        (Harness::ClaudeCode, "claude"),
        (Harness::Codex, "codex"),
        (Harness::Opencode, "opencode"),
        (Harness::Grok, "grok"),
        (Harness::Goose, "goose"),
        (Harness::Hermes, "hermes"),
    ];
    candidates
        .into_iter()
        .filter(|(_, bin)| crate::host_env::resolve_executable(home, path, bin).is_some())
        .map(|(harness, _)| harness)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_only_launchable_binaries_in_stable_order() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".codex")).unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        for executable in ["codex", "opencode", "goose", "hermes"] {
            let path = bin.join(executable);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        assert_eq!(
            detect_with(root.path(), Some(bin.as_os_str())),
            [
                Harness::Codex,
                Harness::Opencode,
                Harness::Goose,
                Harness::Hermes,
            ]
        );
    }

    #[test]
    fn config_directory_without_launchable_binary_is_not_advertised() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".codex")).unwrap();

        assert!(detect_with(root.path(), Some(std::ffi::OsStr::new("/usr/bin:/bin"))).is_empty());
    }
}
