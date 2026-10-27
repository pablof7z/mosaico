use super::*;
use nostr::prelude::EventId;
use nostr_sdk::error::ErrorKind;

#[derive(Debug)]
pub(crate) enum PublishError {
    Rejected { event_id: EventId, message: String },
    Unconfirmed { event_id: EventId, reason: String },
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected { event_id, message } => {
                write!(formatter, "relay rejected event {event_id}: {message}")
            }
            Self::Unconfirmed { event_id, reason } => write!(
                formatter,
                "relay acceptance was not confirmed for event {event_id}: {reason}"
            ),
        }
    }
}

impl std::error::Error for PublishError {}

impl NostrRuntime {
    pub(crate) async fn publish(&self, event: &Event) -> Result<EventId, PublishError> {
        let relay = self
            .client
            .relay(self.relay.clone())
            .await
            .map_err(|error| PublishError::Unconfirmed {
                event_id: event.id,
                reason: error.to_string(),
            })?;
        let Some(relay) = relay else {
            return Err(PublishError::Unconfirmed {
                event_id: event.id,
                reason: "configured relay is absent".to_string(),
            });
        };
        match relay
            .send_event(event)
            .ok_timeout(self.send_timeout)
            .authentication_timeout(self.send_timeout)
            .await
        {
            Ok(output) if output.id() == &event.id && output.status().is_ack() => Ok(event.id),
            Ok(output) => Err(PublishError::Unconfirmed {
                event_id: event.id,
                reason: format!("unexpected SDK result for {}", output.id()),
            }),
            Err(error) if error.kind() == ErrorKind::Rejected => Err(PublishError::Rejected {
                event_id: event.id,
                message: bounded_message(&error.to_string()),
            }),
            Err(error) => Err(PublishError::Unconfirmed {
                event_id: event.id,
                reason: bounded_message(&error.to_string()),
            }),
        }
    }
}

fn bounded_message(message: &str) -> String {
    const MAX_CHARS: usize = 256;
    message.chars().take(MAX_CHARS).collect()
}
