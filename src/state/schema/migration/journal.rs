//! Crash-safe handoff of schema-7 pending writes to NMP's durable queue.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct PendingWrites {
    source_schema: u32,
    event_json: Vec<String>,
}

pub(crate) fn load_pending_writes(state_db: &Path) -> Result<Vec<String>> {
    let path = journal_path(state_db);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
    let journal: PendingWrites =
        serde_json::from_reader(file).with_context(|| format!("reading {}", path.display()))?;
    if journal.source_schema != 7 {
        anyhow::bail!(
            "{} has unsupported source schema {}",
            path.display(),
            journal.source_schema
        );
    }
    Ok(journal.event_json)
}

pub(crate) fn replace_pending_writes(state_db: &Path, rows: &[String]) -> Result<()> {
    if rows.is_empty() {
        return clear_pending_writes(state_db);
    }
    let path = journal_path(state_db);
    let temp = temporary_path(&path);
    let file = File::create(&temp).with_context(|| format!("creating {}", temp.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(
        &mut writer,
        &PendingWrites {
            source_schema: 7,
            event_json: rows.to_vec(),
        },
    )
    .with_context(|| format!("writing {}", temp.display()))?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    fs::rename(&temp, &path)
        .with_context(|| format!("installing migration journal {}", path.display()))?;
    sync_parent(&path)
}

pub(crate) fn clear_pending_writes(state_db: &Path) -> Result<()> {
    let path = journal_path(state_db);
    match fs::remove_file(&path) {
        Ok(()) => sync_parent(&path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

pub(super) fn merge_pending_writes(state_db: &Path, rows: Vec<String>) -> Result<()> {
    let mut merged = load_pending_writes(state_db)?;
    merged.extend(rows);
    let mut seen = BTreeSet::new();
    merged.retain(|row| seen.insert(row.clone()));
    replace_pending_writes(state_db, &merged)
}

fn journal_path(state_db: &Path) -> PathBuf {
    let mut path = state_db.as_os_str().to_os_string();
    path.push(".schema-7-pending-writes.json");
    PathBuf::from(path)
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut temp = path.as_os_str().to_os_string();
    temp.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(temp)
}

fn sync_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}
