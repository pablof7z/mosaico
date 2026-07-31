use super::*;

mod list;

// ── channels (NIP-29 subgroup task rooms) ────────────────────────────────────

pub async fn channels(action: ChannelAction) -> Result<()> {
    fn with_session(mut params: serde_json::Value, session: Option<&str>) -> serde_json::Value {
        if let Some(session) = session.filter(|s| !s.is_empty()) {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("session".into(), serde_json::json!(session));
            }
        }
        params
    }
    match action {
        ChannelAction::Add(args) => return super::channel_add::channel_add(args).await,
        ChannelAction::Read {
            id,
            since,
            limit,
            offset,
            tail,
            live,
            channel,
            session,
        } => {
            crate::cli::messaging::channel_read(crate::cli::messaging::ChannelReadRequest {
                id,
                since,
                limit,
                offset,
                tail,
                live,
                channel,
                session,
            })
            .await?;
        }
        ChannelAction::Search(args) => crate::cli::search::channel_search(args).await?,
        ChannelAction::Send {
            message,
            message_flag,
            attachments,
            tags,
            force,
            channel,
            session,
            wait,
        } => {
            let message =
                crate::cli::messaging::resolve_send_message_body(message_flag.or(message))?;
            let attachments = crate::attachment::canonicalize(attachments)?;
            crate::cli::messaging::channel_send(crate::cli::messaging::ChannelSendRequest {
                message,
                attachments,
                tags,
                force,
                channel,
                session,
                wait,
            })
            .await?;
        }
        ChannelAction::Reply {
            id,
            message,
            message_flag,
            attachments,
            session,
        } => {
            let message =
                crate::cli::messaging::resolve_send_message_body(message_flag.or(message))?;
            let attachments = crate::attachment::canonicalize(attachments)?;
            crate::cli::messaging::channel_reply(id, message, attachments, session).await?;
        }
        ChannelAction::React { id, emoji, session } => {
            crate::cli::messaging::channel_react(id, emoji, session).await?;
        }
        ChannelAction::Create {
            path,
            about,
            agents,
            session,
        } => {
            return super::channel_create::channel_create(path, about, agents, session).await;
        }
        ChannelAction::Edit {
            channel,
            about,
            session,
        } => {
            let v = daemon_call_async(
                "channel_edit",
                crate::cli::rpc_params(with_session(
                    serde_json::json!({
                        "channel": channel.clone(),
                        "about": about.clone(),
                    }),
                    session.as_deref(),
                )),
            )
            .await?;
            let event_id = v["event_id"].as_str().unwrap_or("");
            let suffix = if event_id.is_empty() {
                String::new()
            } else {
                format!(": {}", &event_id[..event_id.len().min(8)])
            };
            println!(
                "updated channel {}{suffix}",
                v["channel"].as_str().unwrap_or(&channel)
            );
        }
        ChannelAction::Init { force } => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let (slug, path) = crate::workspace::register_workspace(&cwd, force)?;
            let response = daemon_call_async(
                "channel_init",
                crate::cli::rpc_params(serde_json::json!({
                    "channel": crate::channel_ref::format_channel_ref(&slug, &[]),
                    "path": path,
                })),
            )
            .await?;
            let workspace = crate::console_style::paint_stdout_workspace(&slug, &slug);
            println!(
                "initialized channel {} at {}",
                response["channel"].as_str().unwrap_or(&workspace),
                response["path"].as_str().unwrap_or_default()
            );
        }
        ChannelAction::List {
            workspace,
            all,
            recursive,
        } => {
            list::run(workspace, all, recursive).await?;
        }
        ChannelAction::Join { channel, session } => {
            let v = daemon_call_async(
                "channel_join",
                crate::cli::rpc_params(with_session(
                    serde_json::json!({ "channel": channel.clone() }),
                    session.as_deref(),
                )),
            )
            .await?;
            println!(
                "joined channel {}",
                v["channel"].as_str().unwrap_or(&channel)
            );
            if let Some(notice) = v["history_notice"]
                .as_str()
                .filter(|notice| !notice.is_empty())
            {
                println!("{notice}");
            }
        }
        ChannelAction::Leave { channel, session } => {
            let v = daemon_call_async(
                "channel_leave",
                crate::cli::rpc_params(with_session(
                    serde_json::json!({ "channel": channel.clone() }),
                    session.as_deref(),
                )),
            )
            .await?;
            println!("left channel {}", v["channel"].as_str().unwrap_or(&channel));
        }
        ChannelAction::Archive { channel, session } => {
            let v = daemon_call_async(
                "channel_archive",
                crate::cli::rpc_params(with_session(
                    serde_json::json!({ "channel": channel.clone() }),
                    session.as_deref(),
                )),
            )
            .await?;
            let removed = v["removed_members"].as_u64().unwrap_or(0);
            println!(
                "archived channel {} (removed {} non-admin member(s))",
                v["channel"].as_str().unwrap_or(&channel),
                removed
            );
        }
    }
    Ok(())
}
