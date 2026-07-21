//! Goose Open Plugins hook installation and status checks.

use super::{write_text, Harness, InstallOpts};
use anyhow::{Context, Result};

pub(super) fn is_installed(_harness: &Harness) -> bool {
    crate::goose_integration::is_installed()
}

pub(super) fn is_present(_harness: &Harness) -> bool {
    crate::goose_integration::is_present()
}

pub(super) fn install(harness: &Harness, opts: &InstallOpts, render: bool) -> Result<()> {
    crate::goose_integration::validate_runtime()?;
    let files = crate::goose_integration::plugin_files()?;
    if opts.dry_run {
        if render {
            for (path, body) in files {
                println!("  would write {} ({} bytes)", path.display(), body.len());
            }
            println!("  would enable Goose plugin mosaico");
        }
        return Ok(());
    }
    for (path, body) in files {
        write_text(&path, body)?;
        if render {
            println!("  wrote {}", path.display());
        }
    }
    crate::goose_integration::enable_plugin()?;
    if !is_installed(harness) {
        anyhow::bail!(
            "Goose Mosaico plugin was written but is not active; ensure Top Of Mind is enabled"
        );
    }
    Ok(())
}

pub(super) fn uninstall(_harness: &Harness, opts: &InstallOpts) -> Result<()> {
    let files = crate::goose_integration::plugin_files()?;
    if opts.dry_run {
        for (path, _) in files {
            println!("  would remove {}", path.display());
        }
        return Ok(());
    }
    for (path, _) in files {
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            println!("  removed {}", path.display());
        }
    }
    let root = crate::goose_integration::plugin_root()?;
    let hooks = root.join("hooks");
    if hooks.is_dir() {
        let _ = std::fs::remove_dir(hooks);
    }
    if root.is_dir() {
        let _ = std::fs::remove_dir(root);
    }
    Ok(())
}
