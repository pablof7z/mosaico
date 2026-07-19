use super::*;
use crate::reconcile::{hook_context::FrameKind, HookContextState};

#[test]
fn agent_context_uses_on_demand_discovery_while_human_view_keeps_roster() {
    let store = seed_store();
    let rec = session(&store);
    store
        .replace_agent_roster(&crate::state::AgentRoster {
            backend_pubkey: "backend".into(),
            host: "laptop".into(),
            slug: "helper".into(),
            use_criteria: "For testing".into(),
            channels: vec!["root".into()],
            updated_at: 2,
        })
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("context should render");
    assert!(text.contains("List agents available to spawn: `mosaico agents list`"));
    assert!(!text.contains("<available-agents>"));
    assert!(!text.contains("<workspace-agents>"));
    assert!(!text.contains("@helper"));
    assert!(text.contains("<member ref=\"@coder\""));

    let human = render_fabric_context_human(&store, input(Some(&rec), "root", 0, 100, true), false)
        .unwrap()
        .unwrap();
    assert!(human.contains("Available agents"));
    assert!(human.contains("@helper  For testing"));

    let empty = Store::open_memory().unwrap();
    empty.upsert_channel("solo", "solo", "", "", 1).unwrap();
    empty
        .upsert_profile(SELF_PK, "coder", "coder", "laptop", false, 1)
        .unwrap();
    let solo = session_record(&empty, "solo", "solo");
    let text = render_fabric_context(&empty, input(Some(&solo), "solo", 0, 100, true)).unwrap();
    assert!(text.contains("<workspace name=\"solo\" channel=\"solo\""));
    assert!(!text.contains("<channel name=\"#solo\""));
    assert!(!text.contains("<members>"), "got: {text}");
}

#[test]
fn human_agent_about_is_compact_and_bounded() {
    let store = seed_store();
    let rec = session(&store);
    let long_about = format!("Routes\\nreview work {}", "carefully ".repeat(40));
    store
        .replace_agent_roster(&crate::state::AgentRoster {
            backend_pubkey: "backend".into(),
            host: "laptop".into(),
            slug: "helper".into(),
            use_criteria: long_about.clone(),
            channels: vec!["root".into()],
            updated_at: 2,
        })
        .unwrap();

    let text = render_fabric_context_human(&store, input(Some(&rec), "root", 0, 100, true), false)
        .unwrap()
        .expect("context should render");
    let expected = crate::agent_about::for_injection(&long_about);
    assert!(text.contains(&expected), "got: {text}");
    assert!(expected.chars().count() <= crate::agent_about::AGENT_ABOUT_MAX_CHARS);
}

#[test]
fn roster_only_delta_does_not_emit_agent_hook_context() {
    let store = seed_store();
    let rec = session(&store);
    let mut state = HookContextState::default();

    let before = capture_inputs(&store, &input(Some(&rec), "root", 100, 200, false)).unwrap();
    let baseline = state.render_context("sess", "turn_start", 100, 200, before);
    assert!(baseline.text.is_none());

    store
        .replace_agent_roster(&crate::state::AgentRoster {
            backend_pubkey: "backend".into(),
            host: "laptop".into(),
            slug: "new-helper".into(),
            use_criteria: "Newly available".into(),
            channels: vec!["root".into()],
            updated_at: 150,
        })
        .unwrap();
    let after = capture_inputs(&store, &input(Some(&rec), "root", 100, 200, false)).unwrap();
    let unchanged = state.render_context("sess", "turn_start", 100, 200, after);

    assert!(unchanged.text.is_none());
    assert_eq!(unchanged.receipt.frame, FrameKind::Unchanged);
}
