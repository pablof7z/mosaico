use super::*;

// ── project ──────────────────────────────────────────────────────────────────

pub(super) async fn project(action: ProjectAction) -> Result<()> {
    match action {
        ProjectAction::List => {
            let v = daemon_call_async("project_list", serde_json::json!({})).await?;
            let projects = v["projects"]
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or(&[]);
            if projects.is_empty() {
                println!("No NIP-29 groups found on the relay.");
                return Ok(());
            }
            let max_slug = projects
                .iter()
                .filter_map(|p| p["slug"].as_str())
                .map(|s| s.len())
                .max()
                .unwrap_or(0);
            for p in projects {
                let slug = p["slug"].as_str().unwrap_or("");
                let about = p["about"].as_str().unwrap_or("");
                if about.is_empty() {
                    println!("{slug}");
                } else {
                    println!("{slug:<max_slug$}  — {about}");
                }
            }
        }
        ProjectAction::Init { force } => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let (slug, path) = crate::project::register_project(&cwd, force)?;
            println!("initialized project {slug} at {}", path.display());
        }
        ProjectAction::Edit {
            description,
            project,
        } => {
            let slug = match project {
                Some(p) => p,
                None => {
                    crate::project::resolve_or_bail(&std::env::current_dir().unwrap_or_default())?
                }
            };
            let v = daemon_call_async(
                "project_edit",
                serde_json::json!({ "project": slug, "description": description }),
            )
            .await?;
            let event_id = v["event_id"].as_str().unwrap_or("?");
            println!("Updated {slug}: {}", &event_id[..event_id.len().min(8)]);
        }
        ProjectAction::Add { project, pubkey } => match pubkey {
            Some(pubkey) => {
                let project = match project {
                    Some(p) => p,
                    None => crate::project::resolve_or_bail(
                        &std::env::current_dir().unwrap_or_default(),
                    )?,
                };
                let v = daemon_call_async(
                    "project_add",
                    serde_json::json!({ "project": project, "pubkey": pubkey }),
                )
                .await?;
                let resolved = v["pubkey"].as_str().unwrap_or(&pubkey);
                println!(
                    "added {} to {}",
                    pubkey_short(resolved).cyan(),
                    project.bold()
                );
            }
            None => {
                let project = match project {
                    Some(p) => p,
                    None => crate::project::resolve_or_bail(
                        &std::env::current_dir().unwrap_or_default(),
                    )?,
                };
                super::project_agents::edit_membership(project).await?;
            }
        },
    }
    Ok(())
}

// ── channels (NIP-29 subgroup task rooms) ────────────────────────────────────

