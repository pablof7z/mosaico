use super::Nip29Provider;
use crate::domain::DomainEvent;
use crate::fabric::nip29::wire::Nip29WireCodec;
use nostr::prelude::{Filter, Kind, PublicKey};
use std::time::Duration;

const PROFILE_FETCH_TIMEOUT: Duration = Duration::from_secs(4);
const PROFILE_READ_DEMAND: &str = "mosaico-profile-read";

impl Nip29Provider {
    /// Resolve one exact-author kind:0 through a bounded product read.
    pub(crate) async fn fetch_profile(&self, pubkey: &str) -> Option<crate::domain::Profile> {
        let author = PublicKey::from_hex(pubkey).ok()?;
        let _serial = self.profile_read.lock().await;
        self.runtime
            .replace_demand(
                PROFILE_READ_DEMAND,
                vec![Filter::new().kind(Kind::Metadata).author(author).limit(1)],
            )
            .await
            .ok()?;
        let result = async {
            self.runtime
                .wait_for_eose(PROFILE_READ_DEMAND, PROFILE_FETCH_TIMEOUT)
                .await
                .ok()?;
            self.runtime
                .snapshot()
                .await
                .ok()?
                .events
                .iter()
                .find_map(|event| {
                    let DomainEvent::Profile(profile) = Nip29WireCodec.decode_event(event)? else {
                        return None;
                    };
                    (profile.agent.pubkey == pubkey).then_some(profile)
                })
        }
        .await;
        let _ = self.runtime.close_demand(PROFILE_READ_DEMAND).await;
        result
    }
}
