//! Typed projection of the current generated Codex app-server turn schema.

use std::collections::HashSet;

use super::super::protocol::RpcErrorObject;
use super::super::transport::RpcError;
use super::outcome::{sanitize_error, TurnFailure, TurnOutcome};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireTurnEnvelope {
    thread_id: String,
    turn: WireTurn,
}

#[derive(Debug, serde::Deserialize)]
struct WireStarted {
    turn: WireTurn,
}

#[derive(Debug, serde::Deserialize)]
struct WireThreadRead {
    thread: WireThread,
}

#[derive(Debug, serde::Deserialize)]
struct WireThread {
    id: String,
    turns: Vec<WireTurn>,
}

#[derive(Debug, serde::Deserialize)]
struct WireTurn {
    id: String,
    status: WireTurnStatus,
    #[serde(default)]
    error: Option<WireTurnError>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum WireTurnStatus {
    Completed,
    Interrupted,
    Failed,
    InProgress,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireTurnError {
    message: String,
    #[serde(default)]
    additional_details: Option<String>,
}

pub(super) enum ObservedTurn {
    InProgress,
    Terminal(TurnOutcome),
}

pub(super) type TurnBaseline = HashSet<String>;

pub(super) fn parse_started(
    thread_id: &str,
    baseline: &TurnBaseline,
    value: serde_json::Value,
) -> Result<(String, ObservedTurn), RpcError> {
    let started: WireStarted = decode("turn/start response", value)?;
    let turn_id = started.turn.id.clone();
    if baseline.contains(&turn_id) {
        return Err(protocol_error(format!(
            "turn/start returned pre-existing turn {turn_id}"
        )));
    }
    let observed = observe_turn(thread_id, &turn_id, started.turn)?;
    Ok((turn_id, observed))
}

pub(super) fn parse_thread_started(
    value: serde_json::Value,
) -> Result<(String, TurnBaseline), RpcError> {
    let started: WireThreadRead = decode("thread/start response", value)?;
    let thread_id = started.thread.id;
    let baseline = started
        .thread
        .turns
        .into_iter()
        .map(|turn| turn.id)
        .collect();
    Ok((thread_id, baseline))
}

pub(super) fn parse_completed(
    thread_id: &str,
    turn_id: &str,
    value: serde_json::Value,
) -> Result<ObservedTurn, RpcError> {
    let completed: WireTurnEnvelope = decode("turn/completed params", value)?;
    if completed.thread_id != thread_id {
        return Err(protocol_error(format!(
            "turn/completed thread mismatch: expected {thread_id}, got {}",
            completed.thread_id
        )));
    }
    observe_turn(thread_id, turn_id, completed.turn)
}

pub(super) fn parse_completed_after(
    thread_id: &str,
    baseline: &TurnBaseline,
    value: serde_json::Value,
) -> Result<Option<(String, ObservedTurn)>, RpcError> {
    let completed: WireTurnEnvelope = decode("turn/completed params", value)?;
    if completed.thread_id != thread_id {
        return Err(protocol_error(format!(
            "turn/completed thread mismatch: expected {thread_id}, got {}",
            completed.thread_id
        )));
    }
    observe_new_turn(thread_id, baseline, completed.turn)
}

pub(super) fn parse_thread_read(
    thread_id: &str,
    turn_id: &str,
    value: serde_json::Value,
) -> Result<Option<ObservedTurn>, RpcError> {
    let read: WireThreadRead = decode("thread/read response", value)?;
    if read.thread.id != thread_id {
        return Err(protocol_error(format!(
            "thread/read id mismatch: expected {thread_id}, got {}",
            read.thread.id
        )));
    }
    read.thread
        .turns
        .into_iter()
        .find(|turn| turn.id == turn_id)
        .map(|turn| observe_turn(thread_id, turn_id, turn))
        .transpose()
}

pub(super) fn parse_turn_baseline(
    thread_id: &str,
    value: serde_json::Value,
) -> Result<TurnBaseline, RpcError> {
    let read = decode_thread_read(thread_id, value)?;
    Ok(read.thread.turns.into_iter().map(|turn| turn.id).collect())
}

pub(super) fn parse_new_turn(
    thread_id: &str,
    baseline: &TurnBaseline,
    value: serde_json::Value,
) -> Result<Option<(String, ObservedTurn)>, RpcError> {
    let read = decode_thread_read(thread_id, value)?;
    let mut new_turns = read
        .thread
        .turns
        .into_iter()
        .filter(|turn| !baseline.contains(&turn.id));
    let Some(turn) = new_turns.next() else {
        return Ok(None);
    };
    if new_turns.next().is_some() {
        return Err(protocol_error(
            "thread/read exposed multiple turns after one turn/start; refusing to guess"
                .to_string(),
        ));
    }
    observe_new_turn(thread_id, baseline, turn)
}

fn decode_thread_read(
    thread_id: &str,
    value: serde_json::Value,
) -> Result<WireThreadRead, RpcError> {
    let read: WireThreadRead = decode("thread/read response", value)?;
    if read.thread.id != thread_id {
        return Err(protocol_error(format!(
            "thread/read id mismatch: expected {thread_id}, got {}",
            read.thread.id
        )));
    }
    Ok(read)
}

fn observe_new_turn(
    thread_id: &str,
    baseline: &TurnBaseline,
    turn: WireTurn,
) -> Result<Option<(String, ObservedTurn)>, RpcError> {
    if baseline.contains(&turn.id) {
        return Ok(None);
    }
    let turn_id = turn.id.clone();
    let observed = observe_turn(thread_id, &turn_id, turn)?;
    Ok(Some((turn_id, observed)))
}

fn observe_turn(thread_id: &str, turn_id: &str, turn: WireTurn) -> Result<ObservedTurn, RpcError> {
    if turn.id != turn_id {
        return Err(protocol_error(format!(
            "turn id mismatch: expected {turn_id}, got {}",
            turn.id
        )));
    }
    let terminal = match turn.status {
        WireTurnStatus::InProgress => return Ok(ObservedTurn::InProgress),
        WireTurnStatus::Completed => TurnOutcome::Completed {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
        },
        WireTurnStatus::Interrupted => TurnOutcome::Interrupted {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
        },
        WireTurnStatus::Failed => TurnOutcome::Failed {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            error: turn.error.map(|error| TurnFailure {
                message: sanitize_error(&error.message),
                additional_details: error
                    .additional_details
                    .as_deref()
                    .map(sanitize_error)
                    .filter(|details| !details.is_empty()),
            }),
        },
    };
    Ok(ObservedTurn::Terminal(terminal))
}

fn decode<T: serde::de::DeserializeOwned>(
    label: &str,
    value: serde_json::Value,
) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|error| {
        protocol_error(format!(
            "{label} does not match the current app-server schema: {error}"
        ))
    })
}

fn protocol_error(message: String) -> RpcError {
    RpcError::Protocol(RpcErrorObject {
        code: -1,
        message,
        data: None,
    })
}
