use super::*;

pub(super) fn required(decoded: &crate::fabric::ProductDecode) -> bool {
    matches!(
        decoded.tail.as_ref(),
        Some(DomainEvent::ChatMessage(chat)) if !chat.attachments.is_empty()
    )
}

pub(super) async fn materialize(
    state: &Arc<DaemonState>,
    decoded: &crate::fabric::ProductDecode,
    event: &Event,
) {
    let Some(DomainEvent::ChatMessage(chat)) = decoded.tail.as_ref() else {
        return;
    };
    if chat.attachments.is_empty() {
        return;
    }
    let event_id = event.id.to_hex();
    let already_materialized = state.with_store(|store| {
        store
            .get_message(&event_id)
            .ok()
            .flatten()
            .is_some_and(|message| !message.attachment_dir.is_empty())
    });
    if already_materialized {
        return;
    }
    // This daemon's OWN message reaches here too: NMP injects the accepted
    // write into the subscription that feeds the demux (#1182), so the send
    // path's local copy and this path's download are two routes to one
    // directory. If the files are already on disk under this event's id, adopt
    // them rather than fetching bytes back out of Blossom.
    if let Some(directory) = crate::attachment_receive::existing_complete(
        &state.snapshot().config.attachment_receive_directory,
        &event_id,
        &chat.attachments,
    ) {
        if let Err(error) =
            state.with_store(|store| store.set_message_attachment_dir(&event_id, &directory))
        {
            tracing::warn!(
                event_id,
                %error,
                "locally copied attachments could not be recorded on the message row"
            );
        }
        return;
    }
    match crate::attachment_receive::download(
        &state.snapshot().config.attachment_receive_directory,
        &event_id,
        &chat.attachments,
    )
    .await
    {
        Ok(Some(directory)) => {
            if let Err(error) =
                state.with_store(|store| store.set_message_attachment_dir(&event_id, &directory))
            {
                tracing::warn!(
                    event_id,
                    %error,
                    "received attachments but could not persist their directory"
                );
            }
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(
            event_id,
            %error,
            "attachment download failed; delivering ordinary message without files"
        ),
    }
}
