use super::*;
use crate::state::RecordMessage;
use nostr::Keys;

struct Fixture {
    state: Arc<DaemonState>,
    pablo: String,
    agent: String,
}

impl Fixture {
    async fn new() -> Self {
        let state = DaemonState::new_for_test().await;
        let pablo = Keys::generate().public_key().to_hex();
        let agent = Keys::generate().public_key().to_hex();
        state.with_store(|store| {
            store.upsert_channel("root", "general", "", "", 1).unwrap();
            store
                .upsert_channel("child", "research", "", "root", 2)
                .unwrap();
            store
                .upsert_channel("deep", "notes", "", "child", 3)
                .unwrap();
            store.upsert_channel("other", "general", "", "", 4).unwrap();
            store
                .upsert_profile(&pablo, "Pablo", "Pablo", "", false, 1)
                .unwrap();
            store
                .upsert_profile_with_agent_slug(
                    &agent,
                    "mist-codex",
                    "mist-codex",
                    "codex",
                    "remote",
                    false,
                    1,
                )
                .unwrap();
        });
        Self {
            state,
            pablo,
            agent,
        }
    }

    fn message(&self, id: &str, channel: &str, author: &str, body: &str, at: u64) {
        self.state.with_store(|store| {
            store
                .record_message(&RecordMessage {
                    message_id: id.into(),
                    thread_id: channel.into(),
                    channel_h: channel.into(),
                    author_pubkey: author.into(),
                    body: body.into(),
                    created_at: at,
                    direction: "inbound".into(),
                    sync_state: "accepted".into(),
                    native_event_id: Some(id.into()),
                    error: None,
                })
                .unwrap();
        });
    }

    fn search(&self, params: serde_json::Value) -> serde_json::Value {
        rpc_channel_search(&self.state, &params).unwrap()
    }
}

fn event_ids(result: &serde_json::Value) -> Vec<&str> {
    result["channels"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|channel| channel["messages"].as_array().unwrap())
        .map(|message| message["event_id"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn subtree_scope_is_recursive_and_results_group_after_global_selection() {
    let f = Fixture::new().await;
    f.message("root-new", "root", &f.pablo, "root new", 50);
    f.message("child", "child", &f.agent, "child", 40);
    f.message("deep", "deep", &f.pablo, "deep", 30);
    f.message("other", "other", &f.agent, "other", 20);
    f.message("root-old", "root", &f.pablo, "root old", 10);

    let scoped = f.search(serde_json::json!({"channels":["/root/research"]}));
    assert_eq!(event_ids(&scoped), ["child", "deep"]);
    assert_eq!(scoped["channels"][0]["ref"], "/root/research");
    assert_eq!(scoped["channels"][1]["ref"], "/root/research/notes");

    let all = f.search(serde_json::json!({}));
    let slash = f.search(serde_json::json!({"channels":["/"]}));
    assert_eq!(all, slash);
    assert_eq!(
        event_ids(&all),
        ["root-new", "root-old", "child", "deep", "other"]
    );
    assert_eq!(all["channels"][0]["ref"], "/root");
    assert_eq!(all["channels"][1]["ref"], "/root/research");
    assert_eq!(all["channels"][2]["ref"], "/root/research/notes");
    assert_eq!(all["channels"][3]["ref"], "/other");
}

#[tokio::test]
async fn identity_recipient_text_and_time_filters_combine_without_membership_checks() {
    let f = Fixture::new().await;
    f.message("match", "deep", &f.pablo, "LAND commit now", 20);
    f.message("wrong-author", "deep", &f.agent, "land commit now", 21);
    f.message("wrong-body", "deep", &f.pablo, "land patch now", 22);
    f.state
        .with_store(|store| store.add_message_recipient("match", &f.agent, None))
        .unwrap();
    f.state
        .with_store(|store| {
            store.set_message_attachment_dir(
                "match",
                std::path::Path::new("/tmp/mosaico-files/match0"),
            )
        })
        .unwrap();
    f.state
        .with_store(|store| store.add_message_recipient("wrong-body", &f.agent, None))
        .unwrap();

    let result = f.search(serde_json::json!({
        "from":["@Pablo"],
        "to":["mist-codex"],
        "contains":["COMMIT"],
        "channels":["/root"],
        "since":20,
        "until":22
    }));
    assert_eq!(event_ids(&result), ["match"]);
    let message = &result["channels"][0]["messages"][0];
    assert_eq!(message["from"], "Pablo");
    assert_eq!(message["recipients"], serde_json::json!(["mist-codex"]));
    assert_eq!(message["attachment_dir"], "/tmp/mosaico-files/match0");
    assert!(message.get("from_pubkey").is_none());
}

#[tokio::test]
async fn cursor_continues_globally_and_is_bound_to_the_normalized_query() {
    let f = Fixture::new().await;
    for (id, at) in [("one", 30), ("two", 20), ("three", 10)] {
        f.message(id, "root", &f.pablo, "commit", at);
    }
    let first = f.search(serde_json::json!({"contains":["COMMIT"],"limit":1}));
    assert_eq!(event_ids(&first), ["one"]);
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = f.search(serde_json::json!({"cursor":cursor}));
    assert_eq!(event_ids(&second), ["two"]);

    let error = rpc_channel_search(
        &f.state,
        &serde_json::json!({
            "contains":["different"],
            "cursor":cursor
        }),
    )
    .unwrap_err();
    assert!(error.to_string().contains("cannot be combined"));
}

#[tokio::test]
async fn exact_request_contract_rejects_workspace_and_invalid_bounds() {
    let f = Fixture::new().await;
    let workspace =
        rpc_channel_search(&f.state, &serde_json::json!({"workspace":"/root"})).unwrap_err();
    assert!(workspace
        .to_string()
        .contains("--workspace is not supported"));

    let bounds =
        rpc_channel_search(&f.state, &serde_json::json!({"since":20,"until":10})).unwrap_err();
    assert!(bounds.to_string().contains("--since"));

    let caller_context = f.search(serde_json::json!({
        "session":"session",
        "pty_session":"pty",
        "harness":"codex",
        "watch_pid":123,
        "agent":"codex",
        "cwd":"/tmp"
    }));
    assert!(caller_context["channels"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn default_limit_is_twenty_and_next_cursor_is_null_at_the_end() {
    let f = Fixture::new().await;
    for at in 1..=21 {
        f.message(&format!("{at:02}"), "root", &f.pablo, "body", at);
    }
    let first = f.search(serde_json::json!({}));
    assert_eq!(event_ids(&first).len(), 20);
    let second = f.search(serde_json::json!({
        "cursor":first["next_cursor"].as_str().unwrap()
    }));
    assert_eq!(event_ids(&second), ["01"]);
    assert!(second["next_cursor"].is_null());
}
