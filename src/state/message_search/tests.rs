use super::*;
use crate::state::RecordMessage;
use nostr::Keys;

struct Fixture {
    store: Store,
    alice: String,
    bob: String,
    carol: String,
}

impl Fixture {
    fn new() -> Self {
        let store = Store::open_memory().unwrap();
        let alice = Keys::generate().public_key().to_hex();
        let bob = Keys::generate().public_key().to_hex();
        let carol = Keys::generate().public_key().to_hex();
        store
            .upsert_profile(&alice, "Pablo", "Pablo", "", false, 1)
            .unwrap();
        store
            .upsert_profile_with_agent_slug(
                &bob,
                "mist-codex",
                "mist-codex",
                "codex",
                "tower",
                false,
                1,
            )
            .unwrap();
        store
            .upsert_profile_with_agent_slug(
                &carol,
                "ember-codex",
                "ember-codex",
                "codex",
                "laptop",
                false,
                1,
            )
            .unwrap();
        Self {
            store,
            alice,
            bob,
            carol,
        }
    }

    fn message(&self, id: &str, channel: &str, author: &str, body: &str, at: u64) {
        self.store
            .record_message(&RecordMessage {
                message_id: id.into(),
                thread_id: channel.into(),
                channel_h: channel.into(),
                author_pubkey: author.into(),
                body: body.into(),
                created_at: at,
                sync_state: "accepted".into(),
                native_event_id: Some(id.into()),
                error: None,
            })
            .unwrap();
    }

    fn query(&self, mut query: MessageSearchQuery) -> MessageSearchPage {
        if query.limit == 0 {
            query.limit = MESSAGE_SEARCH_DEFAULT_LIMIT;
        }
        self.store.search_messages(&query).unwrap()
    }
}

fn ids(page: &MessageSearchPage) -> Vec<&str> {
    page.hits
        .iter()
        .map(|hit| hit.message.message_id.as_str())
        .collect()
}

#[test]
fn every_filter_dimension_is_applied_to_the_local_read_model() {
    let f = Fixture::new();
    f.message("a", "root", &f.alice, "Land the Commit", 10);
    f.message("b", "child", &f.bob, "review the patch", 20);
    f.message("c", "other", &f.carol, "commit follow-up", 30);
    f.store.add_message_recipient("a", &f.bob, None).unwrap();
    f.store.add_message_recipient("b", &f.carol, None).unwrap();

    assert_eq!(
        ids(&f.query(MessageSearchQuery {
            channels: vec!["root".into(), "child".into()],
            ..Default::default()
        })),
        ["b", "a"]
    );
    assert_eq!(
        ids(&f.query(MessageSearchQuery {
            from_pubkeys: vec![f.alice.clone()],
            ..Default::default()
        })),
        ["a"]
    );
    assert_eq!(
        ids(&f.query(MessageSearchQuery {
            to_pubkeys: vec![f.carol.clone()],
            ..Default::default()
        })),
        ["b"]
    );
    assert_eq!(
        ids(&f.query(MessageSearchQuery {
            contains: vec!["COMMIT".into()],
            ..Default::default()
        })),
        ["c", "a"]
    );
    assert_eq!(
        ids(&f.query(MessageSearchQuery {
            since: Some(15),
            until: Some(25),
            ..Default::default()
        })),
        ["b"]
    );
}

#[test]
fn repeated_values_or_within_dimensions_and_dimensions_and_together() {
    let f = Fixture::new();
    f.message("a", "root", &f.alice, "commit docs", 10);
    f.message("b", "root", &f.bob, "review docs", 20);
    f.message("c", "root", &f.carol, "commit code", 30);
    f.store.add_message_recipient("a", &f.carol, None).unwrap();
    f.store.add_message_recipient("c", &f.bob, None).unwrap();

    let page = f.query(MessageSearchQuery {
        from_pubkeys: vec![f.alice.clone(), f.carol.clone()],
        to_pubkeys: vec![f.bob.clone(), f.carol.clone()],
        contains: vec!["docs".into(), "code".into()],
        ..Default::default()
    });
    assert_eq!(ids(&page), ["c", "a"]);
}

