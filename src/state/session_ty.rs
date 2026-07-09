/// A local agent process THIS daemon hosts. OS handles only — never agent
/// identity (that lives in `relay_status`/`relay_profiles`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub session_id: String,
    pub agent_pubkey: String,
    pub agent_slug: String,
    pub channel_h: String,
    pub harness: String,
    pub child_pid: Option<i32>,
    pub transcript_path: Option<String>,
    pub alive: bool,
    pub created_at: u64,
    pub last_seen: u64,
    pub working: bool,
    pub turn_started_at: u64,
    pub last_distill_at: u64,
    pub seen_cursor: u64,
    pub title: String,
    pub activity: String,
    pub resume_id: String,
    /// Consecutive failed status-title generation attempts (reset to 0 on the
    /// next success). Gates the throttled agent-facing heads-up in
    /// `turn_context::start`.
    pub distill_fail_streak: u64,
    /// When the agent-facing heads-up about failing status-title generation
    /// was last injected, so it repeats at most a few times per hour rather
    /// than every turn.
    pub distill_notice_at: u64,
}
