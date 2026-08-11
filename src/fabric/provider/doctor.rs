use std::time::Duration;

use anyhow::Result;
use serde::Serialize;

use super::Nip29Provider;

#[path = "doctor/evidence.rs"]
mod evidence;

use evidence::ProbeStep;

const DOCTOR_PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);
const DOCTOR_PROBE_KIND: u16 = 30_078;
const DOCTOR_PROBE_COORDINATE: &str = "mosaico-doctor";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProbeStatus {
    Verified,
    Skipped,
    Failed,
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
            .publish_group_result_within(
                &group,
                doctor_probe_builder(&marker),
                &keys,
                DOCTOR_PUBLISH_TIMEOUT,
            )
            .await
        {
            Ok((event_id, result)) => evidence::publish(&event_id, result),
            Err(error) => evidence::failed(format!("{error:#}")),
        };
        let filter = doctor_probe_filter(&marker);
        let readback = match self
            .nmp
            .fetch_in_group(&group, filter, 5, Duration::from_secs(5))
            .await
        {
            Ok(read) => evidence::readback(read, true, format!("#h={group} #t={marker}")),
            Err(error) => evidence::failed(format!("{error:#}")),
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
        // `candidates` already came from the relay-signed roster the retained
        // group-records observation materialized, so membership is settled. The
        // one thing left to require is that the group's own kind:39000 was
        // observed too — a roster row for a group whose metadata never arrived
        // is not something to point a publish probe at.
        let mut read_errors = Vec::new();
        for group in candidates {
            match self.with_store(|store| store.get_channel(&group)) {
                Ok(Some(_)) => return Ok(Some(group)),
                Ok(None) => {}
                Err(error) => read_errors.push(format!("{group}: {error:#}")),
            }
        }
        if !read_errors.is_empty() {
            anyhow::bail!(
                "could not verify an existing authorized NIP-29 group: {}",
                read_errors.join("; ")
            );
        }
        Ok(None)
    }

    async fn doctor_read_only(&self) -> DoctorProbe {
        let publish = evidence::skipped(
            "no existing materialized NIP-29 group authorizes the management identity",
        );
        let readback = match self
            .nmp
            .fetch_all_group_metadata(1, Duration::from_secs(5))
            .await
        {
            Ok(read) => evidence::readback(read, false, "group metadata"),
            Err(error) => evidence::failed(format!("relay read failed: {error:#}")),
        };
        DoctorProbe { publish, readback }
    }
}

fn both_failed(summary: String) -> DoctorProbe {
    let step = evidence::failed(summary);
    DoctorProbe {
        publish: step.clone(),
        readback: step,
    }
}

/// The `#h` row is deliberately absent for the same reason it is absent from
/// [`doctor_probe_filter`]: NMP's group doors own it on both sides.
fn doctor_probe_builder(marker: &str) -> nostr::EventBuilder {
    nostr::EventBuilder::new(
        nostr::Kind::from(DOCTOR_PROBE_KIND),
        format!("mosaico doctor {marker}"),
    )
    .tags([
        nostr::Tag::parse(["d", DOCTOR_PROBE_COORDINATE]).expect("static d tag"),
        nostr::Tag::parse(["t", marker]).expect("static t tag"),
    ])
}

/// The `#h` row is deliberately absent: NMP's group read door owns it and
/// REFUSES a caller-supplied context constraint.
fn doctor_probe_filter(marker: &str) -> nmp::Filter {
    crate::nmp_host::read::filter(
        &[DOCTOR_PROBE_KIND],
        &[],
        &[
            ('d', DOCTOR_PROBE_COORDINATE.to_string()),
            ('t', marker.to_string()),
        ],
    )
    .expect("static NMP doctor filter")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nmp::Binding;

    /// The marker is the caller's own constraint; the group scope is NMP's.
    /// The fixed `d` coordinate makes repeated probes replace one another;
    /// `group_demand_at` still owns the absent `#h` constraint.
    #[test]
    fn doctor_readback_uses_one_replaceable_coordinate_and_never_constrains_h() {
        let filter = super::doctor_probe_filter("mosaico-doctor-test");
        assert_eq!(filter.kinds, Some(BTreeSet::from([30_078])));
        assert_eq!(filter.tags.len(), 2);
        assert_eq!(
            filter.tags.get(&nmp::IndexedTagName::new('d').unwrap()),
            Some(&Binding::Literal(BTreeSet::from([
                "mosaico-doctor".to_string()
            ])))
        );
        assert_eq!(
            filter.tags.get(&nmp::IndexedTagName::new('t').unwrap()),
            Some(&Binding::Literal(BTreeSet::from([
                "mosaico-doctor-test".to_string()
            ])))
        );
        assert!(!filter
            .tags
            .contains_key(&nmp::IndexedTagName::new('h').unwrap()));
    }

    #[test]
    fn every_doctor_write_uses_the_same_replaceable_coordinate() {
        let author = nostr::Keys::generate().public_key();
        let first = super::doctor_probe_builder("first").build(author);
        let second = super::doctor_probe_builder("second").build(author);
        let raw_tags = |event: &nostr::UnsignedEvent| {
            event
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect::<Vec<_>>()
        };

        assert_eq!(first.kind, nostr::Kind::from(30_078u16));
        assert_eq!(second.kind, first.kind);
        assert!(raw_tags(&first).contains(&vec!["d".into(), "mosaico-doctor".into()]));
        assert!(raw_tags(&second).contains(&vec!["d".into(), "mosaico-doctor".into()]));
        assert!(raw_tags(&first).contains(&vec!["t".into(), "first".into()]));
        assert!(raw_tags(&second).contains(&vec!["t".into(), "second".into()]));
    }
}
