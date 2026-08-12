use super::chat_target::resolve_chat_target;
use super::resolution::work_root_for;
use super::*;
use crate::util::CHANNEL_MESSAGE_CHAR_LIMIT;
use anyhow::bail;

mod body;
mod coaching;
mod mention_guard;
mod params;
mod react;
mod recipient;
mod recipient_notice;
mod reply;
mod self_target;
#[cfg(test)]
mod tests;
mod unhosted_coaching;
pub(in crate::daemon::server) use params::caller_params;
pub(in crate::daemon::server) use react::rpc_channel_react;
pub(in crate::daemon::server) use recipient::resolve_recipient;
use recipient::TaggedRecipient;
pub(in crate::daemon::server) use reply::rpc_channel_reply;

const COORDINATION_GUIDE: &str = "~/.agents/skills/mosaico/references/coordination-guide.md";

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct ChannelSendParams {
    message: String,
    #[serde(default)]
    attachments: Vec<crate::attachment::Attachment>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    wait_intent: bool,
}

fn parse_params(params: &serde_json::Value) -> Result<ChannelSendParams> {
    self::params::validate_send(params)?;
    serde_json::from_value(params.clone()).context("parsing channel_send params")
}

fn validate_authored_message(message: &str) -> Result<()> {
    if message.chars().count() > CHANNEL_MESSAGE_CHAR_LIMIT {
        bail!(
            "your message is too long; keep authored chat under \
             {CHANNEL_MESSAGE_CHAR_LIMIT} characters. Put detailed material in a file and send \
             it with --attach FILE. Read {COORDINATION_GUIDE}"
        );
    }
    Ok(())
}

fn prepare_outbound_message(
    message: &str,
    attachments: &[crate::attachment::Attachment],
) -> Result<String> {
    validate_authored_message(message)?;
    crate::attachment::prepare_message(message, attachments)
}

fn persist_attachment_directory(event_id: &str, persist: impl FnOnce() -> Result<bool>) {
    if let Err(error) = persist() {
        tracing::warn!(
            event_id,
            %error,
            "local attachments were copied but their directory could not be persisted; \
             continuing without a local attachment directory"
        );
    }
}

fn chat_publish_scope(
    selected_destination: &str,
    pinned_destination: Option<&str>,
    mention_channel: Option<&str>,
) -> String {
    pinned_destination
        .or(mention_channel)
        .unwrap_or(selected_destination)
        .to_string()
}