#[test]
fn ordering_limit_and_cursor_are_stable_across_equal_timestamps() {
    let f = Fixture::new();
    for id in ["a", "b", "c", "d"] {
        f.message(id, "root", &f.alice, id, 10);
    }
    let first = f.query(MessageSearchQuery {
        limit: 2,
        ..Default::default()
    });
    assert_eq!(ids(&first), ["d", "c"]);
    assert_eq!(
        first.next,
        Some(MessageSearchPosition {
            created_at: 10,
            message_id: "c".into()
        })
    );

    let second = f.query(MessageSearchQuery {
        limit: 2,
        before: first.next,
        ..Default::default()
    });
    assert_eq!(ids(&second), ["b", "a"]);
    assert!(second.next.is_none());
}

#[test]
fn contains_is_a_literal_unicode_case_insensitive_substring() {
    let f = Fixture::new();
    f.message("literal", "root", &f.alice, "Deploy 100%_Ready CAFÉ", 1);

    for needle in ["%_", "café"] {
        assert_eq!(
            ids(&f.query(MessageSearchQuery {
                contains: vec![needle.into()],
                ..Default::default()
            })),
            ["literal"]
        );
    }
    assert!(f
        .query(MessageSearchQuery {
            contains: vec!["100x".into()],
            ..Default::default()
        })
        .hits
        .is_empty());
}

#[test]
fn public_identity_resolution_is_global_and_detects_ambiguity() {
    let f = Fixture::new();
    assert_eq!(
        f.store.resolve_message_search_identity("@Pablo").unwrap(),
        f.alice
    );
    assert_eq!(
        f.store
            .resolve_message_search_identity("codex@tower")
            .unwrap(),
        f.bob
    );
    let npub = crate::idref::npub(&f.carol).unwrap();
    assert_eq!(
        f.store.resolve_message_search_identity(&npub).unwrap(),
        f.carol
    );
    assert!(f
        .store
        .resolve_message_search_identity("codex")
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
}

#[test]
fn invalid_limits_are_rejected_at_the_store_boundary() {
    let f = Fixture::new();
    for limit in [0, MESSAGE_SEARCH_MAX_LIMIT + 1] {
        let error = f
            .store
            .search_messages(&MessageSearchQuery {
                limit,
                ..Default::default()
            })
            .unwrap_err();
        assert!(error.to_string().contains("limit"));
    }
}

#[test]
fn backend_management_rows_are_excluded_before_limit_and_cursor_selection() {
    let f = Fixture::new();
    let management = Keys::generate().public_key().to_hex();
    let remote_backend = Keys::generate().public_key().to_hex();
    f.store
        .upsert_profile(
            &remote_backend,
            "remote backend",
            "remote",
            "remote",
            true,
            1,
        )
        .unwrap();
    f.message("ordinary-1", "root", &f.alice, "one", 10);
    f.message("to-backend", "root", &f.alice, "hidden", 20);
    f.message("from-backend", "root", &remote_backend, "hidden", 30);
    f.message("from-management", "root", &management, "hidden", 40);
    f.message("ordinary-2", "root", &f.bob, "two", 50);
    f.store
        .add_message_recipient("to-backend", &remote_backend, None)
        .unwrap();

    let first = f.query(MessageSearchQuery {
        limit: 1,
        backend_pubkey: Some(management.clone()),
        ..Default::default()
    });
    assert_eq!(ids(&first), ["ordinary-2"]);
    assert!(first.next.is_some());
    let second = f.query(MessageSearchQuery {
        limit: 1,
        before: first.next,
        backend_pubkey: Some(management),
        ..Default::default()
    });
    assert_eq!(ids(&second), ["ordinary-1"]);
    assert!(second.next.is_none());
}
