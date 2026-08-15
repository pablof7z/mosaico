//! Pi's directory-shaped global extension installation.

use anyhow::Result;

use super::{
    args::InstallOpts,
    config::{Harness, PI_EXTENSION_FILES, PI_EXTENSION_TS, PI_TOOLS_TS},
    io::write_text,
};

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
