use super::*;

// ── acl (owner-scoped agent authorization) ───────────────────────────────────

pub(super) async fn acl(action: Option<AclAction>) -> Result<()> {
    match action {
        Some(AclAction::Allow { target }) => {
            let v = daemon_call_async(
                "acl",
                serde_json::json!({"action": "allow", "target": target}),
            )
            .await?;
            println!(
                "authorized {} ({})",
                v["slug"].as_str().unwrap_or("").cyan(),
                pubkey_short(v["pubkey"].as_str().unwrap_or(""))
            );
        }
        Some(AclAction::Block { target }) => {
            let v = daemon_call_async(
                "acl",
                serde_json::json!({"action": "block", "target": target}),
            )
            .await?;
            println!(
                "blocked {} ({})",
                v["slug"].as_str().unwrap_or(""),
                pubkey_short(v["pubkey"].as_str().unwrap_or(""))
            );
        }
        Some(AclAction::List) | None => {
            let v = daemon_call_async("acl", serde_json::json!({"action": "list"})).await?;
            println!(
                "{}",
                "pending (claim you as owner, awaiting your decision):".bold()
            );
            let pending = v["pending"].as_array().cloned().unwrap_or_default();
            if pending.is_empty() {
                println!("  (none)");
            } else {
                for p in &pending {
                    println!(
                        "  {} {}  ({})  host {}",
                        "?".yellow(),
                        p["slug"].as_str().unwrap_or("").cyan(),
                        pubkey_short(p["pubkey"].as_str().unwrap_or("")),
                        p["host"].as_str().unwrap_or("").dimmed()
                    );
                }
                println!(
                    "\n  allow:  tenex-edge acl allow <slug|pubkey>\n  block:  tenex-edge acl block <slug|pubkey>"
                );
            }
            println!(
                "\n{} {} authorized, {} blocked",
                "acl:".bold(),
                v["allowed"].as_u64().unwrap_or(0),
                v["blocked"].as_u64().unwrap_or(0)
            );
        }
    }
    Ok(())
}

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
        ProjectAction::Edit {
            description,
            project,
        } => {
            let slug = project.unwrap_or_else(|| {
                crate::project::resolve(&std::env::current_dir().unwrap_or_default())
            });
            let v = daemon_call_async(
                "project_edit",
                serde_json::json!({ "project": slug, "description": description }),
            )
            .await?;
            let event_id = v["event_id"].as_str().unwrap_or("?");
            println!("Updated {slug}: {}", &event_id[..event_id.len().min(8)]);
        }
    }
    Ok(())
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

pub(super) async fn tail(project: Option<String>) -> Result<()> {
    // The daemon owns the single relay connection and streams decoded, rendered
    // fabric lines over the UDS until we disconnect (Ctrl-C). The rendering uses
    // the SAME `render()` daemon-side, so output is identical.
    let scope_label = project.as_deref().unwrap_or("*");
    eprintln!(
        "{} tailing project {} … (Ctrl-C to stop)",
        "tenex-edge".bold(),
        scope_label.cyan()
    );

    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    let stream = client.stream("tail", serde_json::json!({ "project": project }), |item| {
        if let Some(line) = item.get("line").and_then(|l| l.as_str()) {
            println!("{line}");
        }
    });
    tokio::select! {
        _ = tokio::signal::ctrl_c() => Ok(()),
        r = stream => r,
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
        DomainEvent::Presence(p) => format!(
            "{} {}@{} {} ({})",
            "live".green(),
            p.agent.slug.cyan(),
            slugify_host(&p.host),
            p.session_id.to_string().yellow(),
            p.project.dimmed()
        ),
        DomainEvent::Activity(a) => {
            format!("{} {}: {}", "act ".blue(), a.agent.slug.cyan(), a.text)
        }
        DomainEvent::Status(s) if s.is_idle() => {
            format!("{} {} idle", "stat".dimmed(), s.agent.slug.cyan())
        }
        DomainEvent::Status(s) => {
            format!("{} {}: {}", "stat".magenta(), s.agent.slug.cyan(), s.text)
        }
        DomainEvent::Mention(m) => format!(
            "{} {} -> {}{}: {}",
            "msg ".yellow(),
            m.from.slug.cyan(),
            pubkey_short(&m.to_pubkey),
            m.target_session
                .as_ref()
                .map(|s| format!(" ({s})"))
                .unwrap_or_default(),
            m.body
        ),
        DomainEvent::TurnReply(r) => {
            format!("{} {}: {}", "turn".blue(), r.agent.slug.cyan(), r.body)
        }
    }
}
