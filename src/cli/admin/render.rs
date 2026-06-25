use super::*;

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
