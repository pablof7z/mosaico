use super::*;
use mosaico::daemon::client::Client as DaemonClient;
use mosaico::domain::{AgentRef, ChatMessage, DomainEvent};
use mosaico::fabric::nip29::wire::Nip29WireCodec;
use mosaico::identity;
use mosaico::state::{Session, Store};
use nostr_sdk::prelude::{Client as NostrClient, ClientOptions, Filter, Keys, Kind};
use nostr_sdk::NostrSigner;
use std::path::Path;
use std::time::Duration;

fn add_workspace_mapping(home: &Home, channel: &str, path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    let map_path = home.dir.path().join("workspaces.json");
    let mut map = std::fs::read_to_string(&map_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&s).ok())
        .unwrap_or_default();
    map.insert(
        channel.to_string(),
        serde_json::Value::String(path.to_string_lossy().to_string()),
    );
    std::fs::write(&map_path, serde_json::to_string(&map).unwrap()).unwrap();
}

fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn harness_script(native_session: &str, cwd: &Path, injected_log: &Path) -> String {
    let cwd_json = serde_json::to_string(&cwd.to_string_lossy()).unwrap();
    let hook_log = injected_log.with_extension("hook.log");
    let script = format!(
        "printf '{{\"session_id\":\"{}\",\"cwd\":{},\"pid\":%s}}\\n' \"$$\" \
         | \"$MOSAICO_BIN\" harness hook opencode --type session-start >{} 2>&1; \
         while IFS= read -r line; do printf '%s\\n' \"$line\" >> {}; done",
        native_session,
        cwd_json,
        sh_quote(&hook_log),
        sh_quote(injected_log)
    );
    format!("#!/bin/sh\n{script}\n")
}

struct PathGuard(Option<std::ffi::OsString>);

impl Drop for PathGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            unsafe { std::env::set_var("PATH", path) };
        }
    }
}

fn install_opencode_shim(
    home: &Home,
    native_session: &str,
    cwd: &Path,
    injected_log: &Path,
) -> PathGuard {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = home.dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let shim = bin.join("opencode");
    std::fs::write(&shim, harness_script(native_session, cwd, injected_log)).unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        home.dir.path().join("harnesses.json"),
        r#"{"offline-test":{"harness":"opencode","transport":"pty"}}"#,
    )
    .unwrap();
    let old = std::env::var_os("PATH");
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(old.as_deref().unwrap_or_default()));
    unsafe { std::env::set_var("PATH", std::env::join_paths(paths).unwrap()) };
    PathGuard(old)
}

fn kill_pty(pty_id: &str) {
    let _ = mosaico::pty::kill(pty_id);
}

fn pty_diagnostics() -> String {
    let rows = mosaico::pty::read_all_metadata();
    let mut out = rows
        .iter()
        .map(|row| {
            format!(
                "{} live={} command={}",
                row.id,
                mosaico::pty::is_live(&row.id),
                row.command.join(" ")
            )
        })
        .collect::<Vec<_>>();
    for row in rows {
        let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&row.socket) else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
        let _ = std::io::Write::write_all(&mut stream, b"ATTACH 24 80\n");
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stream, &mut buf);
        if !buf.is_empty() {
            out.push(format!(
                "{} backlog={:?}",
                row.id,
                String::from_utf8_lossy(&buf)
            ));
        }
    }
    out.join("; ")
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
    let mut seen = Vec::new();
    assert!(
        wait_until(Duration::from_secs(25), || {
            found = find_alive_session(home, slug, scope);
            seen = Store::open(&home.store_path())
                .and_then(|s| s.list_alive_sessions())
                .unwrap_or_default()
                .into_iter()
                .map(|rec| format!("{}:{}:{}", rec.agent_slug, rec.channel_h, rec.pubkey))
                .collect();
            found.is_some()
        }),
        "session {slug} in {scope} did not become alive; alive={seen:?}; pty={}; daemon_log={}",
        pty_diagnostics(),
        std::fs::read_to_string(home.dir.path().join("daemon.log"))
            .unwrap_or_else(|e| format!("<unreadable: {e}>"))
    );
    found.unwrap()
}

