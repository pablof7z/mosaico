use super::*;
use crate::nmp_client::NmpRelayClient;
use mosaico::domain::{AgentRef, ChatMessage, DomainEvent};
use mosaico::fabric::nip29::wire::Nip29WireCodec;
use nostr::Keys;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

struct EnvRestore(Vec<(&'static str, Option<OsString>)>);

impl EnvRestore {
    fn capture(keys: &[&'static str]) -> Self {
        Self(
            keys.iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect(),
        )
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (key, value) in self.0.drain(..) {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

fn install_fake_codex(home: &Path) -> PathBuf {
    let bin = home.join(".nvm/versions/node/v23.11.1/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let codex = bin.join("codex");
    std::fs::write(
        &codex,
        r#"#!/bin/sh
printf '%s\n' "$PATH" > "$MOSAICO_HOME/captured-codex-path"
if test -f "$CODEX_HOME/config.toml"; then
  printf '%s\n' '--- staged config ---' >> "$MOSAICO_HOME/captured-codex-configs"
  cat "$CODEX_HOME/config.toml" >> "$MOSAICO_HOME/captured-codex-configs"
fi
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  test -n "$id" || continue
  case "$line" in
    *'"method":"thread/start"'*)
      printf '%s\n' "$line" >> "$MOSAICO_HOME/codex-thread-starts.jsonl"
      result="{\"thread\":{\"id\":\"fixture-thread-$id\",\"turns\":[]}}"
      ;;
    *) result='{}' ;;
  esac
  printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result"
done
"#,
    )
    .unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn configure_agents(home: &Home, codex_home: &Path) {
    std::fs::write(
        home.dir.path().join("harnesses.json"),
        r#"{"codex-app-server":{"harness":"codex","transport":"app-server"}}"#,
    )
    .unwrap();
    let native = codex_home.join("agents/native-codex-role.toml");
    std::fs::create_dir_all(native.parent().unwrap()).unwrap();
    std::fs::write(
        native,
        "name='native-codex-role'\ndescription='Native fixture'\ndeveloper_instructions='Use native fixture instructions'\n",
    )
    .unwrap();
    std::fs::write(
        codex_home.join("mosaico-configured-profile.config.toml"),
        "developer_instructions='Use configured Mosaico profile instructions'\n",
    )
    .unwrap();
    mosaico::identity::add_local_agent(
        home.dir.path(),
        "mosaico-configured-role",
        "codex-app-server",
        Some("mosaico-configured-profile"),
        1,
    )
    .unwrap();
    let record = home.dir.path().join("agents/mosaico-configured-role.json");
    let keys = Keys::generate();
    let mut pre_keyless: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
    pre_keyless["secret_key"] = serde_json::json!(keys.secret_key().to_secret_hex());
    pre_keyless["public_key"] = serde_json::json!(keys.public_key().to_hex());
    std::fs::write(record, serde_json::to_string_pretty(&pre_keyless).unwrap()).unwrap();
}

fn start_channel(home: &Home, channel: &str, work_dir: &Path) {
    std::fs::create_dir_all(work_dir).unwrap();
    std::fs::create_dir_all(home.dir.path().join(".claude")).unwrap();
    let mut workspaces = serde_json::Map::new();
    workspaces.insert(
        channel.to_string(),
        serde_json::Value::String(work_dir.to_string_lossy().into_owned()),
    );
    std::fs::write(
        home.dir.path().join("workspaces.json"),
        serde_json::Value::Object(workspaces).to_string(),
    )
    .unwrap();
    rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect daemon");
        client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "claude",
                        "harness_session": unique_session("mgmt-keeper"),
                        "cwd": work_dir,
                        "channel": channel,
                        "watch_pid": std::process::id(),
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("start channel keeper");
    });
    let user = pubkey_of(EXAMPLE_USER_NSEC);
    assert!(wait_until(Duration::from_secs(25), || {
        refresh_channel_members(channel);
        Store::open(&home.store_path())
            .map(|store| store.is_channel_admin(channel, &user).unwrap_or(false))
            .unwrap_or(false)
    }));
}

async fn publish_management_command(channel: &str, body: &str) {
    let keys = Keys::parse(EXAMPLE_USER_NSEC).unwrap();
    let client = NmpRelayClient::connect(keys.clone(), &shared_nip29_relay_url())
        .await
        .expect("connect NMP relay client");
    let chat = ChatMessage {
        from: AgentRef::new(keys.public_key().to_hex(), ""),
        channel: channel.to_string(),
        body: body.to_string(),
        mentioned_pubkeys: vec![pubkey_of(EXAMPLE_BACKEND_SEC_HEX)],
    };
    let builder = Nip29WireCodec
        .encode_event(&DomainEvent::ChatMessage(chat))
        .unwrap();
    let event = builder.sign_with_keys(&keys).unwrap();
    let output = client.send_event(&event).await.unwrap();
    assert!(!output.success.is_empty(), "management kind:9 rejected");
}

fn running_session(home: &Home, channel: &str, slug: &str) -> Option<mosaico::state::Session> {
    Store::open(&home.store_path())
        .ok()?
        .list_running_sessions()
        .ok()?
        .into_iter()
        .find(|session| {
            session.channel_h == channel
                && session.agent_slug == slug
                && session.admitted_transport == "app-server"
        })
}

fn assert_management_add(home: &Home, channel: &str, slug: &str) {
    rt().block_on(publish_management_command(channel, &format!("add {slug}")));
    assert!(
        wait_until(Duration::from_secs(35), || running_session(
            home, channel, slug
        )
        .is_some()),
        "management p-tag did not start {slug}; daemon_log={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
    );
    let session = running_session(home, channel, slug).unwrap();
    assert_eq!(session.observed_harness, "codex");
    assert_eq!(session.admitted_transport, "app-server");
}

#[test]
fn management_p_tag_adds_base_native_and_configured_codex_agents_with_restricted_path() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _restore = EnvRestore::capture(&["HOME", "CODEX_HOME", "PATH"]);
    let home = Home::new();
    unsafe {
        std::env::set_var("HOME", home.dir.path());
        std::env::set_var("CODEX_HOME", home.dir.path().join(".codex"));
        std::env::set_var("PATH", "/usr/bin:/bin");
    }
    write_config(&home, false);
    let nvm_bin = install_fake_codex(home.dir.path());
    configure_agents(&home, &home.dir.path().join(".codex"));
    let channel = unique_session("management-add");
    let work_dir = home.dir.path().join("work");
    start_channel(&home, &channel, &work_dir);

    let interactive = run_cli_with_env_in_dir(&home, &["codex"], &[], &work_dir);
    assert!(
        interactive.status.success(),
        "restricted-PATH PTY launch failed: {}",
        String::from_utf8_lossy(&interactive.stderr)
    );
    let mut interactive_pty = None;
    assert!(wait_until(Duration::from_secs(10), || {
        interactive_pty = mosaico::pty::read_all_metadata()
            .into_iter()
            .find(|metadata| metadata.agent == "codex" && mosaico::pty::is_live(&metadata.id));
        interactive_pty.is_some()
    }));

    for slug in ["codex", "native-codex-role", "mosaico-configured-role"] {
        assert_management_add(&home, &channel, slug);
    }

    let captured_path = std::fs::read_to_string(home.dir.path().join("captured-codex-path"))
        .expect("fake codex captured PATH");
    assert!(
        std::env::split_paths(captured_path.trim()).any(|path| path == nvm_bin),
        "normalized hosted PATH omitted {}: {captured_path}",
        nvm_bin.display()
    );
    let starts = std::fs::read_to_string(home.dir.path().join("codex-thread-starts.jsonl"))
        .expect("fake codex captured thread/start");
    assert!(
        starts.contains("Use native fixture instructions"),
        "{starts}"
    );
    assert!(starts.lines().count() >= 3, "{starts}");
    let staged_configs = std::fs::read_to_string(home.dir.path().join("captured-codex-configs"))
        .expect("configured Mosaico profile staged a Codex config");
    assert!(
        staged_configs.contains("Use configured Mosaico profile instructions"),
        "{staged_configs}"
    );
    let migrated: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.dir.path().join("agents/mosaico-configured-role.json"))
            .unwrap(),
    )
    .unwrap();
    assert!(migrated.get("secret_key").is_none());
    assert!(migrated.get("public_key").is_none());

    let interactive_pty = interactive_pty.unwrap();
    let supervisor_pid = interactive_pty.supervisor_pid;
    let child_pid = interactive_pty.child_pid;
    stop_daemon(&home);
    for slug in ["codex", "native-codex-role", "mosaico-configured-role"] {
        assert!(
            running_session(&home, &channel, slug).is_none(),
            "orderly daemon shutdown left RPC session {slug} running"
        );
    }

    rt().block_on(async {
        Client::connect_or_spawn()
            .await
            .expect("restart daemon after live PTY shutdown boundary");
    });
    assert!(wait_until(Duration::from_secs(10), || {
        mosaico::pty::read_all_metadata()
            .into_iter()
            .any(|metadata| {
                metadata.id == interactive_pty.id
                    && metadata.supervisor_pid == supervisor_pid
                    && metadata.child_pid == child_pid
                    && mosaico::pty::is_live(&metadata.id)
            })
    }));
    mosaico::pty::kill(&interactive_pty.id).unwrap();
    stop_daemon(&home);
}
