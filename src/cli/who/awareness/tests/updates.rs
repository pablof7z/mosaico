use super::*;
use nostr_sdk::prelude::{Keys, ToBech32};

#[test]
fn update_renders_state_activity_and_omits_unchanged_sessions() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-aware", "awareness", "", "");
    chan(
        &store,
        "h-ciflake",
        "ci-flake",
        "Runner issue isolated",
        "h-aware",
    );
    // A sibling session room directly under the root -> a top-level "other channel".
    chan(&store, "session-a9f2", "session-a9f2", "", "h-aware");
    members(&store, "h-aware", &["pk-claude"]);
    members(&store, "h-ciflake", &["pk-a", "pk-b"]);

    status(
        &store,
        "pk-claude",
        "claude",
        "h-aware",
        "Found the stale routing scope after channel switch",
        true,
        960,
    );
    status(&store, "pk-a", "a", "h-ciflake", "fixing runner", true, 975);
    status(
        &store,
        "pk-other",
        "other",
        "session-a9f2",
        "other channel changed",
        true,
        980,
    );
    chat(
        &store,
        "chat-child",
        "h-aware",
        "claude",
        "The stale scope read is in turn_check.",
        970,
    );

    let block = render_awareness_update_since_check(
        &store,
        900,
        "h-aware",
        NOW,
        Some("pk-old"),
        LOCAL_HOST,
    )
    .unwrap();

    assert_has(&block, "[tenex-edge] Fabric updates since your last check");
    assert_has(
        &block,
        "- @claude - Found the stale routing scope after channel switch",
    );
    assert_has(&block, "- #ci-flake -- Runner issue isolated [2 members]");
    assert_has(&block, "- other channel changed [1 member]");
    assert_has(&block, "Activity in #awareness:");
    assert_has(
        &block,
        "[@claude, just now] The stale scope read is in turn_check.",
    );
    assert_lacks(&block, "h-aware");
    assert_lacks(&block, "h-ciflake");
    assert_lacks(&block, "session-a9f2");
    assert_lacks(&block, "joined");
    assert_lacks(&block, "left");
}

#[test]
fn update_activity_excludes_viewers_own_chat() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-aware", "awareness", "", "");
    chat(
        &store,
        "chat-self",
        "h-aware",
        "codex",
        "did you validate it with real usage?",
        960,
    );
    chat(
        &store,
        "chat-other",
        "h-aware",
        "claude",
        "I validated it through the real hook.",
        970,
    );

    let block = render_awareness_update_since_check(
        &store,
        900,
        "h-aware",
        NOW,
        Some("pk-codex"),
        LOCAL_HOST,
    )
    .expect("other activity should still render");

    assert_has(&block, "Activity in #awareness:");
    assert_has(
        &block,
        "[@claude, just now] I validated it through the real hook.",
    );
    assert_lacks(&block, "did you validate it with real usage?");
}

#[test]
fn update_activity_rewrites_mention_entities_to_slugs() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-aware", "awareness", "", "");
    let mentioned = Keys::generate().public_key();
    store
        .upsert_profile(&mentioned.to_hex(), "Ada", "ada", "claude-code", false, 1)
        .unwrap();
    chat(
        &store,
        "chat-mention",
        "h-aware",
        "claude",
        &format!(
            "hey nostr:{} check this out",
            mentioned.to_bech32().unwrap()
        ),
        970,
    );

    let block = render_awareness_update_since_check(&store, 900, "h-aware", NOW, None, LOCAL_HOST)
        .expect("activity should render");

    assert_has(&block, "@ada");
    assert_lacks(&block, "nostr:");
}

#[test]
fn other_active_channels_use_status_titles_without_repeating_old_activity() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-aware", "awareness", "", "");
    chan(&store, "session-a9f2", "session-a9f2", "", "h-aware");
    status(
        &store,
        "pk-codex",
        "codex",
        "session-a9f2",
        "Investigating duplicate session rooms",
        true,
        980,
    );

    let block =
        render_awareness_update_since_check(&store, 900, "h-aware", NOW, None, LOCAL_HOST).unwrap();
    assert_has(&block, "- Investigating duplicate session rooms [1 member]");
    assert_lacks(&block, "session-a9f2");

    let later = render_awareness_update_since_check(&store, 990, "h-aware", NOW, None, LOCAL_HOST);
    assert!(
        later.is_none(),
        "old active channel state must not repeat without new activity; got: {later:?}"
    );
}

#[test]
fn other_active_channels_are_scoped_to_this_project() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-nmp", "nmp", "", "");
    chan(&store, "h-epic123", "epic123", "", "h-nmp");
    status(
        &store,
        "pk-a",
        "a",
        "h-epic123",
        "planning the epic",
        true,
        980,
    );
    chan(&store, "h-other", "other-proj", "", "");
    status(&store, "pk-b", "b", "h-other", "unrelated work", true, 980);
    chan(&store, "h-orphan", "orphan", "", "ghost-parent");
    status(&store, "pk-c", "c", "h-orphan", "ghost work", true, 980);

    let block = render_fabric_view(&store, "h-nmp", NOW, "", "", LOCAL_HOST);
    assert_has(&block, "- #epic123 [1 member]");
    assert_lacks(&block, "other-proj");
    assert_lacks(&block, "unrelated work");
    assert_lacks(&block, "orphan");
    assert_lacks(&block, "ghost work");
}

#[test]
fn other_channels_exclude_the_branch_the_viewer_is_in() {
    let store = Store::open_memory().unwrap();
    let now = 20_000;
    let recent = now - 3 * 60 * 60;
    let stale = now - 4 * 60 * 60 - 1;
    chan(&store, "h-nmp", "nmp", "", "");
    chan(&store, "h-epic123", "epic123", "", "h-nmp");
    chan(&store, "h-epic999", "epic999", "", "h-nmp");
    chan(&store, "h-old", "old", "", "h-nmp");
    chan(&store, "h-e999deep", "e999-deep", "", "h-epic999");
    members(&store, "h-epic999", &["pk-a"]);
    status(
        &store,
        "pk-a",
        "a",
        "h-epic999",
        "sibling work",
        true,
        recent,
    );
    status(&store, "pk-b", "b", "h-e999deep", "deep work", true, recent);
    status(&store, "pk-old", "old", "h-old", "old work", true, stale);

    let block = render_fabric_view(&store, "h-epic123", now, "", "", LOCAL_HOST);
    assert_has(&block, "Other active channels, last 4h:");
    assert_has(&block, "- #epic999 [1 member]");
    assert_lacks(&block, "#epic123 [");
    assert_lacks(&block, "#old");
    assert_lacks(&block, "deep work");
    assert_lacks(&block, "e999-deep");
}

#[test]
fn appeared_member_without_work_text_is_not_announced() {
    let store = Store::open_memory().unwrap();
    chan(&store, "child", "Channel awareness hook", "", "");
    status(&store, "pk-empty", "empty", "child", "", false, 980);

    let block = render_awareness_update_since_check(&store, 900, "child", NOW, None, LOCAL_HOST);
    assert!(
        block.is_none(),
        "appearance without title/activity should not become noise; got: {block:?}"
    );
}
