use super::*;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read as _, Write as _};

#[test]
fn missing_provider_is_a_cli_failure_without_live_metadata_or_session() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    write_config(&home, false);
    let channel = unique_session("missing-provider");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    mosaico::identity::add_local_agent(
        home.dir.path(),
        "missing-provider-role",
        "grok",
        None,
        None,
        1,
    )
    .unwrap();

    let isolated_home = home.dir.path().to_string_lossy().into_owned();
    let output = run_cli_with_env_in_dir(
        &home,
        &["missing-provider-role"],
        &[("HOME", &isolated_home), ("PATH", "/usr/bin:/bin")],
        &work_dir,
    );

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("launch of agent"), "{stderr}");
    assert!(stderr.contains("exited during startup"), "{stderr}");
    assert!(!mosaico::pty::read_all_metadata()
        .into_iter()
        .any(|metadata| metadata.agent == "missing-provider-role"));
    assert!(!Store::open(&home.store_path())
        .unwrap()
        .list_running_sessions()
        .unwrap()
        .into_iter()
        .any(|session| session.agent_slug == "missing-provider-role"));
    stop_daemon(&home);
}

fn incomplete_durable_agent(home: &Home, slug: &str) -> std::path::PathBuf {
    let path = home.dir.path().join(format!("agents/{slug}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&serde_json::json!({
            "slug": slug,
            "created_at": 1,
            "perSessionKey": false,
            "harness": "grok"
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn interactive_launch(home: &Home, cwd: &std::path::Path, slug: &str, answer: &[u8]) -> String {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(bin());
    command.arg(slug);
    command.cwd(cwd);
    command.env("MOSAICO_HOME", home.dir.path());
    command.env("MOSAICO_CONFIG", home.dir.path().join("config.json"));
    command.env("MOSAICO_BIN", bin());
    command.env("MOSAICO_DAEMON_GRACE_S", "30");
    let mut child = pair.slave.spawn_command(command).unwrap();
    let mut reader = pair.master.try_clone_reader().unwrap();
    let output = std::thread::spawn(move || {
        let mut output = String::new();
        reader.read_to_string(&mut output).unwrap();
        output
    });
    let mut writer = pair.master.take_writer().unwrap();
    writer.write_all(answer).unwrap();
    writer.flush().unwrap();
    drop(writer);

    let _status = child.wait().unwrap();
    output.join().unwrap()
}

#[test]
fn interactive_launch_creates_and_persists_a_confirmed_durable_key() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    let channel = unique_session("durable-key-confirm");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let path = incomplete_durable_agent(&home, "chief-of-staff");

    let output = interactive_launch(&home, &work_dir, "chief-of-staff", b"y\r");

    assert!(output.contains("Create and persist one now?"), "{output:?}");
    assert!(output.contains("Created and persisted a key"), "{output:?}");
    let loaded = mosaico::identity::load(home.dir.path(), "chief-of-staff").unwrap();
    assert!(!loaded.per_session_key);
    assert!(loaded.keys.is_some());
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(stored["secret_key"].as_str().is_some());
    assert!(stored["public_key"].as_str().is_some());
    stop_daemon(&home);
}

#[test]
fn declining_durable_key_creation_leaves_the_record_untouched() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    let channel = unique_session("durable-key-decline");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let path = incomplete_durable_agent(&home, "chief-of-staff");
    let before = std::fs::read(&path).unwrap();

    let output = interactive_launch(&home, &work_dir, "chief-of-staff", b"n\r");

    assert!(output.contains("Create and persist one now?"), "{output:?}");
    assert!(output.contains("launch cancelled"), "{output:?}");
    assert_eq!(std::fs::read(path).unwrap(), before);
    stop_daemon(&home);
}
