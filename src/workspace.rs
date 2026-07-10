//! Workspace-slug resolution and the per-machine slug→path map.
//!
//! A *workspace* is identified by a short slug. The slug is resolved from a
//! working directory by the following order:
//!
//!   1. **Git repo name** — derived from `git rev-parse --git-common-dir`, so a
//!      repo and all of its git worktrees resolve to the **same** slug (the
//!      basename of the shared main repo root).
//!   2. **`~/.tenex-edge/workspaces.json`** — a JSON object mapping slugs to
//!      absolute paths. The cwd itself, or its nearest ancestor present in the
//!      map, wins. This is the only way to give a non-git directory a workspace.
//!   3. Otherwise: `Err(NoWorkspace)`. The caller decides how to surface — hooks
//!      exit 0 silently; explicit CLI verbs print a "run `tenex-edge channel
//!      init` or `git init`" message and exit non-zero.
//!
//! The map at `~/.tenex-edge/workspaces.json` is the single source of truth for
//! non-git workspaces. A legacy `projects.json` is read once for migration.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Error returned by [`resolve`]. Carries the working directory that had no
/// resolvable workspace, so callers can format a helpful message.
#[derive(Debug)]
pub struct NoWorkspace {
    pub cwd: PathBuf,
}

impl std::fmt::Display for NoWorkspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no known workspace in {}", self.cwd.display())
    }
}

impl std::error::Error for NoWorkspace {}

/// Resolve the workspace slug for a working directory.
///
/// Returns the slug, or [`NoWorkspace`] when the cwd is not in a git repo and is
/// not registered in `~/.tenex-edge/workspaces.json` (nor any ancestor of it).
pub fn resolve(cwd: &Path) -> std::result::Result<String, NoWorkspace> {
    // 1. git repo name (shared across all worktrees of the same repo).
    if let Some(root) = git_toplevel(cwd) {
        if let Some(name) = basename(&root) {
            return Ok(name);
        }
    }
    // 2. workspaces.json: cwd or nearest ancestor present in the map.
    if let Some(slug) = lookup_in_map(cwd) {
        return Ok(slug);
    }
    // 3. No resolvable workspace.
    Err(NoWorkspace {
        cwd: cwd.to_path_buf(),
    })
}

/// The workspace directory for a working dir: the dir whose slug `resolve`
/// returned. Mirrors `resolve`'s search order:
///   1. the git repo root (derived from git-common-dir, shared across worktrees),
///   2. the nearest ancestor (or cwd itself) present in `workspaces.json`,
///   3. else `None`.
pub fn workspace_dir(cwd: &Path) -> Option<PathBuf> {
    if let Some(root) = git_toplevel(cwd) {
        return Some(root);
    }
    workspace_dir_from_map(cwd)
}

/// Workspace-relative working directory for the public presence/status wire
/// field. Never leaks the absolute `$HOME/...` path:
///   - cwd under a resolvable workspace dir → the relative path (`/`-joined),
///     with the dir itself rendered as `.`.
///   - no resolvable dir → the cwd **basename** (still not absolute).
///   - empty basename (fs root) → empty string.
pub fn rel_cwd(cwd: &Path) -> String {
    if let Some(root) = workspace_dir(cwd) {
        if let Ok(rel) = cwd.strip_prefix(&root) {
            let s = rel.to_string_lossy().replace('\\', "/");
            return if s.is_empty() { ".".to_string() } else { s };
        }
    }
    basename(cwd).unwrap_or_default()
}

/// Like [`resolve`], but on [`NoWorkspace`] returns an `anyhow::Error` whose
/// `Display` form is the user-facing "no known workspace … run `tenex-edge
/// channel init` or `git init`" message. For the explicit-CLI-verb path only;
/// hooks should call [`resolve`] and exit 0 on `Err`.
pub fn resolve_or_bail(cwd: &Path) -> Result<String> {
    resolve(cwd).map_err(|e| {
        anyhow::anyhow!(
            "{e}; run `tenex-edge channel init` or `git init` first, or pass `--workspace <slug>`"
        )
    })
}

/// Register the current directory as a new workspace in `workspaces.json`, using
/// the directory's basename as the slug. Returns the slug and the absolute path
/// that were written. Errors if the basename is empty, the path can't be
/// canonicalized, or the slug is already mapped to a different path (unless
/// `force` overwrites it).
pub fn register_workspace(cwd: &Path, force: bool) -> Result<(String, PathBuf)> {
    let slug = basename(cwd).context("current directory has no basename")?;
    let abs = cwd
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", cwd.display()))?;
    let mut map = read_map()?;
    if let Some(existing) = map.get(&slug) {
        if Path::new(existing.as_str()) == abs.as_path() {
            // Already registered; no-op.
            return Ok((slug, abs));
        }
        if !force {
            anyhow::bail!(
                "slug {slug:?} is already mapped to {existing}; pass --force to overwrite"
            );
        }
    }
    map.insert(slug.clone(), abs.to_string_lossy().to_string());
    write_map(&map)?;
    Ok((slug, abs))
}

