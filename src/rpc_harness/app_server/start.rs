//! At-most-once app-server turn start and authoritative terminal observation.

use std::time::Duration;

use super::turn_protocol::{
    parse_completed, parse_completed_after, parse_new_turn, parse_started, parse_thread_read,
    parse_turn_baseline, ObservedTurn, TurnBaseline,
};
use super::{AppServerClient, TurnOutcome, RPC_TIMEOUT};
use crate::rpc_harness::transport::{RpcError, TurnObserver, TurnSignal};

const TURN_RECONCILE_INTERVAL: Duration = Duration::from_secs(5);

impl AppServerClient {
    /// Send one `turn/start`, then await exact native terminal evidence.
    pub async fn turn_start(&self, thread_id: &str, text: &str) -> Result<TurnOutcome, RpcError> {
        // This read happens before delivery. If it fails, no turn was sent and
        // the caller may safely report a pre-start rejection. Once delivery is
        // attempted, the baseline identifies the exact new turn even when the
        // immediate JSON-RPC response is lost.
        let baseline = self.thread_turn_baseline(thread_id).await?;
        let mut observer = self.handle.register_turn_waiter(thread_id)?;
        let (turn_id, observed) = self
            .start_once_and_observe(thread_id, text, &baseline, &mut observer)
            .await?;
        match observed {
            ObservedTurn::InProgress => {
                self.await_terminal(thread_id, &turn_id, &mut observer)
                    .await
            }
            ObservedTurn::Terminal(outcome) => Ok(outcome),
        }
    }

