//! Authoritative local discovery for harness-native session identifiers.

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeSession {
    pub(crate) harness: crate::session::Harness,
    pub(crate) cwd: Option<PathBuf>,
}

pub(crate) fn discover(native_id: &str) -> Result<Vec<NativeSession>> {
    anyhow::ensure!(!native_id.trim().is_empty(), "native session id is empty");
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .context("HOME is not set; cannot inspect native harness sessions")?;
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let grok_home = std::env::var_os("GROK_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".grok"));
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));

    let mut found = Vec::new();
    discover_claude(&home.join(".claude/projects"), native_id, &mut found)?;
    discover_codex(&codex_home.join("sessions"), native_id, &mut found)?;
    discover_grok(&grok_home.join("sessions"), native_id, &mut found)?;
    discover_opencode(
        &data_home.join("opencode/opencode.db"),
        native_id,
        &mut found,
    )?;
    found.sort_by(|left, right| {
        (left.harness.as_str(), left.cwd.as_deref())
            .cmp(&(right.harness.as_str(), right.cwd.as_deref()))
    });
    found.dedup();
    Ok(found)
}

fn discover_claude(root: &Path, native_id: &str, found: &mut Vec<NativeSession>) -> Result<()> {
    let expected = format!("{native_id}.jsonl");
    visit_files(root, &mut |path| {
        if path.file_name().and_then(|name| name.to_str()) != Some(expected.as_str()) {
            return Ok(());
        }
        if let Some(cwd) = jsonl_cwd(path, |value| {
            value.get("sessionId").and_then(|id| id.as_str()) == Some(native_id)
        })? {
            found.push(NativeSession {
                harness: crate::session::Harness::ClaudeCode,
                cwd,
            });
        }
        Ok(())
    })
}

fn discover_codex(root: &Path, native_id: &str, found: &mut Vec<NativeSession>) -> Result<()> {
    let suffix = format!("-{native_id}.jsonl");
    visit_files(root, &mut |path| {
        let matches_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(&suffix))
            .unwrap_or(false);
        if !matches_name {
            return Ok(());
        }
        if let Some(cwd) = jsonl_cwd(path, |value| {
            value.get("type").and_then(|kind| kind.as_str()) == Some("session_meta")
                && value.pointer("/payload/id").and_then(|id| id.as_str()) == Some(native_id)
        })? {
            found.push(NativeSession {
                harness: crate::session::Harness::Codex,
                cwd,
            });
        }
        Ok(())
    })
}

fn discover_grok(root: &Path, native_id: &str, found: &mut Vec<NativeSession>) -> Result<()> {
    visit_files(root, &mut |path| {
        if path.file_name().and_then(|name| name.to_str()) != Some("summary.json")
            || path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                != Some(native_id)
        {
            return Ok(());
        }
        let value = read_json(path)?;
        if value.pointer("/info/id").and_then(|id| id.as_str()) == Some(native_id) {
            found.push(NativeSession {
                harness: crate::session::Harness::Grok,
                cwd: value
                    .pointer("/info/cwd")
                    .and_then(|cwd| cwd.as_str())
                    .map(PathBuf::from),
            });
        }
        Ok(())
    })
}

fn discover_opencode(
    database: &Path,
    native_id: &str,
    found: &mut Vec<NativeSession>,
) -> Result<()> {
    if !database.is_file() {
        return Ok(());
    }
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening OpenCode session database {}", database.display()))?;
    let cwd = connection
        .query_row(
            "SELECT directory FROM session WHERE id=?1",
            [native_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("querying OpenCode session database {}", database.display()))?;
    if let Some(cwd) = cwd {
        found.push(NativeSession {
            harness: crate::session::Harness::Opencode,
            cwd: (!cwd.trim().is_empty()).then(|| PathBuf::from(cwd)),
        });
    }
    Ok(())
}

fn jsonl_cwd(
    path: &Path,
    matches: impl Fn(&serde_json::Value) -> bool,
) -> Result<Option<Option<PathBuf>>> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("reading native session metadata {}", path.display()))?;
    for line in body.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("parsing native session metadata {}", path.display()))?;
        if matches(&value) {
            let cwd = value
                .get("cwd")
                .or_else(|| value.pointer("/payload/cwd"))
                .and_then(|cwd| cwd.as_str())
                .filter(|cwd| !cwd.trim().is_empty())
                .map(PathBuf::from);
            return Ok(Some(cwd));
        }
    }
    Ok(None)
}

fn read_json(path: &Path) -> Result<serde_json::Value> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("reading native session metadata {}", path.display()))?;
    serde_json::from_str(&body)
        .with_context(|| format!("parsing native session metadata {}", path.display()))
}

fn visit_files(root: &Path, visit: &mut impl FnMut(&Path) -> Result<()>) -> Result<()> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", root.display())),
    };
    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            visit_files(&path, visit)?;
        } else if file_type.is_file() {
            visit(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "native_discovery/tests.rs"]
mod tests;
