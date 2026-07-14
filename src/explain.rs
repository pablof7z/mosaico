//! `explain <handle>` query engine: point at an artifact, get what produced it.
//!
//! The store is opened only by the daemon, so the CLI reaches this through the
//! `explain` RPC; the engine itself is a pure `&Store` query so it is unit-testable
//! against a temp DB. It parses a `scheme:value` handle, then joins the
//! `receipts` and `llm_calls` ledgers:
//!
//! For a published kind:30315 (`event:<id>`) it finds the status receipt by
//! `artifact_ref`, reads the `window_hash` threaded into its `changed_summary`,
//! and rejoins the exact `llm_calls` row. For an inbox event id, the same handle
//! returns delivery receipts explaining inject/defer/retry decisions.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::state::llm_calls::LlmCallRow;
use crate::state::receipts::ReceiptRow;
use crate::state::Store;

/// How many rows a fallback scan (surface-scoped lookups) will examine.
const SCAN_LIMIT: u32 = 500;

/// A parsed `scheme:value` explain handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Handle {
    /// A published nostr event id (e.g. a kind:30315).
    Event(String),
    /// An `llm_calls` row id.
    Llm(i64),
    /// A session, optionally at a timestamp (`session:<id>@<ts>`).
    Session { id: String, at: Option<i64> },
    /// A hook-context render for a session, optionally at a timestamp.
    Hook { id: String, at: Option<i64> },
    /// A reconciler transaction on a surface (`txn:<surface>:<id>[@<ts>]`).
    Txn {
        surface: String,
        id: i64,
        at: Option<i64>,
    },
    /// A subscription channel (`sub:<channel>`).
    Sub { channel: String },
}

/// Parse a `scheme:value` handle. Event ids and session ids never contain a
/// colon, so a single `split_once(':')` cleanly separates the scheme.
pub fn parse_handle(s: &str) -> Result<Handle> {
    let (scheme, value) = s
        .split_once(':')
        .with_context(|| format!("handle must be scheme:value, got `{s}`"))?;
    let value = value.trim();
    Ok(match scheme {
        "event" => Handle::Event(value.to_string()),
        "llm" => Handle::Llm(value.parse().context("llm handle id must be an integer")?),
        "session" => {
            let (id, at) = split_at(value)?;
            Handle::Session { id, at }
        }
        "hook" => {
            let (id, at) = split_at(value)?;
            Handle::Hook { id, at }
        }
        "txn" => {
            let (surface, raw_id) = value
                .split_once(':')
                .context("txn handle must be txn:<surface>:<id>")?;
            let (id, at) = split_at(raw_id)?;
            Handle::Txn {
                surface: surface.to_string(),
                id: id.parse().context("txn id must be an integer")?,
                at,
            }
        }
        "sub" => Handle::Sub {
            channel: value.to_string(),
        },
        other => bail!("unknown handle scheme `{other}` (event|llm|session|hook|txn|sub)"),
    })
}

/// Split `id[@ts]` into its parts.
fn split_at(value: &str) -> Result<(String, Option<i64>)> {
    match value.split_once('@') {
        Some((id, ts)) => Ok((
            id.to_string(),
            Some(ts.parse().context("@<ts> must be unix millis")?),
        )),
        None => Ok((value.to_string(), None)),
    }
}

