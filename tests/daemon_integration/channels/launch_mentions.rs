use super::*;
use nostr_sdk::prelude::{Client as NostrClient, ClientOptions, Filter, Keys, Kind};
use nostr_sdk::NostrSigner;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tenex_edge::daemon::client::Client as DaemonClient;
use tenex_edge::domain::{AgentRef, ChatMessage, DomainEvent};
use tenex_edge::fabric::nip29::wire::Nip29WireCodec;
use tenex_edge::identity;
use tenex_edge::state::{Session, Store};

fn add_project_mapping(home: &Home, project: &str, path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    let map_path = home.dir.path().join("projects.json");
    let mut map = std::fs::read_to_string(&map_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&s).ok())
        .unwrap_or_default();
    map.insert(
        project.to_string(),
        serde_json::Value::String(path.to_string_lossy().to_string()),
    );
    std::fs::write(&map_path, serde_json::to_string(&map).unwrap()).unwrap();
}

fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn harness_command(native_session: &str, cwd: &Path, injected_log: &Path) -> Vec<String> {
    let cwd_json = serde_json::to_string(&cwd.to_string_lossy()).unwrap();
    let hook_log = injected_log.with_extension("hook.log");
    let script = format!(
        "printf '{{\"session_id\":\"{}\",\"cwd\":{},\"pid\":%s}}\\n' \"$$\" \
         | \"$TENEX_EDGE_BIN\" harness hook opencode --type session-start >{} 2>&1; \
         while IFS= read -r line; do printf '%s\\n' \"$line\" >> {}; done",
        native_session,
        cwd_json,
        sh_quote(&hook_log),
        sh_quote(injected_log)
    );
    vec!["sh".to_string(), "-lc".to_string(), script]
}

fn kill_pane(pane_id: &str) {
    let _ = Command::new("tmux")
        .args(["kill-pane", "-t", pane_id])
        .status();
}

fn find_alive_session(home: &Home, slug: &str, scope: &str) -> Option<Session> {
    Store::open(&home.store_path())
        .ok()?
        .list_alive_sessions()
        .ok()?
        .into_iter()
        .find(|rec| rec.agent_slug == slug && rec.channel_h == scope)
}

fn wait_for_alive_session(home: &Home, slug: &str, scope: &str) -> Session {
    let mut found = None;
    assert!(
        wait_until(Duration::from_secs(25), || {
            found = find_alive_session(home, slug, scope);
            found.is_some()
        }),
        "session {slug} in {scope} did not become alive"
    );
    found.unwrap()
}

fn wait_for_group_member(home: &Home, project: &str, pubkey: &str) {
    assert!(
        wait_until(Duration::from_secs(25), || Store::open(&home.store_path())
            .map(|s| {
                refresh_project_members(project);
                s.is_channel_member(project, pubkey).unwrap_or(false)
            })
            .unwrap_or(false)),
        "{pubkey} was not visible as a member of {project}"
    );
}

fn wait_for_injected_log(log: &Path, body: &str) {
    assert!(
        wait_until(Duration::from_secs(25), || std::fs::read_to_string(log)
            .map(|s| s.contains(body))
            .unwrap_or(false)),
        "tmux pane did not receive injected body {body:?}; log={}",
        log.display()
    );
}

async fn nostr_user_client(keys: Keys) -> NostrClient {
    let opts = ClientOptions::default().automatic_authentication(true);
    let client = NostrClient::builder().signer(keys).opts(opts).build();
    client
        .add_relay(shared_nip29_relay_url())
        .await
        .expect("add relay");
    client.connect().await;
    client.wait_for_connection(Duration::from_secs(8)).await;
    let _ = client
        .fetch_events(
            Filter::new().kind(Kind::from(0u16)).limit(1),
            Duration::from_secs(5),
        )
        .await;
    client
}

