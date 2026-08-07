//! Drain writes preserved while schema 7 hands durable publication to NMP.
//!
//! These are the one class of event Mosaico still hands NMP already-signed:
//! bytes an OLDER Mosaico signed for itself, journaled across the v7 migration
//! and never published. They go out through the plain NIP-01 write door
//! ([`NmpHost::publish_signed_to`]) rather than a NIP-29 group door, because a
//! group door composes and signs -- and re-composing these bytes would change
//! the id whoever sent them already saw. Nothing Mosaico writes TODAY comes
//! through here.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use nostr::{Event, JsonUtil};

pub(super) fn spawn(state_db: &Path, state: std::sync::Arc<crate::daemon::server::DaemonState>) {
    let state_db = state_db.to_path_buf();
    tokio::spawn(async move {
        loop {
            match drain_once(&state_db, &state.nmp()).await {
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

async fn drain_once(state_db: &Path, nmp: &crate::nmp_host::NmpHost) -> Result<Drain> {
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
        // Where the bytes go, without reading a single tag out of them. A
        // kind:0 is NIP-01 metadata and belongs to the profile relays;
        // everything Mosaico has ever journaled here is a group event and
        // belongs to the group hosts. Inspecting `h` to decide would be NIP-29
        // routing logic in the app, and the routing does not need it.
        let result = if event.kind.as_u16() == 0 {
            nmp.enqueue_profile_event(&event).map(|_| ())
        } else {
            nmp.publish_signed_to(nmp.group_hosts(), &event).map(|_| ())
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
