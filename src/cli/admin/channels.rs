use super::*;

// ── channels (NIP-29 subgroup task rooms) ────────────────────────────────────

pub async fn channels(action: ChannelAction) -> Result<()> {
    fn resolve_workspace(workspace: Option<String>) -> Result<String> {
        match workspace {
            Some(p) => Ok(p),
            None => crate::daemon::workspace_path::channel_for_path_or_bail(
                &std::env::current_dir().unwrap_or_default(),
            ),
        }
    }
    fn print_ambiguous(verb: &str, channel: &str, v: &serde_json::Value) -> ! {
        let name = v["reference"].as_str().unwrap_or(channel);
        eprintln!("'{name}' is ambiguous — re-run with an exact path:");
        if let Some(refs) = v["ambiguous"].as_array() {
            for r in refs.iter().filter_map(|r| r.as_str()) {
                eprintln!("  mosaico channel {verb} {r}");
            }
        }
        std::process::exit(2);
    }
    fn shell_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
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
        ChannelAction::Send {
            message,
            message_flag,
            attachments,
            tags,
            force,
            channel,
            session,
            long_message,
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
                long_message,
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
            long_message,
        } => {
            let message =
                crate::cli::messaging::resolve_send_message_body(message_flag.or(message))?;
            let attachments = crate::attachment::canonicalize(attachments)?;
            crate::cli::messaging::channel_reply(id, message, attachments, session, long_message)
                .await?;
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
            if let Some(refs) = v["ambiguous"].as_array() {
                let name = v["reference"].as_str().unwrap_or(&channel);
                eprintln!("'{name}' is ambiguous — re-run with an exact path:");
                for r in refs.iter().filter_map(|r| r.as_str()) {
                    eprintln!(
                        "  mosaico channel edit {} --about {}",
                        shell_quote(r),
                        shell_quote(&about)
                    );
                }
                std::process::exit(2);
            }
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
            let workspace = crate::console_style::paint_stdout_workspace(&slug, &slug);
            println!("initialized workspace {workspace} at {}", path.display());
        }
        // `--all-workspaces`: every top-level workspace on the relay.
        ChannelAction::List {
            workspaces: true, ..
        } => {
            let v = daemon_call_async("root_channels", serde_json::json!({})).await?;
            let workspaces = v["channels"]
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or(&[]);
            if workspaces.is_empty() {
                println!("No NIP-29 groups found on the relay.");
                return Ok(());
            }
            let max_slug = workspaces
                .iter()
                .filter_map(|p| p["slug"].as_str())
                .map(|s| s.len())
                .max()
                .unwrap_or(0);
            for p in workspaces {
                let slug = p["slug"].as_str().unwrap_or("");
                let padded = format!("{slug:<max_slug$}");
                let slug = crate::console_style::paint_stdout_workspace(&padded, slug);
                let about = p["about"].as_str().unwrap_or("");
                if about.is_empty() {
                    println!("{slug}");
                } else {
                    println!("{slug}  — {about}");
                }
            }
        }
        ChannelAction::List { workspace, .. } => {
            use owo_colors::Stream::Stdout;
            let parent = resolve_workspace(workspace)?;
            let v =
                daemon_call_async("channel_list", serde_json::json!({ "channel": parent })).await?;
            let rooms = v["rooms"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
            // Root of the tree is the root itself. Colorize ONLY on a real
            // terminal so piped output stays literal-`^slug$`-matchable.
            let parent_label = crate::console_style::paint_stdout_workspace(&parent, &parent);
            println!("{}", parent_label.if_supports_color(Stdout, |s| s.bold()));
            if rooms.is_empty() {
                println!("  (no channels)");
                return Ok(());
            }
            for r in rooms {
                let id = r["child_h"].as_str().unwrap_or("");
                let name = r["name"].as_str().unwrap_or("");
                let about = r["about"].as_str().unwrap_or("");
                let depth = r["depth"].as_u64().unwrap_or(0) as usize;
                // depth 0 = direct child of the root channel → one level of indent.
                let indent = "  ".repeat(depth + 1);
                // Name-first: the human handle leads; the opaque id is a dimmed
                // secondary locator (shown alone only when unnamed). `about` trails.
                let suffix = if about.is_empty() || about == name {
                    String::new()
                } else {
                    format!("  — {about}")
                };
                if name.is_empty() {
                    println!(
                        "{indent}{}{suffix}",
                        id.if_supports_color(Stdout, |s| s.cyan())
                    );
                } else {
                    let name_c = name.if_supports_color(Stdout, |s| s.bold());
                    let id_c = id.if_supports_color(Stdout, |s| s.cyan());
                    println!("{indent}{name_c}  ({id_c}){suffix}");
                }
            }
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
            if v["ambiguous"].is_array() {
                print_ambiguous("join", &channel, &v);
            }
            println!(
                "joined channel {}",
                v["channel"].as_str().unwrap_or(&channel)
            );
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
            if v["ambiguous"].is_array() {
                print_ambiguous("leave", &channel, &v);
            }
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
            if v["ambiguous"].is_array() {
                print_ambiguous("archive", &channel, &v);
            }
            let removed = v["removed_members"].as_u64().unwrap_or(0);
            println!(
                "archived channel {} (removed {} non-admin member(s))",
                v["channel"].as_str().unwrap_or(&channel),
                removed
            );
        }
        ChannelAction::Switch { channel, session } => {
            let v = daemon_call_async(
                "channel_switch",
                crate::cli::rpc_params(with_session(
                    serde_json::json!({ "channel": channel.clone() }),
                    session.as_deref(),
                )),
            )
            .await?;
            // Ambiguous reference: the daemon returns the candidate paths instead
            // of switching. Print them as copy-paste-ready re-runs and exit 2 so a
            // calling agent can branch on the code without parsing prose.
            if v["ambiguous"].is_array() {
                print_ambiguous("switch", &channel, &v);
            }
            println!("switched to channel {}", channel);
        }
    }
    Ok(())
}
