/// One local agent runtime hosted by this daemon. `pubkey` is its sole identity;
/// `runtime_generation` only fences stale asynchronous lifecycle callbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub pubkey: String,
    pub runtime_generation: u64,
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
    pub work_topic: String,
    pub work_topic_set_at: u64,
    pub seen_cursor: u64,
    pub title: String,
    pub activity: String,
    pub distill_fail_streak: u64,
    pub distill_notice_at: u64,
    pub explicit_chat_published_at: u64,
}

impl Session {
    pub fn work_topic_suppresses_distillation(&self, now: u64) -> bool {
        crate::work_topic::suppresses_distillation(&self.work_topic, self.work_topic_set_at, now)
    }

    pub fn visible_work_topic(&self, now: u64) -> Option<&str> {
        crate::work_topic::is_visible(&self.work_topic, self.work_topic_set_at, now)
            .then_some(self.work_topic.as_str())
    }
}
