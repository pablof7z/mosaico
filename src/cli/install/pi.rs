//! Pi's directory-shaped global extension installation.

use anyhow::Result;

use super::{
    args::InstallOpts,
    config::{Harness, PI_EXTENSION_FILES, PI_EXTENSION_TS, PI_TOOLS_TS},
    io::write_text,
};

/// Superseded Pi packages. Installing the directory form must evict both the
/// old single-file integration and the panel that is now part of it.
const LEGACY_NPM_PACKAGE: &str = "npm:pi-mosaico";
const LEGACY_PANEL_PACKAGE: &str = "npm:pi-mosaico-panel";

pub(super) fn install(h: &Harness, opts: &InstallOpts, render: bool) -> Result<()> {
    if opts.uninstall {
        remove_directory(h, opts, render)?;
        return remove_legacy_files(h, opts, render);
    }
    if opts.dry_run {
        if render {
            println!("  would write {}", h.config_path.display());
        }
        return Ok(());
    }
    if h.config_path.exists() {
        std::fs::remove_dir_all(&h.config_path)?;
    }
    for (name, source) in PI_EXTENSION_FILES {
        write_text(&h.config_path.join(name), source)?;
    }
    remove_legacy_files(h, opts, false)?;
    evict_legacy_npm_package(h, opts, render)?;
    if render {
        println!("  wrote {}", h.config_path.display());
    }
    Ok(())
}

pub(super) fn is_installed(h: &Harness) -> bool {
    PI_EXTENSION_FILES.iter().all(|(name, source)| {
        std::fs::read_to_string(h.config_path.join(name))
            .map(|installed| installed == *source)
            .unwrap_or(false)
    })
}

fn remove_directory(h: &Harness, opts: &InstallOpts, render: bool) -> Result<()> {
    if !h.config_path.exists() {
        return Ok(());
    }
    if opts.dry_run {
        if render {
            println!("  would remove {}", h.config_path.display());
        }
    } else {
        std::fs::remove_dir_all(&h.config_path)?;
        if render {
            println!("  removed {}", h.config_path.display());
        }
    }
    Ok(())
}

fn remove_legacy_files(h: &Harness, opts: &InstallOpts, render: bool) -> Result<()> {
    let Some(parent) = h.config_path.parent() else {
        return Ok(());
    };
    for (name, source) in [("mosaico.ts", PI_EXTENSION_TS), ("tools.ts", PI_TOOLS_TS)] {
        let path = parent.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(source) {
            continue;
        }
        if opts.dry_run {
            if render {
                println!("  would remove {}", path.display());
            }
        } else {
            std::fs::remove_file(&path)?;
            if render {
                println!("  removed {}", path.display());
            }
        }
    }
    Ok(())
}

/// Remove superseded Pi packages and local source references so Pi has one
/// Mosaico integration, including its sidebar. Edits `settings.json` in place
/// and drops installed npm package directories.
fn evict_legacy_npm_package(h: &Harness, opts: &InstallOpts, render: bool) -> Result<()> {
    let Some(agent_dir) = h
        .config_path
        .parent()
        .and_then(|extensions| extensions.parent())
    else {
        return Ok(());
    };
    let settings = agent_dir.join("settings.json");
    if settings.is_file() {
        let mut changed = false;
        let mut document: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings)?)
                .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
        for field in ["extensions", "packages"] {
            if let Some(list) = document.get_mut(field).and_then(|v| v.as_array_mut()) {
                let before = list.len();
                list.retain(|entry| {
                    entry
                        .as_str()
                        .is_none_or(|source| !is_superseded_pi_source(source))
                });
                if list.len() != before {
                    changed = true;
                }
            }
        }
        if changed {
            if opts.dry_run {
                if render {
                    println!(
                        "  would evict superseded Pi Mosaico packages from {}",
                        settings.display()
                    );
                }
            } else {
                std::fs::write(&settings, serde_json::to_string_pretty(&document)?)?;
                if render {
                    println!("  evicted {LEGACY_NPM_PACKAGE} from {}", settings.display());
                }
            }
        }
    }
    for package in ["pi-mosaico", "pi-mosaico-panel"] {
        let npm_pkg = agent_dir.join("npm/node_modules").join(package);
        if npm_pkg.is_dir() {
            if opts.dry_run {
                if render {
                    println!("  would remove {}", npm_pkg.display());
                }
            } else {
                std::fs::remove_dir_all(&npm_pkg)?;
                if render {
                    println!("  removed {}", npm_pkg.display());
                }
            }
        }
    }
    Ok(())
}

fn is_superseded_pi_source(source: &str) -> bool {
    source == LEGACY_NPM_PACKAGE
        || source == LEGACY_PANEL_PACKAGE
        || source
            .split('/')
            .any(|segment| segment == "pi-mosaico-panel")
}
