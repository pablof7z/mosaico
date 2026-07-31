use super::*;

fn view() -> StatuslineView {
    StatuslineView {
        agent: "amber-claude".into(),
        host: "Kubrick's Mac".into(),
        session_id: "some-long-uuid".into(),
        work_root: "#mosaico".into(),
        channels: vec!["#mosaico/support".into()],
        working: true,
        title: "Refactoring the inbox".into(),
        activity: "writing tests".into(),
        error: None,
    }
}

#[test]
fn renders_identity_root_session_title_status() {
    assert_eq!(
        render_statusline(&view(), false),
        "amber-claude #mosaico #mosaico/support [Refactoring the inbox] [writing tests]"
    );
}

#[test]
fn busy_with_no_activity_shows_working() {
    let mut v = view();
    v.activity.clear();
    assert!(render_statusline(&v, false).ends_with("[working]"));
}

#[test]
fn idle_shows_idle() {
    let mut v = view();
    v.working = false;
    assert!(render_statusline(&v, false).ends_with("[idle]"));
}

#[test]
fn zero_memberships_are_explicit() {
    let mut v = view();
    v.channels.clear();
    let rendered = render_statusline(&v, false);
    assert!(rendered.contains("no channels"));
    assert!(rendered.contains("[writing tests]"));
}

#[test]
fn multiple_memberships_are_rendered_without_selecting_one() {
    let mut v = view();
    v.channels.push("#other/review".into());
    let rendered = render_statusline(&v, false);
    assert!(rendered.contains("#mosaico/support, #other/review"));
}

#[test]
fn truncates_long_channel_memberships() {
    let mut v = view();
    v.channels = vec![format!("#mosaico/{}", "x".repeat(100))];
    assert!(render_statusline(&v, false).contains('…'));
}

#[test]
fn truncates_long_activity() {
    let mut v = view();
    v.activity = "y".repeat(100);
    assert!(render_statusline(&v, false).contains('…'));
}
