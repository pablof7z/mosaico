use super::*;

// ── doctor ───────────────────────────────────────────────────────────────────

pub async fn doctor() -> Result<()> {
    // The daemon owns the single relay connection, so it performs the probe.
    let v = daemon_call_async("doctor", serde_json::json!({})).await?;
    print!("{}", render_doctor(&v));
    Ok(())
}

pub(super) fn render_doctor(v: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(storage) = v["storage"].as_object() {
        let home_mode = if storage_bool(storage, "tenex_edge_home_set", false) {
            "TENEX_EDGE_HOME set"
        } else {
            "default"
        };
        writeln!(
            out,
            "edge home: {} ({home_mode})",
            storage_str(storage, "edge_home")
        )
        .ok();
        writeln!(out, "config: {}", storage_str(storage, "config_path")).ok();
        writeln!(out, "socket: {}", storage_str(storage, "socket_path")).ok();
        writeln!(out, "lock: {}", storage_str(storage, "lock_path")).ok();
        writeln!(out, "state db: {}", storage_str(storage, "state_db_path")).ok();
        writeln!(
            out,
            "daemon log: {}",
            storage_str(storage, "daemon_log_path")
        )
        .ok();
        if storage_non_default_unacknowledged(storage) {
            writeln!(
                out,
                "warning: daemon is using a non-default edge home; set {}=1 to acknowledge an isolated home",
                crate::config::ISOLATED_HOME_ACK_ENV
            )
            .ok();
        }
    }
    if let Some(relays) = v["relays"].as_array() {
        let relays: Vec<&str> = relays.iter().filter_map(|r| r.as_str()).collect();
        writeln!(out, "relays: {relays:?}").ok();
    }
    if let Some(pk) = v["probe_pubkey"].as_str() {
        writeln!(out, "probe pubkey: {pk}").ok();
    }
    writeln!(out, "publish: {}", v["publish"].as_str().unwrap_or("?")).ok();
    writeln!(out, "read-back: {}", v["readback"].as_str().unwrap_or("?")).ok();
    out
}

fn storage_str<'a>(storage: &'a serde_json::Map<String, serde_json::Value>, key: &str) -> &'a str {
    storage.get(key).and_then(|v| v.as_str()).unwrap_or("?")
}

fn storage_bool(
    storage: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
) -> bool {
    storage
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

fn storage_non_default_unacknowledged(
    storage: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    !storage_bool(storage, "edge_home_is_default", true)
        && !storage_bool(storage, "isolated_home_acknowledged", false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doctor_json(
        edge_home_is_default: bool,
        isolated_home_acknowledged: bool,
    ) -> serde_json::Value {
        serde_json::json!({
            "storage": {
                "edge_home": "/tmp/te",
                "config_path": "/tmp/te/config.json",
                "socket_path": "/tmp/te/daemon.sock",
                "lock_path": "/tmp/te/daemon.lock",
                "state_db_path": "/tmp/te/state.db",
                "daemon_log_path": "/tmp/te/daemon.log",
                "tenex_edge_home_set": true,
                "edge_home_is_default": edge_home_is_default,
                "isolated_home_acknowledged": isolated_home_acknowledged
            },
            "relays": ["wss://relay.example"],
            "probe_pubkey": "abc",
            "publish": "ok",
            "readback": "ok"
        })
    }

    #[test]
    fn render_doctor_prints_storage_paths_and_home_mode() {
        let rendered = render_doctor(&doctor_json(true, false));
        assert!(rendered.contains("edge home: /tmp/te (TENEX_EDGE_HOME set)"));
        assert!(rendered.contains("config: /tmp/te/config.json"));
        assert!(rendered.contains("socket: /tmp/te/daemon.sock"));
        assert!(rendered.contains("lock: /tmp/te/daemon.lock"));
        assert!(rendered.contains("state db: /tmp/te/state.db"));
        assert!(rendered.contains("daemon log: /tmp/te/daemon.log"));
        assert!(!rendered.contains("warning:"));
    }

    #[test]
    fn render_doctor_warns_for_unacknowledged_non_default_home() {
        let rendered = render_doctor(&doctor_json(false, false));
        assert!(rendered.contains("warning: daemon is using a non-default edge home"));
        assert!(rendered.contains(crate::config::ISOLATED_HOME_ACK_ENV));
    }

    #[test]
    fn render_doctor_suppresses_warning_for_acknowledged_isolated_home() {
        let rendered = render_doctor(&doctor_json(false, true));
        assert!(!rendered.contains("warning:"));
    }
}
