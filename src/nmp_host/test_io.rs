use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;

use anyhow::Result;
use nmp::WriteStatus;
use nostr::Event;

use super::NmpHost;

struct ScriptedError {
    context: String,
    detail: String,
}

enum WriteResult {
    Statuses(Vec<WriteStatus>),
    Error(ScriptedError),
}

enum ReadResult {
    Events(Vec<Event>),
}

#[derive(Default)]
pub(super) struct TestIo {
    writes: Mutex<VecDeque<WriteResult>>,
    reads: Mutex<VecDeque<ReadResult>>,
}

impl TestIo {
    pub(super) fn take_write(&self) -> Option<Result<Receiver<WriteStatus>>> {
        let scripted = self.writes.lock().unwrap().pop_front()?;
        Some(match scripted {
            WriteResult::Statuses(statuses) => {
                let (sender, receiver) = mpsc::channel();
                for status in statuses {
                    sender.send(status).expect("scripted receipt receiver");
                }
                Ok(receiver)
            }
            WriteResult::Error(error) => Err(anyhow::anyhow!(error.detail).context(error.context)),
        })
    }

    pub(super) fn take_read(&self) -> Option<Result<Vec<Event>>> {
        let scripted = self.reads.lock().unwrap().pop_front()?;
        Some(match scripted {
            ReadResult::Events(events) => Ok(events),
        })
    }
}

impl NmpHost {
    pub(crate) fn script_write_statuses(&self, statuses: Vec<WriteStatus>) {
        self.test_io
            .writes
            .lock()
            .unwrap()
            .push_back(WriteResult::Statuses(statuses));
    }

    pub(crate) fn script_write_error(&self, context: &str, detail: &str) {
        self.test_io
            .writes
            .lock()
            .unwrap()
            .push_back(WriteResult::Error(ScriptedError {
                context: context.into(),
                detail: detail.into(),
            }));
    }

    pub(crate) fn script_read_events(&self, events: Vec<Event>) {
        self.test_io
            .reads
            .lock()
            .unwrap()
            .push_back(ReadResult::Events(events));
    }

    pub(crate) fn wait_background_receipts(&self) {
        self.background_receipts.wait_idle();
    }
}
