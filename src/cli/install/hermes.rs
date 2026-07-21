//! Hermes user-plugin installation and status checks.

use super::{config, write_text, Harness, InstallOpts};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

fn plugin_files(home: &Path) -> [(PathBuf, &'static str); 2] {
    let plugin = home.join("plugins/mosaico");
    [
        (plugin.join("plugin.yaml"), config::HERMES_PLUGIN_YAML),
        (plugin.join("__init__.py"), config::HERMES_PLUGIN_PY),
    ]
}

fn hermes_home(harness: &Harness) -> &Path {
    harness
        .config_path
        .parent()
        .and_then(Path::parent)
        .expect("Hermes config path is <home>/plugins/mosaico")
}

fn profile_homes(harness: &Harness) -> Vec<PathBuf> {
    let root = hermes_home(harness);
    let mut homes = vec![root.to_path_buf()];
    if root
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        == Some("profiles")
    {
        return homes;
    }
    if let Ok(entries) = std::fs::read_dir(root.join("profiles")) {
        homes.extend(
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir()),
        );
    }
    homes.sort();
    homes
}

fn enabled_from_json(body: &[u8]) -> bool {
    serde_json::from_slice::<Vec<serde_json::Value>>(body)
        .ok()
        .is_some_and(|plugins| {
            plugins.iter().any(|plugin| {
                ["key", "name", "id"]
                    .into_iter()
                    .filter_map(|field| plugin.get(field).and_then(|value| value.as_str()))
                    .any(|value| value == "mosaico")
            })
        })
}

fn command_for_home(home: &Path) -> Command {
    let profile = if home
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        == Some("profiles")
    {
        home.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("default")
    } else {
        "default"
    };
    let mut command = Command::new("hermes");
    command
        .args(["--profile", profile])
        .env("HERMES_HOME", home);
    command
}

fn plugin_enabled(home: &Path) -> bool {
    command_for_home(home)
        .args(["plugins", "list", "--enabled", "--json"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| enabled_from_json(&output.stdout))
}

pub(super) fn is_installed(harness: &Harness) -> bool {
    profile_homes(harness).into_iter().all(|home| {
        plugin_files(&home).into_iter().all(|(path, expected)| {
            std::fs::read_to_string(path)
                .map(|body| body == expected)
                .unwrap_or(false)
        }) && plugin_enabled(&home)
    })
}

pub(super) fn is_present(harness: &Harness) -> bool {
    profile_homes(harness)
        .into_iter()
        .any(|home| home.join("plugins/mosaico").exists())
}

fn run_plugin_command(home: &Path, action: &str) -> Result<()> {
    let mut command = command_for_home(home);
    command.args(["plugins", action, "mosaico"]);
    if action == "enable" {
        command.arg("--no-allow-tool-override");
    }
    let output = command.output().with_context(|| {
        format!(
            "running hermes plugins {action} mosaico for {}",
            home.display()
        )
    })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(
            "hermes plugins {action} mosaico failed{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        );
    }
    Ok(())
}

pub(super) fn install(harness: &Harness, opts: &InstallOpts, render: bool) -> Result<()> {
    for home in profile_homes(harness) {
        let files = plugin_files(&home);
        if opts.dry_run {
            for (path, body) in files {
                if render {
                    println!("  would write {} ({} bytes)", path.display(), body.len());
                }
            }
            if render {
                println!("  would enable Hermes plugin mosaico in {}", home.display());
            }
            continue;
        }
        for (path, body) in files {
            write_text(&path, body)?;
            if render {
                println!("  wrote {}", path.display());
            }
        }
        run_plugin_command(&home, "enable")?;
        if render {
            println!("  enabled Hermes plugin mosaico in {}", home.display());
        }
    }
    Ok(())
}

pub(super) fn uninstall(harness: &Harness, opts: &InstallOpts) -> Result<()> {
    for home in profile_homes(harness) {
        if opts.dry_run {
            println!(
                "  would disable Hermes plugin mosaico in {}",
                home.display()
            );
            for (path, _) in plugin_files(&home) {
                println!("  would remove {}", path.display());
            }
            continue;
        }
        if plugin_enabled(&home) {
            run_plugin_command(&home, "disable")?;
            println!("  disabled Hermes plugin mosaico in {}", home.display());
        }
        for (path, _) in plugin_files(&home) {
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
                println!("  removed {}", path.display());
            }
        }
        let plugin_dir = home.join("plugins/mosaico");
        if plugin_dir.is_dir() {
            let _ = std::fs::remove_dir(plugin_dir);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{enabled_from_json, profile_homes};
    use crate::cli::install::config::Harness;

    #[test]
    fn detects_enabled_plugin_by_canonical_key() {
        assert!(enabled_from_json(
            br#"[{"name":"Mosaico","key":"mosaico","source":"user"}]"#
        ));
        assert!(!enabled_from_json(br#"[{"name":"other","key":"other"}]"#));
        assert!(!enabled_from_json(b"not json"));
    }

    #[test]
    fn default_install_targets_existing_named_profiles() {
        let root = tempfile::tempdir().unwrap();
        let hermes = root.path().join(".hermes");
        std::fs::create_dir_all(hermes.join("profiles/reviewer")).unwrap();
        std::fs::create_dir_all(hermes.join("profiles/planner")).unwrap();
        let harness = Harness {
            id: "hermes",
            display: "Hermes",
            config_path: hermes.join("plugins/mosaico"),
            detected: true,
        };

        assert_eq!(
            profile_homes(&harness),
            [
                hermes.clone(),
                hermes.join("profiles/planner"),
                hermes.join("profiles/reviewer")
            ]
        );
    }

    #[test]
    fn named_profile_install_does_not_escape_to_siblings() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join(".hermes/profiles/reviewer");
        std::fs::create_dir_all(&profile).unwrap();
        let harness = Harness {
            id: "hermes",
            display: "Hermes",
            config_path: profile.join("plugins/mosaico"),
            detected: true,
        };

        assert_eq!(profile_homes(&harness), [profile]);
    }
}
