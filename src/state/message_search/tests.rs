use super::*;
use crate::state::{Profile, RelayEvent, TestRelayDelivery};
use nostr::Keys;
use std::cell::RefCell;

struct Fixture {
    store: Store,
    profiles: Vec<Profile>,
    events: RefCell<Vec<RelayEvent>>,
    alice: String,
    bob: String,
    carol: String,
}

impl Fixture {
    fn new() -> Self {
        let alice = Keys::generate().public_key().to_hex();
        let bob = Keys::generate().public_key().to_hex();
        let carol = Keys::generate().public_key().to_hex();
        let profiles = vec![
            profile(&alice, "Pablo", "", ""),
            profile(&bob, "mist-codex", "codex", "tower"),
            profile(&carol, "ember-codex", "codex", "laptop"),
        ];
        let fixture = Self {
            store: Store::open_memory().unwrap(),
            profiles,
            events: RefCell::new(Vec::new()),
            alice,
            bob,
            carol,
        };
        fixture.refresh();
        fixture
    }

    fn message(&self, id: &str, channel: &str, author: &str, body: &str, at: u64) {
        self.events.borrow_mut().push(RelayEvent {
            id: id.into(),
            kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
            pubkey: author.into(),
            created_at: at,
            channel_h: channel.into(),
            d_tag: String::new(),
            content: body.into(),
            tags_json: "[]".into(),
        });
        self.refresh();
    }

    fn recipient(&self, id: &str, pubkey: &str) {
        let mut events = self.events.borrow_mut();
        let event = events.iter_mut().find(|event| event.id == id).unwrap();
        event.tags_json = serde_json::to_string(&vec![vec!["p", pubkey]]).unwrap();
        drop(events);
        self.refresh();
    }

    fn refresh(&self) {
        self.store.install_test_nmp_relay_delivery(
            TestRelayDelivery::new()
                .profiles(self.profiles.clone())
                .events(self.events.borrow().clone()),
        );
    }

    fn query(&self, mut query: MessageSearchQuery) -> MessageSearchPage {
        if query.limit == 0 {
            query.limit = MESSAGE_SEARCH_DEFAULT_LIMIT;
        }
        self.store.search_messages(&query).unwrap()
    }
}

fn profile(pubkey: &str, name: &str, agent_slug: &str, host: &str) -> Profile {
    Profile {
        pubkey: pubkey.into(),
        name: name.into(),
        slug: name.into(),
        agent_slug: agent_slug.into(),
        host: host.into(),
        is_backend: false,
        agents: Vec::new(),
        workspaces: Vec::new(),
        updated_at: 1,
    }
}

fn ids(page: &MessageSearchPage) -> Vec<&str> {
    page.hits
        .iter()
        .map(|hit| hit.message.message_id.as_str())
        .collect()
}

#[test]
fn filter_dimensions_apply_to_the_nmp_projection() {
    let f = Fixture::new();
    f.message("a", "root", &f.alice, "Land the Commit", 10);
    f.message("b", "child", &f.bob, "review the patch", 20);
    f.message("c", "other", &f.carol, "commit follow-up", 30);
    f.recipient("a", &f.bob);
    f.recipient("b", &f.carol);

    assert_eq!(
        ids(&f.query(MessageSearchQuery {
            channels: vec!["root".into(), "child".into()],
            ..Default::default()
        })),
        ["b", "a"]
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
            since: Some(5),
            until: Some(35),
            ..Default::default()
        })),
        ["c", "a"]
    );
}

#[test]
fn ordering_limit_and_cursor_are_stable_for_equal_timestamps() {
    let f = Fixture::new();
    for id in ["a", "b", "c", "d"] {
        f.message(id, "root", &f.alice, id, 10);
    }
    let first = f.query(MessageSearchQuery {
        limit: 2,
        ..Default::default()
    });
    assert_eq!(ids(&first), ["d", "c"]);

    let second = f.query(MessageSearchQuery {
        limit: 2,
        before: first.next,
        ..Default::default()
    });
    assert_eq!(ids(&second), ["b", "a"]);
    assert!(second.next.is_none());
}

#[test]
fn identity_resolution_uses_nmp_profiles() {
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
    assert!(f
        .store
        .resolve_message_search_identity("codex")
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
}

#[test]
fn backend_traffic_is_excluded_before_paging() {
    let mut f = Fixture::new();
    let backend = Keys::generate().public_key().to_hex();
    let mut backend_profile = profile(&backend, "backend", "", "remote");
    backend_profile.is_backend = true;
    f.profiles.push(backend_profile);
    f.message("ordinary", "root", &f.alice, "one", 10);
    f.message("hidden", "root", &backend, "hidden", 20);
    f.refresh();

    assert_eq!(ids(&f.query(MessageSearchQuery::default())), ["ordinary"]);
}

#[test]
fn invalid_limits_are_rejected() {
    let f = Fixture::new();
    for limit in [0, MESSAGE_SEARCH_MAX_LIMIT + 1] {
        assert!(f
            .store
            .search_messages(&MessageSearchQuery {
                limit,
                ..Default::default()
            })
            .unwrap_err()
            .to_string()
            .contains("limit"));
    }
}
