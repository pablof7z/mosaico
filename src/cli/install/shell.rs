//! Mosaico-owned shell aliases for routing native harness commands through Mosaico.
//!
//! Setup may write one delimited block into the operator's shell profile. Only
//! the text between the markers belongs to Mosaico: everything else in the file
//! is foreign content that is read, preserved, and written back untouched.

mod profile;

use super::{write_text, Harness};
use anyhow::{bail, Result};
use profile::{current_profile, known_profiles, read_optional, Syntax};
use std::collections::HashSet;
use std::ops::Range;

const BLOCK_START: &str = "# >>> mosaico harness wrappers >>>";
const BLOCK_END: &str = "# <<< mosaico harness wrappers <<<";

/// Whether this machine has a shell profile Mosaico knows how to own a block in.
/// Interactive flows hide the wrapper choice instead of failing at write time.
pub(super) fn supported() -> bool {
    current_profile().is_ok()
}

/// Fail before any harness file is touched if the profile cannot be edited.
pub(super) fn preflight_current_profile() -> Result<()> {
    let profile = current_profile()?;
    locate_block(&read_optional(&profile.path)?)?;
    Ok(())
}

/// Harness ids whose wrapper is already installed in the current profile.
pub(super) fn configured_wrappers(all: &[Harness]) -> Result<HashSet<&str>> {
    let profile = current_profile()?;
    let content = read_optional(&profile.path)?;
    managed_ids(&content, all, profile.syntax)
}

/// Rewrite the owned block so it contains exactly `selected` and nothing else.
pub(super) fn sync_wrappers(all: &[Harness], selected: &[&Harness], dry_run: bool) -> Result<()> {
    let profile = current_profile()?;
    let content = read_optional(&profile.path)?;
    let selected_ids = selected.iter().map(|harness| harness.id).collect();
    let lines = wrapper_lines(all, &selected_ids, profile.syntax);
    let updated = rewrite_block(&content, &lines)?;
    if updated == content {
        return Ok(());
    }
    if dry_run {
        println!(
            "  would update shell wrappers in {}",
            profile.path.display()
        );
        for line in &lines {
            println!("    {line}");
        }
        return Ok(());
    }
    write_text(&profile.path, &updated)?;
    if lines.is_empty() {
        println!("  removed shell wrappers from {}", profile.path.display());
    } else {
        println!("  wrote shell wrappers to {}", profile.path.display());
        println!("  reload with: source {}", profile.path.display());
    }
    Ok(())
}

/// Drop `ids` from every profile Mosaico may have written, keeping the rest.
/// Scoped uninstall relies on this leaving other harnesses' wrappers in place.
pub(super) fn remove_wrappers(all: &[Harness], ids: &[&str], dry_run: bool) -> Result<()> {
    let remove = ids.iter().copied().collect::<HashSet<_>>();
    let mut updates = Vec::new();
    for profile in known_profiles()? {
        if !profile.path.exists() {
            continue;
        }
        let content = read_optional(&profile.path)?;
        let configured = managed_ids(&content, all, profile.syntax)?;
        let keep = configured
            .difference(&remove)
            .copied()
            .collect::<HashSet<_>>();
        let lines = wrapper_lines(all, &keep, profile.syntax);
        let updated = rewrite_block(&content, &lines)?;
        if updated != content {
            updates.push((profile.path, updated));
        }
    }
    for (path, updated) in updates {
        if dry_run {
            println!("  would remove shell wrapper(s) from {}", path.display());
        } else {
            write_text(&path, &updated)?;
            println!("  removed shell wrapper(s) from {}", path.display());
        }
    }
    Ok(())
}

fn wrapper_lines(all: &[Harness], selected: &HashSet<&str>, syntax: Syntax) -> Vec<String> {
    all.iter()
        .filter(|harness| selected.contains(harness.id))
        .map(|harness| wrapper_line(harness, syntax))
        .collect()
}

/// The alias a wrapped harness installs. The trailing `--` forwards native
/// arguments, so `codex resume last` still reaches Codex through Mosaico.
fn wrapper_line(harness: &Harness, syntax: Syntax) -> String {
    let command = harness.command();
    match syntax {
        Syntax::Posix => format!("alias {command}=\"mosaico {command} --\""),
        Syntax::Fish => format!("alias {command} \"mosaico {command} --\""),
    }
}

/// How the wrapper is described to an operator choosing one interactively.
pub(super) fn wrapper_preview(harness: &Harness) -> String {
    wrapper_line(harness, Syntax::Posix)
}

fn managed_ids<'a>(content: &str, all: &'a [Harness], syntax: Syntax) -> Result<HashSet<&'a str>> {
    let Some(range) = locate_block(content)? else {
        return Ok(HashSet::new());
    };
    let block = &content[range];
    Ok(all
        .iter()
        .filter(|harness| {
            block
                .lines()
                .any(|line| line == wrapper_line(harness, syntax))
        })
        .map(|harness| harness.id)
        .collect())
}

fn rewrite_block(content: &str, lines: &[String]) -> Result<String> {
    let range = locate_block(content)?;
    let block = if lines.is_empty() {
        None
    } else {
        Some(format!(
            "{BLOCK_START}\n# Managed by `mosaico setup`; rerun setup to change this list.\n{}\n{BLOCK_END}\n",
            lines.join("\n")
        ))
    };
    match (range, block) {
        (None, None) => Ok(content.to_string()),
        (None, Some(block)) => Ok(append_block(content, &block)),
        (Some(range), Some(block)) => {
            let mut updated = content.to_string();
            updated.replace_range(range, &block);
            Ok(updated)
        }
        (Some(mut range), None) => {
            if range.start > 0 && content[..range.start].ends_with("\n\n") {
                range.start -= 1;
            }
            let mut updated = content.to_string();
            updated.replace_range(range, "");
            Ok(updated)
        }
    }
}

fn append_block(content: &str, block: &str) -> String {
    let mut updated = content.to_string();
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

/// Byte range of the owned block, or `None` when the profile has none. A
/// duplicated or half-open marker pair means someone edited it by hand, so we
/// refuse to guess which text is ours.
fn locate_block(content: &str) -> Result<Option<Range<usize>>> {
    let starts = content.match_indices(BLOCK_START).collect::<Vec<_>>();
    let ends = content.match_indices(BLOCK_END).collect::<Vec<_>>();
    if starts.is_empty() && ends.is_empty() {
        return Ok(None);
    }
    if starts.len() != 1 || ends.len() != 1 || starts[0].0 >= ends[0].0 {
        bail!("malformed Mosaico shell-wrapper block; refusing to edit the profile");
    }
    let start = starts[0].0;
    let end_marker = ends[0].0 + BLOCK_END.len();
    if (start > 0 && content.as_bytes()[start - 1] != b'\n')
        || content
            .as_bytes()
            .get(end_marker)
            .is_some_and(|byte| *byte != b'\n')
    {
        bail!("Mosaico shell-wrapper markers must occupy their own lines");
    }
    let end = if content.as_bytes().get(end_marker) == Some(&b'\n') {
        end_marker + 1
    } else {
        end_marker
    };
    Ok(Some(start..end))
}

#[cfg(test)]
#[path = "shell/tests.rs"]
mod tests;
