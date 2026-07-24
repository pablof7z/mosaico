use super::document::{
    normalize_label, normalize_pubkeys, normalize_relay, normalize_relays, normalize_secret,
    split_csv,
};
use super::LOCAL_RELAY_URL;
use anyhow::{Context as _, Result};
use dialoguer::{Confirm, Input, Password, Select};
use nostr_sdk::{Keys, PublicKey, ToBech32 as _};
use owo_colors::OwoColorize;
use serde_json::{json, Value};

const RELAY_CHOICES: [&str; 2] = [
    "Start a private fabric on this computer",
    "Connect to an existing fabric",
];
const OPERATOR_CHOICES: [&str; 3] = [
    "Create a new identity on this computer",
    "Use an existing secret key (nsec or hex)",
    "Use an existing public key (npub or hex; CLI stays read-only)",
];

pub(super) fn onboard_interactively(doc: &mut Value) -> Result<()> {
    println!("\n{}", "Welcome to Mosaico".bold());
    println!("Connect the agent apps on this computer so they can see and reach each other.");
    println!("You can change advanced network and identity settings later with `mosaico setup`.\n");

    prompt_for_fabric(doc)?;
    prompt_for_operator(doc)?;

    if Confirm::new()
        .with_prompt("Customize advanced settings?")
        .default(false)
        .interact()?
    {
        edit_fresh_advanced(doc)?;
    }
    Ok(())
}

pub(super) fn edit_interactively(doc: &mut Value) -> Result<()> {
    let current =
        crate::config::Config::from_json_str(&doc.to_string(), &crate::config::hostname())?;
    let has_user_nsec = current.user_nsec().is_some();

    prompt_for_fabric(doc)?;
    let pubkeys = Input::<String>::new()
        .with_prompt("Allowed human identities (npub or hex, comma-separated)")
        .with_initial_text(current.whitelisted_pubkeys.join(","))
        .allow_empty(true)
        .interact_text()?;
    let label = Input::<String>::new()
        .with_prompt("Host label")
        .with_initial_text(current.host)
        .interact_text()?;

    let indexer = Input::<String>::new()
        .with_prompt("Public-profile lookup relay")
        .with_initial_text(current.indexer_relay)
        .interact_text()?;
    let per_session_rooms = Confirm::new()
        .with_prompt("Give each human-started session its own channel?")
        .default(current.per_session_rooms)
        .interact()?;
    let secret_action = Select::new()
        .with_prompt("Local human identity")
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

fn prompt_for_fabric(doc: &mut Value) -> Result<()> {
    let current =
        crate::config::Config::from_json_str(&doc.to_string(), &crate::config::hostname())?;
    let relay_choice = Select::new()
        .with_prompt("Where should your fabric live?")
        .items(&RELAY_CHOICES)
        .default(relay_choice_default(&current.relays))
        .interact()?;
    let relays = match relay_choice {
        0 => vec![LOCAL_RELAY_URL.to_string()],
        _ => {
            let raw = Input::<String>::new()
                .with_prompt("Existing relay URL(s), comma-separated")
                .with_initial_text(
                    current
                        .relays
                        .iter()
                        .filter(|relay| relay.as_str() != LOCAL_RELAY_URL)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(","),
                )
                .interact_text()?;
            normalize_relays(&split_csv(&raw))?
        }
    };
    doc.as_object_mut()
        .expect("configuration is an object")
        .insert("relays".into(), json!(relays));
    Ok(())
}

fn prompt_for_operator(doc: &mut Value) -> Result<()> {
    let choice = Select::new()
        .with_prompt("How should Mosaico identify you?")
        .items(&OPERATOR_CHOICES)
        .default(0)
        .interact()?;
    match choice {
        0 => {
            let keys = Keys::generate();
            let secret = keys.secret_key().to_bech32()?;
            set_primary_operator(doc, keys.public_key(), Some(secret));
        }
        1 => {
            let secret = Password::new()
                .with_prompt("Existing operator nsec or hex secret")
                .interact()?;
            let secret = normalize_secret(&secret)?;
            let keys = Keys::parse(&secret).context("invalid operator secret key")?;
            set_primary_operator(doc, keys.public_key(), Some(secret));
        }
        _ => {
            let public = Input::<String>::new()
                .with_prompt("Existing operator npub or hex public key")
                .interact_text()?;
            let public = PublicKey::parse(public.trim()).context("invalid operator public key")?;
            set_primary_operator(doc, public, None);
        }
    }
    Ok(())
}

fn set_primary_operator(doc: &mut Value, public: PublicKey, secret: Option<String>) {
    let object = doc.as_object_mut().expect("configuration is an object");
    object.insert("whitelistedPubkeys".into(), json!([public.to_hex()]));
    match secret {
        Some(secret) => {
            object.insert("userNsec".into(), json!(secret));
        }
        None => {
            object.remove("userNsec");
        }
    }
}

fn edit_fresh_advanced(doc: &mut Value) -> Result<()> {
    let current =
        crate::config::Config::from_json_str(&doc.to_string(), &crate::config::hostname())?;
    let additional = Input::<String>::new()
        .with_prompt("Additional human identities (npub or hex, comma-separated; optional)")
        .allow_empty(true)
        .interact_text()?;
    let label = Input::<String>::new()
        .with_prompt("Device name")
        .with_initial_text(current.host)
        .interact_text()?;
    let indexer = Input::<String>::new()
        .with_prompt("Public-profile lookup relay")
        .with_initial_text(current.indexer_relay)
        .interact_text()?;
    let per_session_rooms = Confirm::new()
        .with_prompt("Give each human-started session its own channel?")
        .default(current.per_session_rooms)
        .interact()?;

    let mut operators = current.whitelisted_pubkeys;
    operators.extend(normalize_pubkeys(&additional)?);
    operators.sort();
    operators.dedup();
    let object = doc.as_object_mut().expect("configuration is an object");
    object.insert("whitelistedPubkeys".into(), json!(operators));
    object.insert("backendName".into(), json!(normalize_label(&label)?));
    object.insert("indexerRelay".into(), json!(normalize_relay(&indexer)?));
    object.insert("perSessionRooms".into(), json!(per_session_rooms));
    Ok(())
}

fn relay_choice_default(relays: &[String]) -> usize {
    if relays.is_empty() || relays == [LOCAL_RELAY_URL] {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_prompt_has_no_implicit_public_service() {
        assert_eq!(
            RELAY_CHOICES,
            [
                "Start a private fabric on this computer",
                "Connect to an existing fabric",
            ]
        );
        assert_eq!(relay_choice_default(&[]), 0);
        assert_eq!(relay_choice_default(&[LOCAL_RELAY_URL.into()]), 0);
        assert_eq!(relay_choice_default(&["wss://relay.example".into()]), 1);
    }

    #[test]
    fn generated_operator_can_own_the_local_fabric() {
        let mut doc = super::super::document::baseline_document();
        let keys = Keys::generate();
        set_primary_operator(
            &mut doc,
            keys.public_key(),
            Some(keys.secret_key().to_bech32().unwrap()),
        );
        doc.as_object_mut()
            .unwrap()
            .insert("relays".into(), json!([LOCAL_RELAY_URL]));

        super::super::document::ensure_complete(&mut doc).unwrap();
        let setup = super::super::document::summarize_document(&doc).unwrap();

        assert!(setup.local_relay);
        assert_eq!(setup.owner_pubkey, Some(keys.public_key().to_hex()));
        assert!(doc["userNsec"]
            .as_str()
            .is_some_and(|value| value.starts_with("nsec")));
    }
}
