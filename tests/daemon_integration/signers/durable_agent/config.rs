use super::*;

fn path(home: &Home, slug: &str) -> std::path::PathBuf {
    home.dir.path().join("agents").join(format!("{slug}.json"))
}

pub(super) fn configure_durable_agent(home: &Home, slug: &str) -> String {
    mosaico::identity::load_or_create(home.dir.path(), slug, "codex", None, 1).unwrap();
    let keys = nostr::Keys::generate();
    let mut config = read_agent_config(home, slug);
    config["perSessionKey"] = serde_json::json!(false);
    config["secret_key"] = serde_json::json!(keys.secret_key().to_secret_hex());
    config["public_key"] = serde_json::json!(keys.public_key().to_hex());
    write_agent_config(home, slug, &config);
    keys.public_key().to_hex()
}

pub(super) fn read_agent_config(home: &Home, slug: &str) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path(home, slug)).unwrap()).unwrap()
}

pub(super) fn write_agent_config(home: &Home, slug: &str, config: &serde_json::Value) {
    std::fs::write(
        path(home, slug),
        serde_json::to_string_pretty(config).unwrap(),
    )
    .unwrap();
}

pub(super) fn lease_count(db: &rusqlite::Connection, pubkey: &str) -> u64 {
    db.query_row(
        "SELECT COUNT(*) FROM handle_leases WHERE pubkey=?1",
        [pubkey],
        |row| row.get(0),
    )
    .unwrap()
}
