use super::*;

struct HermesHomeGuard {
    home: Option<std::ffi::OsString>,
    hermes_home: Option<std::ffi::OsString>,
}

impl Drop for HermesHomeGuard {
    fn drop(&mut self) {
        match self.home.take() {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        match self.hermes_home.take() {
            Some(value) => unsafe { std::env::set_var("HERMES_HOME", value) },
            None => unsafe { std::env::remove_var("HERMES_HOME") },
        }
    }
}

fn install_hermes_profile_shim(
    home: &Home,
    profile: &str,
    native_session: &str,
    cwd: &Path,
    injected_log: &Path,
    argv_log: &Path,
) -> HermesHomeGuard {
    use std::os::unix::fs::PermissionsExt as _;

    let hermes_home = home.dir.path().join("hermes-home");
    let profile_dir = hermes_home.join("profiles").join(profile);
    std::fs::create_dir_all(&profile_dir).unwrap();
    std::fs::write(
        profile_dir.join("profile.yaml"),
        "description: Builds and validates scoped changes.\n",
    )
    .unwrap();

    let cwd_json = serde_json::to_string(&cwd.to_string_lossy()).unwrap();
    let hook_log = injected_log.with_extension("hook.log");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}; \
         printf '{{\"session_id\":\"{}\",\"cwd\":{},\"pid\":%s}}\\n' \"$$\" \
         | \"$MOSAICO_BIN\" harness hook hermes --type session-start >{} 2>&1; \
         while IFS= read -r line; do printf '%s\\n' \"$line\" >> {}; done\n",
        sh_quote(argv_log),
        native_session,
        cwd_json,
        sh_quote(&hook_log),
        sh_quote(injected_log),
    );
    let shim = home.dir.path().join(".local/bin/hermes");
    std::fs::create_dir_all(shim.parent().unwrap()).unwrap();
    std::fs::write(&shim, script).unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();

    let previous_home = std::env::var_os("HOME");
    let previous_hermes_home = std::env::var_os("HERMES_HOME");
    unsafe { std::env::set_var("HOME", home.dir.path()) };
    unsafe { std::env::set_var("HERMES_HOME", hermes_home) };
    HermesHomeGuard {
        home: previous_home,
        hermes_home: previous_hermes_home,
    }
}

#[test]
fn operator_kind9_injects_into_working_launch_session() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("kind9-launch");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let log = home.dir.path().join("launch-injected.log");
    let native_session = unique_session("launch-native");
    let agent = "launch-kind9";
    let _path = install_opencode_shim(&home, &native_session, &work_dir, &log);
    identity::add_local_agent(home.dir.path(), agent, "offline-test", None, 1)
        .expect("add launch agent");

    let pty_id = rt().block_on(async {
        let mut c = DaemonClient::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": agent,
                    "root": channel,
                    "channel": channel,
                    "cwd": work_dir,
                }),
            )
            .await
            .expect("pty_spawn");
        v["pty_id"].as_str().unwrap().to_string()
    });

    let rec = wait_for_alive_session(&home, agent, &channel);
    wait_for_group_member(&home, &channel, &rec.pubkey);
    Store::open(&home.store_path())
        .unwrap()
        .apply_session_turn_started(
            &rec.pubkey,
            rec.runtime_generation,
            mosaico::util::now_secs(),
        )
        .expect("mark launch session working");

    // Launch-time admission already bound this exact pubkey to its typed PTY
    // endpoint. Later delivery must not reopen mutable slug configuration to
    // rediscover how the live session is hosted.
    std::fs::write(
        home.dir.path().join("agents").join(format!("{agent}.json")),
        b"{ invalid after launch",
    )
    .expect("corrupt post-launch agent config");

    let body = format!("operator relay injection {}", unique_session("body"));
    let event_id = rt().block_on(publish_user_kind9(&channel, &body, &rec.pubkey));
    wait_for_injected_log(&log, &body);

    let mut observed = None;
    assert!(
        wait_until(Duration::from_secs(25), || {
            observed = observed_chat(&event_id);
            observed.is_some()
        }),
        "operator kind:9 should be exposed as observed user-authored chat"
    );
    let observed = observed.expect("observed operator chat");
    assert_eq!(observed["body"].as_str(), Some(body.as_str()));
    assert_eq!(
        observed["from_pubkey"].as_str(),
        Some(pubkey_of(EXAMPLE_USER_NSEC).as_str())
    );
    let read_params = serde_json::json!({
        "session": &rec.pubkey,
        "channel": format!("#{channel}"),
        "limit": 1000,
    });
    let messages = read_channel_messages(read_params.clone()).expect("public channel read");
    let agent_messages_before = messages
        .iter()
        .filter(|event| event["from_pubkey"].as_str() == Some(rec.pubkey.as_str()))
        .count();

    let legacy_transcript = home.dir.path().join("legacy-transcript.jsonl");
    std::fs::write(
        &legacy_transcript,
        r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"must remain private"}]}}"#,
    )
    .unwrap();
    rt().block_on(async {
        let mut client = DaemonClient::connect_or_spawn().await.expect("connect");
        client
            .call(
                "turn_start",
                serde_json::json!({
                    "pty_session": pty_id,
                    "transcript_path": legacy_transcript,
                }),
            )
            .await
            .expect("start turn with removed transcript input");
        client
            .call("turn_end", serde_json::json!({"pty_session": pty_id}))
            .await
            .expect("finish injected turn");
    });
    assert!(
        !wait_until(Duration::from_secs(2), || read_channel_messages(
            read_params.clone()
        )
        .map(|messages| messages
            .iter()
            .filter(|event| event["from_pubkey"].as_str() == Some(rec.pubkey.as_str()))
            .count()
            > agent_messages_before)
        .unwrap_or(false)),
        "turn completion must not publish an implicit channel message"
    );

    kill_pty(&pty_id);
    stop_daemon(&home);
}

