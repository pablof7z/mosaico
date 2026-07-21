use super::skills;
use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::cli) enum SkillHealthState {
    Missing,
    Stale,
    Healthy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::cli) struct SkillTargetHealth {
    pub label: &'static str,
    pub path: PathBuf,
    pub state: SkillHealthState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::cli) struct SkillHealth {
    pub canonical_path: PathBuf,
    pub targets: Vec<SkillTargetHealth>,
}

pub(in crate::cli) fn skill_health() -> Result<SkillHealth> {
    skills::health()
}

pub(in crate::cli) fn repair_skill() -> Result<SkillHealth> {
    skills::repair()
}
