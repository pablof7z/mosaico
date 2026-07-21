//! Install the bundled runtime skill without depending on a source checkout.
mod bundle;

use super::config::{claude_detected, home_dir};
use super::{InstallOpts, SkillHealth, SkillHealthState, SkillTargetHealth};
use anyhow::{Context, Result};
use bundle::SKILL_FILES;
use owo_colors::OwoColorize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
struct SkillTarget {
    label: &'static str,
    path: PathBuf,
    kind: SkillTargetKind,
}

#[derive(Debug, Clone, Copy)]
enum SkillTargetKind {
    BundledCopy,
    AgentsSkillLink,
}

fn agents_skill_path(home: &Path) -> PathBuf {
    home.join(".agents/skills/mosaico")
}

fn skill_targets() -> Result<Vec<SkillTarget>> {
    let home = home_dir()?;
    let mut targets = vec![SkillTarget {
        label: "agents",
        path: agents_skill_path(&home),
        kind: SkillTargetKind::BundledCopy,
    }];
    let claude = SkillTarget {
        label: "claude",
        path: home.join(".claude/skills/mosaico"),
        kind: SkillTargetKind::AgentsSkillLink,
    };
    if claude_detected()? || claude.path.symlink_metadata().is_ok() {
        targets.push(claude);
    }
    Ok(targets)
}

fn is_bundled_skill(path: &Path) -> bool {
    SKILL_FILES.iter().all(|(relative, expected)| {
        std::fs::read_to_string(path.join(relative)).is_ok_and(|actual| actual == *expected)
    })
}

fn target_health(target: &SkillTarget, agents: &Path) -> SkillTargetHealth {
    let state = if target.path.symlink_metadata().is_err() {
        SkillHealthState::Missing
    } else {
        let healthy = match target.kind {
            SkillTargetKind::BundledCopy => {
                !target.path.is_symlink() && is_bundled_skill(&target.path)
            }
            SkillTargetKind::AgentsSkillLink => {
                target.path.is_symlink()
                    && target.path.canonicalize().ok() == agents.canonicalize().ok()
                    && is_bundled_skill(agents)
            }
        };
        if healthy {
            SkillHealthState::Healthy
        } else {
            SkillHealthState::Stale
        }
    };
    SkillTargetHealth {
        label: target.label,
        path: target.path.clone(),
        state,
    }
}

pub(in crate::cli) fn health() -> Result<SkillHealth> {
    let canonical_path = agents_skill_path(&home_dir()?);
    let targets = skill_targets()?
        .iter()
        .map(|target| target_health(target, &canonical_path))
        .collect();
    Ok(SkillHealth {
        canonical_path,
        targets,
    })
}

pub(in crate::cli) fn repair() -> Result<SkillHealth> {
    let agents = agents_skill_path(&home_dir()?);
    for target in skill_targets()? {
        apply_target(&target, &agents)?;
    }
    health()
}

pub(super) fn print_status() -> Result<()> {
    println!("{}", "mosaico skill status".bold());
    for target in super::skill_health()?.targets {
        let installed = if target.state == SkillHealthState::Healthy {
            "installed".green().to_string()
        } else {
            "-".dimmed().to_string()
        };
        println!(
            "  {:<8} {:<10} {}",
            target.label.cyan(),
            installed,
            installed_detail(&target.path).dimmed()
        );
    }
    Ok(())
}

pub(super) fn selection_label() -> Result<String> {
    let targets = super::skill_health()?.targets;
    let installed = targets
        .iter()
        .filter(|target| target.state == SkillHealthState::Healthy)
        .count();
    let status = match installed {
        0 => "-".dimmed().to_string(),
        n if n == targets.len() => "installed".green().to_string(),
        _ => "partial".yellow().to_string(),
    };
    let detail = targets
        .iter()
        .map(|target| target.path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "{:<18} {:<14} {}",
        "mosaico skill".cyan().bold(),
        status,
        detail.dimmed()
    ))
}

