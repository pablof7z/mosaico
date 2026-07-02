//! Symlink the repo-local `skills/tenex-edge` skill into harness skill directories.

use super::config::{claude_detected, home_dir};
use super::InstallOpts;
use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use std::path::{Path, PathBuf};

const SKILL_REL: &str = "skills/tenex-edge";
const SKILL_MARKER: &str = "name: tenex-edge";

#[derive(Debug, Clone)]
struct SkillTarget {
    label: &'static str,
    path: PathBuf,
}

fn skill_targets() -> Vec<SkillTarget> {
    let home = home_dir();
    let mut targets = vec![SkillTarget {
        label: "agents",
        path: home.join(".agents/skills/tenex-edge"),
    }];
    if claude_detected() {
        targets.push(SkillTarget {
            label: "claude",
            path: home.join(".claude/skills/tenex-edge"),
        });
    }
    targets
}

/// Resolve `skills/tenex-edge` inside the tenex-edge repo checkout.
fn skill_source_dir() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest.join(SKILL_REL);
    if is_skill_tree(&candidate) {
        return candidate
            .canonicalize()
            .with_context(|| format!("canonicalizing {}", candidate.display()));
    }

    let mut dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    while let Some(d) = dir {
        let candidate = d.join(SKILL_REL);
        if is_skill_tree(&candidate) {
            return candidate
                .canonicalize()
                .with_context(|| format!("canonicalizing {}", candidate.display()));
        }
        dir = d.parent().map(Path::to_path_buf);
    }

    anyhow::bail!(
        "cannot find {SKILL_REL} in the tenex-edge repo; run `tenex-edge install` from a repo checkout"
    );
}

fn is_skill_tree(path: &Path) -> bool {
    path.join("SKILL.md").is_file()
        && std::fs::read_to_string(path.join("SKILL.md"))
            .map(|content| content.contains(SKILL_MARKER))
            .unwrap_or(false)
}

pub(super) fn print_status() {
    println!("{}", "tenex-edge skill status".bold());
    let source = skill_source_dir().ok();
    if let Some(src) = &source {
        println!(
            "  {:<8} {}",
            "source".dimmed(),
            src.display().to_string().dimmed()
        );
    }
    for target in skill_targets() {
        let installed = if is_installed(&target.path, source.as_deref()) {
            "installed".green().to_string()
        } else {
            "-".dimmed().to_string()
        };
        let detail = installed_link_detail(&target.path, source.as_deref())
            .unwrap_or_else(|| target.path.display().to_string());
        println!(
            "  {:<8} {:<10} {}",
            target.label.cyan(),
            installed,
            detail.dimmed()
        );
    }
}

pub(super) fn install(opts: &InstallOpts) -> Result<()> {
    let source = skill_source_dir()?;
    let verb = if opts.uninstall {
        "Uninstalling skill from"
    } else {
        "Linking skill into"
    };
    let flag = if opts.dry_run { " (dry-run)" } else { "" };

    for target in skill_targets() {
        println!("\n{} {}{flag}", verb.bold(), target.label.cyan().bold());
        if opts.uninstall {
            uninstall_target(&target, opts.dry_run)?;
        } else {
            install_target(&target, &source, opts.dry_run)?;
        }
    }
    Ok(())
}

fn is_installed(path: &Path, expected_source: Option<&Path>) -> bool {
    let Ok(resolved) = path.canonicalize() else {
        return false;
    };
    if !is_skill_tree(&resolved) {
        return false;
    }
    match expected_source {
        Some(src) => resolved == src.canonicalize().unwrap_or_else(|_| src.to_path_buf()),
        None => true,
    }
}

fn installed_link_detail(path: &Path, expected_source: Option<&Path>) -> Option<String> {
    if !is_installed(path, expected_source) {
        return None;
    }
    let resolved = path.canonicalize().ok()?;
    if path.is_symlink() {
        Some(format!("{} -> {}", path.display(), resolved.display()))
    } else {
        Some(resolved.display().to_string())
    }
}

fn install_target(target: &SkillTarget, source: &Path, dry_run: bool) -> Result<()> {
    if dry_run {
        println!(
            "  would symlink {} -> {}",
            target.path.display(),
            source.display()
        );
        return Ok(());
    }

    link_skill(&target.path, source)?;
    println!("  linked {} -> {}", target.path.display(), source.display());
    Ok(())
}

