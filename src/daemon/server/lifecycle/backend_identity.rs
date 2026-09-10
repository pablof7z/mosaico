//! Load the configured backend identity used by product call sites.

use anyhow::{Context, Result};
use nostr::key::Keys;

use crate::config::{self, Config};

pub(in crate::daemon::server) fn load_backend() -> Result<(Config, Keys)> {
    config::ensure_attachment_receive_directory().context("ensuring attachmentReceiveDirectory")?;
    let mut cfg = Config::load().context("loading config")?;
    let backend_nsec = match cfg.backend_nsec().filter(|key| !key.trim().is_empty()) {
        Some(key) => key.clone(),
        None => {
            let key = config::ensure_mosaico_private_key()
                .context("ensuring stable backend identity for NIP-42")?;
            cfg = Config::load().context("reloading config with backend identity")?;
            key
        }
    };
    let keys =
        Keys::parse(&backend_nsec).context("mosaicoPrivateKey is not a valid Nostr secret key")?;
    Ok((cfg, keys))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_upserts_missing_attachment_directory_without_touching_unknown_fields() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join(".mosaico");
        std::fs::create_dir_all(&home).unwrap();
        let path = home.join("config.json");
        let key = Keys::generate().secret_key().to_secret_hex();
        std::fs::write(
            &path,
            serde_json::json!({
                "relay": "wss://relay.example",
                "mosaicoPrivateKey": key,
                "unknown": {"keep": true},
            })
            .to_string(),
        )
        .unwrap();
        let mut env = crate::test_env::EnvGuard::set("MOSAICO_HOME", &home);
        env.set_var("MOSAICO_CONFIG", &path);

        let (config, _) = load_backend().unwrap();
        let persisted: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();

        assert_eq!(
            config.attachment_receive_directory,
            home.join("tmp/attachments")
        );
        assert_eq!(persisted["unknown"]["keep"], true);
        assert_eq!(
            persisted["attachmentReceiveDirectory"],
            home.join("tmp/attachments").display().to_string()
        );
        assert!(!config.attachment_receive_directory.exists());
    }
}
