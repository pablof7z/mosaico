use super::{AutoReplyTracker, PendingAutoReply};

fn pending(channel: &str, event: &str) -> PendingAutoReply {
    PendingAutoReply {
        channel_h: channel.to_string(),
        trigger_event_id: event.to_string(),
    }
}

#[test]
fn armed_turn_with_no_publish_yields_pending() {
    let mut t = AutoReplyTracker::default();
    t.arm("s1", "chan", "evt1");
    assert_eq!(t.take("s1"), Some(pending("chan", "evt1")));
}

#[test]
fn explicit_publish_cancels_auto_reply() {
    let mut t = AutoReplyTracker::default();
    t.arm("s1", "chan", "evt1");
    t.note_published("s1");
    assert_eq!(t.take("s1"), None);
}

#[test]
fn non_injected_turn_has_nothing_to_publish() {
    let mut t = AutoReplyTracker::default();
    assert_eq!(t.take("s-never-armed"), None);
}

#[test]
fn newest_mention_supersedes_earlier_unanswered() {
    let mut t = AutoReplyTracker::default();
    t.arm("s1", "chan", "evt1");
    t.arm("s1", "chan2", "evt2");
    assert_eq!(t.take("s1"), Some(pending("chan2", "evt2")));
}

#[test]
fn take_is_one_shot() {
    let mut t = AutoReplyTracker::default();
    t.arm("s1", "chan", "evt1");
    assert!(t.take("s1").is_some());
    assert!(t.take("s1").is_none());
}