pub(super) async fn channels(action: ChannelsAction) -> Result<()> {
    fn resolve_project(project: Option<String>) -> Result<String> {
        match project {
            Some(p) => Ok(p),
            None => crate::project::resolve_or_bail(&std::env::current_dir().unwrap_or_default()),
        }
    }
    match action {
        ChannelsAction::Create {
            name,
            agents,
            project,
            message,
        } => {
            let parent = resolve_project(project)?;
            if agents.is_empty() {
                bail!("at least one --agent slug@backend is required");
            }
            // Parse each `slug@backend` on the LAST `@` (agent slugs never contain `@`).
            let mut parsed: Vec<serde_json::Value> = Vec::with_capacity(agents.len());
            for a in &agents {
                let (slug, backend) = a
                    .rsplit_once('@')
                    .filter(|(s, b)| !s.is_empty() && !b.is_empty())
                    .with_context(|| format!("malformed --agent {a:?}: expected slug@backend"))?;
                parsed.push(serde_json::json!({ "slug": slug, "backend": backend }));
            }
            let brief = match &message {
                Some(path) => std::fs::read_to_string(path)
                    .with_context(|| format!("reading --message {}", path.display()))?,
                None => String::new(),
            };
            let v = daemon_call_async(
                "channels_create",
                serde_json::json!({
                    "parent": parent,
                    "name": name,
                    "agents": parsed,
                    "brief": brief,
                    // Caller identity so the daemon auto-adds the creating agent
                    // to the new room (resolved like the messaging commands).
                    "agent": crate::cli::agent_env_slug(),
                    "channel": crate::cli::channel_env(),
                    "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
                    "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
                }),
            )
            .await?;
            let child = v["child_h"].as_str().unwrap_or("?");
            let path = v["display_path"].as_str().unwrap_or("");
            let oid = v["orchestration_event_id"].as_str().unwrap_or("");
            println!("created channel {} ({})", child.bold(), path);
            if let Some(admins) = v["admins"].as_array() {
                println!("  admins copied: {}", admins.len());
            }
            if let Some(joined) = v["creator"].as_str() {
                if !joined.is_empty() {
                    println!("  joined as {}", pubkey_short(joined).cyan());
                }
            }
            if !oid.is_empty() {
                println!("  orchestration kind:9 {}", &oid[..oid.len().min(8)]);
            }
        }
        ChannelsAction::List { project } => {
            use owo_colors::Stream::Stdout;
            let parent = resolve_project(project)?;
            let v =
                daemon_call_async("channels_list", serde_json::json!({ "project": parent })).await?;
            let rooms = v["rooms"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
            // Root of the tree is the project itself. Colorize ONLY when stdout is a
            // real terminal: piped/captured output (the e2e harness, shell
            // substitution) must be plain so callers can match the slug literally —
            // `.bold()` would otherwise wrap it in ANSI escapes and a `^slug$` grep
            // would never match.
            println!("{}", parent.if_supports_color(Stdout, |s| s.bold()));
            if rooms.is_empty() {
                println!("  (no channels)");
                return Ok(());
            }
            for r in rooms {
                let id = r["child_h"].as_str().unwrap_or("");
                let name = r["name"].as_str().unwrap_or("");
                let depth = r["depth"].as_u64().unwrap_or(0) as usize;
                // depth 0 = direct child of the project root → one level of indent.
                let indent = "  ".repeat(depth + 1);
                let id_c = id.if_supports_color(Stdout, |s| s.cyan());
                if name.is_empty() {
                    println!("{indent}{id_c}");
                } else {
                    println!("{indent}{id_c}  — {name}");
                }
            }
        }
        ChannelsAction::Switch { channel } => {
            // Update TENEX_EDGE_CHANNEL in the current tmux pane's environment
            // and call a daemon RPC to rebind this session to the new channel.
            let tmux_pane = std::env::var("TMUX_PANE").ok();
            if let Some(pane) = tmux_pane {
                std::process::Command::new("tmux")
                    .args(["set-environment", "-t", &pane, "TENEX_EDGE_CHANNEL", &channel])
                    .status()
                    .context("tmux set-environment failed")?;
            }
            match daemon_call_async(
                "channels_switch",
                serde_json::json!({ "channel": channel }),
            )
            .await
            {
                Ok(_) => {}
                Err(e) => eprintln!("warning: daemon channels_switch failed ({}); env updated", e),
            }
            println!("switched to channel {}", channel);
        }
    }
    Ok(())
}

// ── agent (local keystore) ────────────────────────────────────────────────────

pub(super) async fn agent(action: AgentAction) -> Result<()> {
    let edge_home = crate::config::edge_home();
    match action {
        AgentAction::List => {
            let rows = crate::identity::list_local_agent_details(&edge_home);
            if rows.is_empty() {
                println!("No local agents in {}", edge_home.join("agents").display());
                println!("Add one with: tenex-edge agent add <slug> [-- <command>]");
                return Ok(());
            }
            let max_slug = rows.iter().map(|r| r.slug.len()).max().unwrap_or(0);
            for r in &rows {
                let cmd = r
                    .command
                    .as_ref()
                    .map(|c| c.join(" "))
                    .unwrap_or_else(|| "(default harness)".to_string());
                println!(
                    "{:<max_slug$}  {}  {}",
                    r.slug.bold(),
                    pubkey_short(&r.pubkey).cyan(),
                    cmd.dimmed()
                );
            }
        }
        AgentAction::Add {
            slug,
            projects,
            command,
        } => {
            let command = if command.is_empty() {
                None
            } else {
                Some(command)
            };
            let (id, created) =
                crate::identity::add_local_agent(&edge_home, &slug, command, now_secs())?;
            let verb = if created { "created" } else { "updated" };
            println!(
                "{} {} {}",
                verb,
                slug.bold(),
                pubkey_short(&id.pubkey_hex()).cyan()
            );
            match &id.command {
                Some(c) => println!("  spawns: {}", c.join(" ").dimmed()),
                None => println!("  spawns: {}", "(default harness)".dimmed()),
            }
            // Publish the kind:0 identity card so the agent is discoverable on the
            // indexer relay immediately, not just after its first session. Best
            // effort: the keypair already exists locally and the session engine
            // re-publishes the same Profile on first run, so a publish failure
            // (e.g. daemon/relay down) must not fail the create.
            if created {
                publish_profile(&slug).await;
            }
            assign_to_projects(&id.pubkey_hex(), &projects).await;
        }
        AgentAction::Assign { slug, projects } => {
            let pubkey = crate::identity::list_local_agent_details(&edge_home)
                .into_iter()
                .find(|a| a.slug == slug)
                .map(|a| a.pubkey)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no such local agent: {slug} (add it with `tenex-edge agent add {slug}`)"
                    )
                })?;
            println!("{} {}", slug.bold(), pubkey_short(&pubkey).cyan());
            assign_to_projects(&pubkey, &projects).await;
        }
        AgentAction::Remove { slug } => {
            match crate::identity::remove_local_agent(&edge_home, &slug)? {
                Some(parked) => {
                    println!(
                        "removed {} (key parked at {})",
                        slug.bold(),
                        parked.display()
                    );
                }
                None => {
                    eprintln!("no such local agent: {slug}");
                }
            }
        }
    }
    Ok(())
}

