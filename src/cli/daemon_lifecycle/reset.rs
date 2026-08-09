use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::daemon::storage_paths::StoragePaths;

mod paths;

const RUNTIME_FILES: &[&str] = &[
    "state.db",
    "state.db-wal",
    "state.db-shm",
    "state.db-journal",
    "daemon.sock",
    "daemon.log",
];
const RUNTIME_DIRS: &[&str] = &[
    "sessions",
    "pty",
    "tmp",
    "harness-profiles",
    "harness-context",
    "relay-assist",
    "logs",
];
const CONTROL_FILES: &[&str] = &["daemon.inhibit", "daemon.lock"];

pub(super) fn run() -> Result<()> {
    // Resolve and audit every destructive target before stopping or reaping
    // anything. A bad attachment target must leave the live instance intact.
    let plan = ResetPlan::prepare()?;
    std::fs::create_dir_all(&plan.storage.mosaico_home)
        .with_context(|| format!("creating {}", plan.storage.mosaico_home.display()))?;
    ensure_inhibitor()?;

    if !super::request_shutdown() {
        bail!("selected daemon did not stop; no runtime state was deleted")
    }
    let Some(_startup) = crate::daemon::client::StartupLock::try_acquire()? else {
        bail!("selected daemon is running or starting; no runtime state was deleted")
    };

    // The flock stays held through reaping and every destructive operation.
    // A manual client cannot start a daemon that re-adopts sessions mid-reset.
    let report = crate::pty::reap_home_supervisors()?;
    if !report.is_clean() {
        bail!(
            "failed to reap selected instance supervisors; no databases were deleted: {}",
            report.errors.join("; ")
        );
    }
    plan.execute()?;
    eprintln!(
        "[mosaico] reset runtime state for selected instance {:?} at {}\n\
         [mosaico] kept configuration, harness definitions, and agent profile declarations\n\
         [mosaico] hooks remain inhibited; run `mosaico daemon restart` when ready",
        plan.storage.instance,
        plan.storage.mosaico_home.display()
    );
    Ok(())
}

struct ResetPlan {
    storage: StoragePaths,
    attachment_directory: PathBuf,
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
}

impl ResetPlan {
    fn prepare() -> Result<Self> {
        let mut storage = StoragePaths::current();
        if storage.config_path.exists() {
            let body = std::fs::read_to_string(&storage.config_path)
                .with_context(|| format!("reading {}", storage.config_path.display()))?;
            storage.attachment_receive_directory =
                crate::config::Config::from_json_str(&body, &crate::config::hostname())?
                    .attachment_receive_directory;
        }
        let files = RUNTIME_FILES
            .iter()
            .map(|name| storage.mosaico_home.join(name))
            .collect::<Vec<_>>();
        let mut directories = RUNTIME_DIRS
            .iter()
            .map(|name| storage.mosaico_home.join(name))
            .collect::<Vec<_>>();
        let controls = CONTROL_FILES
            .iter()
            .map(|name| storage.mosaico_home.join(name))
            .collect::<Vec<_>>();
        let external_pty_sockets = paths::external_pty_socket_directory()?;
        let attachment_directory = paths::resolved(&storage.attachment_receive_directory)?;
        let selected_home = paths::resolved(&storage.mosaico_home)?;
        for target in files
            .iter()
            .chain(&directories)
            .chain(std::iter::once(&storage.nmp_store_path))
        {
            paths::audit_internal_target(target, &selected_home)?;
        }
        for target in &controls {
            paths::audit_internal_control_file(target, &selected_home)?;
        }
        directories.push(external_pty_sockets);
        if storage.attachment_receive_directory.exists() && !attachment_directory.is_dir() {
            bail!(
                "attachmentReceiveDirectory is not a directory: {}",
                storage.attachment_receive_directory.display()
            );
        }

        let protected = paths::protected_surfaces(&storage)?;
        paths::audit_selected_home(&storage, &protected)?;
        for target in files
            .iter()
            .chain(&controls)
            .chain(&directories)
            .chain(std::iter::once(&storage.nmp_store_path))
            .chain(std::iter::once(&attachment_directory))
        {
            paths::audit_target(target, &storage, &protected)?;
        }
        Ok(Self {
            storage,
            attachment_directory,
            files,
            directories,
        })
    }

    fn execute(&self) -> Result<()> {
        clear_directory(&self.attachment_directory)?;
        for path in &self.files {
            remove_file(path)?;
        }
        for path in &self.directories {
            remove_directory(path)?;
        }
        // NMP is last: a preceding filesystem failure must never leave old
        // SQLite/session state pointing at an already-empty NMP store.
        crate::nmp_host::store::reset(&self.storage.nmp_store_path)
    }
}

fn ensure_inhibitor() -> Result<()> {
    let path = crate::daemon::inhibit_path();
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(&path)
                .with_context(|| format!("inspecting existing inhibitor {}", path.display()))?;
            if !metadata.file_type().is_file() {
                bail!(
                    "existing inhibitor is not a regular file: {}",
                    path.display()
                );
            }
            Ok(())
        }
        Err(error) => Err(error).with_context(|| format!("creating inhibitor {}", path.display())),
    }
}

fn clear_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(path).with_context(|| format!("reading {}", path.display()))? {
        let entry = entry.with_context(|| format!("reading entry in {}", path.display()))?;
        let metadata = entry.file_type()?;
        if metadata.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn remove_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

fn remove_directory(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}
