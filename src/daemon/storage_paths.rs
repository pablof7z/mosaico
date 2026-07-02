use crate::config;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct StoragePaths {
    pub(crate) edge_home: PathBuf,
    pub(crate) config_path: PathBuf,
    pub(crate) socket_path: PathBuf,
    pub(crate) lock_path: PathBuf,
    pub(crate) daemon_log_path: PathBuf,
    pub(crate) state_db_path: PathBuf,
    pub(crate) tenex_edge_home_set: bool,
    pub(crate) edge_home_is_default: bool,
    pub(crate) isolated_home_acknowledged: bool,
}

impl StoragePaths {
    pub(crate) fn current() -> Self {
        let home = config::edge_home_selection();
        Self {
            edge_home: home.edge_home,
            config_path: config::config_path(),
            socket_path: super::socket_path(),
            lock_path: super::lock_path(),
            daemon_log_path: super::log_path(),
            state_db_path: super::store_path(),
            tenex_edge_home_set: home.tenex_edge_home_set,
            edge_home_is_default: home.edge_home_is_default,
            isolated_home_acknowledged: config::isolated_home_acknowledged(),
        }
    }
}