#[test]
fn native_hermes_profile_persists_admission_and_receives_tagged_delivery() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("hermes-profile-launch");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let injected_log = home.dir.path().join("hermes-injected.log");
    let argv_log = home.dir.path().join("hermes-argv.log");
    let native_session = unique_session("hermes-native");
    let _hermes_home = install_hermes_profile_shim(
        &home,
        "builder",
        &native_session,
        &work_dir,
        &injected_log,
        &argv_log,
    );

    let pty_id = rt().block_on(async {
        let mut client = DaemonClient::connect_or_spawn().await.expect("connect");
        let response = client
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": "builder",
                    "root": channel,
                    "channel": channel,
                    "cwd": work_dir,
                }),
            )
            .await
            .expect("spawn native Hermes profile");
        response["pty_id"].as_str().unwrap().to_string()
    });

    let session = wait_for_alive_session(&home, "builder", &channel);
    wait_for_group_member(&home, &channel, &session.pubkey);
    assert_eq!(session.observed_harness, "hermes");
    assert_eq!(session.admitted_bundle, "hermes-pty");
    assert_eq!(session.admitted_transport, "pty");

    let store = Store::open(&home.store_path()).unwrap();
    assert_eq!(
        pty_session_for_session(&store, &session.pubkey).as_deref(),
        Some(pty_id.as_str())
    );
    let mut resume = None;
    assert!(
        wait_until(Duration::from_secs(10), || {
            resume = Store::open(&home.store_path())
                .and_then(|store| store.native_resume_locator(&session.pubkey, "hermes"))
                .unwrap_or(None);
            resume.is_some()
        }),
        "Hermes hook did not bind resume locator; argv={}; pty={}; hook={}; calls={}; daemon={}",
        std::fs::read_to_string(&argv_log).unwrap_or_default(),
        pty_diagnostics(),
        std::fs::read_to_string(injected_log.with_extension("hook.log")).unwrap_or_default(),
        std::fs::read_to_string(
            home.dir
                .path()
                .join("sessions")
                .join(&native_session)
                .join("hook-calls.jsonl")
        )
        .unwrap_or_default(),
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
    );
    assert_eq!(
        resume.map(|locator| locator.locator_value),
        Some(native_session.clone())
    );
    assert_eq!(
        std::fs::read_to_string(&argv_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["--profile", "builder"]
    );

    Store::open(&home.store_path())
        .unwrap()
        .apply_session_turn_started(
            &session.pubkey,
            session.runtime_generation,
            mosaico::util::now_secs(),
        )
        .expect("mark Hermes profile working");
    let body = format!("Hermes profile delivery {}", unique_session("body"));
    rt().block_on(publish_user_kind9(&channel, &body, &session.pubkey));
    wait_for_injected_log(&injected_log, &body);

    kill_pty(&pty_id);
    stop_daemon(&home);
}
