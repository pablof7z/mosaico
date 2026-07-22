use super::*;

fn participant(id: &str, live: bool, busy: bool) -> ParticipantSnapshot {
    ParticipantSnapshot {
        pubkey: id.into(),
        label: format!("{id}-codex"),
        host: "host".into(),
        runtime_generation: Some(1),
        live,
        busy,
    }
}

fn message(id: usize, author: &str, at: u64) -> ConversationMessage {
    ConversationMessage {
        message_id: format!("m{id}"),
        author_pubkey: author.into(),
        created_at: at,
        substantive: true,
    }
}

fn evidence(busy: &[&str]) -> ConversationEvidence {
    let participants = [
        participant("a1", true, busy.contains(&"a1")),
        participant("a2", true, busy.contains(&"a2")),
        participant("a3", true, busy.contains(&"a3")),
    ];
    let messages = [
        message(1, "a1", 100),
        message(2, "a2", 105),
        message(3, "a3", 110),
        message(4, "a1", 115),
        message(5, "a2", 120),
        message(6, "a3", 125),
    ];
    detect_root_conversation("root", true, &messages, &[], &participants).unwrap()
}

#[test]
fn conversation_cohort_is_independent_from_busy_subset() {
    let found = evidence(&["a1", "a2"]);
    assert_eq!(found.cohort_pubkeys(), vec!["a1", "a2", "a3"]);
    assert_eq!(found.busy_pubkeys, vec!["a1", "a2"]);
}

#[test]
fn stopped_speakers_and_silent_members_are_not_in_the_cohort() {
    let participants = [
        participant("a1", true, true),
        participant("a2", true, false),
        participant("stopped", false, false),
        participant("silent", true, true),
    ];
    let messages = [
        message(1, "a1", 100),
        message(2, "a2", 105),
        message(3, "stopped", 110),
        message(4, "a1", 115),
        message(5, "a2", 120),
        message(6, "stopped", 125),
        message(7, "a1", 130),
        message(8, "a2", 135),
    ];
    let found = detect_root_conversation("root", true, &messages, &[], &participants).unwrap();
    assert_eq!(found.cohort_pubkeys(), vec!["a1", "a2"]);
}

#[test]
fn child_channels_do_not_nudge() {
    let found = evidence(&["a1", "a2"]);
    let messages = [message(1, "a1", 100), message(2, "a2", 130)];
    assert!(detect_root_conversation("child", false, &messages, &[], &found.cohort).is_none());
}

#[test]
fn reactions_count_as_engagement_but_not_as_conversation_participation() {
    let participants = [
        participant("a1", true, true),
        participant("a2", true, false),
        participant("a3", true, false),
        participant("a4", true, false),
        participant("silent", true, false),
    ];
    let messages = [
        message(1, "a1", 100),
        message(2, "a2", 110),
        message(3, "a1", 120),
        message(4, "a2", 130),
    ];
    assert!(detect_root_conversation("root", true, &messages, &[], &participants).is_some());

    let reactions = [
        ConversationReaction {
            reactor_pubkey: "a3".into(),
            target_message_id: "m2".into(),
        },
        ConversationReaction {
            reactor_pubkey: "a4".into(),
            target_message_id: "m3".into(),
        },
    ];
    assert!(detect_root_conversation("root", true, &messages, &reactions, &participants).is_none());

    let six_messages = [
        message(1, "a1", 100),
        message(2, "a2", 105),
        message(3, "a1", 110),
        message(4, "a2", 115),
        message(5, "a1", 120),
        message(6, "a2", 125),
    ];
    let found =
        detect_root_conversation("root", true, &six_messages, &reactions, &participants).unwrap();
    assert_eq!(found.cohort_pubkeys(), ["a1", "a2"]);
    assert_eq!(found.engaged_count, 4);
}

#[test]
fn losing_and_winning_checks_both_arm_the_ten_second_cooldown() {
    let mut state = ChannelNudgeState::default();
    let current = evidence(&["a1", "a2"]);
    assert!(state
        .consider("a1", current.clone(), 100, u64::MAX)
        .is_none());
    assert!(state.consider("a1", current.clone(), 109, 0).is_none());
    assert!(state.consider("a1", current, 110, 0).is_some());
}

#[test]
fn each_busy_agent_wins_less_often_as_the_busy_subset_grows() {
    let two_busy_cutoff = u64::MAX / 4;
    let five_busy_cutoff = u64::MAX / 25;

    assert!(lottery_wins(u64::MAX, 1));
    assert!(lottery_wins(two_busy_cutoff, 2));
    assert!(!lottery_wins(two_busy_cutoff.saturating_add(1), 2));
    assert!(lottery_wins(five_busy_cutoff, 5));
    assert!(!lottery_wins(five_busy_cutoff.saturating_add(1), 5));
    assert!(five_busy_cutoff < two_busy_cutoff);
}

#[test]
fn idle_conversation_participant_never_enters_the_lottery() {
    let mut state = ChannelNudgeState::default();
    assert!(state
        .consider("a3", evidence(&["a1", "a2"]), 100, 0)
        .is_none());
}

#[test]
fn active_offer_suppresses_further_lottery_checks_until_expiry() {
    let mut state = ChannelNudgeState::default();
    let current = evidence(&["a1"]);
    assert!(state.consider("a1", current.clone(), 100, 0).is_some());
    assert!(state.consider("a1", current.clone(), 110, 0).is_none());
    assert!(state.current_offer("a1", 220).is_some());
    assert!(state.current_offer("a1", 221).is_none());
    assert!(state.consider("a1", current, 221, 0).is_some());
}

#[test]
fn acknowledgement_and_parent_pointer_are_not_substantive() {
    assert!(!is_substantive_message("ok"));
    assert!(!is_substantive_message("Moving this to #reviews"));
    assert!(is_substantive_message("I verified the integration tests"));
}

#[test]
fn rendered_offer_discloses_the_exact_move_effects() {
    let text = render_nudge(&MoveOffer {
        evidence: evidence(&["a1", "a2"]),
        offered_at: 100,
        expires_at: 220,
    });
    assert!(text.contains("mosaico --yes-lets-move <new-channel-name> <about>"));
    assert!(text.contains("all 3 participating agents plus human users and admins"));
    assert!(text.len() < 400, "nudge must stay compact: {text}");
}
