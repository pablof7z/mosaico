//! Goose runtime, plugin, and Top Of Mind configuration health.

use super::{atomic_write_unchecked, HOOKS_JSON, PLUGIN_JSON};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

const MIN_GOOSE_VERSION: [u64; 3] = [1, 43, 0];

pub(crate) fn plugin_root() -> Result<PathBuf> {
    let root = std::env::var_os("GOOSE_PATH_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .context("HOME is not set; cannot resolve Goose's user plugin directory")?;
    Ok(root.join(".agents/plugins/mosaico"))
}

pub(crate) fn plugin_files() -> Result<[(PathBuf, &'static str); 2]> {
    let root = plugin_root()?;
    Ok([
        (root.join("plugin.json"), PLUGIN_JSON),
        (root.join("hooks/hooks.json"), HOOKS_JSON),
    ])
}

pub(crate) fn is_present() -> bool {
    plugin_root().is_ok_and(|path| path.exists())
}

pub(crate) fn is_installed() -> bool {
    plugin_files().is_ok_and(|files| {
        files.into_iter().all(|(path, expected)| {
            std::fs::read_to_string(path).is_ok_and(|body| body == expected)
        })
    }) && validate_runtime().is_ok()
        && !plugin_disabled()
        && !plugin_config_disabled()
        && top_of_mind_enabled()
}

pub(crate) fn validate_runtime() -> Result<()> {
    let output = std::process::Command::new("goose")
        .arg("--version")
        .output()
        .context("running `goose --version`")?;
    if !output.status.success() {
        bail!("`goose --version` failed with {}", output.status);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = parse_version(&stdout)
        .with_context(|| format!("could not parse Goose version from {stdout:?}"))?;
    if version < MIN_GOOSE_VERSION {
        bail!(
            "Goose {}.{}.{} is unsupported; fabric context requires Goose 1.43.0 or newer",
            version[0],
            version[1],
            version[2]
        );
    }
    Ok(())
}

pub(super) fn parse_version(raw: &str) -> Option<[u64; 3]> {
    raw.split_whitespace().find_map(|word| {
        let clean = word.trim_matches(|ch: char| !ch.is_ascii_digit() && ch != '.');
        let mut parts = clean.split('.');
        let version = [
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        ];
        parts.next().is_none().then_some(version)
    })
}

fn goose_config_dir() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("GOOSE_PATH_ROOT").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(root).join("config"));
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|root| root.join("goose"))
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|root| root.join(".config/goose"))
        })
}

fn user_settings_path() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("GOOSE_PATH_ROOT").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(root).join(".config/goose/settings.json"));
    }
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|root| root.join(".config/goose/settings.json"))
}

fn plugin_disabled() -> bool {
    let Some(path) = user_settings_path() else {
        return false;
    };
    let Ok(body) = std::fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| value["disabledPlugins"].as_array().cloned())
        .is_some_and(|plugins| plugins.iter().any(|name| name.as_str() == Some("mosaico")))
}

fn plugin_config_disabled() -> bool {
    let Some(path) = goose_config_dir().map(|root| root.join("config.yaml")) else {
        return false;
    };
    let Ok(body) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&body) else {
        return false;
    };
    let Ok(plugin) = plugin_root() else {
        return false;
    };
    value["plugins"][plugin.to_string_lossy().as_ref()]["enabled"].as_bool() == Some(false)
}

fn top_of_mind_enabled() -> bool {
    let Some(path) = goose_config_dir().map(|root| root.join("config.yaml")) else {
        return true;
    };
    let Ok(body) = std::fs::read_to_string(path) else {
        return true;
    };
    serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&body)
        .ok()
        .and_then(|value| value["extensions"]["tom"]["enabled"].as_bool())
        .unwrap_or(true)
}

pub(crate) fn enable_plugin() -> Result<()> {
    enable_in_settings()?;
    enable_in_config()
}

fn enable_in_settings() -> Result<()> {
    let path = user_settings_path().context("cannot resolve Goose settings path")?;
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let mut value: serde_json::Value = serde_json::from_str(&body)
        .with_context(|| format!("parsing Goose settings {}", path.display()))?;
    let Some(disabled) = value["disabledPlugins"].as_array_mut() else {
        return Ok(());
    };
    let before = disabled.len();
    disabled.retain(|name| name.as_str() != Some("mosaico"));
    if disabled.len() == before {
        return Ok(());
    }
    let rendered = serde_json::to_string_pretty(&value)? + "\n";
    atomic_write_unchecked(&path, &rendered)
}

fn enable_in_config() -> Result<()> {
    let path = goose_config_dir()
        .context("cannot resolve Goose config path")?
        .join("config.yaml");
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let mut value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&body)
        .with_context(|| format!("parsing Goose config {}", path.display()))?;
    let plugin = plugin_root()?.to_string_lossy().to_string();
    let mut changed = false;
    if value["plugins"][plugin.as_str()]["enabled"].as_bool() == Some(false) {
        value["plugins"][plugin.as_str()]["enabled"] = serde_yaml_ng::Value::Bool(true);
        changed = true;
    }
    if value["extensions"]["tom"]["enabled"].as_bool() == Some(false) {
        value["extensions"]["tom"]["enabled"] = serde_yaml_ng::Value::Bool(true);
        changed = true;
    }
    if !changed {
        return Ok(());
    }
    let rendered = serde_yaml_ng::to_string(&value)?;
    atomic_write_unchecked(&path, &rendered)
}
