//! Sanitized terminal app-server turn outcomes.

pub(super) const MAX_ERROR_CHARS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnFailure {
    pub message: String,
    pub additional_details: Option<String>,
}

/// Authoritative terminal status from the current Codex app-server protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    Completed {
        thread_id: String,
        turn_id: String,
    },
    Failed {
        thread_id: String,
        turn_id: String,
        error: Option<TurnFailure>,
    },
    Interrupted {
        thread_id: String,
        turn_id: String,
    },
}

impl TurnOutcome {
    pub fn thread_id(&self) -> &str {
        match self {
            Self::Completed { thread_id, .. }
            | Self::Failed { thread_id, .. }
            | Self::Interrupted { thread_id, .. } => thread_id,
        }
    }

    pub fn turn_id(&self) -> &str {
        match self {
            Self::Completed { turn_id, .. }
            | Self::Failed { turn_id, .. }
            | Self::Interrupted { turn_id, .. } => turn_id,
        }
    }
}

impl std::fmt::Display for TurnOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Completed { turn_id, .. } => write!(f, "turn {turn_id} completed"),
            Self::Interrupted { turn_id, .. } => write!(f, "turn {turn_id} interrupted"),
            Self::Failed { turn_id, error, .. } => {
                write!(f, "turn {turn_id} failed")?;
                if let Some(error) = error {
                    write!(f, ": {}", error.message)?;
                }
                Ok(())
            }
        }
    }
}

pub(super) fn sanitize_error(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_ERROR_CHARS)
        .collect()
}
