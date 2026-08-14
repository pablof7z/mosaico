use std::collections::{BTreeSet, VecDeque};
use std::sync::Mutex;

use anyhow::Result;
use nmp::nip29::GroupSnapshot;
use nmp::{AccessContext, AcquisitionEvidence, Row, SourceEvidence, SourceStatus};
use nostr::Event;

use super::read::{BoundedRead, BoundedReadTermination};
use super::NmpHost;

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
    ///
    /// There is deliberately no scripted success: acceptance is NMP writing
    /// the write down, and the id Mosaico gets back comes out of NMP's own
    /// publish queue. A faked acceptance would have to fake that queue too,
    /// which is how a test ends up asserting against a second implementation
    /// of the thing it is testing.
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

impl NmpHost {
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
        self.script_read(
            events,
            SourceStatus::FinishedStoredEvents,
            BoundedReadTermination::RelaySettled,
        );
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
                "scripted NMP group snapshot id {:?} does not match requested group {group:?}",
                snapshot.id
            ))
        })
    }

    fn script_read(
        &self,
        events: Vec<Event>,
        status: SourceStatus,
        termination: BoundedReadTermination,
    ) {
        let relay =
            nmp::RelayUrl::parse("wss://scripted-read.example").expect("static scripted relay");
        let rows = events
            .into_iter()
            .map(|event| Row {
                event,
                sources: BTreeSet::from([relay.clone()]),
            })
            .collect();
        let evidence = vec![AcquisitionEvidence {
            sources: vec![SourceEvidence {
                relay,
                access: AccessContext::Public,
                reconciled_through: None,
                status,
            }],
            shortfall: Vec::new(),
        }];
        self.test_io
            .reads
            .lock()
            .unwrap()
            .push_back(ReadResult::Snapshot(BoundedRead {
                rows,
                evidence,
                termination,
            }));
    }
}
