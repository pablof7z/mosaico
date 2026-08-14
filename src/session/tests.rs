use super::*;

#[test]
fn pi_is_a_canonical_harness_and_default_target() {
    let pi = Harness::from_str("pi");
    assert_eq!(pi.as_str(), "pi");
    assert_eq!(pi.agent_slug(), "pi");
}

#[test]
fn no_group_override_mints_when_per_session_rooms_enabled() {
    assert_eq!(
        decide_session_room(None, "my-repo", true),
        RoomDecision::Mint {
            parent: "my-repo".into()
        }
    );
    assert_eq!(
        decide_session_room(Some(""), "my-repo", true),
        RoomDecision::Mint {
            parent: "my-repo".into()
        }
    );
}

#[test]
fn no_group_override_uses_root_when_per_session_rooms_disabled() {
    assert_eq!(
        decide_session_room(None, "my-repo", false),
        RoomDecision::UseExisting {
            group: "my-repo".into()
        }
    );
    assert_eq!(
        decide_session_room(Some(""), "my-repo", false),
        RoomDecision::UseExisting {
            group: "my-repo".into()
        }
    );
}

#[test]
fn group_override_uses_existing_regardless_of_flag() {
    for flag in [true, false] {
        assert_eq!(
            decide_session_room(Some("issue-514"), "my-repo", flag),
            RoomDecision::UseExisting {
                group: "issue-514".into()
            }
        );
    }
}
