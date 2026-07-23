use anyhow::{bail, Result};

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            pub(crate) fn parse(value: &str) -> Result<Self> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    _ => bail!("unknown {} value {value:?}", stringify!($name)),
                }
            }
        }
    };
}

string_enum!(RuntimeState {
    Running => "running",
    Stopping => "stopping",
    Stopped => "stopped",
});

string_enum!(PresentationState {
    Unavailable => "unavailable",
    Headed => "headed",
    Headless => "headless",
});

string_enum!(WorkState {
    Idle => "idle",
    Working => "working",
});

string_enum!(RecoveryState {
    Pending => "pending",
    Ready => "ready",
    Revoked => "revoked",
});

string_enum!(StopReason {
    Unknown => "unknown",
    AttachedCleanExit => "attached_clean_exit",
    IdleEvicted => "idle_evicted",
    HeadlessExit => "headless_exit",
    Crash => "crash",
    OperatorKill => "operator_kill",
    Revoked => "revoked",
    Superseded => "superseded",
});

/// One durable agent session and its current runtime incarnation.
///
/// Runtime generation fences process callbacks. Lifecycle and attachment epochs
/// independently fence persisted timers and supervisor presentation events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub pubkey: String,
    pub runtime_generation: u64,
    pub agent_slug: String,
    pub channel_h: String,
    pub work_root: String,
    pub readiness_parent: String,
    /// Harness observed from the admitted launch plan or the external process.
    /// Hook host claims never write this field.
    pub observed_harness: String,
    /// Last hook host claim, retained only for mismatch diagnostics.
    pub claimed_harness: String,
    /// Launch-selected harness bundle. Empty for externally discovered sessions.
    pub admitted_bundle: String,
    /// Hosted transport selected when admitted (`pty`/`acp`/`app-server`).
    pub admitted_transport: String,
    /// Where the endpoint facts came from (`launch`, `hook`, or `migration`).
    pub endpoint_provenance: String,
    pub child_pid: Option<i32>,
    pub transcript_path: Option<String>,
    pub runtime_state: RuntimeState,
    pub presentation_state: PresentationState,
    pub work_state: WorkState,
    pub recovery_state: RecoveryState,
    pub lifecycle_epoch: u64,
    pub attachment_epoch: u64,
    pub idle_since: u64,
    pub idle_deadline: u64,
    pub stopped_at: u64,
    pub stop_reason: Option<StopReason>,
    pub turn_count: u64,
    pub created_at: u64,
    pub last_seen: u64,
    pub turn_started_at: u64,
    pub seen_cursor: u64,
    pub title: String,
    pub explicit_chat_published_at: u64,
    /// Semantic transition time for the canonical public-state projection.
    pub state_changed_at: u64,
}

impl Session {
    pub fn is_running(&self) -> bool {
        self.runtime_state == RuntimeState::Running
    }

    pub fn is_working(&self) -> bool {
        self.work_state == WorkState::Working
    }

    pub fn can_fresh_relaunch_exact(&self) -> bool {
        self.runtime_state == RuntimeState::Stopped && self.recovery_state == RecoveryState::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_enum_values_are_closed() {
        assert_eq!(
            RuntimeState::parse("running").unwrap(),
            RuntimeState::Running
        );
        assert!(RuntimeState::parse("alive").is_err());
        assert_eq!(StopReason::IdleEvicted.as_str(), "idle_evicted");
    }
}
