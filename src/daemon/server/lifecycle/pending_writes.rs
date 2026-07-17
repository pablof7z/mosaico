//! Drain writes preserved while schema 7 hands durable publication to NMP.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use nostr_sdk::prelude::{Event, JsonUtil};

use crate::nmp_host::NmpHost;
pub(super) fn spawn(state_db: &Path, nmp: &Arc<NmpHost>) {
    let state_db = state_db.to_path_buf();
    let nmp = nmp.clone();
    tokio::spawn(async move {
        loop {
            match drain_once(&state_db, &nmp).await {
                Ok(Drain::Complete { imported }) => {
                    if imported > 0 {
                        tracing::info!(imported, "schema migration pending writes imported");
                    }
                    return;
                }
                Ok(Drain::Remaining {
                    imported,
                    count,
                    error,
                }) => {
                    tracing::warn!(
                        imported,
                        remaining = count,
                        error,
                        "schema migration pending writes retained for retry"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        error = %format!("{error:#}"),
                        "schema migration pending-write journal could not be drained"
                    );
                }
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

enum Drain {
    Complete {
        imported: usize,
    },
    Remaining {
        imported: usize,
        count: usize,
        error: String,
    },
}

async fn drain_once(state_db: &Path, nmp: &NmpHost) -> Result<Drain> {
    let rows = crate::state::load_pending_writes(state_db)?;
    if rows.is_empty() {
        return Ok(Drain::Complete { imported: 0 });
    }
    let mut remaining = Vec::new();
    let mut imported = 0;
    let mut last_error = String::new();
    for (index, event_json) in rows.iter().enumerate() {
        let event = match Event::from_json(event_json) {
            Ok(event) => event,
            Err(error) => {
                last_error = format!("invalid signed event: {error}");
                remaining.push(event_json.clone());
                continue;
            }
        };
        let result = if has_group_tag(&event) {
            nmp.enqueue_group_event(&event).map(|_| ())
        } else if event.kind.as_u16() == 0 {
            nmp.enqueue_profile_event(&event).map(|_| ())
        } else {
            Err(anyhow::anyhow!(
                "schema-7 pending event {} has neither one h tag nor profile kind",
                event.id
            ))
        };
        match result {
            Ok(()) => imported += 1,
            Err(error) => {
                last_error = format!("{error:#}");
                remaining.extend(rows[index..].iter().cloned());
                break;
            }
        }
    }
    crate::state::replace_pending_writes(state_db, &remaining)
        .context("updating pending-write migration journal")?;
    if remaining.is_empty() {
        Ok(Drain::Complete { imported })
    } else {
        Ok(Drain::Remaining {
            imported,
            count: remaining.len(),
            error: last_error,
        })
    }
}

fn has_group_tag(event: &Event) -> bool {
    event.tags.iter().any(|tag| {
        let fields = tag.as_slice();
        (fields.first().map(String::as_str) == Some("h"))
            && fields.get(1).is_some_and(|group| !group.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag};

    #[test]
    fn signed_single_and_multi_group_events_are_migration_writes() {
        for groups in [["one", ""], ["one", "two"]] {
            let tags = groups
                .into_iter()
                .filter(|group| !group.is_empty())
                .map(|group| Tag::parse(["h", group]).unwrap());
            let event = EventBuilder::new(Kind::TextNote, "migration")
                .tags(tags)
                .sign_with_keys(&Keys::generate())
                .unwrap();
            assert!(has_group_tag(&event));
        }
    }

    #[test]
    fn non_group_events_are_not_sent_through_nmp_group_routing() {
        let event = EventBuilder::new(Kind::Metadata, "{}")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(!has_group_tag(&event));
    }
}