// ── workspaces.json map ──────────────────────────────────────────────────────

/// The on-disk map at `<edge_home>/workspaces.json`: a JSON object mapping
/// slugs to absolute paths.
fn map_path() -> PathBuf {
    crate::config::edge_home().join("workspaces.json")
}

/// The legacy on-disk map path, read once for migration if the current map is
/// absent. Existing installs registered non-git workspaces here.
fn legacy_map_path() -> PathBuf {
    crate::config::edge_home().join("projects.json")
}

/// Read the slug→path map. A MISSING file is "no workspaces registered yet" (an
/// empty map) — unless a legacy map exists, in which case it is read for
/// migration. A PRESENT-but-unparseable file is a hard error: defaulting to
/// empty would silently drop every non-git workspace the user registered, so we
/// surface the parse failure instead of guessing.
fn read_map() -> Result<std::collections::BTreeMap<String, String>> {
    let p = map_path();
    let (path, s) = match std::fs::read_to_string(&p) {
        Ok(s) => (p, s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Fall back to the legacy map for one-time migration.
            let legacy = legacy_map_path();
            match std::fs::read_to_string(&legacy) {
                Ok(s) => (legacy, s),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
                Err(e) => return Err(e).with_context(|| format!("reading {}", legacy.display())),
            }
        }
        Err(e) => {
            return Err(e).with_context(|| format!("reading {}", p.display()));
        }
    };
    // Tolerate either a bare object or any whitespace; serde_json handles both.
    serde_json::from_str::<std::collections::BTreeMap<String, String>>(&s)
        .with_context(|| format!("parsing {} (corrupt workspace map)", path.display()))
}

/// Write the slug→path map, creating the parent dir if necessary.
fn write_map(obj: &std::collections::BTreeMap<String, String>) -> Result<()> {
    let p = map_path();
    if let Some(parent) = p.parent() {
        crate::config::ensure_dir(parent)?;
    }
    let s = serde_json::to_string_pretty(obj).context("serializing workspaces.json")?;
    std::fs::write(&p, s).with_context(|| format!("writing {}", p.display()))?;
    Ok(())
}

/// Look up `cwd` (or its nearest ancestor) in the map. Returns the slug for the
/// nearest ancestor present, or `None` if no ancestor is registered.
fn lookup_in_map(cwd: &Path) -> Option<String> {
    let map = match read_map() {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "lookup_in_map: workspace map unreadable — treating as no registered workspaces");
            return None;
        }
    };
    if map.is_empty() {
        return None;
    }
    let abs = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let mut dir: Option<PathBuf> = Some(abs);
    while let Some(ref d) = dir {
        if let Some(slug) = map.iter().find_map(|(s, p)| {
            let canon = Path::new(p.as_str())
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(p.as_str()));
            (canon == d.as_path()).then_some(s.clone())
        }) {
            return Some(slug);
        }
        let parent = d.parent().map(|p| p.to_path_buf());
        if dir.as_deref() == parent.as_deref() {
            break;
        }
        dir = parent;
    }
    None
}

/// Like [`lookup_in_map`], but returns the workspace **dir** (the ancestor that
/// is registered), not the slug.
fn workspace_dir_from_map(cwd: &Path) -> Option<PathBuf> {
    let map = match read_map() {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "workspace_dir_from_map: workspace map unreadable — treating as no registered workspaces");
            return None;
        }
    };
    if map.is_empty() {
        return None;
    }
    let abs = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let mut dir: Option<PathBuf> = Some(abs);
    while let Some(ref d) = dir {
        if map.values().any(|p| {
            let canon = Path::new(p.as_str())
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(p.as_str()));
            canon == d.as_path()
        }) {
            return Some(d.clone());
        }
        let parent = d.parent().map(|p| p.to_path_buf());
        if dir.as_deref() == parent.as_deref() {
            break;
        }
        dir = parent;
    }
    None
}

// ── git ──────────────────────────────────────────────────────────────────────

fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    // Use --git-common-dir to get the shared git directory across worktrees.
    // For the main repo, this is .git; for worktrees, it's the main .git.
    // Then get its parent to find the actual repo root.
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--git-common-dir"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    let git_dir = PathBuf::from(trimmed);
    // If it's relative, resolve it relative to cwd
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        cwd.join(&git_dir)
    };
    // The repo root is the parent of the .git directory
    git_dir.parent().map(|p| p.to_path_buf())
}

fn basename(p: &Path) -> Option<String> {
    p.file_name().map(|n| n.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests;