    async fn start_once_and_observe(
        &self,
        thread_id: &str,
        text: &str,
        baseline: &TurnBaseline,
        observer: &mut TurnObserver,
    ) -> Result<(String, ObservedTurn), RpcError> {
        // After this line is sent, a timeout cannot distinguish rejection from
        // acceptance. Native evidence races the exact response; no replay.
        let started = self.handle.request(
            "turn/start",
            serde_json::json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": text }]
            }),
        );
        tokio::pin!(started);
        let mut reconcile = reconciliation_clock();

        loop {
            tokio::select! {
                response = &mut started => {
                    return response.and_then(|value| parse_started(thread_id, baseline, value));
                },
                signal = observer.recv() => match signal {
                    Some(signal) => match new_turn_from_signal(thread_id, baseline, signal) {
                        Ok(Some(turn)) => return Ok(turn),
                        Ok(None) => {}
                        Err(error) => tracing::warn!(
                            thread_id,
                            %error,
                            "invalid app-server start notification; reconciling"
                        ),
                    },
                    None => return Err(RpcError::ChildExited),
                },
                _ = reconcile.tick() => {}
            }
            let read = self.thread_read_new_turn(thread_id, baseline);
            tokio::pin!(read);
            loop {
                tokio::select! {
                    response = &mut started => {
                        return response.and_then(|value| parse_started(thread_id, baseline, value));
                    },
                    signal = observer.recv() => match signal {
                        Some(signal) => match new_turn_from_signal(thread_id, baseline, signal) {
                            Ok(Some(turn)) => return Ok(turn),
                            Ok(None) => {}
                            Err(error) => tracing::warn!(
                                thread_id,
                                %error,
                                "invalid app-server start notification during reconciliation"
                            ),
                        },
                        None => return Err(RpcError::ChildExited),
                    },
                    result = &mut read => match result {
                        Ok(Some(turn)) => return Ok(turn),
                        Ok(None) => break,
                        Err(RpcError::ChildExited) => return Err(RpcError::ChildExited),
                        Err(error) => {
                            tracing::warn!(
                                thread_id,
                                %error,
                                "app-server start reconciliation failed; keeping turn active"
                            );
                            break;
                        }
                    },
                }
            }
        }
    }

    async fn await_terminal(
        &self,
        thread_id: &str,
        turn_id: &str,
        observer: &mut TurnObserver,
    ) -> Result<TurnOutcome, RpcError> {
        let mut reconcile = reconciliation_clock();
        loop {
            tokio::select! {
                signal = observer.recv() => match signal {
                    Some(signal) => match terminal_from_signal(thread_id, turn_id, signal) {
                        Ok(Some(outcome)) => return Ok(outcome),
                        Ok(None) => {}
                        Err(error) => tracing::warn!(
                            thread_id,
                            turn_id,
                            %error,
                            "invalid app-server lifecycle notification; reconciling"
                        ),
                    },
                    None => return Err(RpcError::ChildExited),
                },
                _ = reconcile.tick() => {}
            }
            if let Some(outcome) = self
                .reconcile_while_observing(thread_id, turn_id, observer)
                .await?
            {
                return Ok(outcome);
            }
        }
    }

    async fn reconcile_while_observing(
        &self,
        thread_id: &str,
        turn_id: &str,
        observer: &mut TurnObserver,
    ) -> Result<Option<TurnOutcome>, RpcError> {
        let read = self.thread_read_turn(thread_id, turn_id);
        tokio::pin!(read);
        loop {
            tokio::select! {
                result = &mut read => return match result {
                    Ok(Some(ObservedTurn::Terminal(outcome))) => Ok(Some(outcome)),
                    Ok(Some(ObservedTurn::InProgress) | None) => Ok(None),
                    Err(RpcError::ChildExited) => Err(RpcError::ChildExited),
                    Err(error) => {
                        tracing::warn!(
                            thread_id,
                            turn_id,
                            %error,
                            "app-server turn reconciliation failed; keeping turn active"
                        );
                        Ok(None)
                    }
                },
                signal = observer.recv() => match signal {
                    Some(signal) => match terminal_from_signal(thread_id, turn_id, signal) {
                        Ok(Some(outcome)) => return Ok(Some(outcome)),
                        Ok(None) => {}
                        Err(error) => tracing::warn!(
                            thread_id,
                            turn_id,
                            %error,
                            "invalid app-server lifecycle notification during reconciliation"
                        ),
                    },
                    None => return Err(RpcError::ChildExited),
                },
            }
        }
    }

    async fn thread_read_turn(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<Option<ObservedTurn>, RpcError> {
        let value = self.thread_read(thread_id).await?;
        parse_thread_read(thread_id, turn_id, value)
    }

    async fn thread_turn_baseline(&self, thread_id: &str) -> Result<TurnBaseline, RpcError> {
        if let Some(baseline) = self.handle.take_turn_baseline(thread_id) {
            return Ok(baseline);
        }
        let value = self.thread_read(thread_id).await?;
        parse_turn_baseline(thread_id, value)
    }

    async fn thread_read_new_turn(
        &self,
        thread_id: &str,
        baseline: &TurnBaseline,
    ) -> Result<Option<(String, ObservedTurn)>, RpcError> {
        let value = self.thread_read(thread_id).await?;
        parse_new_turn(thread_id, baseline, value)
    }

    async fn thread_read(&self, thread_id: &str) -> Result<serde_json::Value, RpcError> {
        self.handle
            .request_timeout(
                "thread/read",
                serde_json::json!({
                    "threadId": thread_id,
                    "includeTurns": true
                }),
                RPC_TIMEOUT,
            )
            .await
    }
}

fn terminal_from_signal(
    thread_id: &str,
    turn_id: &str,
    signal: TurnSignal,
) -> Result<Option<TurnOutcome>, RpcError> {
    let observed = match signal {
        TurnSignal::Completed(params) => parse_completed(thread_id, turn_id, params)?,
        TurnSignal::Reconcile => return Ok(None),
    };
    match observed {
        ObservedTurn::InProgress => {
            tracing::warn!(
                thread_id,
                turn_id,
                "turn/completed carried inProgress; reconciling"
            );
            Ok(None)
        }
        ObservedTurn::Terminal(outcome) => Ok(Some(outcome)),
    }
}

fn new_turn_from_signal(
    thread_id: &str,
    baseline: &TurnBaseline,
    signal: TurnSignal,
) -> Result<Option<(String, ObservedTurn)>, RpcError> {
    match signal {
        TurnSignal::Completed(params) => parse_completed_after(thread_id, baseline, params),
        TurnSignal::Reconcile => Ok(None),
    }
}

fn reconciliation_clock() -> tokio::time::Interval {
    let duration = if cfg!(test) {
        Duration::from_millis(25)
    } else {
        TURN_RECONCILE_INTERVAL
    };
    let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + duration, duration);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}
