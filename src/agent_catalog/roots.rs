use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DiscoveryRoots {
    pub codex: PathBuf,
    pub codex_profiles: PathBuf,
    pub claude: PathBuf,
    pub opencode: PathBuf,
    pub hermes: PathBuf,
    pub kimi: PathBuf,
    pub shared_agents: PathBuf,
}

impl DiscoveryRoots {
    pub fn for_user_home(home: &Path) -> Self {
        Self {
            codex: home.join(".codex/agents"),
            codex_profiles: home.join(".codex"),
            claude: home.join(".claude/agents"),
            opencode: home.join(".config/opencode/agents"),
            hermes: home.join(".hermes/profiles"),
            kimi: home.join(".kimi-code/agents"),
            shared_agents: home.join(".agents/agents"),
        }
    }

    pub fn installed() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is required to discover installed harness agents")?;
        let mut roots = Self::for_user_home(&home);
        if let Some(codex_home) = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
            roots.codex_profiles = PathBuf::from(codex_home);
            roots.codex = roots.codex_profiles.join("agents");
        }
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            roots.opencode = PathBuf::from(xdg).join("opencode/agents");
        }
        if let Some(hermes_home) = std::env::var_os("HERMES_HOME").filter(|v| !v.is_empty()) {
            roots.hermes = hermes_profiles_root(&home, &PathBuf::from(hermes_home));
        }
        if let Some(kimi_home) = std::env::var_os("KIMI_CODE_HOME").filter(|v| !v.is_empty()) {
            roots.kimi = PathBuf::from(kimi_home).join("agents");
        }
        Ok(roots)
    }
}

fn hermes_profiles_root(home: &Path, hermes_home: &Path) -> PathBuf {
    let native_root = home.join(".hermes");
    if hermes_home.starts_with(&native_root) {
        return native_root.join("profiles");
    }
    if hermes_home.parent().and_then(Path::file_name) == Some(std::ffi::OsStr::new("profiles")) {
        return hermes_home.parent().unwrap().to_path_buf();
    }
    hermes_home.join("profiles")
}
