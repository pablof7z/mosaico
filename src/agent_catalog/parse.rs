use super::{AgentScope, NativeAgentProfile};
use crate::session::Harness;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Default, serde::Deserialize)]
struct HermesProfileMeta {
    #[serde(default)]
    description: String,
}

pub(super) fn discover_dir(
    dir: &Path,
    harness: Harness,
    scope: AgentScope,
    out: &mut Vec<NativeAgentProfile>,
) -> Result<()> {
    discover_dir_inner(dir, harness, &scope, out)
}

pub(super) fn discover_hermes_profiles(
    dir: &Path,
    scope: AgentScope,
    out: &mut Vec<NativeAgentProfile>,
) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", dir.display())),
    };
    for entry in entries {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let Some(slug) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if slug == "default" || !valid_hermes_profile_name(slug) {
            continue;
        }
        let metadata_path = path.join("profile.yaml");
        let use_criteria = read_hermes_description(&metadata_path);
        let modified_at = modified_at(&metadata_path)
            .or_else(|| modified_at(&path))
            .unwrap_or(0);
        out.push(NativeAgentProfile {
            slug: slug.to_string(),
            use_criteria,
            harness: Harness::Hermes,
            path,
            scope: scope.clone(),
            modified_at,
        });
    }
    Ok(())
}

fn read_hermes_description(path: &Path) -> String {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|body| serde_yaml_ng::from_str::<HermesProfileMeta>(&body).ok())
        .map(|meta| meta.description.trim().to_string())
        .unwrap_or_default()
}

fn valid_hermes_profile_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    name.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || (index > 0 && matches!(byte, b'-' | b'_'))
    })
}

fn modified_at(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
}

fn discover_dir_inner(
    dir: &Path,
    harness: Harness,
    scope: &AgentScope,
    out: &mut Vec<NativeAgentProfile>,
) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", dir.display())),
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() && harness == Harness::ClaudeCode {
            discover_dir_inner(&path, harness, scope, out)?;
            continue;
        }
        let expected = if harness == Harness::Codex {
            "toml"
        } else {
            "md"
        };
        if path.extension().and_then(|value| value.to_str()) != Some(expected) {
            continue;
        }
        if let Some(profile) = parse_profile(&path, harness, scope.clone())? {
            out.push(profile);
        }
    }
    Ok(())
}

fn parse_profile(
    path: &Path,
    harness: Harness,
    scope: AgentScope,
) -> Result<Option<NativeAgentProfile>> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("reading native agent profile {}", path.display()))?;
    let parsed = match harness {
        Harness::Codex => Some(parse_codex(&body, path)?),
        Harness::ClaudeCode => Some(parse_claude(&body, path)?),
        Harness::Opencode => parse_opencode(&body, path)?,
        Harness::Grok | Harness::Goose | Harness::Hermes | Harness::Unknown => None,
    };
    let Some((slug, use_criteria)) = parsed else {
        return Ok(None);
    };
    let modified_at = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    Ok(Some(NativeAgentProfile {
        slug,
        use_criteria,
        harness,
        path: path.to_path_buf(),
        scope,
        modified_at,
    }))
}

fn parse_codex(body: &str, path: &Path) -> Result<(String, String)> {
    let value: toml::Value =
        toml::from_str(body).with_context(|| format!("parsing Codex agent {}", path.display()))?;
    let name = required_string(&value, "name", path)?;
    let description = required_string(&value, "description", path)?;
    required_string(&value, "developer_instructions", path)?;
    Ok((name, description))
}

fn required_string(value: &toml::Value, key: &str, path: &Path) -> Result<String> {
    value
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .with_context(|| format!("native agent {} requires non-empty {key:?}", path.display()))
}

fn parse_claude(body: &str, path: &Path) -> Result<(String, String)> {
    let fields = markdown_frontmatter(body, path)?;
    Ok((
        required_field(&fields, "name", path)?,
        required_field(&fields, "description", path)?,
    ))
}

fn parse_opencode(body: &str, path: &Path) -> Result<Option<(String, String)>> {
    let fields = markdown_frontmatter(body, path)?;
    if matches!(fields.get("disable").map(String::as_str), Some("true"))
        || matches!(fields.get("mode").map(String::as_str), Some("subagent"))
    {
        return Ok(None);
    }
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .context("native agent path has no UTF-8 filename stem")?;
    Ok(Some((
        stem.to_string(),
        required_field(&fields, "description", path)?,
    )))
}

fn markdown_frontmatter(body: &str, path: &Path) -> Result<BTreeMap<String, String>> {
    let frontmatter = body
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---"))
        .map(|(frontmatter, _)| frontmatter)
        .with_context(|| format!("native agent {} requires YAML frontmatter", path.display()))?;
    Ok(frontmatter
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), unquote(value.trim())))
        .collect::<BTreeMap<_, _>>())
}

fn required_field(fields: &BTreeMap<String, String>, key: &str, path: &Path) -> Result<String> {
    fields
        .get(key)
        .cloned()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("native agent {} requires {key:?}", path.display()))
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
        .to_string()
}
