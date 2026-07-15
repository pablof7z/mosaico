//! Translate an agent's optional harness-specific profile name through the selected
//! `(harness, transport)` driver.

use std::path::{Path, PathBuf};

use super::driver::ProfileMechanism;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexHomePlan {
    pub source: PathBuf,
    pub target: PathBuf,
}

/// A materialized plan: extra argv/env plus scratch files to write pre-launch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfilePlan {
    /// Harness-native profile selector appended after driver and bundle args.
    pub extra_argv: Vec<String>,
    /// Extra env for the child (e.g. `OPENCODE_CONFIG=<path>`).
    pub extra_env: Vec<(String, String)>,
    /// Files to materialize before launch: (absolute path, contents).
    pub files: Vec<(PathBuf, String)>,
    /// Isolated Codex home to prepare before writing the composed config.
    pub codex_home: Option<CodexHomePlan>,
}

impl ProfilePlan {
    pub fn extend(&mut self, other: Self) {
        self.extra_argv.extend(other.extra_argv);
        self.extra_env.extend(other.extra_env);
        self.files.extend(other.files);
        if other.codex_home.is_some() {
            self.codex_home = other.codex_home;
        }
    }

    pub fn materialize(&self) -> anyhow::Result<()> {
        if let Some(home) = &self.codex_home {
            super::codex_profile::prepare_home(home)?;
        }
        for (path, contents) in &self.files {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    anyhow::anyhow!("creating profile dir {}: {e}", parent.display())
                })?;
            }
            std::fs::write(path, contents)
                .map_err(|e| anyhow::anyhow!("writing profile file {}: {e}", path.display()))?;
        }
        Ok(())
    }
}

/// Build the transport-specific plan for one named profile.
pub fn plan_profile(
    mech: ProfileMechanism,
    profile: Option<&str>,
    scratch_dir: &Path,
    codex_home: Option<&Path>,
) -> anyhow::Result<ProfilePlan> {
    let Some(profile) = profile.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(ProfilePlan::default());
    };
    match mech {
        ProfileMechanism::CliFlag { flag } => Ok(ProfilePlan {
            extra_argv: vec![flag.to_string(), profile.to_string()],
            ..Default::default()
        }),
        ProfileMechanism::CodexAppServer => super::codex_profile::plan(
            profile,
            &codex_home
                .map(Path::to_path_buf)
                .unwrap_or(super::codex_profile::source_home()?),
            scratch_dir,
        ),
        ProfileMechanism::Unsupported => {
            anyhow::bail!("selected harness transport does not support a named profile")
        }
    }
}
