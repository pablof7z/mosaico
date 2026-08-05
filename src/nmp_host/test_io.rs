use std::collections::VecDeque;
use std::sync::Mutex;

use anyhow::Result;
use nostr::Event;

use super::NmpHost;

struct ScriptedError {
    context: String,
    detail: String,
}

enum ReadResult {
    Events(Vec<Event>),
}

#[derive(Default)]
pub(super) struct TestIo {
    writes: Mutex<VecDeque<ScriptedError>>,
    reads: Mutex<VecDeque<ReadResult>>,
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

    pub(super) fn take_read(&self) -> Option<Result<Vec<Event>>> {
        let scripted = self.reads.lock().unwrap().pop_front()?;
        Some(match scripted {
            ReadResult::Events(events) => Ok(events),
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

    pub(crate) fn script_read_events(&self, events: Vec<Event>) {
        self.test_io
            .reads
            .lock()
            .unwrap()
            .push_back(ReadResult::Events(events));
    }
}
