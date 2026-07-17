/// One local agent runtime hosted by this daemon. `pubkey` is its sole identity;
/// `runtime_generation` only fences stale asynchronous lifecycle callbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub pubkey: String,
    pub runtime_generation: u64,
    pub agent_slug: String,
    pub channel_h: String,
    /// Host-local workspace root resolved from the session's cwd. This is
    /// runtime execution context, never relay-authored channel metadata.
    pub work_root: String,
    /// Immediate parent known when the session was admitted. Used only as a
    /// readiness fallback while relay metadata is absent.
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
    pub alive: bool,
    pub created_at: u64,
    pub last_seen: u64,
    pub working: bool,
    pub turn_started_at: u64,
    pub seen_cursor: u64,
    pub title: String,
    pub explicit_chat_published_at: u64,
}
