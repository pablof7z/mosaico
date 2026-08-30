use std::collections::VecDeque;
use std::sync::Mutex;

use anyhow::Result;
use nostr::event::Event;

use crate::group_snapshot::GroupSnapshot;
use crate::row::Row;

use super::read::{BoundedRead, BoundedReadTermination};
use super::FavaHost;

struct ScriptedError {
    context: String,
    detail: String,
}

enum ReadResult {
    Snapshot(BoundedRead),
}

#[derive(Default)]
pub(super) struct TestIo {
    writes: Mutex<VecDeque<ScriptedError>>,
    reads: Mutex<VecDeque<ReadResult>>,
    group_snapshots: Mutex<VecDeque<GroupSnapshot>>,
}

impl TestIo {
    /// A scripted REFUSAL at the publish door, and nothing else.
    pub(super) fn take_write(&self) -> Option<Result<()>> {
        let scripted = self.writes.lock().unwrap().pop_front()?;
        Some(Err(
            anyhow::anyhow!(scripted.detail).context(scripted.context)
        ))
    }

    pub(super) fn take_read(&self) -> Option<Result<BoundedRead>> {
        let scripted = self.reads.lock().unwrap().pop_front()?;
        Some(match scripted {
            ReadResult::Snapshot(snapshot) => Ok(snapshot),
        })
    }
}

impl FavaHost {
    pub(crate) fn script_write_error(&self, context: &str, detail: &str) {
        self.test_io
            .writes
            .lock()
            .unwrap()
            .push_back(ScriptedError {
                context: context.into(),
                detail: detail.into(),
            });
    }

    pub(crate) fn script_read_settled_events(&self, events: Vec<Event>) {
        let rows = events.into_iter().map(|event| Row { event }).collect();
        self.test_io
            .reads
            .lock()
            .unwrap()
            .push_back(ReadResult::Snapshot(BoundedRead {
                rows,
                termination: BoundedReadTermination::RelaySettled,
            }));
    }

    pub(crate) fn script_group_snapshot(&self, snapshot: GroupSnapshot) {
        self.test_io
            .group_snapshots
            .lock()
            .unwrap()
            .push_back(snapshot);
    }

    pub(crate) fn take_scripted_group_snapshot(
        &self,
        group: &str,
    ) -> Option<Result<GroupSnapshot>> {
        let snapshot = self.test_io.group_snapshots.lock().unwrap().pop_front()?;
        Some(if snapshot.id == group {
            Ok(snapshot)
        } else {
            Err(anyhow::anyhow!(
                "scripted group snapshot id {:?} does not match requested group {group:?}",
                snapshot.id
            ))
        })
    }
}