fn wait_for_group_member(home: &Home, channel: &str, pubkey: &str) {
    assert!(
        wait_until(Duration::from_secs(25), || Store::open(&home.store_path())
            .map(|s| {
                refresh_channel_members(channel);
                s.is_channel_member(channel, pubkey).unwrap_or(false)
            })
            .unwrap_or(false)),
        "{pubkey} was not visible as a member of {channel}; daemon_log={}; group_log={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log"))
            .unwrap_or_else(|e| format!("<{e}>")),
        std::fs::read_to_string(home.dir.path().join("logs/group-mgmt.log"))
            .unwrap_or_else(|e| format!("<{e}>"))
    );
}

fn wait_for_injected_log(log: &Path, body: &str) {
    assert!(
        wait_until(Duration::from_secs(25), || std::fs::read_to_string(log)
            .map(|s| s.contains(body))
            .unwrap_or(false)),
        "PTY session did not receive injected body {body:?}; log={}",
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

async fn publish_user_kind9(channel: &str, body: &str, mentioned_pubkey: &str) -> String {
    let keys = Keys::parse(EXAMPLE_USER_NSEC).expect("operator keys");
    let client = nostr_user_client(keys.clone()).await;
    let chat = ChatMessage {
        from: AgentRef::new(keys.public_key().to_hex(), ""),
        channel: channel.to_string(),
        body: body.to_string(),
        mentioned_pubkeys: vec![mentioned_pubkey.to_string()],
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

#[path = "launch_mentions/working.rs"]
mod working;

#[test]
fn operator_kind9_to_offline_local_agent_spawns_and_injects() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("kind9-spawn");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);

    let agent = "offline-kind9";
    let log = home.dir.path().join("offline-injected.log");
    let native_session = unique_session("offline-native");
    let _path = install_opencode_shim(&home, &native_session, &work_dir, &log);
    identity::add_local_agent(home.dir.path(), agent, "offline-test", None, 1)
        .expect("add local agent");
    // Seed an offline profile so the mention resolves the p-tagged pubkey to this
    // agent. With no native id to resume, the handler falls through to a fresh
    // spawn (a new session that mints its own pubkey).
    let agent_pubkey = Keys::generate().public_key().to_hex();
    Store::open(&home.store_path())
        .unwrap()
        .upsert_profile_with_agent_slug(&agent_pubkey, agent, agent, agent, "test-host", false, 1)
        .expect("seed offline profile");

    rt().block_on(async {
        let mut c = DaemonClient::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({
                "agent": "keeper",
                "harness_session": unique_session("keeper"),
                "cwd": work_dir,
                "watch_pid": std::process::id(),
            }),
        )
        .await
        .expect("keeper session_start");
    });
    let keeper = wait_for_alive_session(&home, "keeper", &channel);
    wait_for_group_member(&home, &channel, &keeper.pubkey);

    rt().block_on(async {
        let mut c = DaemonClient::connect_or_spawn().await.expect("connect");
        let add = c
            .call(
                "channel_add_member",
                serde_json::json!({
                    "channel": channel,
                    "pubkey": agent_pubkey,
                    "cwd": work_dir,
                }),
            )
            .await
            .expect("channel_add_member offline agent");
        assert_eq!(add["pubkey"], agent_pubkey);
    });
    wait_for_group_member(&home, &channel, &agent_pubkey);

    let body = format!("wake offline agent {}", unique_session("body"));
    rt().block_on(async {
        publish_user_kind9(&channel, &body, &agent_pubkey).await;
    });

    let rec = wait_for_alive_session(&home, agent, &channel);
    let store = Store::open(&home.store_path()).unwrap();
    let instance = store
        .session_identity(&rec.pubkey)
        .unwrap()
        .expect("spawned session identity");
    assert_eq!(instance.pubkey, rec.pubkey);
    wait_for_injected_log(&log, &body);

    let pty_id = pty_session_for_session(&store, &rec.pubkey).expect("spawned pty endpoint");
    kill_pty(&pty_id);
    stop_daemon(&home);
}
