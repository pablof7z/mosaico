//! Project-slug resolution and the per-machine slug→path map.
//!
//! A *project* is identified by a short slug. The slug is resolved from a
//! working directory by the following order:
//!
//!   1. **Git repo name** — derived from `git rev-parse --git-common-dir`, so a
//!      repo and all of its git worktrees resolve to the **same** slug (the
//!      basename of the shared main repo root).
//!   2. **`~/.tenex-edge/projects.json`** — a JSON object mapping slugs to
//!      absolute paths. The cwd itself, or its nearest ancestor present in the
//!      map, wins. This is the only way to give a non-git directory a project.
//!   3. Otherwise: `Err(NoProject)`. The caller decides how to surface — hooks
//!      exit 0 silently; explicit CLI verbs print a "run `tenex-edge channel
//!      init` or `git init`" message and exit non-zero.
//!
//! There is no longer a `.tenex/project.json` file. The map at
//! `~/.tenex-edge/projects.json` is the single source of truth for non-git
//! projects.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Error returned by [`resolve`]. Carries the working directory that had no
/// resolvable project, so callers can format a helpful message.
#[derive(Debug)]
pub struct NoProject {
    pub cwd: PathBuf,
}

impl std::fmt::Display for NoProject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no known project in {}", self.cwd.display())
    }
}

impl std::error::Error for NoProject {}

/// Resolve the project slug for a working directory.
///
/// Returns the slug, or [`NoProject`] when the cwd is not in a git repo and is
/// not registered in `~/.tenex-edge/projects.json` (nor any ancestor of it).
pub fn resolve(cwd: &Path) -> std::result::Result<String, NoProject> {
    // 1. git repo name (shared across all worktrees of the same repo).
    if let Some(root) = git_toplevel(cwd) {
        if let Some(name) = basename(&root) {
            return Ok(name);
        }
    }
    // 2. projects.json: cwd or nearest ancestor present in the map.
    if let Some(slug) = lookup_in_map(cwd) {
        return Ok(slug);
    }
    // 3. No resolvable project.
    Err(NoProject {
        cwd: cwd.to_path_buf(),
    })
}

/// The project ROOT directory for a working dir: the dir whose slug `resolve`
/// returned. Mirrors `resolve`'s search order:
///   1. the git repo root (derived from git-common-dir, shared across worktrees),
///   2. the nearest ancestor (or cwd itself) present in `projects.json`,
///   3. else `None`.
pub fn project_root(cwd: &Path) -> Option<PathBuf> {
    if let Some(root) = git_toplevel(cwd) {
        return Some(root);
    }
    project_root_from_map(cwd)
}

/// Project-relative working directory for the public presence/status wire field.
/// Never leaks the absolute `$HOME/...` path:
///   - cwd under a resolvable project root → the relative path (`/`-joined),
///     with the root itself rendered as `.`.
///   - no resolvable root → the cwd **basename** (still not absolute).
///   - empty basename (fs root) → empty string.
pub fn rel_cwd(cwd: &Path) -> String {
    if let Some(root) = project_root(cwd) {
        if let Ok(rel) = cwd.strip_prefix(&root) {
            let s = rel.to_string_lossy().replace('\\', "/");
            return if s.is_empty() { ".".to_string() } else { s };
        }
    }
    basename(cwd).unwrap_or_default()
}

/// Like [`resolve`], but on [`NoProject`] returns an `anyhow::Error` whose
/// `Display` form is the user-facing "no known project … run `tenex-edge
/// channel init` or `git init`" message. For the explicit-CLI-verb path only;
/// hooks should call [`resolve`] and exit 0 on `Err`.
pub fn resolve_or_bail(cwd: &Path) -> Result<String> {
    resolve(cwd).map_err(|e| {
        anyhow::anyhow!(
            "{e}; run `tenex-edge channel init` or `git init` first, or pass `--project <slug>`"
        )
    })
}

