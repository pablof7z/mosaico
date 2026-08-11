use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupOperationStage {
    Configuration,
    Build,
    Publish,
}

impl fmt::Display for GroupOperationStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Configuration => "configuration",
            Self::Build => "event construction",
            Self::Publish => "NMP publish",
        };
        formatter.write_str(label)
    }
}

/// Exact provenance for a group-management operation that did not complete.
/// The detail may come from pre-publication setup or from NMP's typed terminal
/// result, and is never reclassified by polling Mosaico's projected roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GroupOperationError {
    operation: String,
    stage: GroupOperationStage,
    detail: String,
}

impl GroupOperationError {
    pub(crate) fn new(
        operation: impl Into<String>,
        stage: GroupOperationStage,
        error: impl fmt::Display,
    ) -> Self {
        Self {
            operation: operation.into(),
            stage,
            detail: error.to_string(),
        }
    }

    pub(crate) fn from_anyhow(
        operation: impl Into<String>,
        stage: GroupOperationStage,
        error: &anyhow::Error,
    ) -> Self {
        Self {
            operation: operation.into(),
            stage,
            detail: format!("{error:#}"),
        }
    }
}

impl fmt::Display for GroupOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {} failed: {}",
            self.operation, self.stage, self.detail
        )
    }
}

impl std::error::Error for GroupOperationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GroupPublishOutcome {
    Published,
    Failed(GroupOperationError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GroupMutationOutcome {
    Published,
    Failed(GroupOperationError),
}

impl GroupPublishOutcome {
    pub(crate) fn is_published(&self) -> bool {
        matches!(self, Self::Published)
    }
}

impl GroupMutationOutcome {
    pub(crate) fn is_published(&self) -> bool {
        matches!(self, Self::Published)
    }

    pub(crate) fn require_published(self, action: impl fmt::Display) -> anyhow::Result<()> {
        match self {
            Self::Published => Ok(()),
            Self::Failed(error) => Err(anyhow::Error::new(error).context(action.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GroupMutationOutcome, GroupOperationError, GroupOperationStage, GroupPublishOutcome,
    };

    #[test]
    fn publish_failure_provenance_survives_mutation_and_operator_rendering() {
        let receipt_error = anyhow::anyhow!("Previous I/O error occurred").context(
            "durable-store persistence failure [fault=latched durability=absent reopen=required]",
        );
        let publish = GroupPublishOutcome::Failed(GroupOperationError::new(
            "9000 put-user",
            GroupOperationStage::Publish,
            format!("{receipt_error:#}"),
        ));
        let mutation = match publish {
            GroupPublishOutcome::Published => GroupMutationOutcome::Published,
            GroupPublishOutcome::Failed(error) => GroupMutationOutcome::Failed(error),
        };

        let error = mutation
            .require_published("joining /mosaico/dev")
            .unwrap_err();
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("Previous I/O error occurred"),
            "{rendered}"
        );
        assert!(
            rendered.contains("fault=latched durability=absent reopen=required"),
            "{rendered}"
        );
        assert!(
            rendered.contains("9000 put-user NMP publish failed"),
            "{rendered}"
        );
        assert!(
            !rendered.to_ascii_lowercase().contains("membership"),
            "{rendered}"
        );
        assert!(
            !rendered.to_ascii_lowercase().contains("admin"),
            "{rendered}"
        );
    }
}
