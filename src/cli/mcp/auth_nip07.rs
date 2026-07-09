use anyhow::{Context, Result};
use nostr_sdk::prelude::{Event, JsonUtil, Kind, TagKind};

use super::auth_types::AuthorizeForm;

const NIP07_CONTENT: &str = "tenex-edge OAuth login";
const NIP07_EVENT_MAX_BYTES: usize = 8192;
const LOGIN_WINDOW_SECS: u64 = 300;
const FUTURE_SKEW_SECS: u64 = 120;

pub(super) fn pubkey_for_form(
    form: &AuthorizeForm,
    public_url: &str,
    challenge: &str,
) -> Result<String> {
    let claimed = form
        .nip07_pubkey
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .context("missing NIP-07 pubkey")?;
    let event_json = form
        .nip07_event
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .context("missing NIP-07 signed event")?;
    verify_login_event(event_json, claimed, public_url, challenge)
}

fn verify_login_event(
    event_json: &str,
    claimed_pubkey: &str,
    public_url: &str,
    challenge: &str,
) -> Result<String> {
    anyhow::ensure!(
        event_json.len() <= NIP07_EVENT_MAX_BYTES,
        "NIP-07 event too large"
    );
    let event = Event::from_json(event_json).context("invalid NIP-07 event")?;
    event.verify().context("invalid NIP-07 event signature")?;
    let pubkey = event.pubkey.to_hex();
    anyhow::ensure!(
        super::auth_support::normalize_pubkey(claimed_pubkey) == pubkey,
        "NIP-07 pubkey mismatch"
    );
    anyhow::ensure!(
        event.kind == Kind::HttpAuth,
        "NIP-07 event must be kind 27235"
    );
    anyhow::ensure!(event.content == NIP07_CONTENT, "unexpected NIP-07 content");
    verify_timestamp(event.created_at.as_secs())?;
    verify_tag(
        &event,
        TagKind::u(),
        &format!("{public_url}/oauth/authorize"),
        "u",
    )?;
    verify_tag(&event, TagKind::Method, "POST", "method")?;
    verify_tag(&event, TagKind::Challenge, challenge, "challenge")?;
    Ok(pubkey)
}

fn verify_timestamp(created_at: u64) -> Result<()> {
    let now = crate::util::now_secs();
    anyhow::ensure!(
        created_at <= now + FUTURE_SKEW_SECS,
        "NIP-07 event is from the future"
    );
    anyhow::ensure!(
        created_at + LOGIN_WINDOW_SECS >= now,
        "NIP-07 event is stale"
    );
    Ok(())
}

fn verify_tag(event: &Event, kind: TagKind<'_>, expected: &str, name: &str) -> Result<()> {
    let actual = event
        .tags
        .find(kind)
        .and_then(|tag| tag.content())
        .with_context(|| format!("missing NIP-07 {name} tag"))?;
    anyhow::ensure!(actual == expected, "invalid NIP-07 {name} tag");
    Ok(())
}

#[cfg(test)]
mod tests {
    use nostr_sdk::prelude::{EventBuilder, Keys, Tag, TagKind};

    use super::*;

    #[test]
    fn verifies_signed_login_event() {
        let keys = Keys::generate();
        let public_url = "https://edge.f7z.io";
        let challenge = "challenge-1";
        let event = login_event(&keys, public_url, challenge);
        let pubkey = verify_login_event(
            &event.as_json(),
            &keys.public_key().to_hex(),
            public_url,
            challenge,
        )
        .expect("valid NIP-07 event");
        assert_eq!(pubkey, keys.public_key().to_hex());
    }

    #[test]
    fn rejects_wrong_challenge() {
        let keys = Keys::generate();
        let public_url = "https://edge.f7z.io";
        let event = login_event(&keys, public_url, "challenge-1");
        let err = verify_login_event(
            &event.as_json(),
            &keys.public_key().to_hex(),
            public_url,
            "challenge-2",
        )
        .unwrap_err();
        assert!(err.to_string().contains("challenge"));
    }

    fn login_event(keys: &Keys, public_url: &str, challenge: &str) -> Event {
        EventBuilder::new(Kind::HttpAuth, NIP07_CONTENT)
            .tags([
                Tag::custom(TagKind::u(), [format!("{public_url}/oauth/authorize")]),
                Tag::custom(TagKind::Method, ["POST"]),
                Tag::custom(TagKind::Challenge, [challenge.to_string()]),
            ])
            .sign_with_keys(keys)
            .expect("sign event")
    }
}
