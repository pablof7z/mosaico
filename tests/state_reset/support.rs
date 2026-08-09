use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(super) const CONFIRM: &str = "--yes-i-know-this-wipes-local-state";

pub(super) fn selected_home(root: &Path) -> PathBuf {
    root.join(".mosaico-instances/relay1")
}

pub(super) fn pty_socket_directory(home: &Path) -> PathBuf {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in home.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from("/tmp")
        .join(format!("mosaico-pty-{uid}"))
        .join(format!("{hash:016x}"))
}

pub(super) fn command(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mosaico"))
        .args(args)
        .env("HOME", root)
        .env("MOSAICO", "relay1")
        .env_remove("MOSAICO_HOME")
        .env_remove("MOSAICO_CONFIG")
        .output()
        .expect("run exact Mosaico binary")
}

pub(super) fn command_with_paths(root: &Path, home: &Path, config: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mosaico"))
        .args(args)
        .env("HOME", root)
        .env_remove("MOSAICO")
        .env("MOSAICO_HOME", home)
        .env("MOSAICO_CONFIG", config)
        .output()
        .expect("run exact Mosaico binary with explicit storage paths")
}

pub(super) fn command_with_paths_and_temp(
    root: &Path,
    home: &Path,
    config: &Path,
    temp_root: &Path,
    args: &[&str],
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mosaico"))
        .args(args)
        .env("HOME", root)
        .env("TMPDIR", temp_root)
        .env_remove("MOSAICO")
        .env("MOSAICO_HOME", home)
        .env("MOSAICO_CONFIG", config)
        .output()
        .expect("run exact Mosaico binary with explicit temp and storage paths")
}

pub(super) fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("fixture file has a parent")).unwrap();
    std::fs::write(path, bytes).unwrap();
}

pub(super) fn superseded_epoch_store(path: &Path) {
    use redb::{Database, TableDefinition};

    let database = Database::create(path).expect("create retired-epoch fixture");
    let transaction = database.begin_write().expect("begin retired-epoch write");
    {
        let mut marker = transaction
            .open_table(TableDefinition::<&str, u64>::new("schema_meta_v6"))
            .expect("open retired marker table");
        marker.insert("version", 10u64).expect("write marker");
    }
    transaction.commit().expect("commit retired marker");
}

pub(super) fn seed_configuration(
    root: &Path,
    home: &Path,
    attachment_directory: &Path,
) -> Vec<(PathBuf, Vec<u8>)> {
    seed_configuration_at(root, home, &home.join("config.json"), attachment_directory)
}

pub(super) fn seed_configuration_at(
    root: &Path,
    home: &Path,
    config_path: &Path,
    attachment_directory: &Path,
) -> Vec<(PathBuf, Vec<u8>)> {
    let config = serde_json::json!({
        "whitelistedPubkeys": [],
        "backendName": "reset-contract",
        "relays": ["wss://relay.invalid"],
        "indexerRelay": "wss://relay.invalid",
        "mosaicoPrivateKey": nostr::Keys::generate().secret_key().to_secret_hex(),
        "attachmentReceiveDirectory": attachment_directory,
    })
    .to_string()
    .into_bytes();
    let files = vec![
        (config_path.to_path_buf(), config),
        (
            home.join("harnesses.json"),
            br#"{"codex":{"harness":"codex","transport":"pty"}}"#.to_vec(),
        ),
        (
            home.join("agents/writer.json"),
            br#"{"slug":"writer","harness":"codex","perSessionKey":true}"#.to_vec(),
        ),
        (
            home.join("workspaces.json"),
            br#"{"work":"/work"}"#.to_vec(),
        ),
        (
            home.join("mcp-clients.json"),
            br#"{"registered":true}"#.to_vec(),
        ),
        (home.join("operator-kept.txt"), b"keep me".to_vec()),
        (
            root.join(".codex/agents/reviewer.toml"),
            b"name = 'reviewer'".to_vec(),
        ),
    ];
    for (path, bytes) in &files {
        write(path, bytes);
    }
    files
}

pub(super) fn seed_runtime(home: &Path) {
    let store = mosaico::state::Store::open(&home.join("state.db"))
        .expect("seed a current-schema Mosaico state database");
    drop(store);
    for path in [
        "daemon.sock",
        "daemon.log",
        "sessions/one/hook-calls.jsonl",
        "pty/one.json",
        "tmp/attachments/received.bin",
        "harness-profiles/one/config.toml",
        "harness-context/goose/session.md",
        "relay-assist/transcript.json",
        "logs/group-mgmt.log",
    ] {
        write(&home.join(path), b"runtime");
    }
}
