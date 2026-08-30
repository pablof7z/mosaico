//! Bounded read projections through the daemon's sole fava engine.

use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{Context, Result};
use fava::{EventValue, Observation, ObservationClosed, Query, RelayUrl};

use super::FavaHost;
use super::query::FetchFilter;
use crate::row::Row;

/// One bounded fava observation snapshot without erasing why observation stopped.
#[derive(Debug, Clone)]
pub(crate) struct BoundedRead {
    pub(crate) rows: Vec<Row>,
    pub(crate) termination: BoundedReadTermination,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BoundedReadTermination {
    /// Every planned source reached end-of-stored-events.
    RelaySettled,
    /// The caller's failure bound elapsed before relay settlement.
    TimedOut,
    /// The engine closed the observation before relay settlement.
    SubscriptionClosed,
}

impl FavaHost {
    /// Read the relay-signed records describing ONE group — kinds
    /// 39000/39001/39002 keyed on `d`.
    pub(crate) async fn fetch_group_records(
        &self,
        group: &str,
        _max_rows: usize,
        timeout: Duration,
    ) -> Result<BoundedRead> {
        let query = self.one_group_records_query(group)?;
        self.fetch_query(query, timeout).await
    }

    /// Read bounded profile state from the configured app and indexer hosts.
    pub(crate) async fn fetch_profiles(
        &self,
        filter: FetchFilter,
        _max_rows: usize,
        timeout: Duration,
    ) -> Result<BoundedRead> {
        #[cfg(test)]
        if let Some(result) = self.test_io.take_read() {
            return result;
        }
        let query = filter
            .into_query(self.profile_relays.iter().cloned())
            .context("building profile fetch query")?;
        self.fetch_query(query, timeout).await
    }

    async fn fetch_query(&self, query: Query, timeout: Duration) -> Result<BoundedRead> {
        #[cfg(test)]
        if let Some(result) = self.test_io.take_read() {
            return result;
        }
        let mut observation = self
            .fava
            .observe(query)
            .await
            .context("opening bounded fava observation")?;
        let result = tokio::time::timeout(timeout, wait_settled(&mut observation)).await;
        observation.close();
        match result {
            Ok(Ok(rows)) => Ok(BoundedRead {
                rows,
                termination: BoundedReadTermination::RelaySettled,
            }),
            Ok(Err(ObservationClosed)) => Ok(BoundedRead {
                rows: Vec::new(),
                termination: BoundedReadTermination::SubscriptionClosed,
            }),
            Err(_elapsed) => {
                // Timeout: return latest snapshot
                let snapshot = observation.current();
                let rows = snapshot_rows(&snapshot);
                Ok(BoundedRead {
                    rows,
                    termination: BoundedReadTermination::TimedOut,
                })
            }
        }
    }
}

/// Drive the observation until all relays have sent EOSE.
async fn wait_settled(observation: &mut Observation) -> Result<Vec<Row>, ObservationClosed> {
    loop {
        let snapshot = observation.changed().await?;
        if snapshot.evidence.all_relays_stored_events_complete() {
            return Ok(snapshot_rows(&snapshot));
        }
    }
}

fn snapshot_rows(snapshot: &fava::QuerySnapshot) -> Vec<Row> {
    snapshot
        .events
        .iter()
        .filter_map(|record| match record.event() {
            EventValue::Signed(e) => Some(Row { event: e.clone() }),
            EventValue::Unsigned(_) => None,
        })
        .collect()
}

/// Build a `FetchFilter` from flat slices (used by fabric callers).
pub(crate) fn filter(
    kinds: &[u16],
    authors: &[String],
    tags: &[(char, String)],
) -> Result<FetchFilter> {
    super::query::filter(kinds, authors, tags)
}

#[cfg(test)]
mod tests;