/// Publish an agent's kind:0 identity card via the daemon (which owns the
/// transport pool including the indexer relay). Best effort: failures are
/// reported but never abort agent creation.
async fn publish_profile(slug: &str) {
    match daemon_call_async("publish_profile", serde_json::json!({ "slug": slug })).await {
        Ok(v) => {
            let event_id = v["event_id"].as_str().unwrap_or("?");
            let short = &event_id[..event_id.len().min(8)];
            println!("  published profile (kind:0) {}", short.dimmed());
        }
        Err(e) => eprintln!("  profile publish deferred to first session: {e}"),
    }
}

/// Add `pubkey` to each project's NIP-29 group via the daemon's `project_add`
/// RPC. Per-project failures (e.g. operator not a group admin) are reported but
/// do not abort the remaining assignments.
async fn assign_to_projects(pubkey: &str, projects: &[String]) {
    for project in projects {
        match daemon_call_async(
            "project_add",
            serde_json::json!({ "project": project, "pubkey": pubkey }),
        )
        .await
        {
            Ok(_) => println!("  assigned to {}", project.bold()),
            Err(e) => eprintln!("  failed to assign to {}: {}", project.bold(), e),
        }
    }
}

// ── doctor ───────────────────────────────────────────────────────────────────

pub(super) async fn doctor() -> Result<()> {
    // The daemon owns the single relay connection, so it performs the probe.
    let v = daemon_call_async("doctor", serde_json::json!({})).await?;
    if let Some(relays) = v["relays"].as_array() {
        let relays: Vec<&str> = relays.iter().filter_map(|r| r.as_str()).collect();
        println!("relays: {relays:?}");
    }
    if let Some(pk) = v["probe_pubkey"].as_str() {
        println!("probe pubkey: {pk}");
    }
    println!("publish: {}", v["publish"].as_str().unwrap_or("?"));
    println!("read-back: {}", v["readback"].as_str().unwrap_or("?"));
    Ok(())
}

// ── tail ─────────────────────────────────────────────────────────────────────

/// Options for the `tail` command.
pub(super) struct TailOpts {
    pub(super) project: Option<String>,
    pub(super) agent: Option<String>,
    pub(super) host: Option<String>,
    pub(super) since: Option<String>,
    pub(super) backfill: Option<u64>,
    pub(super) only: Option<String>,
    pub(super) exclude: Option<String>,
    pub(super) include: Option<String>,
    pub(super) all: bool,
    pub(super) compact: bool,
    pub(super) relative: bool,
    pub(super) no_emoji: bool,
    pub(super) no_color: bool,
    pub(super) json: bool,
    pub(super) no_follow: bool,
    pub(super) live: bool,
}