fn uninstall_target(target: &SkillTarget, dry_run: bool) -> Result<()> {
    if target.path.symlink_metadata().is_err() {
        println!("  nothing to remove");
        return Ok(());
    }
    if dry_run {
        println!("  would remove {}", target.path.display());
        return Ok(());
    }
    remove_skill_link(&target.path)?;
    println!("  removed {}", target.path.display());
    Ok(())
}

fn link_skill(link: &Path, source: &Path) -> Result<()> {
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    if link.symlink_metadata().is_ok() {
        remove_skill_link(link)?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, link)
        .with_context(|| format!("linking {} -> {}", link.display(), source.display()))?;
    #[cfg(not(unix))]
    anyhow::bail!("skill symlinks are only supported on Unix");
    Ok(())
}

fn remove_skill_link(path: &Path) -> Result<()> {
    let meta = path
        .symlink_metadata()
        .with_context(|| format!("reading {}", path.display()))?;
    if meta.file_type().is_symlink() {
        std::fs::remove_file(path)
            .with_context(|| format!("removing symlink {}", path.display()))?;
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))?;
        return Ok(());
    }
    std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static HOME_LOCK: Mutex<()> = Mutex::new(());

    struct HomeGuard {
        _lock: MutexGuard<'static, ()>,
        previous: Option<String>,
    }

    impl HomeGuard {
        fn set(path: &Path) -> Self {
            let _lock = HOME_LOCK.lock().expect("home env lock");
            let previous = std::env::var("HOME").ok();
            // SAFETY: serialized by HOME_LOCK; restored on drop.
            unsafe { std::env::set_var("HOME", path) };
            Self { _lock, previous }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(home) => unsafe { std::env::set_var("HOME", home) },
                None => unsafe { std::env::remove_var("HOME") },
            }
        }
    }

    fn opts(dry_run: bool, uninstall: bool) -> InstallOpts {
        InstallOpts {
            all: false,
            harness: None,
            dry_run,
            status: false,
            uninstall,
        }
    }

    #[test]
    fn install_symlinks_agents_skill_to_repo_source() {
        let temp = tempfile::tempdir().unwrap();
        let _home = HomeGuard::set(temp.path());
        let source = skill_source_dir().unwrap();

        install(&opts(false, false)).unwrap();

        let link = temp.path().join(".agents/skills/tenex-edge");
        assert!(link.is_symlink());
        assert_eq!(link.canonicalize().unwrap(), source);
    }

    #[test]
    fn install_symlinks_claude_skill_when_claude_dir_exists() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".claude")).unwrap();
        let _home = HomeGuard::set(temp.path());
        let source = skill_source_dir().unwrap();

        install(&opts(false, false)).unwrap();

        let link = temp.path().join(".claude/skills/tenex-edge");
        assert!(link.is_symlink());
        assert_eq!(link.canonicalize().unwrap(), source);
    }

    #[test]
    fn uninstall_removes_symlink_only() {
        let temp = tempfile::tempdir().unwrap();
        let _home = HomeGuard::set(temp.path());
        let source = skill_source_dir().unwrap();

        install(&opts(false, false)).unwrap();
        install(&opts(false, true)).unwrap();

        assert!(!temp.path().join(".agents/skills/tenex-edge").exists());
        assert!(source.join("SKILL.md").is_file());
    }

    #[test]
    fn install_replaces_stale_symlink_with_repo_source() {
        let temp = tempfile::tempdir().unwrap();
        let stale = temp.path().join("stale-skill");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("SKILL.md"), "name: tenex-edge\nold").unwrap();

        let link = temp.path().join(".agents/skills/tenex-edge");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&stale, &link).unwrap();

        let _home = HomeGuard::set(temp.path());
        let source = skill_source_dir().unwrap();
        install(&opts(false, false)).unwrap();

        assert!(link.is_symlink());
        assert_eq!(link.canonicalize().unwrap(), source);
        assert!(stale.join("SKILL.md").is_file());
    }

    #[test]
    fn install_replaces_copied_tree_with_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join(".agents/skills/tenex-edge");
        std::fs::create_dir_all(&link).unwrap();
        std::fs::write(link.join("SKILL.md"), "name: tenex-edge\nold").unwrap();

        let _home = HomeGuard::set(temp.path());
        let source = skill_source_dir().unwrap();
        install(&opts(false, false)).unwrap();

        assert!(link.is_symlink());
        assert_eq!(link.canonicalize().unwrap(), source);
    }
}