/// Resolve a handle against the ledgers into a joined record. This is the query
/// layer the RPC calls; the CLI renders the returned value (human or `--json`).
pub fn explain(store: &Store, handle: &Handle) -> Result<Value> {
    match handle {
        Handle::Event(id) => {
            let receipts = store.receipts_by_artifact_ref_prefix(id)?;
            let llm = receipts
                .iter()
                .find_map(|r| join_llm(store, r).transpose())
                .transpose()?;
            Ok(record("event", receipts, llm))
        }
        Handle::Llm(id) => {
            let call = store.get_llm_call(*id)?;
            let receipts = match &call {
                Some(c) => receipts_for_window(store, &c.window_hash)?,
                None => Vec::new(),
            };
            Ok(record("llm", receipts, call))
        }
        Handle::Session { id, at } => {
            let call = match at {
                Some(ts) => store.find_llm_call_near(id, *ts)?,
                None => store.latest_llm_calls_for_pubkey(id, 1)?.into_iter().next(),
            };
            let receipts = match &call {
                Some(c) => receipts_for_window(store, &c.window_hash)?,
                None => Vec::new(),
            };
            Ok(record("session", receipts, call))
        }
        Handle::Hook { id, at } => {
            let rows = match at {
                Some(ts) => store
                    .find_hook_receipt_for_pubkey_near(id, *ts)?
                    .into_iter()
                    .collect(),
                None => store.latest_hook_receipts_for_pubkey(id, 1)?,
            };
            Ok(record("hook", rows, None))
        }
        Handle::Txn { surface, id, at } => {
            let mut rows = store.receipts_for_surface_transaction(surface, *id)?;
            select_near(&mut rows, *at);
            if at.is_some() {
                rows.truncate(1);
            }
            Ok(record("txn", rows, None))
        }
        Handle::Sub { channel } => {
            let rows: Vec<ReceiptRow> = store
                .latest_receipts_for_surface("subscriptions", SCAN_LIMIT)?
                .into_iter()
                .filter(|r| r.commands.contains(channel) || r.changed_summary.contains(channel))
                .collect();
            Ok(record("sub", rows.into_iter().take(1).collect(), None))
        }
    }
}

/// Join a status receipt to the `llm_calls` row that produced it via the
/// `window_hash` threaded into its `changed_summary`. Non-status receipts (and
/// receipts with no window hash) simply carry no LLM inputs.
fn join_llm(store: &Store, r: &ReceiptRow) -> Result<Option<LlmCallRow>> {
    let Some(wh) = window_hash_of(r) else {
        return Ok(None);
    };
    // Most recent round-trip for that window is the one that fed this publish.
    Ok(store.llm_calls_by_window_hash(&wh)?.into_iter().next_back())
}

/// Status receipts recorded for an llm_call's window (reverse of [`join_llm`]).
fn receipts_for_window(store: &Store, window_hash: &str) -> Result<Vec<ReceiptRow>> {
    Ok(store
        .latest_receipts_for_surface("status", SCAN_LIMIT)?
        .into_iter()
        .filter(|r| window_hash_of(r).as_deref() == Some(window_hash))
        .collect())
}

/// Read the `window_hash` a status receipt carries in its `changed_summary`.
fn window_hash_of(r: &ReceiptRow) -> Option<String> {
    serde_json::from_str::<Value>(&r.changed_summary)
        .ok()?
        .get("window_hash")?
        .as_str()
        .map(str::to_string)
}

/// Order rows by proximity to `at` (nearest first); by newest first when absent.
fn select_near(rows: &mut [ReceiptRow], at: Option<i64>) {
    match at {
        Some(ts) => rows.sort_by_key(|r| (r.created_at - ts).abs()),
        None => rows.sort_by_key(|r| std::cmp::Reverse(r.created_at)),
    }
}

/// Assemble the joined record the RPC returns and the CLI renders.
fn record(kind: &str, receipts: Vec<ReceiptRow>, llm: Option<LlmCallRow>) -> Value {
    json!({
        "kind": kind,
        "receipts": receipts.iter().map(receipt_json).collect::<Vec<_>>(),
        "llm_call": llm.as_ref().map(llm_json),
    })
}

/// A receipt row as plain JSON (Trellis-free; the CLI renders it).
pub fn receipt_json(r: &ReceiptRow) -> Value {
    json!({
        "id": r.id,
        "surface": r.surface,
        "transaction_id": r.transaction_id,
        "revision": r.revision,
        "changed_summary": r.changed_summary,
        "commands": r.commands,
        "artifact_ref": r.artifact_ref,
        "created_at": r.created_at,
    })
}

/// An llm_call row as plain JSON — the verbatim inputs a 30315 rejoins to.
pub fn llm_json(c: &LlmCallRow) -> Value {
    json!({
        "id": c.id,
        "pubkey": c.pubkey,
        "window_hash": c.window_hash,
        "provider": c.provider,
        "model": c.model,
        "system_prompt": c.system_prompt,
        "transcript_slice": c.transcript_slice,
        "raw_response": c.raw_response,
        "parsed_title": c.parsed_title,
        "parsed_activity": c.parsed_activity,
        "created_at": c.created_at,
    })
}

#[cfg(test)]
mod tests;
