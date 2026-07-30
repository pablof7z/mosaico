use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BoundaryAction {
    Allow,
    Warn,
    Deny,
}

impl BoundaryAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Warn => "warn",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct CrossProjectBoundary {
    pub read: BoundaryAction,
    pub write: BoundaryAction,
}

impl Default for CrossProjectBoundary {
    fn default() -> Self {
        Self {
            read: BoundaryAction::Warn,
            write: BoundaryAction::Deny,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawAgents {
    pub(super) behavior: RawBehavior,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct RawBehavior {
    pub(super) cross_project_boundary: CrossProjectBoundary,
}
