use crate::daemon_harness::shared_nip29_relay_url;
use crate::fava_client::FavaRelayClient;
use mosaico::fabric::nip29::wire::KIND_PROFILE;
use nostr::event::{EventBuilder, FinalizeEvent as _, Kind, Tag};
use nostr::key::Keys;

pub(super) async fn publish_profile(keys: &Keys, name: &str, agent_slug: &str) {
    let client = FavaRelayClient::connect(keys.clone(), &shared_nip29_relay_url())
        .await
        .expect("connect Fava relay client");
    let builder = EventBuilder::new(
        Kind::from(KIND_PROFILE),
        serde_json::json!({ "name": name }).to_string(),
    )
    .tags([Tag::parse(["agent-slug", agent_slug]).unwrap()]);
    let signed = builder.finalize(keys).expect("sign profile");
    let out = client.send_event(&signed).await.expect("publish profile");
    assert!(
        !out.success.is_empty(),
        "profile publish rejected: success={:?} failed={:?}",
        out.success,
        out.failed
    );
}
