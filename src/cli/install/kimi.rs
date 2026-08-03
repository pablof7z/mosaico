//! Kimi Code `config.toml` hook integration.

use super::{write_text, Harness, InstallOpts};
use anyhow::{bail, Context, Result};
use std::ops::Range;

const BLOCK_START: &str = "# >>> mosaico kimi hooks >>>";
const BLOCK_END: &str = "# <<< mosaico kimi hooks <<<";

const HOOKS: &[(&str, &str, Option<&str>)] = &[
    ("SessionStart", "session-start", None),
    ("SessionEnd", "session-end", None),
    ("UserPromptSubmit", "user-prompt-submit", None),
    (
        "PreToolUse",
        "pre-tool-use",
        Some("Read|Write|Edit|Glob|Grep|ReadMediaFile"),
    ),
    // Kimi ignores PostToolUse stdout. Its Stop hook can block the stop and
    // feed pending context back while closing Mosaico's turn accounting.
    ("Stop", "stop", None),
];

pub(super) fn preflight(harness: &Harness) -> Result<()> {
    let body = read_optional(harness)?;
    locate_block(&body)?;
    parse_toml(harness, &body)
}

pub(super) fn install(harness: &Harness, opts: &InstallOpts, render: bool) -> Result<()> {
    let body = read_optional(harness)?;
    let updated = rewrite(&body, opts.uninstall)?;
    parse_toml(harness, &updated)?;

    if opts.dry_run {
        if render {
            let verb = if opts.uninstall { "remove" } else { "write" };
            println!(
                "  would {verb} Kimi hooks in {}",
                harness.config_path.display()
            );
        }
        return Ok(());
    }
    if updated != body {
        write_text(&harness.config_path, &updated)?;
    }
    if render {
        let verb = if opts.uninstall { "removed" } else { "wrote" };
        println!("  {verb} Kimi hooks in {}", harness.config_path.display());
    }
    Ok(())
}

pub(super) fn is_present(harness: &Harness) -> bool {
    std::fs::read_to_string(&harness.config_path)
        .map(|body| body.contains(BLOCK_START) || body.contains(BLOCK_END))
        .unwrap_or(false)
}

pub(super) fn is_installed(harness: &Harness) -> bool {
    let Ok(body) = read_optional(harness) else {
        return false;
    };
    let Ok(range) = locate_block(&body) else {
        return false;
    };
    range.is_some_and(|range| body[range].trim_end() == managed_block().trim_end())
        && toml::from_str::<toml::Value>(&body).is_ok()
}

fn read_optional(harness: &Harness) -> Result<String> {
    match std::fs::read_to_string(&harness.config_path) {
        Ok(body) => Ok(body),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => {
            Err(error).with_context(|| format!("reading {}", harness.config_path.display()))
        }
    }
}

fn parse_toml(harness: &Harness, body: &str) -> Result<()> {
    toml::from_str::<toml::Value>(body)
        .map(|_| ())
        .with_context(|| format!("{} is not valid TOML", harness.config_path.display()))
}

fn managed_block() -> String {
    let mut body = format!("{BLOCK_START}\n");
    for (event, hook_type, matcher) in HOOKS {
        body.push_str("[[hooks]]\n");
        body.push_str(&format!("event = {event:?}\n"));
        if let Some(matcher) = matcher {
            body.push_str(&format!("matcher = {matcher:?}\n"));
        }
        body.push_str(&format!(
            "command = {:?}\ntimeout = 5\n\n",
            format!("mosaico harness hook kimi --type {hook_type}")
        ));
    }
    body.push_str(BLOCK_END);
    body.push('\n');
    body
}

fn rewrite(body: &str, uninstall: bool) -> Result<String> {
    let range = locate_block(body)?;
    let replacement = (!uninstall).then(managed_block);
    match (range, replacement) {
        (None, None) => Ok(body.to_string()),
        (None, Some(block)) => Ok(append_block(body, &block)),
        (Some(mut range), None) => {
            if range.start > 0 && body[..range.start].ends_with("\n\n") {
                range.start -= 1;
            }
            let mut updated = body.to_string();
            updated.replace_range(range, "");
            Ok(updated)
        }
        (Some(range), Some(block)) => {
            let mut updated = body.to_string();
            updated.replace_range(range, &block);
            Ok(updated)
        }
    }
}

fn append_block(body: &str, block: &str) -> String {
    let mut updated = body.to_string();
    if !updated.is_empty() {
        if !updated.ends_with('\n') {
            updated.push('\n');
        }
        if !updated.ends_with("\n\n") {
            updated.push('\n');
        }
    }
    updated.push_str(block);
    updated
}

fn locate_block(body: &str) -> Result<Option<Range<usize>>> {
    let starts = body.match_indices(BLOCK_START).collect::<Vec<_>>();
    let ends = body.match_indices(BLOCK_END).collect::<Vec<_>>();
    if starts.is_empty() && ends.is_empty() {
        return Ok(None);
    }
    if starts.len() != 1 || ends.len() != 1 || starts[0].0 >= ends[0].0 {
        bail!("malformed Mosaico Kimi hook block; refusing to edit config.toml");
    }
    let start = starts[0].0;
    let end_marker = ends[0].0 + BLOCK_END.len();
    if (start > 0 && body.as_bytes()[start - 1] != b'\n')
        || body
            .as_bytes()
            .get(end_marker)
            .is_some_and(|byte| *byte != b'\n')
    {
        bail!("Mosaico Kimi hook markers must occupy their own lines");
    }
    let end = if body.as_bytes().get(end_marker) == Some(&b'\n') {
        end_marker + 1
    } else {
        end_marker
    };
    Ok(Some(start..end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harness(path: std::path::PathBuf) -> Harness {
        Harness {
            id: "kimi",
            display: "Kimi Code",
            config_path: path,
            detected: true,
        }
    }

    #[test]
    fn install_preserves_foreign_toml_and_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        std::fs::write(
            &path,
            "default_model = \"test\"\n\n[[hooks]]\nevent = \"Notification\"\ncommand = \"notify\"\n",
        )
        .unwrap();
        let harness = harness(path.clone());
        install(&harness, &InstallOpts::default(), false).unwrap();
        let once = std::fs::read_to_string(&path).unwrap();
        install(&harness, &InstallOpts::default(), false).unwrap();
        assert_eq!(once, std::fs::read_to_string(&path).unwrap());
        assert!(once.contains("command = \"notify\""));
        assert!(is_installed(&harness));
        let value: toml::Value = toml::from_str(&once).unwrap();
        let hooks = value["hooks"].as_array().unwrap();
        assert_eq!(hooks.len(), 6);
        let stop = hooks
            .iter()
            .find(|hook| hook["event"].as_str() == Some("Stop"))
            .unwrap();
        assert!(stop["command"].as_str().unwrap().ends_with("--type stop"));
    }

    #[test]
    fn uninstall_removes_only_the_managed_block() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        let harness = harness(path.clone());
        install(&harness, &InstallOpts::default(), false).unwrap();
        let opts = InstallOpts {
            uninstall: true,
            ..InstallOpts::default()
        };
        install(&harness, &opts, false).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "");
        assert!(!is_present(&harness));
    }

    #[test]
    fn malformed_markers_fail_without_overwriting() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        std::fs::write(&path, format!("{BLOCK_START}\n")).unwrap();
        let harness = harness(path);
        assert!(preflight(&harness)
            .unwrap_err()
            .to_string()
            .contains("malformed"));
    }
}
