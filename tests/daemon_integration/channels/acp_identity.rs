use super::*;
use nostr_sdk::prelude::Keys;
use std::time::Duration;

#[test]
fn acp_agent_receives_the_signer_matching_its_assigned_pubkey() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert_acp_identity("opencode", None);
}

#[test]
fn goose_acp_agent_receives_signer_and_hosted_prompt() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert_acp_identity("goose", Some("Goose daemon delivery probe"));
}

fn assert_acp_identity(harness: &str, prompt: Option<&str>) {
    let home = Home::new();
    write_config(&home, false);
    if harness == "goose" {
        let plugin = home.dir.path().join(".agents/plugins/mosaico");
        std::fs::create_dir_all(plugin.join("hooks")).unwrap();
        std::fs::write(
            plugin.join("plugin.json"),
            include_str!("../../../integrations/goose/plugin.json"),
        )
        .unwrap();
        std::fs::write(
            plugin.join("hooks/hooks.json"),
            include_str!("../../../integrations/goose/hooks/hooks.json"),
        )
        .unwrap();
    }
    std::fs::write(
        home.dir.path().join("harnesses.json"),
        serde_json::json!({
            "test-acp": {"harness": harness, "transport": "acp"}
        })
        .to_string(),
    )
    .unwrap();
    let agent = format!("{harness}-acp-agent-nsec");
    mosaico::identity::add_local_agent(home.dir.path(), &agent, "test-acp", None, 1).unwrap();
    let channel = format!("{harness}-acp-agent-nsec-{}", std::process::id());
    let work_dir = home.dir.path().join(&channel);
    std::fs::create_dir_all(&work_dir).unwrap();
    std::fs::write(
        home.dir.path().join("workspaces.json"),
        serde_json::json!({&channel: &work_dir}).to_string(),
    )
    .unwrap();

    let isolated_home = home.dir.path().to_string_lossy().into_owned();
    let mut args = vec![agent.as_str()];
    args.extend(prompt);
    let xdg_config = home
        .dir
        .path()
        .join(".config")
        .to_string_lossy()
        .into_owned();
    let out = run_cli_with_env_in_dir(
        &home,
        &args,
        &[("HOME", &isolated_home), ("XDG_CONFIG_HOME", &xdg_config)],
        &work_dir,
    );
    assert!(
        out.status.success(),
        "ACP launch failed: {}\ndaemon log:\n{}",
        String::from_utf8_lossy(&out.stderr),
        daemon_log(&home)
    );
    let capture = home.dir.path().join("captured-acp-identity");
    assert!(
        wait_until(Duration::from_secs(10), || capture.exists()),
        "ACP harness did not capture its assigned identity\ndaemon log:\n{}",
        daemon_log(&home)
    );

    let captured = std::fs::read_to_string(&capture).unwrap();
    let mut lines = captured.lines();
    let pubkey = lines.next().expect("captured pubkey");
    let nsec = lines.next().expect("captured nsec");
    assert_eq!(Keys::parse(nsec).unwrap().public_key().to_hex(), pubkey);
    assert!(wait_until(Duration::from_secs(10), || {
        Store::open(&home.store_path())
            .and_then(|store| store.native_resume_locator(pubkey, harness))
            .is_ok_and(|locator| locator.is_some())
    }));
    let store = Store::open(&home.store_path()).unwrap();
    let session = store.get_session(pubkey).unwrap().unwrap();
    assert_eq!(session.observed_harness, harness);
    assert_eq!(session.admitted_bundle, "test-acp");
    assert_eq!(session.admitted_transport, "acp");
    assert_eq!(session.endpoint_provenance, "launch");
    assert!(store
        .locator_for_session(pubkey, harness, "acp")
        .unwrap()
        .is_some());
    assert_eq!(
        store
            .native_resume_locator(pubkey, harness)
            .unwrap()
            .unwrap()
            .locator_value,
        "test-native-session"
    );
    if let Some(prompt) = prompt {
        let prompt_capture = home.dir.path().join("captured-acp-prompts");
        assert!(
            wait_until(Duration::from_secs(10), || std::fs::read_to_string(
                &prompt_capture
            )
            .is_ok_and(|body| body.contains(prompt))),
            "hosted {harness} launch did not deliver {prompt:?} through session/prompt; daemon log:\n{}",
            daemon_log(&home)
        );
        if harness == "goose" {
            let context_path = home.dir.path().join("captured-goose-context");
            assert!(
                wait_until(Duration::from_secs(10), || context_path.exists()),
                "Goose ACP hook did not publish Top Of Mind; hook={}",
                std::fs::read_to_string(home.dir.path().join("captured-goose-hook"))
                    .unwrap_or_default()
            );
            let context = std::fs::read_to_string(context_path).unwrap();
            assert!(context.contains("<mosaico>"), "context={context:?}");
            assert!(context.contains(&channel), "context={context:?}");
        }
    }

    stop_daemon(&home);
}

fn daemon_log(home: &Home) -> String {
    std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
}
