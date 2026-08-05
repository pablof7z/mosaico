use super::*;

impl NmpHost {
    /// Submit a signed group event and let the fixed receipt pool retain every
    /// terminal result after this call returns.
    ///
    /// The `h` the event already carries is VALIDATED against `group` by the
    /// door, never appended: appending would change the bytes and therefore
    /// the id the caller already handed out. A missing, wrong or duplicated
    /// `h` is a refusal here rather than a repair.
    pub(crate) fn enqueue_group_event(&self, group: &str, event: &Event) -> Result<EventId> {
        crate::relay_log::log_outgoing_event(event);
        let operation = super::group_operation(event.kind.as_u16());
        let intent = self
            .group(group)?
            .signed_intent(event.clone())
            .map_err(|error| anyhow::anyhow!("minting a signed NIP-29 group write: {error}"))?;
        self.enqueue_background_intents(
            event.id,
            operation,
            vec![BackgroundIntent {
                target: group.to_string(),
                intent,
            }],
        )?;
        Ok(event.id)
    }

    /// Submit a signed event that is in SEVERAL groups at once.
    ///
    /// The one write NMP's group door cannot mint, and the only hand-minted
    /// `WriteIntent` left in Mosaico. `nmp::nip29::Group` is one relay scope
    /// plus one group id, so `signed_intent` refuses a second `h` row with
    /// `AmbiguousContext` -- correctly, for a door whose whole product is that
    /// an event cannot be composed under one group and routed as another.
    /// Mosaico's kind:30315 session status is genuinely in every channel the
    /// session occupies: one replaceable coordinate, one `h` per channel, and
    /// splitting it into one write per channel would have each copy replace
    /// the last. There is no multi-group scope to mint it from.
    ///
    /// Tracked upstream as pablof7z/nmp#1281. When that door exists this
    /// method is deleted, not adapted.
    pub(crate) fn enqueue_multi_group_event(&self, event: &Event) -> Result<EventId> {
        crate::relay_log::log_outgoing_event(event);
        let relays: Vec<_> = self.relays.iter().cloned().collect();
        if relays.is_empty() {
            anyhow::bail!("cannot publish a NIP-29 event without a configured group host");
        }
        let intent = WriteIntent {
            payload: WritePayload::Signed(event.clone()),
            routing: WriteRouting::Explicit(relays),
            identity: super::identity_of(Some(event.pubkey)),
            correlation: None,
        };
        self.enqueue_background_intents(
            event.id,
            super::group_operation(event.kind.as_u16()),
            vec![BackgroundIntent {
                target: "every group host".to_string(),
                intent,
            }],
        )?;
        Ok(event.id)
    }

    /// Publish a kind:0 copy to every configured app/indexer relay.
    pub(crate) fn enqueue_profile_event(&self, event: &Event) -> Result<EventId> {
        if event.kind.as_u16() != 0 {
            anyhow::bail!(
                "profile enqueue requires kind:0, got {}",
                event.kind.as_u16()
            );
        }
        let intents = self
            .profile_relays
            .iter()
            .enumerate()
            .map(|(index, relay)| BackgroundIntent {
                target: format!("{index}:{relay}"),
                intent: WriteIntent {
                    payload: WritePayload::Signed(event.clone()),
                    routing: WriteRouting::Explicit(vec![relay.clone()]),
                    identity: super::identity_of(Some(event.pubkey)),
                    correlation: None,
                },
            })
            .collect::<Vec<_>>();
        self.enqueue_background_intents(event.id, "profile", intents)?;
        Ok(event.id)
    }

    pub(super) fn enqueue_background_intents(
        &self,
        event_id: EventId,
        operation: &str,
        intents: Vec<BackgroundIntent>,
    ) -> Result<()> {
        require_configured_host_count(intents.len())?;
        let permit = self
            .background_receipts
            .reserve(operation, event_id, intents.len())?;
        let submission = collect_background_receivers(intents, |intent| {
            self.publish_intent(intent, "submitting background NMP write")
        });
        if let Some(error) = submission.error {
            self.background_receipts
                .submission_failed(operation, event_id, &error);
            if !submission.receivers.is_empty() {
                if let Err(observe_error) = self.background_receipts.observe(
                    permit,
                    operation,
                    event_id,
                    submission.receivers,
                    false,
                ) {
                    tracing::warn!(
                        operation,
                        source_ref = %event_id,
                        error = %format!("{observe_error:#}"),
                        "partial background NMP receipts could not be observed"
                    );
                }
            }
            return Err(error);
        }
        self.background_receipts
            .observe(permit, operation, event_id, submission.receivers, true)
    }

    pub(crate) fn background_write_snapshot(&self) -> BackgroundWriteSnapshot {
        self.background_receipts.snapshot()
    }
}

pub(super) struct BackgroundIntent {
    pub(super) target: String,
    pub(super) intent: WriteIntent,
}

pub(super) struct BackgroundSubmission {
    pub(super) receivers: Vec<(String, FifoReceiver<WriteFact>)>,
    pub(super) error: Option<anyhow::Error>,
}

pub(super) fn collect_background_receivers(
    intents: Vec<BackgroundIntent>,
    mut publish: impl FnMut(WriteIntent) -> Result<FifoReceiver<WriteFact>>,
) -> BackgroundSubmission {
    let mut receivers = Vec::with_capacity(intents.len());
    for targeted in intents {
        match publish(targeted.intent) {
            Ok(receiver) => receivers.push((targeted.target, receiver)),
            Err(error) => {
                return BackgroundSubmission {
                    receivers,
                    error: Some(error),
                };
            }
        }
    }
    BackgroundSubmission {
        receivers,
        error: None,
    }
}

fn require_configured_host_count(count: usize) -> Result<()> {
    if count == 0 {
        anyhow::bail!("cannot publish a NIP-29 event without a configured group host");
    }
    Ok(())
}
