use super::*;

struct HermesEnvGuard {
    home: Option<std::ffi::OsString>,
    hermes_home: Option<std::ffi::OsString>,
    path: Option<std::ffi::OsString>,
}

impl Drop for HermesEnvGuard {
    fn drop(&mut self) {
        for (key, value) in [
            ("HOME", self.home.take()),
            ("HERMES_HOME", self.hermes_home.take()),
            ("PATH", self.path.take()),
        ] {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

fn install_hermes_shim(
    home: &Home,
    native_session: &str,
    cwd: &Path,
    context_log: &Path,
    argv_log: &Path,
) -> HermesEnvGuard {
    use std::os::unix::fs::PermissionsExt as _;

    let bin = home.dir.path().join("hermes-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let cwd_json = serde_json::to_string(&cwd.to_string_lossy()).unwrap();
    let hook_log = context_log.with_extension("hook.log");
    let script = format!(
        "#!/bin/sh\n\
         printf '%s\\n' \"$@\" > {}\n\
         printf '{{\"session_id\":\"{}\",\"cwd\":{},\"pid\":%s}}\\n' \"$$\" \
         | \"$MOSAICO_BIN\" harness hook hermes --type session-start >>{} 2>&1\n\
         while IFS= read -r line; do\n\
           printf '{{\"session_id\":\"{}\",\"cwd\":{},\"pid\":%s}}\\n' \"$$\" \
           | \"$MOSAICO_BIN\" harness hook hermes --type user-prompt-submit \
           >>{} 2>>{}\n\
         done\n",
        sh_quote(argv_log),
        native_session,
        cwd_json,
        sh_quote(&hook_log),
        native_session,
        cwd_json,
        sh_quote(context_log),
        sh_quote(&hook_log),
    );
    let shim = bin.join("hermes");
    std::fs::write(&shim, script).unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();

    let previous_path = std::env::var_os("PATH");
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        previous_path.as_deref().unwrap_or_default(),
    ));
    let guard = HermesEnvGuard {
        home: std::env::var_os("HOME"),
        hermes_home: std::env::var_os("HERMES_HOME"),
        path: previous_path,
    };
    unsafe {
        std::env::set_var("HOME", home.dir.path());
        std::env::set_var("HERMES_HOME", home.dir.path().join("hermes-home"));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    }
    guard
}

#[test]
fn hermes_pty_launch_injects_relay_fabric_context_through_pre_llm_hook() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("hermes-context");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let context_log = home.dir.path().join("hermes-context.log");
    let argv_log = home.dir.path().join("hermes-argv.log");
    let native_session = unique_session("hermes-native");
    let _path = install_hermes_shim(&home, &native_session, &work_dir, &context_log, &argv_log);
    std::fs::write(
        home.dir.path().join("harnesses.json"),
        r#"{"hermes-context-e2e":{"harness":"hermes","transport":"pty"}}"#,
    )
    .unwrap();
    identity::add_local_agent(
        home.dir.path(),
        "hermes-context",
        "hermes-context-e2e",
        None,
        1,
    )
    .expect("add Hermes agent");

    let pty_id = rt().block_on(async {
        let mut client = DaemonClient::connect_or_spawn().await.expect("connect");
        let response = client
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": "hermes-context",
                    "root": channel,
                    "channel": channel,
                    "cwd": work_dir,
                }),
            )
            .await
            .expect("spawn Hermes PTY");
        response["pty_id"].as_str().unwrap().to_string()
    });

    let session = wait_for_alive_session(&home, "hermes-context", &channel);
    wait_for_group_member(&home, &channel, &session.pubkey);
    let marker = format!("HERMES_FABRIC_{}", unique_session("marker"));
    let event_id = rt().block_on(publish_user_kind9(&channel, &marker, &session.pubkey));
    assert!(
        wait_until(Duration::from_secs(25), || observed_chat(&event_id)
            .is_some_and(
                |message| message["body"].as_str() == Some(marker.as_str())
            )),
        "relay-published Hermes marker never reached the public reader; daemon={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default(),
    );
    mosaico::pty::inject(&pty_id, "inspect current fabric\n", false, false)
        .expect("submit Hermes prompt");
    assert!(
        wait_until(Duration::from_secs(25), || std::fs::read_to_string(
            &context_log
        )
        .is_ok_and(|context| context.contains(&marker))),
        "Hermes pre-LLM hook never received canonical fabric marker; context={}; hook={}; daemon={}",
        std::fs::read_to_string(&context_log).unwrap_or_default(),
        std::fs::read_to_string(context_log.with_extension("hook.log")).unwrap_or_default(),
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default(),
    );
    assert!(std::fs::read_to_string(&argv_log)
        .unwrap()
        .trim()
        .is_empty());

    kill_pty(&pty_id);
    stop_daemon(&home);
}