pub(in crate::daemon::server) async fn rpc_channel_send(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p = parse_params(params)?;
    let prepared_message = prepare_outbound_message(&p.message, &p.attachments)?;
    mention_guard::check(&p.message, &p.tags, p.force)?;
    let anchor = CallerAnchor::from_params(params);
    let rec = resolve_session(state, &anchor)?;
    let target = resolve_chat_target(state, &rec, p.channel.as_deref(), "channel send")?;
    // `--channel` is destination selection only. When present it pins the event
    // to that channel; it never changes caller identity or message content.
    let destination = target.channel_h;
    super::mcp_actor::ensure_membership_if_actor(state, &rec, &destination).await?;
    let pinned_destination = target.explicit.then_some(destination.clone());
    let mut tagged = Vec::new();
    for raw in &p.tags {
        let label = raw.trim().trim_start_matches('@');
        if label.is_empty() {
            bail!("tag must not be empty");
        }
        let target = state
            .with_store(|s| resolve_recipient(s, &destination, &state.host(), label))
            .with_context(|| format!("resolving --tag {raw:?}"))?;
        self_target::reject(&rec.pubkey, &target.pubkey, self_target::Action::Tag(label))?;
        let same_work_root = state.with_store(|s| -> Result<bool> {
            Ok(work_root_for(s, &destination)? == work_root_for(s, &target.channel)?)
        })?;
        if target.channel != destination && !same_work_root {
            let (target_ref, destination_ref) = state.with_store(|store| {
                (
                    channel_resolve::channel_reference_for(store, &target.channel),
                    channel_resolve::channel_reference_for(store, &destination),
                )
            });
            bail!(
                "tagged agent is in channel {}, but this chat is for channel {}",
                target_ref?,
                destination_ref?
            );
        }
        if tagged
            .iter()
            .any(|entry: &TaggedRecipient| entry.pubkey == target.pubkey)
        {
            continue;
        }
        tagged.push(TaggedRecipient {
            label: label.to_string(),
            pubkey: target.pubkey,
            channel: target.channel,
        });
    }
    let mentioned_pubkeys = tagged
        .iter()
        .map(|target| target.pubkey.clone())
        .collect::<Vec<_>>();
    let mentioned_labels = tagged
        .iter()
        .map(|target| target.label.clone())
        .collect::<Vec<_>>();
    let recipient_reminders =
        state.with_store(|store| recipient_notice::suspension_reminders(store, &tagged))?;
    let publish_scope = chat_publish_scope(
        &destination,
        pinned_destination.as_deref(),
        tagged.first().map(|target| target.channel.as_str()),
    );
    let ambient_prefix_notice = if p.tags.is_empty() && !p.force {
        let backend_pubkey = state.backend_pubkey().unwrap_or_default();
        match state.with_store(|store| {
            coaching::untagged_agent_prefix(
                store,
                &p.message,
                &publish_scope,
                &rec.pubkey,
                &backend_pubkey,
            )
        }) {
            Ok(notice) => notice,
            Err(error) => {
                tracing::debug!(%error, "optional untagged-recipient coaching unavailable");
                None
            }
        }
    } else {
        None
    };

    // The authored text limit and every label check have passed before the
    // first upload, so an overlong/unsafe request cannot orphan a Blossom blob.
    let instance = state.session_instance(&rec);
    let chat_signing_keys = state.session_signing_keys(&rec.pubkey)?;
    let relays = &state.config().relays;
    let uploaded_attachments =
        crate::attachment::upload_all(&p.attachments, relays, &state.nmp(), &chat_signing_keys)
            .await?;
    let formatted = body::format_tagged_body(&prepared_message, &tagged)?;
    let body_to_send = formatted.wire;
    let chat = ChatMessage {
        from: instance.agent_ref(),
        channel: publish_scope.clone(),
        body: body_to_send.clone(),
        mentioned_pubkeys: mentioned_pubkeys.clone(),
        attachments: uploaded_attachments.clone(),
    };
    // Keep the exact lifecycle fence through local publish acceptance. A
    // concurrent forget cannot delete the route/signing authority after this
    // check and still let the retained key publish before relay cleanup.
    let publish_fence =
        super::managed_lifecycle::lock_session_route_for_publish(state, &rec, &publish_scope)
            .await?;
    let published = publish_fence
        .publish_chat(state, &chat, &chat_signing_keys)
        .await?;
    let event_id = published.event_id;
    let local_directory = match crate::attachment_receive::copy_local(
        &state.config().attachment_receive_directory,
        &event_id,
        &p.attachments,
    ) {
        Ok(directory) => directory,
        Err(error) => {
            tracing::warn!(
                event_id,
                %error,
                "local attachment copy failed; continuing without a local attachment directory"
            );
            None
        }
    };
    // Publish acceptance is not observation. Keep only the product-local file
    // path here; the NMP Added row is the sole trigger for message routing.
    persist_attachment_directory(&event_id, || match local_directory.as_ref() {
        Some(directory) => {
            state.with_store(|store| store.set_message_attachment_dir(&event_id, directory))
        }
        None => Ok(false),
    });

    let channel_ref = state
        .with_store(|store| super::channel_resolve::channel_reference_for(store, &publish_scope))?;
    let mut coaching = Vec::new();
    if let Some(label) = formatted.stripped_label {
        coaching.push(coaching::redundant_prefix(label));
    }
    if let Some(notice) = coaching::ack_like(&formatted.message) {
        coaching.push(notice);
    }
    if let Some(notice) = ambient_prefix_notice {
        coaching.push(notice);
    }
    coaching.extend(unhosted_coaching::notices(
        state,
        &rec,
        &publish_scope,
        &mentioned_pubkeys,
        p.wait_intent,
        now_secs(),
    ));
    Ok(serde_json::json!({
        "event_id": event_id,
        "channel": channel_ref,
        "mentioned_pubkeys": mentioned_pubkeys,
        "mentioned_labels": mentioned_labels,
        "recipient_reminders": recipient_reminders,
        "coaching": coaching,
    }))
}
