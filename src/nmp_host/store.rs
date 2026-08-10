//! Why NMP refused to open the durable store, as a condition an operator acts on.
//!
//! One distinction carries this whole module, and it is the reason NMP made it
//! a type instead of a sentence: **a superseded schema epoch is fixed by
//! deleting the store, and nothing else is.** A refused lock, an unreadable
//! path, damaged current-epoch bytes — deleting the file fixes none of them,
//! and if the bytes are merely damaged it destroys the only copy of writes NMP
//! accepted and had not yet published.
//!
//! Before `nmp` #920 both arrived here as one opaque string, so a daemon that
//! would not start said the same thing whether the operator's store was a
//! retired epoch or their disk was failing. Establishing which one it was cost
//! a read-only forensic investigation of a 1.05 GB file. Mosaico now branches
//! on [`nmp::EngineError`] and never on its text.

use std::path::Path;

use nmp::{Engine, EngineConfig, EngineError};

/// What NMP said about the store when it refused to open it.
///
/// Deliberately three cases, not one per `EngineError`: an operator's next
/// action is the same across every non-epoch refusal, and NMP's own doc for
/// `StoreOpenFailed` is the positive claim that discarding is never the
/// recovery for any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoreCondition {
    /// The durable bytes are not the schema epoch this build supports. NMP
    /// migrates nothing across an epoch and cannot read anything inside a
    /// store it refused, so the only way forward is to discard the file.
    ///
    /// `found` is `None` when the store carries no marker this build can read,
    /// which includes a marker written at an address a superseded epoch owned.
    /// **`None` means "not this epoch", never "no data"** — that is the case
    /// the incident hit, and rendering it as an empty store would be a lie
    /// that costs someone their publish queue.
    SupersededEpoch {
        path: String,
        expected: u64,
        found: Option<u64>,
    },
    /// Another engine already owns this store — in practice, a daemon that is
    /// already running for this home.
    HeldByAnotherOwner { path: String },
    /// The store could not be opened for any other reason: a refused lock, an
    /// unresolvable path, an I/O failure, damaged current-epoch bytes.
    /// Discarding the store is **not** the recovery for any of these.
    Unusable { reason: String },
}

impl StoreCondition {
    /// Read NMP's typed refusal out of an error [`super::NmpHost::open`]
    /// returned.
    ///
    /// `None` means the open did not fail on the store at all — an unparseable
    /// relay URL, a refused engine thread, identity registration — and the
    /// caller should report the error it already has.
    pub(crate) fn of_open_error(error: &anyhow::Error) -> Option<Self> {
        match error.downcast_ref::<EngineError>()? {
            EngineError::StoreUnsupportedSchema {
                path,
                expected,
                found,
            } => Some(Self::SupersededEpoch {
                path: path.clone(),
                expected: *expected,
                found: *found,
            }),
            EngineError::StoreAlreadyOpen { path } => {
                Some(Self::HeldByAnotherOwner { path: path.clone() })
            }
            EngineError::StoreOpenFailed { reason } => Some(Self::Unusable {
                reason: reason.clone(),
            }),
            // Everything else `Engine::new` can fail with is not about the
            // store. The wildcard is deliberate and its direction is the safe
            // one: a store-open variant NMP adds later reads as "not a store
            // condition", which loses a diagnosis. It can never invent one, and
            // so can never send anyone to delete a file.
            _ => None,
        }
    }

