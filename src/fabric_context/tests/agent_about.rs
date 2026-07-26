use super::host_profiles::advertise_host;
use super::*;
use crate::reconcile::{hook_context::FrameKind, HookContextState};

#[test]
fn canonical_context_and_human_view_keep_host_capabilities() {
    let store = seed_store();
    let rec = session(&store);
    advertise_host(
        &store,
        "backend",
        "laptop",
        &[("helper", "For testing")],
        &["root"],
        2,
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("context should render");
    assert!(!text.contains("mosaico agents list"));
    assert!(!text.contains("<available-agents>"));
    assert!(!text.contains("<workspace-agents>"));
    assert!(text.contains("<host name=\"laptop\">"), "{text}");
    assert!(text.contains("Workspaces:\n      * root"), "{text}");
    assert!(
        text.contains("<agent ref=\"helper@laptop\" about=\"For testing\" />"),
        "{text}"
    );
    assert!(text.contains("<members>"), "{text}");

    let human = render_fabric_context_human(&store, input(Some(&rec), "root", 0, 100, true), false)
        .unwrap()
        .unwrap();
    assert!(human.contains("Available agents"));
    assert!(human.contains("@helper@laptop  For testing"));

    let empty = Store::open_memory().unwrap();
    empty.upsert_channel("solo", "solo", "", "", 1).unwrap();
    empty.replace_channel_members("solo", &[], 1).unwrap();
    empty.replace_channel_admins("solo", &[], 1).unwrap();
    empty
        .upsert_profile(SELF_PK, "coder", "coder", "laptop", false, 1)
        .unwrap();
    let solo = session_record(&empty, "solo", "solo");
    let text = render_fabric_context(&empty, input(Some(&solo), "solo", 0, 100, true)).unwrap();
    assert!(!text.contains("<workspace"));
    assert!(text.contains("<channel name=\"/solo\" agents=\"0\""));
    assert!(!text.contains("<members>"), "got: {text}");
}

#[test]
fn human_agent_about_is_compact_and_bounded() {
    let store = seed_store();
    let rec = session(&store);
    let long_about = format!("Routes\\nreview work {}", "carefully ".repeat(40));
    advertise_host(
        &store,
        "backend",
        "laptop",
        &[("helper", &long_about)],
        &["root"],
        2,
    );

    let text = render_fabric_context_human(&store, input(Some(&rec), "root", 0, 100, true), false)
        .unwrap()
        .expect("context should render");
    let expected = crate::agent_about::for_injection(&long_about);
    assert!(text.contains(&expected), "got: {text}");
    assert!(expected.chars().count() <= crate::agent_about::AGENT_ABOUT_MAX_CHARS);
}

#[test]
fn host_profile_delta_emits_through_the_canonical_hook_context() {
    let store = seed_store();
    let rec = session(&store);
    let mut state = HookContextState::default();

    let before = capture_inputs(&store, &input(Some(&rec), "root", 100, 200, false)).unwrap();
    let baseline = state.render_context("sess", "turn_start", 100, 200, before);
    let baseline = baseline
        .text
        .expect("a fresh hook cache re-baselines joined members");
    assert!(baseline.contains("<channel name=\"/root\""), "{baseline}");
    assert!(!baseline.contains("<hosts>"), "{baseline}");

    advertise_host(
        &store,
        "backend",
        "laptop",
        &[("new-helper", "Newly available")],
        &["root"],
        150,
    );
    let after = capture_inputs(&store, &input(Some(&rec), "root", 100, 200, false)).unwrap();
    let changed = state.render_context("sess", "turn_start", 100, 200, after);

    let text = changed.text.expect("new capability is a delta");
    assert!(text.contains("<hosts>"), "{text}");
    assert!(
        text.contains("<agent ref=\"new-helper@laptop\" about=\"Newly available\" />"),
        "{text}"
    );
    assert!(!text.contains("<channels>"), "{text}");
    assert_eq!(changed.receipt.frame, FrameKind::Delta);
}
