use crate::daemon_harness::*;
use mosaico::daemon::client::Client;
use mosaico::state::Store;

#[test]
fn native_pre_tool_guard_warns_reads_denies_writes_and_ignores_shells() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    let workspaces = tempfile::Builder::new()
        .prefix("mosaico-boundary-")
        .tempdir_in("/var/tmp")
        .unwrap();
    let alpha = workspaces.path().join("alpha");
    let beta = workspaces.path().join("beta");
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    std::fs::write(
        home.dir.path().join("workspaces.json"),
        serde_json::json!({"alpha": alpha, "beta": beta}).to_string(),
    )
    .unwrap();
    let store = Store::open(&home.store_path()).unwrap();
    for (workspace, path) in [("alpha", &alpha), ("beta", &beta)] {
        store
            .upsert_channel(workspace, workspace, "", "", 1)
            .unwrap();
        store
            .upsert_workspace(workspace, &path.to_string_lossy(), 1)
            .unwrap();
    }
    drop(store);

    rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.unwrap();
        client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "codex",
                        "harness_session": "boundary-session",
                        "cwd": alpha,
                    }),
                    "codex",
                ),
            )
            .await
            .unwrap();
    });

    let event = |tool: &str, input: serde_json::Value| {
        serde_json::json!({
            "session_id": "boundary-session",
            "cwd": alpha,
            "tool_name": tool,
            "tool_input": input,
        })
        .to_string()
    };
    let read = run_cli_stdin(
        &home,
        &["harness", "hook", "codex", "--type", "pre-tool-use"],
        &event(
            "Read",
            serde_json::json!({"file_path": beta.join("README.md")}),
        ),
    );
    let write = run_cli_stdin(
        &home,
        &["harness", "hook", "codex", "--type", "pre-tool-use"],
        &event(
            "Write",
            serde_json::json!({"file_path": beta.join("src/lib.rs")}),
        ),
    );
    let shell = run_cli_stdin(
        &home,
        &["harness", "hook", "codex", "--type", "pre-tool-use"],
        &event(
            "Bash",
            serde_json::json!({"command": format!("cat {}/README.md", beta.display())}),
        ),
    );

    let read: serde_json::Value = serde_json::from_slice(&read.stdout).unwrap();
    let write: serde_json::Value = serde_json::from_slice(&write.stdout).unwrap();
    assert!(read["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("Mosaico workspace /beta"));
    assert_eq!(write["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(shell.stdout.is_empty(), "shell text must remain unaffected");
}

#[test]
fn unsupported_grok_pre_tool_surface_remains_unaffected() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.unwrap();
        client.call("ping", serde_json::json!({})).await.unwrap();
    });
    let output = run_cli_stdin(
        &home,
        &["harness", "hook", "grok", "--type", "pre-tool-use"],
        r#"{
          "session_id":"unknown",
          "cwd":"/workspace",
          "tool_name":"Write",
          "tool_input":{"file_path":"/another-workspace/file"}
        }"#,
    );

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}
