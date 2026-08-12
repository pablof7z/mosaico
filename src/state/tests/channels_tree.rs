use super::super::*;

#[test]
fn channel_human_name_distinguishes_root_slug_from_unnamed_session_room() {
    let channel = |channel_h: &str, name: &str, parent: &str| Channel {
        channel_h: channel_h.into(),
        name: name.into(),
        about: String::new(),
        parent: parent.into(),
        created_at: 1,
        updated_at: 1,
    };
    assert_eq!(
        channel("mosaico", "mosaico", "").human_name(),
        Some("mosaico")
    );
    assert_eq!(
        channel("ab12cd34", "support", "proj").human_name(),
        Some("support")
    );
    assert_eq!(
        channel("session-x1", "session-x1", "proj").human_name(),
        None
    );
    assert_eq!(channel("", "", "").human_name(), None);
    assert_eq!(channel("ab12cd34", "   ", "proj").human_name(), None);
}