/// Register the current directory as a new project in `projects.json`, using
/// the directory's basename as the slug. Returns the slug and the absolute path
/// that were written. Errors if the basename is empty, the path can't be
/// canonicalized, or the slug is already mapped to a different path (unless
/// `force` overwrites it).
pub fn register_project(cwd: &Path, force: bool) -> Result<(String, PathBuf)> {
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

// ── projects.json map ────────────────────────────────────────────────────────

/// The on-disk map at `<edge_home>/projects.json`: a JSON object mapping
/// slugs to absolute paths.
fn map_path() -> PathBuf {
    crate::config::edge_home().join("projects.json")
}

/// Read the slug→path map. A MISSING file is "no projects registered yet" (an
/// empty map). A PRESENT-but-unparseable file is a hard error: defaulting to
/// empty would silently drop every non-git project the user registered, so we
/// surface the parse failure instead of guessing.
fn read_map() -> Result<std::collections::BTreeMap<String, String>> {
    let p = map_path();
    let s = match std::fs::read_to_string(&p) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => {
            return Err(e).with_context(|| format!("reading {}", p.display()));
        }
    };
    // Tolerate either a bare object or any whitespace; serde_json handles both.
    serde_json::from_str::<std::collections::BTreeMap<String, String>>(&s)
        .with_context(|| format!("parsing {} (corrupt projects.json)", p.display()))
}

/// Write the slug→path map, creating the parent dir if necessary.
fn write_map(obj: &std::collections::BTreeMap<String, String>) -> Result<()> {
    let p = map_path();
    if let Some(parent) = p.parent() {
        crate::config::ensure_dir(parent)?;
    }
    let s = serde_json::to_string_pretty(obj).context("serializing projects.json")?;
    std::fs::write(&p, s).with_context(|| format!("writing {}", p.display()))?;
    Ok(())
}

