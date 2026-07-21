use super::*;

struct GooseEnvGuard {
    home: Option<std::ffi::OsString>,
    path: Option<std::ffi::OsString>,
    xdg_config: Option<std::ffi::OsString>,
    goose_root: Option<std::ffi::OsString>,
}

impl Drop for GooseEnvGuard {
    fn drop(&mut self) {
        restore("HOME", self.home.take());
        restore("PATH", self.path.take());
        restore("XDG_CONFIG_HOME", self.xdg_config.take());
        restore("GOOSE_PATH_ROOT", self.goose_root.take());
    }
}

fn restore(key: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
}

fn install_goose_shim(
    home: &Home,
    native_session: &str,
    cwd: &Path,
    context_log: &Path,
    argv_log: &Path,
) -> GooseEnvGuard {
    use std::os::unix::fs::PermissionsExt as _;

    let bin = home.dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let cwd_json = serde_json::to_string(&cwd.to_string_lossy()).unwrap();
    let hook_log = context_log.with_extension("hook.log");
    let script = format!(
        "#!/bin/sh\n\
         if [ \"${{1:-}}\" = --version ]; then echo 1.43.0; exit 0; fi\n\
         printf '%s\\n' \"$@\" > {}\n\
         printf '{{\"session_id\":\"{}\",\"working_dir\":{},\"pid\":%s}}\\n' \"$$\" \
         | \"$MOSAICO_BIN\" harness hook goose --type session-start >>{} 2>&1\n\
         while IFS= read -r line; do\n\
           printf '{{\"session_id\":\"{}\",\"working_dir\":{},\"pid\":%s}}\\n' \"$$\" \
           | \"$MOSAICO_BIN\" harness hook goose --type user-prompt-submit >>{} 2>&1\n\
           cat \"$GOOSE_MOIM_MESSAGE_FILE\" > {}\n\
         done\n",
        sh_quote(argv_log),
        native_session,
        cwd_json,
        sh_quote(&hook_log),
        native_session,
        cwd_json,
        sh_quote(&hook_log),
        sh_quote(context_log),
    );
    let shim = bin.join("goose");
    std::fs::write(&shim, script).unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();

    let plugin = home.dir.path().join(".agents/plugins/mosaico");
    std::fs::create_dir_all(plugin.join("hooks")).unwrap();
    std::fs::write(
        plugin.join("plugin.json"),
        include_str!("../../../../integrations/goose/plugin.json"),
    )
    .unwrap();
    std::fs::write(
        plugin.join("hooks/hooks.json"),
        include_str!("../../../../integrations/goose/hooks/hooks.json"),
    )
    .unwrap();

    std::fs::write(
        home.dir.path().join("harnesses.json"),
        r#"{"goose-e2e":{"harness":"goose","transport":"pty"}}"#,
    )
    .unwrap();
    let old_path = std::env::var_os("PATH");
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        old_path.as_deref().unwrap_or_default(),
    ));
    let guard = GooseEnvGuard {
        home: std::env::var_os("HOME"),
        path: old_path,
        xdg_config: std::env::var_os("XDG_CONFIG_HOME"),
        goose_root: std::env::var_os("GOOSE_PATH_ROOT"),
    };
    unsafe {
        std::env::set_var("HOME", home.dir.path());
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        std::env::set_var("XDG_CONFIG_HOME", home.dir.path().join(".config"));
        std::env::remove_var("GOOSE_PATH_ROOT");
    }
    guard
}

#[test]
fn goose_pty_launch_injects_canonical_fabric_context_through_top_of_mind() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("goose-context");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let context_log = home.dir.path().join("goose-context.log");
    let argv_log = home.dir.path().join("goose-argv.log");
    let native_session = unique_session("goose-native");
    let _env = install_goose_shim(&home, &native_session, &work_dir, &context_log, &argv_log);
    identity::add_local_agent(home.dir.path(), "goose", "goose-e2e", None, 1)
        .expect("add Goose agent");

    let pty_id = rt().block_on(async {
        let mut client = DaemonClient::connect_or_spawn().await.expect("connect");
        let response = client
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": "goose",
                    "root": channel,
                    "channel": channel,
                    "cwd": work_dir,
                }),
            )
            .await
            .expect("spawn Goose PTY");
        response["pty_id"].as_str().unwrap().to_string()
    });

    let session = wait_for_alive_session(&home, "goose", &channel);
    wait_for_group_member(&home, &channel, &session.pubkey);
    let marker = format!("GOOSE_FABRIC_{}", unique_session("marker"));
    Store::open(&home.store_path())
        .unwrap()
        .insert_event(&mosaico::state::RelayEvent {
            id: format!("{:064x}", mosaico::util::now_secs()),
            kind: mosaico::fabric::nip29::wire::KIND_CHAT as u32,
            pubkey: pubkey_of(EXAMPLE_USER_NSEC),
            created_at: mosaico::util::now_secs(),
            channel_h: channel.clone(),
            d_tag: String::new(),
            content: marker.clone(),
            tags_json: serde_json::json!([["h", channel], ["p", session.pubkey]]).to_string(),
        })
        .unwrap();
    mosaico::pty::inject(&pty_id, "inspect current fabric", true, true)
        .expect("submit Goose prompt");
    assert!(
        wait_until(Duration::from_secs(25), || std::fs::read_to_string(
            &context_log
        )
        .is_ok_and(|context| context.contains(&marker))),
        "Goose Top Of Mind never received canonical fabric marker; context={}; hook={}; daemon={}",
        std::fs::read_to_string(&context_log).unwrap_or_default(),
        std::fs::read_to_string(context_log.with_extension("hook.log")).unwrap_or_default(),
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default(),
    );
    assert_eq!(
        std::fs::read_to_string(&argv_log).unwrap().trim(),
        "session"
    );

    kill_pty(&pty_id);
    stop_daemon(&home);
}