async fn publish_user_kind9(project: &str, body: &str, mentioned_pubkey: &str) -> String {
    let keys = Keys::parse(EXAMPLE_USER_NSEC).expect("operator keys");
    let client = nostr_user_client(keys.clone()).await;
    let chat = ChatMessage {
        from: AgentRef::new(keys.public_key().to_hex(), ""),
        project: project.to_string(),
        body: body.to_string(),
        mentioned_pubkey: Some(mentioned_pubkey.to_string()),
    };
    let builder = Nip29WireCodec
        .encode_event(&DomainEvent::ChatMessage(chat))
        .expect("encode kind:9");
    let unsigned = builder.build(keys.public_key());
    let signed = keys.sign_event(unsigned).await.expect("sign kind:9");
    let out = client.send_event(&signed).await.expect("publish kind:9");
    assert!(
        !out.success.is_empty(),
        "operator kind:9 was rejected: success={:?} failed={:?}",
        out.success,
        out.failed
    );
    signed.id.to_hex()
}

#[test]
fn operator_kind9_injects_into_running_launch_session() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let project = unique_session("kind9-launch");
    let work_dir = home.dir.path().join(&project);
    add_project_mapping(&home, &project, &work_dir);
    let log = home.dir.path().join("launch-injected.log");
    let native_session = unique_session("launch-native");
    let agent = "launch-kind9";

    let pane_id = rt().block_on(async {
        let mut c = DaemonClient::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "tmux_spawn",
                serde_json::json!({
                    "agent": agent,
                    "project": project,
                    "channel": project,
                    "cwd": work_dir,
                    "base_command": harness_command(&native_session, &work_dir, &log),
                }),
            )
            .await
            .expect("tmux_spawn");
        v["pane_id"].as_str().unwrap().to_string()
    });

    let rec = wait_for_alive_session(&home, agent, &project);
    wait_for_group_member(&home, &project, &rec.agent_pubkey);

    let body = format!("operator relay injection {}", unique_session("body"));
    rt().block_on(async {
        publish_user_kind9(&project, &body, &rec.agent_pubkey).await;
    });
    wait_for_injected_log(&log, &body);

    let store = Store::open(&home.store_path()).unwrap();
    let messages = chat_in_channel(&store, &project);
    assert!(
        messages
            .iter()
            .any(|m| m.content == body && m.pubkey == pubkey_of(EXAMPLE_USER_NSEC)),
        "operator kind:9 should be materialized as user-authored chat"
    );

    kill_pane(&pane_id);
    stop_daemon(&home);
}

#[test]
fn operator_kind9_to_offline_local_agent_spawns_and_injects() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let project = unique_session("kind9-spawn");
    let work_dir = home.dir.path().join(&project);
    add_project_mapping(&home, &project, &work_dir);

    let agent = "offline-kind9";
    let log = home.dir.path().join("offline-injected.log");
    let native_session = unique_session("offline-native");
    let (agent_id, _) = identity::add_local_agent(
        home.dir.path(),
        agent,
        Some(harness_command(&native_session, &work_dir, &log)),
        1,
    )
    .expect("add local agent");
    let agent_pubkey = agent_id.pubkey_hex();

    rt().block_on(async {
        let mut c = DaemonClient::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({
                "agent": "keeper",
                "session_id": unique_session("keeper"),
                "cwd": work_dir,
                "watch_pid": std::process::id(),
            }),
        )
        .await
        .expect("keeper session_start");
        let add = c
            .call(
                "project_add",
                serde_json::json!({"project": project, "pubkey": agent_pubkey}),
            )
            .await
            .expect("project_add offline agent");
        assert_eq!(add["pubkey"], agent_pubkey);
    });
    wait_for_group_member(&home, &project, &agent_pubkey);

    let body = format!("wake offline agent {}", unique_session("body"));
    rt().block_on(async {
        publish_user_kind9(&project, &body, &agent_pubkey).await;
    });

    let rec = wait_for_alive_session(&home, agent, &project);
    assert_eq!(rec.agent_pubkey, agent_pubkey);
    wait_for_injected_log(&log, &body);

    let store = Store::open(&home.store_path()).unwrap();
    let pane_id = tmux_pane_for_session(&store, &rec.session_id).expect("spawned tmux endpoint");
    kill_pane(&pane_id);
    stop_daemon(&home);
}