pub(super) async fn tail(opts: TailOpts) -> Result<()> {
    if opts.live {
        eprintln!(
            "tenex-edge tail --live: the full-screen TUI dashboard is not yet implemented. \
             Use bare `tenex-edge tail` for the live scrolling feed."
        );
        return Ok(());
    }

    // Resolve color + emoji settings: explicit flags override env/TTY.
    let use_color =
        !opts.no_color && std::env::var("NO_COLOR").is_err() && std::io::stdout().is_terminal();
    let use_emoji = !opts.no_emoji;

    // Parse --since into a unix timestamp.
    let since_ts: u64 = opts.since.as_deref().map(parse_since).unwrap_or(0);

    let scope_label = opts.project.as_deref().unwrap_or("*");
    if !opts.json {
        eprintln!(
            "{} tailing project {} … (Ctrl-C to stop)",
            if use_color {
                "tenex-edge".bold().to_string()
            } else {
                "tenex-edge".to_string()
            },
            if use_color {
                scope_label.cyan().to_string()
            } else {
                scope_label.to_string()
            },
        );
    }

    // Build the category filter set.
    let cats_only: Option<std::collections::HashSet<String>> = opts
        .only
        .as_deref()
        .map(|s| s.split(',').map(|c| c.trim().to_lowercase()).collect());
    let cats_exclude: std::collections::HashSet<String> = opts
        .exclude
        .as_deref()
        .map(|s| s.split(',').map(|c| c.trim().to_lowercase()).collect())
        .unwrap_or_default();
    let cats_include: std::collections::HashSet<String> = opts
        .include
        .as_deref()
        .map(|s| s.split(',').map(|c| c.trim().to_lowercase()).collect())
        .unwrap_or_default();

    // Minimum tier: default hides tier 0 (profile); --all includes all; --v same.
    let min_tier: u8 = if opts.all { 0 } else { 1 };

    let params = serde_json::json!({
        "project": opts.project,
        "backfill": opts.backfill.unwrap_or(20),
        "since": since_ts,
    });

    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;

    let agent_filter = opts.agent.clone();
    let host_filter = opts.host.clone();
    let is_json = opts.json;
    let no_follow = opts.no_follow;
    let compact = opts.compact;
    let relative = opts.relative;

    let stream = client.stream("tail", params, move |item| {
        // Deserialize TailEvent.
        let ev: crate::daemon::tail_event::TailEvent = match serde_json::from_value(item.clone()) {
            Ok(e) => e,
            Err(_) => {
                // Fallback: if we get an old {line} format, print it.
                if let Some(line) = item.get("line").and_then(|l| l.as_str()) {
                    println!("{line}");
                }
                return;
            }
        };

        // Apply agent/host filters.
        if let Some(ref ag) = agent_filter {
            let ev_agent = match &ev {
                crate::daemon::tail_event::TailEvent::Msg { from, .. } => from.as_str(),
                crate::daemon::tail_event::TailEvent::Turn { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Status { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Join { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Leave { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Sess { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Profile { agent, .. } => agent.as_str(),
                _ => "",
            };
            if !ev_agent.is_empty() && ev_agent != ag.as_str() {
                return;
            }
        }
        if let Some(ref h) = host_filter {
            let ev_host = match &ev {
                crate::daemon::tail_event::TailEvent::Join { host, .. } => host.as_str(),
                crate::daemon::tail_event::TailEvent::Leave { host, .. } => host.as_str(),
                crate::daemon::tail_event::TailEvent::Profile { host, .. } => host.as_str(),
                _ => "",
            };
            if !ev_host.is_empty() && ev_host != h.as_str() {
                return;
            }
        }

        // Tier filter.
        if ev.tier() < min_tier && !cats_include.contains(ev.category()) {
            return;
        }

        // Category filters.
        let cat = ev.category();
        if let Some(ref only) = cats_only {
            if !only.contains(cat) {
                return;
            }
        }
        if cats_exclude.contains(cat) && !cats_include.contains(cat) {
            return;
        }

        // Render.
        if is_json {
            if let Ok(s) = serde_json::to_string(&ev) {
                println!("{s}");
            }
        } else {
            let line = render_tail_event(&ev, use_color, use_emoji, relative, compact);
            println!("{line}");
        }
    });

    if no_follow {
        // For no-follow: run with a short timeout to get just the backfill.
        // The daemon will keep streaming; we disconnect after receiving the
        // initial batch. Since we can't easily detect "backfill done", we
        // use a small sleep approach: connect, get backfill, disconnect.
        tokio::select! {
            r = stream => r,
            _ = tokio::time::sleep(Duration::from_millis(500)) => Ok(()),
        }
    } else {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => Ok(()),
            r = stream => r,
        }
    }
}

/// Parse a --since value into a unix timestamp.
/// Accepts: unix seconds ("1700000000"), or durations ("1h", "30m", "2d").
pub(super) fn parse_since(s: &str) -> u64 {
    let now = now_secs();
    if let Ok(ts) = s.parse::<u64>() {
        return ts;
    }
    // Simple duration parsing: Nh, Nm, Nd, Ns.
    let s = s.trim();
    let (num_str, unit) = s.split_at(s.len().saturating_sub(1));
    if let Ok(n) = num_str.trim().parse::<u64>() {
        let secs = match unit {
            "h" | "H" => n * 3600,
            "m" | "M" => n * 60,
            "d" | "D" => n * 86400,
            "s" | "S" | _ => n,
        };
        return now.saturating_sub(secs);
    }
    0
}

/// Render a `TailEvent` to a human-readable string.
///
/// `use_color` and `use_emoji` are passed explicitly so this fn is testable
/// without side-effects from TTY detection or NO_COLOR.
pub fn render_tail_event(
    ev: &crate::daemon::tail_event::TailEvent,
    use_color: bool,
    use_emoji: bool,
    relative: bool,
    compact: bool,
) -> String {
    use crate::daemon::tail_event::TailEvent;
    use crate::util::session_codename;

    let ts = ev.ts();
    let ts_str = if relative {
        let age = now_secs().saturating_sub(ts);
        if age < 60 {
            format!("{age}s ago")
        } else if age < 3600 {
            format!("{}m ago", age / 60)
        } else {
            format!("{}h ago", age / 3600)
        }
    } else {
        // Wall-clock HH:MM:SS.
        let h = (ts % 86400) / 3600;
        let m = (ts % 3600) / 60;
        let s = ts % 60;
        format!("{h:02}:{m:02}:{s:02}")
    };

    // Helper: colorize if color enabled.
    macro_rules! col {
        ($text:expr, $color:ident) => {
            if use_color {
                $text.$color().to_string()
            } else {
                $text.to_string()
            }
        };
    }

    // Session codename helper.
    let sess_code = |sid: &str| session_codename(sid);

    match ev {
        TailEvent::Msg {
            project,
            from,
            from_session,
            to,
            to_session,
            body,
            ..
        } => {
            let cat = col!("msg  ", yellow);
            let arrow = if use_emoji { "→" } else { "->" };
            let sess = from_session
                .as_deref()
                .map(|s| format!("[{}]", sess_code(s)))
                .unwrap_or_default();
            let to_sess = to_session
                .as_deref()
                .map(|s| format!("[{}]", sess_code(s)))
                .unwrap_or_default();
            let snippet = if compact {
                String::new()
            } else {
                let body_clean: String = body.chars().take(72).collect();
                let body_clean = body_clean.replace('\n', " ");
                let ellipsis = if body.len() > 72 { "…" } else { "" };
                format!(" \"{}{}\"", body_clean, ellipsis)
            };
            format!(
                "{ts_str}  {cat}  {}@{project}{sess}  {arrow} {}{to_sess}{snippet}",
                col!(from, cyan),
                col!(to, cyan),
            )
        }

        TailEvent::Sync {
            from, to, state, ..
        } => {
            let (cat, color_fn): (&str, fn(&str) -> String) = match state.as_str() {
                "failed" => ("sync ", |s| {
                    if true {
                        s.red().to_string()
                    } else {
                        s.to_string()
                    }
                }),
                _ => ("sync ", |s| s.cyan().to_string()),
            };
            let cat_str = if use_color {
                match state.as_str() {
                    "failed" => col!(cat, red),
                    _ => col!(cat, cyan),
                }
            } else {
                cat.to_string()
            };
            let _ = color_fn; // suppress unused warning
            let glyph = if use_emoji {
                match state.as_str() {
                    "delivered" => "✓",
                    "failed" => "✗",
                    _ => "~",
                }
            } else {
                match state.as_str() {
                    "delivered" => "[ok]",
                    "failed" => "[x]",
                    _ => "~",
                }
            };
            format!("{ts_str}  {cat_str}  {from} → {to}  {glyph} {state}")
        }

        TailEvent::Turn {
            project,
            agent,
            session,
            state,
            elapsed_s,
            ..
        } => {
            let cat = col!("turn ", green);
            let sess = format!("[{}]", sess_code(session));
            let (glyph, detail) = if state == "working" {
                let g = if use_emoji { "▶" } else { ">" };
                (g, " started working".to_string())
            } else {
                let g = if use_emoji { "⏸" } else { "||" };
                let dur = elapsed_s
                    .map(|e| format!(" ({})", fmt_duration(e)))
                    .unwrap_or_default();
                (g, format!(" idle{dur}"))
            };
            format!(
                "{ts_str}  {cat}  {}@{project}{sess}  {glyph}{detail}",
                col!(agent, cyan),
            )
        }

        TailEvent::Status {
            project,
            agent,
            text,
            active,
            ..
        } => {
            let cat = col!("stat ", magenta);
            let label = match (text.is_empty(), *active) {
                (true, true) => "working".to_string(),
                (true, false) => "idle".to_string(),
                (false, true) => text.clone(),
                (false, false) => format!("{text} · idle"),
            };
            format!("{ts_str}  {cat}  {}@{project}  {label}", col!(agent, cyan))
        }

        TailEvent::Join {
            project,
            agent,
            host,
            session,
            rel_cwd,
            ..
        } => {
            let cat = col!("join ", green);
            let sess = format!("[{}]", sess_code(session));
            let cwd_info = if rel_cwd.is_empty() || rel_cwd == "." {
                String::new()
            } else {
                format!(" ({})", rel_cwd)
            };
            format!(
                "{ts_str}  {cat}  {}@{host}{sess}  online ({project}{cwd_info})",
                col!(agent, cyan),
            )
        }

        TailEvent::Leave {
            project,
            agent,
            host,
            session,
            online_s,
            ..
        } => {
            let cat = col!("leave", dimmed);
            let sess = format!("[{}]", sess_code(session));
            let dur = fmt_duration(*online_s);
            format!(
                "{ts_str}  {cat}  {}@{host}{sess}  offline (was online {dur}, {project})",
                col!(agent, cyan),
            )
        }

        TailEvent::Sess {
            project,
            agent,
            session,
            state,
            rel_cwd,
            ..
        } => {
            let cat = col!("sess ", blue);
            let sess = format!("[{}]", sess_code(session));
            let cwd_info = if rel_cwd.is_empty() || rel_cwd == "." {
                String::new()
            } else {
                format!(" (rel_cwd: {rel_cwd})")
            };
            format!(
                "{ts_str}  {cat}  {}@{project}{sess}  session {state}{cwd_info}",
                col!(agent, cyan),
            )
        }

        TailEvent::Proj { project, about, .. } => {
            let cat = col!("proj ", dimmed);
            let snippet: String = about.chars().take(60).collect();
            format!("{ts_str}  {cat}  {project}  {snippet}")
        }

        TailEvent::Profile {
            agent,
            host,
            pubkey,
            ..
        } => {
            let cat = col!("id   ", dimmed);
            let pk_short = &pubkey[..pubkey.len().min(8)];
            format!(
                "{ts_str}  {cat}  {}@{host}  ({pk_short})",
                col!(agent, cyan)
            )
        }
    }
}

fn fmt_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Public alias so the daemon's `tail` RPC can render fabric lines identically
/// to the old in-process `tail`.
pub fn render_fabric(de: &DomainEvent) -> String {
    render(de)
}

fn render(de: &DomainEvent) -> String {
    match de {
        DomainEvent::Profile(p) => {
            format!(
                "{} {}@{}",
                "id  ".dimmed(),
                p.agent.slug.cyan(),
                p.host.dimmed()
            )
        }
        DomainEvent::Activity(a) => {
            format!("{} {}: {}", "act ".blue(), a.agent.slug.cyan(), a.text)
        }
        DomainEvent::Status(s) if s.is_idle() => {
            let label = if s.title.trim().is_empty() {
                "idle".to_string()
            } else {
                format!("{} · idle", s.title)
            };
            format!("{} {} {}", "stat".dimmed(), s.agent.slug.cyan(), label)
        }
        DomainEvent::Status(s) => {
            format!("{} {}: {}", "stat".magenta(), s.agent.slug.cyan(), s.title)
        }
        DomainEvent::ChatMessage(c) => format!(
            "{} {}@{}{}: {}",
            "chat".green(),
            c.from.slug.cyan(),
            c.project,
            c.mentioned_pubkey
                .as_deref()
                .map(|pk| format!(" mentions {}", pubkey_short(pk)))
                .unwrap_or_default(),
            c.body
        ),
        DomainEvent::Proposal(p) => {
            format!(
                "{} {}: {} ({})",
                "prop".magenta(),
                p.agent.slug.cyan(),
                p.title,
                p.d
            )
        }
    }
}