    /// The slug a machine reader branches on, and the `state` `mosaico doctor`
    /// reports.
    pub(crate) fn state(&self) -> &'static str {
        match self {
            Self::SupersededEpoch { .. } => "superseded-epoch",
            Self::HeldByAnotherOwner { .. } => "held-by-another-owner",
            Self::Unusable { .. } => "unusable",
        }
    }

    /// The store this condition is about, when NMP named one.
    pub(crate) fn path(&self) -> Option<&str> {
        match self {
            Self::SupersededEpoch { path, .. } | Self::HeldByAnotherOwner { path } => Some(path),
            Self::Unusable { .. } => None,
        }
    }

    /// The fact, in one line.
    pub(crate) fn summary(&self) -> String {
        match self {
            Self::SupersededEpoch {
                path,
                expected,
                found,
            } => {
                let marker = match found {
                    Some(found) => format!("is schema epoch {found}"),
                    None => "carries no schema marker this build can read".to_string(),
                };
                format!(
                    "the NMP store at {path} {marker}, not the epoch {expected} this build \
                     supports. NMP migrates nothing across an epoch and reads nothing inside a \
                     store it refused, so it cannot report what the file holds — this is not a \
                     claim the store is empty"
                )
            }
            Self::HeldByAnotherOwner { path } => {
                format!("the NMP store at {path} is already owned by another process")
            }
            Self::Unusable { reason } => {
                format!("the NMP store could not be opened: {reason}")
            }
        }
    }

    /// What to do about it, exactly. Mosaico never performs a reset on its own.
    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::SupersededEpoch { .. } => "run `mosaico daemon reset-state \
                 --yes-i-know-this-wipes-local-state`, then `mosaico daemon restart`. The reset \
                 permanently deletes all local Mosaico runtime state for the selected instance, \
                 including session history, the relay-backed read cache, and any write NMP \
                 accepted but had not published. Configuration, harness definitions, and agent \
                 profile declarations are preserved"
                .to_string(),
            Self::HeldByAnotherOwner { .. } => "one daemon per home owns the store. Stop the \
                 running owner with `mosaico daemon stop` before starting another"
                .to_string(),
            Self::Unusable { .. } => "do NOT delete the store. A fresh file fixes none of these, \
                 and against damaged current-epoch bytes it destroys the only copy of writes NMP \
                 accepted and has not published. Check the path, its permissions, and the disk, \
                 then start the daemon again"
                .to_string(),
        }
    }
}

/// Render an `Engine::new` refusal as the sentence whoever reads a log line
/// can act on, keeping the typed [`EngineError`] as the error's source so a
/// caller still branches on [`StoreCondition::of_open_error`] and never on this
/// text.
pub(crate) fn opening_refused(error: EngineError) -> anyhow::Error {
    let error = anyhow::Error::new(error);
    match StoreCondition::of_open_error(&error) {
        Some(condition) => {
            let context = format!("{}. {}", condition.summary(), condition.remedy());
            error.context(context)
        }
        None => error.context("starting NMP engine"),
    }
}

/// Ask the store at `path` what condition it is in, and release it again.
///
/// `None` means there is nothing to report: no store exists yet (the daemon
/// creates one on first boot), or it opened cleanly. This exists so
/// `mosaico doctor` can name why a daemon will not start **without** a daemon
/// — which is the whole situation, since a store NMP refuses is a daemon that
/// exits before it can answer an RPC.
///
/// **The caller must hold the daemon startup lock for this instance.** Asking
/// the question takes ownership of the store, and a daemon binds its socket
/// *before* it opens NMP — so an unguarded probe can win the race against a
/// daemon that is slow to start and make it exit with `StoreAlreadyOpen`. A
/// store big enough to be slow is exactly the one someone runs `doctor` at.
pub(crate) fn probe(path: &Path) -> Option<StoreCondition> {
    if !path.exists() {
        return None;
    }
    match Engine::new(probe_config(path)) {
        Ok(engine) => {
            // Hold it for as little as possible: a daemon may be starting.
            engine.shutdown();
            None
        }
        Err(EngineError::StoreUnsupportedSchema {
            path,
            expected,
            found,
        }) => Some(StoreCondition::SupersededEpoch {
            path,
            expected,
            found,
        }),
        Err(EngineError::StoreAlreadyOpen { path }) => {
            Some(StoreCondition::HeldByAnotherOwner { path })
        }
        Err(EngineError::StoreOpenFailed { reason }) => Some(StoreCondition::Unusable { reason }),
        Err(error) => Some(StoreCondition::Unusable {
            reason: error.to_string(),
        }),
    }
}

/// Remove one closed NMP store through NMP's owned destructive API.
///
/// The caller owns the human confirmation and must hold the selected daemon's
/// startup lock across this call. NMP independently refuses a store that any
/// process still owns and removes its full on-disk representation, including
/// the sidecar lock.
pub(crate) fn reset(path: &Path) -> anyhow::Result<()> {
    Engine::reset_persistent_store(path)
        .map_err(|error| anyhow::anyhow!("resetting the NMP persistent store: {error}"))
}

fn probe_config(path: &Path) -> EngineConfig {
    EngineConfig {
        store_path: Some(path.to_string_lossy().into_owned()),
        ..EngineConfig::default()
    }
}

#[cfg(test)]
#[path = "store/tests.rs"]
mod tests;