pub(super) fn install(opts: &InstallOpts) -> Result<()> {
    if !opts.uninstall && !opts.dry_run {
        let repaired = super::repair_skill()?;
        for target in repaired.targets {
            println!(
                "\n{} {}",
                "Installing skill into".bold(),
                target.label.cyan().bold()
            );
            if target.path == repaired.canonical_path {
                println!(
                    "  wrote {} bundled files to {}",
                    SKILL_FILES.len(),
                    target.path.display()
                );
            } else {
                println!(
                    "  linked {} -> {}",
                    target.path.display(),
                    repaired.canonical_path.display()
                );
            }
        }
        return Ok(());
    }

    let agents = agents_skill_path(&home_dir()?);
    let verb = if opts.uninstall {
        "Uninstalling skill from"
    } else {
        "Installing skill into"
    };
    let flag = if opts.dry_run { " (dry-run)" } else { "" };

    for target in skill_targets()? {
        println!("\n{} {}{flag}", verb.bold(), target.label.cyan().bold());
        if opts.uninstall {
            uninstall_target(&target, opts.dry_run)?;
        } else {
            install_target(&target, &agents, opts.dry_run)?;
        }
    }
    Ok(())
}

fn installed_detail(path: &Path) -> String {
    if path.is_symlink() {
        if let Ok(target) = std::fs::read_link(path) {
            return format!("{} -> {}", path.display(), target.display());
        }
    }
    path.display().to_string()
}

fn install_target(target: &SkillTarget, agents: &Path, dry_run: bool) -> Result<()> {
    if dry_run {
        match target.kind {
            SkillTargetKind::BundledCopy => println!(
                "  would write {} bundled files to {}",
                SKILL_FILES.len(),
                target.path.display()
            ),
            SkillTargetKind::AgentsSkillLink => println!(
                "  would symlink {} -> {}",
                target.path.display(),
                agents.display()
            ),
        }
        return Ok(());
    }

    apply_target(target, agents)?;
    match target.kind {
        SkillTargetKind::BundledCopy => {
            println!(
                "  wrote {} bundled files to {}",
                SKILL_FILES.len(),
                target.path.display()
            );
        }
        SkillTargetKind::AgentsSkillLink => {
            println!("  linked {} -> {}", target.path.display(), agents.display());
        }
    }
    Ok(())
}

fn apply_target(target: &SkillTarget, agents: &Path) -> Result<()> {
    match target.kind {
        SkillTargetKind::BundledCopy => write_bundled_skill(&target.path),
        SkillTargetKind::AgentsSkillLink => link_skill(&target.path, agents),
    }
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
    remove_skill(&target.path)?;
    println!("  removed {}", target.path.display());
    Ok(())
}

fn write_bundled_skill(target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .context("bundled skill target has no parent directory")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let staging = parent.join(format!(".mosaico.install-{}", std::process::id()));
    if staging.symlink_metadata().is_ok() {
        remove_skill(&staging)?;
    }
    for (relative, contents) in SKILL_FILES {
        let path = staging.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))?;
    }
    if target.symlink_metadata().is_ok() {
        remove_skill(target)?;
    }
    std::fs::rename(&staging, target)
        .with_context(|| format!("installing bundled skill at {}", target.display()))?;
    Ok(())
}

fn link_skill(link: &Path, source: &Path) -> Result<()> {
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    if link.symlink_metadata().is_ok() {
        remove_skill(link)?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, link)
        .with_context(|| format!("linking {} -> {}", link.display(), source.display()))?;
    #[cfg(not(unix))]
    anyhow::bail!("skill symlinks are only supported on Unix");
    Ok(())
}

fn remove_skill(path: &Path) -> Result<()> {
    let meta = path
        .symlink_metadata()
        .with_context(|| format!("reading {}", path.display()))?;
    if meta.file_type().is_symlink() || meta.is_file() {
        std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    } else {
        std::fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
