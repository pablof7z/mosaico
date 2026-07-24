use super::super::relay;
use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn harness(
    id: &'static str,
    display: &'static str,
    detected: bool,
) -> crate::cli::install::config::Harness {
    crate::cli::install::config::Harness {
        id,
        display,
        config_path: std::path::PathBuf::from("/tmp/x"),
        detected,
    }
}

fn fixture() -> Onboarding {
    Onboarding::new(vec![
        harness("claude-code", "Claude Code", true),
        harness("codex", "Codex", false),
    ])
    .expect("build onboarding")
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

#[test]
fn generated_identity_is_valid() {
    let state = fixture();
    assert!(nostr::Keys::parse(&state.identity.nsec).is_ok());
    assert!(state.identity.npub.starts_with("npub1"));
    assert_eq!(state.identity.pubkey_hex.len(), 64);
}

#[test]
fn device_name_default_is_bounded() {
    let state = fixture();
    assert!(!state.device_name.is_empty());
    assert!(state.device_name.chars().count() <= DEVICE_NAME_CAP);
}

#[test]
fn detected_harnesses_are_preselected() {
    let state = fixture();
    assert_eq!(state.selected, vec![true, false]);
}

#[test]
fn device_name_editing_respects_cap() {
    let mut state = fixture();
    state.step = Step::DeviceName;
    state.device_name.clear();
    for _ in 0..40 {
        state.handle_key(press(KeyCode::Char('a')));
    }
    assert_eq!(state.device_name.chars().count(), DEVICE_NAME_CAP);
    state.handle_key(press(KeyCode::Backspace));
    assert_eq!(state.device_name.chars().count(), DEVICE_NAME_CAP - 1);
}

#[test]
fn harness_toggle_and_navigation() {
    let mut state = fixture();
    state.step = Step::Harnesses;
    // toggle off the pre-selected claude-code
    state.handle_key(press(KeyCode::Char(' ')));
    assert_eq!(state.selected[0], false);
    // move down and toggle on codex
    state.handle_key(press(KeyCode::Down));
    state.handle_key(press(KeyCode::Char(' ')));
    assert_eq!(state.selected, vec![false, true]);
}

#[test]
fn relay_choice_maps_from_cursor() {
    let mut state = fixture();
    assert_eq!(state.relay_choice(), RelayChoice::Existing);
    state.relay_cursor = 1;
    assert_eq!(state.relay_choice(), RelayChoice::Assist);
    state.relay_cursor = 2;
    assert_eq!(state.relay_choice(), RelayChoice::Manual);
}

#[test]
fn existing_relay_branch_enters_url_step() {
    let mut state = fixture();
    state.step = Step::Relay;
    state.relay_cursor = 0; // Existing
    state.handle_key(press(KeyCode::Enter));
    assert_eq!(state.step, Step::RelayUrl);
}

#[test]
fn existing_url_enter_triggers_probe() {
    let mut state = fixture();
    state.step = Step::RelayUrl;
    state.relay_cursor = 0; // Existing
    state.relay_url = "wss://relay.example".into();
    assert!(matches!(
        state.handle_key(press(KeyCode::Enter)),
        Action::ProbeRelay(_)
    ));
}

#[test]
fn manual_flow_prefills_url_and_reaches_commit() {
    let mut state = fixture();
    state.step = Step::Relay;
    state.relay_cursor = 2; // Manual
    state.handle_key(press(KeyCode::Enter)); // → RelayUrl, prefilled
    assert_eq!(state.step, Step::RelayUrl);
    assert_eq!(state.relay_url, SUGGESTED_RELAY);
    state.handle_key(press(KeyCode::Enter)); // Manual accepts URL → Review
    assert_eq!(state.step, Step::Review);
    assert!(matches!(
        state.handle_key(press(KeyCode::Enter)),
        Action::Commit
    ));
}

#[test]
fn assist_available_with_rpc_harness() {
    let state = fixture(); // claude-code + codex
    assert_eq!(state.assistable_harness(), Some("claude-code"));
}

#[test]
fn assist_blocked_without_rpc_harness() {
    let mut state = Onboarding::new(vec![harness("grok", "Grok Build", true)]).unwrap();
    state.step = Step::Relay;
    state.relay_cursor = 1; // Assist
    let action = state.handle_key(press(KeyCode::Enter));
    assert!(matches!(action, Action::None));
    assert_eq!(state.step, Step::Relay);
    assert!(matches!(state.relay_status, RelayStatus::Failed(_)));
}

#[test]
fn assist_flow_starts_deploy() {
    let mut state = fixture();
    state.step = Step::Relay;
    state.relay_cursor = 1; // Assist
    state.handle_key(press(KeyCode::Enter)); // → RelayUrl, prefilled
    assert_eq!(state.step, Step::RelayUrl);
    assert_eq!(state.relay_url, SUGGESTED_RELAY);
    let action = state.handle_key(press(KeyCode::Enter)); // → StartDeploy
    assert!(matches!(action, Action::StartDeploy(_)));
    assert_eq!(state.step, Step::Deploy);
}

#[test]
fn probe_usable_advances_to_review() {
    let mut state = fixture();
    state.step = Step::RelayUrl;
    state.on_probe(relay::Probe::Usable);
    assert_eq!(state.step, Step::Review);
    assert!(matches!(state.relay_status, RelayStatus::Usable));
}

#[test]
fn probe_failure_stays_on_url_step() {
    let mut state = fixture();
    state.step = Step::RelayUrl;
    state.on_probe(relay::Probe::Unreachable("boom".into()));
    assert_eq!(state.step, Step::RelayUrl);
    assert!(matches!(state.relay_status, RelayStatus::Failed(_)));
}
