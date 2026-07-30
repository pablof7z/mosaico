use super::PresentationSnapshot;
use anyhow::{Context, Result};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub(crate) struct SupervisorExitReport {
    pub(crate) pty_id: String,
    pub(crate) child_success: Option<bool>,
    pub(crate) child_exit_code: Option<u32>,
    #[serde(default)]
    pub(crate) command: Vec<String>,
    #[serde(default)]
    pub(crate) diagnostic_tail: String,
    pub(crate) presentation: PresentationSnapshot,
    pub(crate) recorded_at: u64,
}

pub(crate) fn persist(report: &SupervisorExitReport) -> Result<()> {
    let directory = super::session_dir();
    std::fs::create_dir_all(&directory)?;
    let destination = path(&report.pty_id);
    let temporary = directory.join(format!(
        ".{}.exit-{}.tmp",
        report.pty_id,
        std::process::id()
    ));
    let file = std::fs::File::create(&temporary)
        .with_context(|| format!("creating {}", temporary.display()))?;
    serde_json::to_writer(file, report)?;
    std::fs::rename(&temporary, &destination)
        .with_context(|| format!("publishing {}", destination.display()))?;
    Ok(())
}

pub(crate) fn remove(pty_id: &str) {
    let _ = std::fs::remove_file(path(pty_id));
}

pub(crate) fn read_all() -> Vec<SupervisorExitReport> {
    let Ok(entries) = std::fs::read_dir(super::session_dir()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".exit.json"))
        .filter_map(|entry| {
            let bytes = std::fs::read(entry.path()).ok()?;
            serde_json::from_slice(&bytes).ok()
        })
        .collect()
}

fn path(pty_id: &str) -> std::path::PathBuf {
    super::session_dir().join(format!("{pty_id}.exit.json"))
}
