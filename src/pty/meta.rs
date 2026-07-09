use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchMetadata {
    pub id: String,
    pub socket: String,
    pub supervisor_pid: u32,
    pub agent: String,
    pub root: String,
    pub cwd: String,
    #[serde(default)]
    pub ephemeral: bool,
    pub command: Vec<String>,
}

pub fn session_dir() -> PathBuf {
    crate::config::edge_home().join("pty")
}

pub fn session_socket(id: &str) -> PathBuf {
    socket_dir_for(&crate::config::edge_home(), current_uid()).join(format!("{id}.sock"))
}

fn socket_dir_for(edge_home: &std::path::Path, uid: u32) -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/tmp")
            .join(format!("tenex-edge-pty-{uid}"))
            .join(edge_home_hash(edge_home))
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir()
            .join(format!("tenex-edge-pty-{uid}"))
            .join(edge_home_hash(edge_home))
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

fn edge_home_hash(edge_home: &std::path::Path) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in edge_home.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn metadata_path(id: &str) -> PathBuf {
    session_dir().join(format!("{id}.json"))
}

pub fn write_metadata(meta: &LaunchMetadata) -> Result<()> {
    std::fs::create_dir_all(session_dir()).context("creating pty session directory")?;
    let bytes = serde_json::to_vec_pretty(meta)?;
    std::fs::write(metadata_path(&meta.id), bytes).context("writing pty metadata")
}

pub fn remove_metadata(id: &str) -> Result<()> {
    match std::fs::remove_file(metadata_path(id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context("removing pty metadata"),
    }
}

pub fn read_all_metadata() -> Vec<LaunchMetadata> {
    let Ok(entries) = std::fs::read_dir(session_dir()) else {
        return Vec::new();
    };
    let mut out = entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<LaunchMetadata>(&bytes).ok())
        .collect::<Vec<_>>();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

pub fn resolve_socket(id_or_path: &str) -> PathBuf {
    let path = PathBuf::from(id_or_path);
    if path.components().count() > 1 || id_or_path.ends_with(".sock") {
        path
    } else if let Ok(bytes) = std::fs::read(metadata_path(id_or_path)) {
        serde_json::from_slice::<LaunchMetadata>(&bytes)
            .ok()
            .map(|meta| PathBuf::from(meta.socket))
            .unwrap_or_else(|| session_socket(id_or_path))
    } else {
        session_socket(id_or_path)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn socket_path_stays_short_for_long_edge_home() {
        use std::os::unix::ffi::OsStrExt;

        let edge_home = std::path::Path::new(
            "/var/folders/kx/13lj0yd976x0tn90z1ntqbn80000gn/T/tenex-edge-e2e/edge-b/edge",
        );
        let path = super::socket_dir_for(edge_home, 501).join("testing-lead-1783399436-28334.sock");

        assert!(path.as_os_str().as_bytes().len() < 100);
    }
}
