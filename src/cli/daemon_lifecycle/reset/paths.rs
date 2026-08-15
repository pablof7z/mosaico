//! Resolve and audit every path before the reset mutates process or disk state.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::daemon::storage_paths::StoragePaths;

const CONFIG_NAMES: &[&str] = &[
    "config.json",
    "presets.json",
    "agents",
    "workspaces.json",
    "mcp-clients.json",
];

pub(super) struct ProtectedSurfaces {
    targets: Vec<PathBuf>,
    root_barriers: Vec<PathBuf>,
}

pub(super) fn external_pty_socket_directory() -> Result<PathBuf> {
    let socket = crate::pty::session_socket("reset-path-probe");
    let directory = socket.parent().context("PTY socket path has no parent")?;
    audit_pty_socket_directory(directory)?;
    Ok(directory.to_path_buf())
}

fn audit_pty_socket_directory(directory: &Path) -> Result<()> {
    let parent = directory
        .parent()
        .context("PTY socket directory has no parent")?;
    // `/tmp` commonly resolves through an OS-owned symlink. The Mosaico-owned,
    // predictable parent and selected-home leaf must themselves be real dirs.
    reject_symlink(parent)?;
    reject_symlink(directory)
}

pub(super) fn audit_internal_target(target: &Path, selected_home: &Path) -> Result<()> {
    reject_symlink(target)?;
    let target = resolved(target)?;
    if !target.starts_with(selected_home) {
        bail!(
            "refusing runtime target outside the selected instance: {}",
            target.display()
        );
    }
    Ok(())
}

pub(super) fn audit_internal_control_file(target: &Path, selected_home: &Path) -> Result<()> {
    audit_internal_target(target, selected_home)?;
    if target.exists() && !std::fs::symlink_metadata(target)?.file_type().is_file() {
        bail!(
            "reset control path is not a regular file: {}",
            target.display()
        );
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing symlinked runtime target: {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

pub(super) fn protected_surfaces(storage: &StoragePaths) -> Result<ProtectedSurfaces> {
    let mut paths = CONFIG_NAMES
        .iter()
        .map(|name| storage.mosaico_home.join(name))
        .collect::<Vec<_>>();
    paths.push(storage.config_path.clone());
    let mut root_barriers = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        root_barriers.extend(other_instance_roots(&home, &storage.mosaico_home)?);
        let roots = crate::agent_catalog::DiscoveryRoots::installed()?;
        root_barriers.extend([
            roots.codex,
            roots.codex_profiles,
            roots.claude,
            roots.opencode,
            roots.hermes,
            roots.kimi,
            roots.shared_agents,
        ]);
    }
    let root_barriers = root_barriers
        .into_iter()
        .map(|path| resolved(&path))
        .collect::<Result<Vec<_>>>()?;
    paths.extend(root_barriers.iter().cloned());
    let targets = paths
        .into_iter()
        .map(|path| resolved(&path))
        .collect::<Result<Vec<_>>>()?;
    Ok(ProtectedSurfaces {
        targets,
        root_barriers,
    })
}

fn other_instance_roots(home: &Path, selected: &Path) -> Result<Vec<PathBuf>> {
    let mut instances = vec![home.join(".mosaico")];
    let named_root = home.join(".mosaico-instances");
    if named_root.exists() {
        for entry in std::fs::read_dir(&named_root)
            .with_context(|| format!("reading named instances in {}", named_root.display()))?
        {
            instances.push(entry?.path());
        }
    }
    let selected = resolved(selected)?;
    let mut protected = Vec::new();
    for instance in instances {
        let instance = resolved(&instance)?;
        if instance != selected {
            protected.push(instance);
        }
    }
    Ok(protected)
}

pub(super) fn audit_target(
    target: &Path,
    storage: &StoragePaths,
    protected: &ProtectedSurfaces,
) -> Result<()> {
    let target = resolved(target)?;
    let root = Path::new(std::path::MAIN_SEPARATOR_STR);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let broad = [
        Some(root.to_path_buf()),
        home,
        Some(storage.mosaico_home.clone()),
        Some(std::env::temp_dir()),
    ];
    for unsafe_root in broad.into_iter().flatten() {
        if target == resolved(&unsafe_root)? {
            bail!(
                "refusing dangerously broad reset target: {}",
                target.display()
            );
        }
    }
    if let Some(surface) = protected
        .targets
        .iter()
        .find(|path| overlaps(&target, path))
    {
        bail!(
            "refusing reset target {} because it overlaps preserved configuration {}",
            target.display(),
            surface.display()
        );
    }
    Ok(())
}

pub(super) fn audit_selected_home(
    storage: &StoragePaths,
    protected: &ProtectedSurfaces,
) -> Result<()> {
    let selected = resolved(&storage.mosaico_home)?;
    audit_selected_root(&selected, &protected.root_barriers)
}

fn audit_selected_root(selected: &Path, root_barriers: &[PathBuf]) -> Result<()> {
    let broad = [
        PathBuf::from(std::path::MAIN_SEPARATOR_STR),
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default(),
        std::env::temp_dir(),
    ];
    for unsafe_root in broad.iter().filter(|path| !path.as_os_str().is_empty()) {
        let unsafe_root = resolved(unsafe_root)?;
        if selected == unsafe_root || unsafe_root.starts_with(selected) {
            bail!(
                "refusing dangerously broad selected Mosaico home: {}",
                selected.display()
            );
        }
    }
    if let Some(surface) = root_barriers
        .iter()
        .find(|surface| overlaps(selected, surface))
    {
        bail!(
            "refusing selected Mosaico home {} because it overlaps protected state {}",
            selected.display(),
            surface.display()
        );
    }
    Ok(())
}

fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

pub(super) fn resolved(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("reset target must be absolute: {}", path.display());
    }
    if path.exists() {
        return std::fs::canonicalize(path)
            .with_context(|| format!("resolving reset target {}", path.display()));
    }
    let mut cursor = path;
    let mut suffix = Vec::new();
    while !cursor.exists() {
        let name = cursor
            .file_name()
            .context("reset target has no existing ancestor")?;
        suffix.push(name.to_os_string());
        cursor = cursor
            .parent()
            .context("reset target has no existing ancestor")?;
    }
    let mut absolute = std::fs::canonicalize(cursor)
        .with_context(|| format!("resolving reset target ancestor {}", cursor.display()))?;
    for part in suffix.into_iter().rev() {
        absolute.push(part);
    }
    Ok(absolute)
}

#[cfg(test)]
#[path = "paths/tests.rs"]
mod tests;
