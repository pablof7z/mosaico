//! Durable retry driver for orchestration and session-dispatch claims.
//!
//! Dispatch claims each target synchronously, before any await; the spawned
//! handler runs and finishes it through `claim_outcome`. A failure is stored as an
//! attempt with its own `retry_at` (see `state::event_claims`), and the demux
//! loop sleeps until the earliest stored due time. All accounting is in claim
//! rows: the driver never infers whether a handler ran. Due events are
//! re-dispatched from the current view one by one, and offline mentions from
//! their stored payload, instead of replaying the whole view. Because the due
//! time is read from the store, a restart inside a backoff still retries.

use super::*;
use nostr::event::Event;
use tokio::time::Instant;

/// Most events one timer pass re-dispatches; the rest stay due for the next.
const DUE_BATCH_LIMIT: usize = 64;
/// How long after a demux starts (startup or reload) an event absent from the
/// view is presumed not to have arrived yet rather than to be gone.
const ABSENCE_GRACE: Duration = Duration::from_secs(5 * 60);
/// First and largest pass-level backoff after a pass makes no progress.
const STALL_BASE_BACKOFF: Duration = Duration::from_secs(30);
const STALL_MAX_BACKOFF: Duration = Duration::from_secs(5 * 60);

/// The demux loop's retry timer state.
pub(super) struct Driver {
    /// When this demux started (startup or config reload). Until
    /// `ABSENCE_GRACE` later the view may not hold events that still exist.
    started: Instant,
    /// Due events absent from the view during the grace window: not charged,
    /// and not arming the timer until the view changes or the grace ends.
    parked: BTreeSet<String>,
    /// Pass-level backoff after a pass that hit a store error or left its
    /// earliest due claim still due (for example writes failing on a full
    /// disk): the timer does not fire before `until`.
    floor: Option<Floor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Floor {
    until: Instant,
    escalations: u32,
}

impl Driver {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            parked: BTreeSet::new(),
            floor: None,
        }
    }

    /// Back off the whole timer: `min(30s * 2^k, 5min)` for the k-th
    /// consecutive escalation, with one warning per escalation.
    fn escalate(&mut self, why: &str) -> Instant {
        let escalations = self.floor.map_or(0, |floor| floor.escalations + 1);
        let delay =
            (STALL_BASE_BACKOFF * 2u32.saturating_pow(escalations.min(16))).min(STALL_MAX_BACKOFF);
        let until = Instant::now() + delay;
        tracing::warn!(
            escalations,
            backoff_secs = delay.as_secs(),
            why,
            "claim retry pass made no progress; backing off"
        );
        self.floor = Some(Floor { until, escalations });
        until
    }

    #[cfg(test)]
    pub(super) fn floor_until(&self) -> Option<Instant> {
        self.floor.map(|floor| floor.until)
    }

    /// A new view level may hold a parked event again, so re-arm for all.
    pub(super) fn view_changed(&mut self) {
        self.parked.clear();
    }

    fn grace_ends(&self) -> Instant {
        self.started + ABSENCE_GRACE
    }

    /// When the timer should next fire: the earliest stored due time among
    /// unparked claims, or the end of the grace window while events are
    /// parked. `None` waits for a wake.
    ///
    /// A scan error does not stall retries: it backs the timer off instead.
    pub(super) fn wait(&mut self, state: &Arc<DaemonState>) -> Option<Instant> {
        let next_due = match state.with_store(|store| store.next_claim_due_excluding(&self.parked))
        {
            Ok(next_due) => next_due
                .map(|due| Instant::now() + Duration::from_secs(due.saturating_sub(now_secs()))),
            Err(error) => {
                tracing::debug!(%error, "claim retry scan failed");
                let floor = match self.floor {
                    Some(floor) if floor.until > Instant::now() => floor.until,
                    _ => self.escalate("claim retry scan failed"),
                };
                return Some(floor);
            }
        };
        let grace = (!self.parked.is_empty()).then(|| self.grace_ends());
        let next = next_due.into_iter().chain(grace).min()?;
        Some(self.floor.map_or(next, |floor| next.max(floor.until)))
    }

    /// Re-run everything due. Offline mentions recover from their stored
    /// payload. Orchestration and session-dispatch events re-dispatch from
    /// the current view, and dispatch claims synchronously: afterwards each
    /// due claim is `processing`, owned and finished by its handler.
    ///
    /// The driver decides nothing from timing. A stored claim key that this
    /// backend's dispatch of the event does not produce can never be claimed,
    /// so it is terminal. An event absent from the view is parked during the
    /// grace window after start or reload; once the view has had that long
    /// to converge, each due pass it is still absent charges an attempt,
    /// ending terminal at the cap (see docs/daemon-design.md).
    ///
    /// A pass that hits a store error, or after which its earliest due claim
    /// is still due, made no progress and backs the timer off; a pass that
    /// made progress clears the backoff.
    pub(super) fn redispatch_due(&mut self, state: &Arc<DaemonState>) {
        let converged = Instant::now() >= self.grace_ends();
        if converged {
            self.parked.clear();
        }
        let now = now_secs();
        let before = state.with_store(|store| store.earliest_due_claim(&self.parked));
        let passed = self.pass(state, converged, now);
        let after = state.with_store(|store| store.earliest_due_claim(&self.parked));
        let stalled = match (before, passed, after) {
            (Ok(Some(before)), Ok(()), Ok(Some(after))) => before.due <= now && before == after,
            (Ok(_), Ok(()), Ok(_)) => false,
            _ => true,
        };
        if stalled {
            self.escalate("a store error, or the earliest due claim is still due after the pass");
        } else {
            self.floor = None;
        }
    }

    /// One pass. A store error on one event is logged and skipped so it cannot
    /// block the rest; the pass still reports it, which backs the timer off.
    fn pass(&mut self, state: &Arc<DaemonState>, converged: bool, now: u64) -> Result<()> {
        offline_mention::drive_retries(state);
        let due = state.with_store(|store| store.due_driven_claim_events(now, &self.parked))?;
        let mut had_error = false;
        let view = state.nip29_view().current();
        let mut dispatched = 0;
        for event_id in due {
            let short = &event_id[..event_id.len().min(8)];
            let Some(event) = view
                .content()
                .iter()
                .find(|event| event.id.to_hex() == event_id)
            else {
                if converged {
                    if let Err(error) =
                        state.with_store(|store| store.charge_absent_driven_claims(&event_id, now))
                    {
                        tracing::error!(event_id = %short, %error, "charging an absent claim failed");
                        had_error = true;
                    }
                } else {
                    tracing::debug!(event_id = %short, "due claim retry waits for its event to reach the view");
                    self.parked.insert(event_id);
                }
                continue;
            };
            if dispatched == DUE_BATCH_LIMIT {
                break;
            }
            if let Err(error) = fail_foreign_claims(state, event, &event_id) {
                tracing::error!(event_id = %short, %error, "checking foreign claim keys failed");
                had_error = true;
                continue;
            }
            tracing::debug!(event_id = %short, "re-dispatching due claim retry");
            chat_ops::dispatch(state, event);
            dispatched += 1;
        }
        if had_error {
            anyhow::bail!("a store error during the pass");
        }
        Ok(())
    }
}

/// Terminally fail the event's open claims whose keys this backend's dispatch
/// would never produce (for example a key naming a previous backend identity).
fn fail_foreign_claims(state: &Arc<DaemonState>, event: &Event, event_id: &str) -> Result<()> {
    let expected = chat_ops::driven_claim_keys(state, event);
    state.with_store(|store| {
        for key in store.open_driven_claim_keys(event_id)? {
            if !expected.contains(&key) {
                store.fail_orchestration_target(
                    event_id,
                    &key,
                    now_secs(),
                    "claim key is not one this backend's dispatch produces",
                )?;
            }
        }
        Ok::<_, anyhow::Error>(())
    })
}

/// Sleep until `due`, or forever when nothing is due.
pub(super) async fn sleep_until(due: Option<Instant>) {
    match due {
        Some(due) => tokio::time::sleep_until(due).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
#[path = "claim_retries/tests.rs"]
mod tests;
