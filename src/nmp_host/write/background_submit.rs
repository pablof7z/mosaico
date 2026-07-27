use super::*;

impl NmpHost {
    /// Submit a signed group event and let the fixed receipt pool retain every
    /// terminal result after this call returns.
    pub(crate) fn enqueue_group_event(&self, event: &Event) -> Result<EventId> {
        crate::relay_log::log_outgoing_event(event);
        let operation = match event.kind.as_u16() {
            crate::fabric::nip29::wire::KIND_STATUS => "status",
            7 => "reaction",
            _ => "group_event",
        };
        let intents = self.signed_group_intents(event)?;
        self.enqueue_background_intents(event.id, operation, intents)?;
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
                    durability: Durability::Durable,
                    routing: WriteRouting::PinnedHost(HostAuthority::from_selected_host(
                        relay.clone(),
                    )),
                    identity_override: Some(event.pubkey),
                },
            })
            .collect::<Vec<_>>();
        self.enqueue_background_intents(event.id, "profile", intents)?;
        Ok(event.id)
    }

    pub(super) fn submit_signed_group(&self, event: &Event) -> Result<Vec<Receiver<WriteStatus>>> {
        crate::relay_log::log_outgoing_event(event);
        let intents = self
            .signed_group_intents(event)?
            .into_iter()
            .map(|targeted| targeted.intent)
            .collect();
        self.submit_intents(intents, "submitting signed NMP write")
    }

    fn signed_group_intents(&self, event: &Event) -> Result<Vec<BackgroundIntent>> {
        let template = event_template(event)?;
        self.relays
            .iter()
            .enumerate()
            .map(|(index, relay)| {
                let mut intent = group_intent(relay.clone(), template.clone())?;
                intent.payload = WritePayload::Signed(event.clone());
                intent.identity_override = Some(event.pubkey);
                Ok(BackgroundIntent {
                    target: format!("{index}:{relay}"),
                    intent,
                })
            })
            .collect()
    }

    fn enqueue_background_intents(
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
    pub(super) receivers: Vec<(String, Receiver<WriteStatus>)>,
    pub(super) error: Option<anyhow::Error>,
}

pub(super) fn collect_background_receivers(
    intents: Vec<BackgroundIntent>,
    mut publish: impl FnMut(WriteIntent) -> Result<Receiver<WriteStatus>>,
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
