use std::time::Duration;

use anyhow::Result;
use serde::Serialize;

use super::Nip29Provider;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProbeStatus {
    Verified,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProbeStep {
    pub(crate) status: ProbeStatus,
    pub(crate) summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DoctorProbe {
    pub(crate) publish: ProbeStep,
    pub(crate) readback: ProbeStep,
}

impl Nip29Provider {
    /// Active checked publish plus exact readback. Background observations are
    /// separate historical evidence, not a replacement for this current probe.
    pub(crate) async fn doctor_probe(&self) -> DoctorProbe {
        let marker = format!("mosaico-doctor-{}", crate::util::opaque_group_id());
        let group = match self.doctor_probe_group().await {
            Ok(Some(group)) => group,
            Ok(None) => return self.doctor_read_only().await,
            Err(error) => return both_failed(format!("{error:#}")),
        };
        let Some(keys) = self.management_keys() else {
            return both_failed("management signing identity is unavailable".into());
        };
        let publish = match self
            .nmp
            .publish_group_builder(doctor_probe_builder(&group, &marker), &keys, true)
            .await
        {
            Ok(id) => ProbeStep {
                status: ProbeStatus::Verified,
                summary: format!("ACK ({})", crate::util::pubkey_short(&id.to_hex())),
            },
            Err(error) => ProbeStep {
                status: ProbeStatus::Failed,
                summary: format!("{error:#}"),
            },
        };
        let filter = doctor_probe_filter(&marker);
        let readback = match self
            .nmp
            .fetch_in_group(&group, filter, 5, Duration::from_secs(5))
            .await
        {
            Ok(events) if !events.is_empty() => ProbeStep {
                status: ProbeStatus::Verified,
                summary: format!("{} event(s) with #h={group} #t={marker}", events.len()),
            },
            Ok(_) => ProbeStep {
                status: ProbeStatus::Failed,
                summary: format!("0 event(s) with #h={group} #t={marker}"),
            },
            Err(error) => ProbeStep {
                status: ProbeStatus::Failed,
                summary: format!("{error:#}"),
            },
        };
        DoctorProbe { publish, readback }
    }

    async fn doctor_probe_group(&self) -> Result<Option<String>> {
        let pubkey = self
            .management_pubkey()
            .ok_or_else(|| anyhow::anyhow!("management signing identity is unavailable"))?;
        let candidates = self.with_store(|store| store.list_channels_where_member(&pubkey))?;
        if candidates.is_empty() {
            return Ok(None);
        }
        let mut fetch_errors = Vec::new();
        for group in candidates {
            match self.fetch_group_state(&group).await {
                Ok((true, roles, members))
                    if roles.contains_key(&pubkey) || members.contains(&pubkey) =>
                {
                    return Ok(Some(group));
                }
                Ok(_) => {}
                Err(error) => fetch_errors.push(format!("{group}: {error:#}")),
            }
        }
        if !fetch_errors.is_empty() {
            anyhow::bail!(
                "could not verify an existing authorized NIP-29 group: {}",
                fetch_errors.join("; ")
            );
        }
        Ok(None)
    }

    async fn doctor_read_only(&self) -> DoctorProbe {
        let publish = ProbeStep {
            status: ProbeStatus::Skipped,
            summary: "no existing materialized NIP-29 group authorizes the management identity"
                .into(),
        };
        let readback = match self
            .nmp
            .fetch_all_group_metadata(1, Duration::from_secs(5))
            .await
        {
            Ok(events) => ProbeStep {
                status: ProbeStatus::Skipped,
                summary: format!("relay read OK ({} metadata event(s))", events.len()),
            },
            Err(error) => ProbeStep {
                status: ProbeStatus::Failed,
                summary: format!("relay read failed: {error:#}"),
            },
        };
        DoctorProbe { publish, readback }
    }
}

fn both_failed(summary: String) -> DoctorProbe {
    let step = ProbeStep {
        status: ProbeStatus::Failed,
        summary,
    };
    DoctorProbe {
        publish: step.clone(),
        readback: step,
    }
}

fn doctor_probe_builder(group: &str, marker: &str) -> nostr::EventBuilder {
    nostr::EventBuilder::new(nostr::Kind::from(1u16), format!("mosaico doctor {marker}")).tags([
        nostr::Tag::parse(["h", group]).expect("static h tag"),
        nostr::Tag::parse(["t", marker]).expect("static t tag"),
    ])
}

/// The `#h` row is deliberately absent: NMP's group read door owns it and
/// REFUSES a caller-supplied context constraint.
fn doctor_probe_filter(marker: &str) -> nmp::Filter {
    crate::nmp_host::read::filter(&[1], &[], &[('t', marker.to_string())])
        .expect("static NMP doctor filter")
}

#[cfg(test)]
mod tests {
    /// The marker is the caller's own constraint; the group scope is NMP's.
    /// Asserting two tags here would now be asserting the refusal condition —
    /// `group_demand_at` rejects a selection that already constrains `#h`.
    #[test]
    fn doctor_readback_is_scoped_to_a_unique_marker_and_never_to_h() {
        let filter = super::doctor_probe_filter("mosaico-doctor-test");
        assert_eq!(filter.tags.len(), 1);
        assert!(!filter
            .tags
            .contains_key(&nmp::IndexedTagName::new('h').unwrap()));
    }
}
