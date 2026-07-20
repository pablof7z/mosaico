use super::*;
use crate::test_env::EnvGuard;

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

#[test]
fn discovers_claude_and_codex_from_authoritative_metadata() {
    let home = tempfile::tempdir().unwrap();
    let id = "same-native-id";
    let mut env = EnvGuard::set("HOME", home.path());
    env.set_var("CODEX_HOME", home.path().join("codex-home"));
    write(
        &home
            .path()
            .join(format!(".claude/projects/repo/{id}.jsonl")),
        &format!(r#"{{"sessionId":"{id}","cwd":"/work/claude"}}"#),
    );
    write(
        &home.path().join(format!(
            "codex-home/sessions/2026/07/20/rollout-now-{id}.jsonl"
        )),
        &format!(r#"{{"type":"session_meta","payload":{{"id":"{id}","cwd":"/work/codex"}}}}"#),
    );

    let found = discover(id).unwrap();
    assert_eq!(found.len(), 2);
    assert!(found.contains(&NativeSession {
        harness: crate::session::Harness::ClaudeCode,
        cwd: Some(PathBuf::from("/work/claude")),
    }));
    assert!(found.contains(&NativeSession {
        harness: crate::session::Harness::Codex,
        cwd: Some(PathBuf::from("/work/codex")),
    }));
}

#[test]
fn discovers_grok_and_opencode_without_id_shape_guessing() {
    let home = tempfile::tempdir().unwrap();
    let id = "not-a-uuid";
    let mut env = EnvGuard::set("HOME", home.path());
    env.set_var("GROK_HOME", home.path().join("grok-home"));
    env.set_var("XDG_DATA_HOME", home.path().join("data-home"));
    write(
        &home
            .path()
            .join("grok-home/sessions/repo/not-a-uuid/summary.json"),
        r#"{"info":{"id":"not-a-uuid","cwd":"/work/grok"}}"#,
    );
    let database = home.path().join("data-home/opencode/opencode.db");
    std::fs::create_dir_all(database.parent().unwrap()).unwrap();
    let connection = Connection::open(&database).unwrap();
    connection
        .execute(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT NOT NULL)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO session (id, directory) VALUES (?1, ?2)",
            [id, "/work/opencode"],
        )
        .unwrap();
    drop(connection);

    let found = discover(id).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].harness, crate::session::Harness::Grok);
    assert_eq!(found[1].harness, crate::session::Harness::Opencode);
}

#[test]
fn unrelated_uuid_shaped_files_do_not_match() {
    let home = tempfile::tempdir().unwrap();
    let _env = EnvGuard::set("HOME", home.path());
    write(
        &home
            .path()
            .join(".claude/projects/repo/not-the-request.jsonl"),
        r#"{"sessionId":"different","cwd":"/work"}"#,
    );

    assert!(discover("019f7f5c-575d-7640-958d-e7428d4d77b0")
        .unwrap()
        .is_empty());
}
