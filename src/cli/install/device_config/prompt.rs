use super::document::{
    normalize_label, normalize_pubkeys, normalize_relay, normalize_relays, normalize_secret,
    split_csv,
};
use super::LOCAL_RELAY_URL;
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

    let local = current.relays == [LOCAL_RELAY_URL];
    let public = current.relays == [crate::config::DEFAULT_RELAY];
    let relay_choice = Select::new()
        .with_prompt("Fabric relay")
        .items(&[
            "Mosaico public relay",
            "Bundled local relay",
            "Custom relay URL(s)",
        ])
        .default(if public {
            0
        } else if local {
            1
        } else {
            2
        })
        .interact()?;
    let relays = match relay_choice {
        0 => vec![crate::config::DEFAULT_RELAY.to_string()],
        1 => vec![LOCAL_RELAY_URL.to_string()],
        _ => {
            let raw = Input::<String>::new()
                .with_prompt("Relay URL(s), comma-separated")
                .with_initial_text(current.relays.join(","))
                .interact_text()?;
            normalize_relays(&split_csv(&raw))?
        }
    };
    let indexer = Input::<String>::new()
        .with_prompt("Profile indexer relay")
        .with_initial_text(current.indexer_relay)
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
    object.insert("relays".into(), json!(relays));
    object.insert("indexerRelay".into(), json!(normalize_relay(&indexer)?));
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
