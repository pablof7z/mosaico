//! Why fava's write store refused to open, as a condition an operator acts on.
//!
//! The one distinction that carries this module: **a superseded schema epoch
//! is fixed by deleting the store, and nothing else is.** A refused lock, an
//! unreadable path, damaged bytes — discarding the file fixes none of them.

use std::path::Path;

use fava_write_store::WriteStoreError;

/// What the write store said about the store when it refused to open it.
///
/// Three cases, not one per error: an operator's next action is the same
/// across every non-epoch refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StoreCondition {
    /// The durable bytes are not the schema epoch this build supports.
    /// Discarding the file is the only way forward.
    SupersededEpoch {
        path: String,
        expected: u64,
        found: Option<u64>,
    },
    /// Another process already owns this store — in practice, a daemon that is
    /// already running for this home.
    HeldByAnotherOwner { path: String },
    /// The store could not be opened for any other reason: a refused lock, an
    /// unresolvable path, an I/O failure, damaged bytes.
    /// Discarding the store is **not** the recovery for any of these.
    Unusable { reason: String },
}

/// Private wrapper so `of_open_error` can downcast without parsing strings.
#[derive(Debug)]
struct StoreOpenRefusal {
    condition: StoreCondition,
}

impl std::fmt::Display for StoreOpenRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.condition.summary())
    }
}

impl std::error::Error for StoreOpenRefusal {}

impl StoreCondition {
    /// Read the typed refusal out of an error [`super::NmpHost::open`] returned.
    ///
    /// `None` means the open did not fail on the store at all.
    pub(crate) fn of_open_error(error: &anyhow::Error) -> Option<Self> {
        error
            .downcast_ref::<StoreOpenRefusal>()
            .map(|r| r.condition.clone())
    }

    /// The slug emitted in the daemon's structured startup log.
    pub(crate) fn state(&self) -> &'static str {
        match self {
            Self::SupersededEpoch { .. } => "superseded-epoch",
            Self::HeldByAnotherOwner { .. } => "held-by-another-owner",
            Self::Unusable { .. } => "unusable",
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
                    "the write store at {path} {marker}, not the epoch {expected} this build \
                     supports. The store migrates nothing across an epoch and reads nothing inside \
                     a store it refused, so it cannot report what the file holds — this is not a \
                     claim the store is empty"
                )
            }
            Self::HeldByAnotherOwner { path } => {
                format!("the write store at {path} is already owned by another process")
            }
            Self::Unusable { reason } => {
                format!("the write store could not be opened: {reason}")
            }
        }
    }

    /// What to do about it, exactly.
    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::SupersededEpoch { .. } => "run `mosaico daemon reset-state \
                 --yes-i-know-this-wipes-local-state`, then `mosaico daemon restart`. The reset \
                 permanently deletes all local Mosaico runtime state for the selected instance, \
                 including session history, the relay-backed read cache, and any write the store \
                 accepted but had not published. Configuration, harness definitions, and agent \
                 profile declarations are preserved"
                .to_string(),
            Self::HeldByAnotherOwner { .. } => "one daemon per home owns the store. Stop the \
                 running owner with `mosaico daemon stop` before starting another"
                .to_string(),
            Self::Unusable { .. } => "do NOT delete the store. A fresh file fixes none of these, \
                 and against damaged bytes it destroys the only copy of writes the store accepted \
                 and has not published. Check the path, its permissions, and the disk, then start \
                 the daemon again"
                .to_string(),
        }
    }
}

/// Classify a `WriteStoreError` into a `StoreCondition` using the path context.
fn classify(error: &WriteStoreError, path: &str) -> StoreCondition {
    let reason = error.to_string();
    if reason.contains("schema version") || reason.contains("does not exist") {
        StoreCondition::SupersededEpoch {
            path: path.to_string(),
            // fava uses schema version 4; we don't know found from the error message
            expected: 4,
            found: None,
        }
    } else if reason.contains("already open") || reason.contains("exclusive lock") {
        StoreCondition::HeldByAnotherOwner {
            path: path.to_string(),
        }
    } else {
        StoreCondition::Unusable { reason }
    }
}

/// Render a `RedbWriteStore::open` refusal as the sentence whoever reads a log
/// line can act on, keeping the typed [`StoreCondition`] in the error chain so
/// callers can branch on [`StoreCondition::of_open_error`] and never on text.
pub(crate) fn opening_refused(error: WriteStoreError, path: &Path) -> anyhow::Error {
    let path_str = path.to_string_lossy().into_owned();
    let condition = classify(&error, &path_str);
    let message = format!("{}. {}", condition.summary(), condition.remedy());
    let refusal = StoreOpenRefusal { condition };
    anyhow::Error::new(refusal).context(message)
}

/// Remove one closed write store at the given path.
///
/// The caller owns the human confirmation and must hold the selected daemon's
/// startup lock across this call.
pub(crate) fn reset(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path).map_err(|error| {
        anyhow::anyhow!("resetting the write store at {}: {error}", path.display())
    })
}

#[cfg(test)]
#[path = "store/tests.rs"]
mod tests;
