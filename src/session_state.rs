//! Canonical user-facing session state.
//!
//! Runtime and transport facts stay below this boundary. Every surface renders
//! one of these four product states, and the hosting daemon publishes the same
//! normalized value for peers.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// The session is online and mid-turn.
    Working,
    /// The session is online, between turns, and can be driven automatically.
    Idle,
    /// The session is online and between turns, but must be resumed manually.
    Suspended,
    /// The session is not live.
    #[default]
    Offline,
}

impl SessionState {
    /// Normalize host facts into the one user-facing state vocabulary.
    pub fn classify(live: bool, working: bool, automatic_delivery: bool) -> Self {
        if !live {
            Self::Offline
        } else if working {
            Self::Working
        } else if automatic_delivery {
            Self::Idle
        } else {
            Self::Suspended
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Idle => "idle",
            Self::Suspended => "suspended",
            Self::Offline => "offline",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "working" => Some(Self::Working),
            "idle" => Some(Self::Idle),
            "suspended" => Some(Self::Suspended),
            "offline" => Some(Self::Offline),
            _ => None,
        }
    }

    pub fn is_working(self) -> bool {
        self == Self::Working
    }

    pub fn is_live(self) -> bool {
        self != Self::Offline
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_four_product_states() {
        assert_eq!(
            SessionState::classify(true, true, false),
            SessionState::Working
        );
        assert_eq!(
            SessionState::classify(true, false, true),
            SessionState::Idle
        );
        assert_eq!(
            SessionState::classify(true, false, false),
            SessionState::Suspended
        );
        assert_eq!(
            SessionState::classify(false, true, true),
            SessionState::Offline
        );
    }
}
