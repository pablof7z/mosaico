use super::document::{normalize_label, normalize_pubkeys, normalize_relay, normalize_secret};
use anyhow::Result;
use dialoguer::{Confirm, Input, Password, Select};
use serde_json::{json, Value};

pub(super) fn edit_interactively(doc: &mut Value) -> Result<()> {
    let current =
        crate::config::Config::from_json_str(&doc.to_string(), &crate::config::hostname())?;
    let has_user_nsec = current.user_nsec().is_some();
    let pubkeys = Input::<String>::new()
        .with_prompt("Operator pubkeys (hex or npub, comma-separated; blank = none)")
        .with_initial_text(current.whitelisted_pubkeys.join(","))
        .allow_empty(true)
        .interact_text()?;
    let label = Input::<String>::new()
        .with_prompt("Host label")
        .with_initial_text(current.host)
        .interact_text()?;

    let relay = Input::<String>::new()
        .with_prompt("Fabric relay URL")
        .with_initial_text(current.relay)
        .interact_text()?;
    let per_session_rooms = Confirm::new()
        .with_prompt("Create a separate room for each human-started session?")
        .default(current.per_session_rooms)
        .interact()?;
    let secret_action = Select::new()
        .with_prompt("Human CLI signing key")
        .items(if has_user_nsec {
            &["Preserve existing key", "Replace key", "Remove key"][..]
        } else {
            &["Leave unset", "Set key"][..]
        })
        .default(0)
        .interact()?;

    let object = doc.as_object_mut().expect("configuration is an object");
    object.insert(
        "whitelistedPubkeys".into(),
        json!(normalize_pubkeys(&pubkeys)?),
    );
    object.insert("backendName".into(), json!(normalize_label(&label)?));
    object.insert("relay".into(), json!(normalize_relay(&relay)?));
    object.remove("relays");
    object.remove("indexerRelay");
    object.insert("perSessionRooms".into(), json!(per_session_rooms));
    match (has_user_nsec, secret_action) {
        (true, 1) | (false, 1) => {
            let secret = Password::new()
                .with_prompt("Operator nsec or hex secret")
                .with_confirmation("Confirm operator secret", "Secrets did not match")
                .interact()?;
            object.insert("userNsec".into(), json!(normalize_secret(&secret)?));
        }
        (true, 2) => {
            object.remove("userNsec");
        }
        _ => {}
    }
    Ok(())
}