/// Look up `cwd` (or its nearest ancestor) in the map. Returns the slug for the
/// nearest ancestor present, or `None` if no ancestor is registered.
fn lookup_in_map(cwd: &Path) -> Option<String> {
    let map = match read_map() {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "lookup_in_map: projects.json unreadable — treating as no registered projects");
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

/// Like [`lookup_in_map`], but returns the project **root** dir (the ancestor
/// that is registered), not the slug.
fn project_root_from_map(cwd: &Path) -> Option<PathBuf> {
    let map = match read_map() {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "project_root_from_map: projects.json unreadable — treating as no registered projects");
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
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;

    fn isolated_home() -> (EnvGuard, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let g = EnvGuard::set("TENEX_EDGE_HOME", dir.path());
        (g, dir)
    }

    fn write_projects_map(entries: &[(&str, &str)]) {
        let mut obj = std::collections::BTreeMap::new();
        for (slug, path) in entries {
            obj.insert(slug.to_string(), path.to_string());
        }
        write_map(&obj).unwrap();
    }

    // -- resolve: git path ----------------------------------------------------

    // No git tests here: git resolution depends on the host having git and
    // the tempdir not being inside a repo. Covered by integration tests.

    // -- resolve: projects.json path ------------------------------------------

    #[test]
    fn resolve_returns_slug_when_cwd_is_in_map() {
        let (_g, _home) = isolated_home();
        let dir = tempfile::tempdir().unwrap();
        let abs = std::fs::canonicalize(dir.path()).unwrap();
        write_projects_map(&[("my-project", abs.to_str().unwrap())]);
        assert_eq!(resolve(&abs).unwrap(), "my-project");
    }

    #[test]
    fn resolve_walks_upward_to_find_ancestor_in_map() {
        let (_g, _home) = isolated_home();
        let dir = tempfile::tempdir().unwrap();
        let abs = std::fs::canonicalize(dir.path()).unwrap();
        write_projects_map(&[("ancestor-proj", abs.to_str().unwrap())]);
        let nested = abs.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(resolve(&nested).unwrap(), "ancestor-proj");
    }

    #[test]
    fn resolve_errors_when_no_git_and_not_in_map() {
        let (_g, _home) = isolated_home();
        let dir = tempfile::tempdir().unwrap();
        let abs = std::fs::canonicalize(dir.path()).unwrap();
        // Empty map, no git (tempdir is outside any repo on macOS).
        write_projects_map(&[]);
        let result = resolve(&abs);
        // tempdir is outside any repo, so this should be Err.
        // (If the tempdir happens to be inside a git repo on CI, skip.)
        if git_toplevel(&abs).is_none() {
            assert!(result.is_err());
        }
    }

    // -- project_root ----------------------------------------------------------

    #[test]
    fn project_root_from_map_returns_ancestor_dir() {
        let (_g, _home) = isolated_home();
        let dir = tempfile::tempdir().unwrap();
        let abs = std::fs::canonicalize(dir.path()).unwrap();
        write_projects_map(&[("proj", abs.to_str().unwrap())]);
        let nested = abs.join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(project_root(&nested), Some(abs));
    }

    // -- rel_cwd --------------------------------------------------------------

    #[test]
    fn rel_cwd_root_is_dot() {
        let (_g, _home) = isolated_home();
        let dir = tempfile::tempdir().unwrap();
        let abs = std::fs::canonicalize(dir.path()).unwrap();
        write_projects_map(&[("p", abs.to_str().unwrap())]);
        assert_eq!(rel_cwd(&abs), ".");
    }

    #[test]
    fn rel_cwd_subdir_is_relative_joined() {
        let (_g, _home) = isolated_home();
        let dir = tempfile::tempdir().unwrap();
        let abs = std::fs::canonicalize(dir.path()).unwrap();
        write_projects_map(&[("p", abs.to_str().unwrap())]);
        let sub = abs.join("worktree1").join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(rel_cwd(&sub), "worktree1/nested");
    }

    // -- register_project -----------------------------------------------------

    #[test]
    fn register_project_writes_basename_and_path() {
        let (_g, _home) = isolated_home();
        let dir = tempfile::tempdir().unwrap();
        let abs = std::fs::canonicalize(dir.path()).unwrap();
        // Create a subdir with a known name so basename is deterministic.
        let proj_dir = abs.join("the-proj");
        std::fs::create_dir_all(&proj_dir).unwrap();
        let (slug, written_path) = register_project(&proj_dir, false).unwrap();
        assert_eq!(slug, "the-proj");
        assert_eq!(written_path, std::fs::canonicalize(&proj_dir).unwrap());
        // The map now contains the entry.
        let map = read_map().unwrap();
        assert_eq!(
            map.get("the-proj").map(|s| s.as_str()),
            Some(std::fs::canonicalize(&proj_dir).unwrap().to_str().unwrap())
        );
    }

    #[test]
    fn register_project_errors_on_duplicate_slug_different_path() {
        let (_g, _home) = isolated_home();
        let a = tempfile::tempdir().unwrap();
        let a_abs = std::fs::canonicalize(a.path()).unwrap();
        let a_proj = a_abs.join("dup");
        std::fs::create_dir_all(&a_proj).unwrap();
        register_project(&a_proj, false).unwrap();

        // Different path, same slug basename.
        let b = tempfile::tempdir().unwrap();
        let b_abs = std::fs::canonicalize(b.path()).unwrap();
        let b_proj = b_abs.join("dup");
        std::fs::create_dir_all(&b_proj).unwrap();
        let err = register_project(&b_proj, false);
        assert!(err.is_err());
        let msg = format!("{}", err.unwrap_err());
        assert!(
            msg.contains("already mapped") || msg.contains("already in use"),
            "msg = {msg}"
        );
    }

    #[test]
    fn register_project_force_overwrites_duplicate() {
        let (_g, _home) = isolated_home();
        let a = tempfile::tempdir().unwrap();
        let a_abs = std::fs::canonicalize(a.path()).unwrap();
        let a_proj = a_abs.join("dup");
        std::fs::create_dir_all(&a_proj).unwrap();
        register_project(&a_proj, false).unwrap();

        let b = tempfile::tempdir().unwrap();
        let b_abs = std::fs::canonicalize(b.path()).unwrap();
        let b_proj = b_abs.join("dup");
        std::fs::create_dir_all(&b_proj).unwrap();
        let (slug, path) = register_project(&b_proj, true).unwrap();
        assert_eq!(slug, "dup");
        assert_eq!(path, std::fs::canonicalize(&b_proj).unwrap());
    }

    #[test]
    fn register_project_idempotent_when_same_path() {
        let (_g, _home) = isolated_home();
        let dir = tempfile::tempdir().unwrap();
        let abs = std::fs::canonicalize(dir.path()).unwrap();
        let proj = abs.join("idem");
        std::fs::create_dir_all(&proj).unwrap();
        register_project(&proj, false).unwrap();
        // Registering again with the same path should succeed (no-op).
        let (slug, path) = register_project(&proj, false).unwrap();
        assert_eq!(slug, "idem");
        assert_eq!(path, std::fs::canonicalize(&proj).unwrap());
    }
}
