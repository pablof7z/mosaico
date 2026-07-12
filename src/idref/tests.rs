use super::*;

#[test]
fn agent_label_preserves_backend_label() {
    assert_eq!(agent_label("codex", "myBackend"), "codex@myBackend");
    assert_eq!(agent_label("claude", "laptop"), "claude@laptop");
}

#[test]
fn session_handle_is_session_code_dash_agent() {
    assert_eq!(
        session_handle("codex", "willow-echo-042"),
        "willow-echo-042-codex"
    );
    assert_eq!(
        session_handle("codex", "willow-echo-042-codex"),
        "willow-echo-042-codex"
    );
    assert_eq!(session_handle("", "willow-echo-042"), "willow-echo-042");
    assert_eq!(
        session_handle("codex", "profiled-member"),
        "profiled-member-codex"
    );
}

#[test]
fn profile_name_combines_session_code_and_agent_slug() {
    assert_eq!(
        session_handle_from_profile_name("willow-echo-042", "codex"),
        "willow-echo-042-codex"
    );
    assert_eq!(
        session_handle_from_profile_name("profiled-member", ""),
        "profiled-member"
    );
}

#[test]
fn agent_ref_from_is_bare_local_and_qualified_remote() {
    assert_eq!(agent_ref_from("developer", "laptop", "laptop"), "developer");
    assert_eq!(agent_ref_from("developer", "", "laptop"), "developer");
    assert_eq!(
        agent_ref_from("developer", "myBackend", "laptop"),
        "developer@myBackend"
    );
}

#[test]
fn session_label_preserves_session_handle() {
    assert_eq!(
        session_label("te-abc-0", "willow-echo-042-codex", "laptop"),
        "willow-echo-042-codex"
    );
    assert_eq!(session_label("te-abc-0", "codex", "laptop"), "codex");
    assert_eq!(session_label("te-abc-0", "", "laptop"), "te-abc-0");
}

#[test]
fn event_short_id_truncates_to_eight() {
    assert_eq!(event_short_id("0123456789abcdef"), "01234567");
    assert_eq!(event_short_id("abc"), "abc");
}

#[test]
fn npub_and_hex_normalize_to_the_same_permanent_identity() {
    let keys = nostr_sdk::prelude::Keys::generate();
    let hex = keys.public_key().to_hex();
    let bech32 = npub(&hex).expect("npub");
    assert_eq!(normalize_pubkey(&hex).as_deref(), Some(hex.as_str()));
    assert_eq!(normalize_pubkey(&bech32).as_deref(), Some(hex.as_str()));
    assert!(normalize_pubkey("raw-session-id").is_none());
}

#[test]
fn permanent_pubkey_profile_name_is_not_turned_into_a_handle() {
    let keys = nostr_sdk::prelude::Keys::generate();
    let hex = keys.public_key().to_hex();
    let npub = npub(&hex).unwrap();
    assert_eq!(session_handle_from_profile_name(&npub, "codex"), npub);
    assert_eq!(session_handle_from_profile_name(&hex, "codex"), hex);
}

#[test]
fn durable_agent_profile_keeps_bare_slug() {
    assert_eq!(
        session_handle_from_profile_name("chief-of-staff", "chief-of-staff"),
        "chief-of-staff"
    );
}

#[test]
fn parse_at_is_backend_label_not_channel() {
    match parse_ref("codex@myBackend") {
        Ref::Agent { slug, host } => {
            assert_eq!(slug, "codex");
            assert_eq!(host, "myBackend");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn agent_backend_ref_preserves_backend_label() {
    let r = parse_agent_backend_ref("claude@myBackend").unwrap();
    assert_eq!(r.slug, "claude");
    assert_eq!(r.backend.as_deref(), Some("myBackend"));

    let local = parse_agent_backend_ref("codex").unwrap();
    assert_eq!(local.slug, "codex");
    assert_eq!(local.backend, None);

    assert!(parse_agent_backend_ref("claude@").is_none());
    assert!(parse_agent_backend_ref("@laptop").is_none());
}

#[test]
fn parse_pubkey_and_token() {
    let hex = "a".repeat(64);
    assert!(matches!(parse_ref(&hex), Ref::Pubkey(_)));
    assert!(matches!(parse_ref("npub1abcdef"), Ref::Pubkey(_)));
    assert!(matches!(parse_ref("haiku1"), Ref::Token(_)));
    assert!(matches!(parse_ref("codex"), Ref::Token(_)));
}

#[test]
fn extract_inline_mentions() {
    assert_eq!(
        extract_mentions("hey @sable-grove-179-haiku and @codex, look"),
        vec!["sable-grove-179-haiku".to_string(), "codex".to_string()]
    );
    assert_eq!(
        extract_mentions("ping @claude@tower please"),
        vec!["claude@tower".to_string()]
    );
    assert_eq!(
        extract_mentions("hey @willow-echo-042-codex how are you?"),
        vec!["willow-echo-042-codex".to_string()]
    );
    assert_eq!(extract_mentions("ping @codex."), vec!["codex".to_string()]);
    assert!(extract_mentions("email dev@example.com please").is_empty());
    assert_eq!(
        extract_mentions("@haiku1 @haiku1"),
        vec!["haiku1".to_string()]
    );
}
