//! Embedded mosaico agent skill for MCP clients that lack a local skill install.
//!
//! Exposed as read-only resources (`mosaico://skill…`) and the `mosaico.skill`
//! tool (for clients that do not surface resources).

use anyhow::{bail, Result};
use serde_json::{json, Value};

pub(super) const SKILL_URI: &str = "mosaico://skill";
const SKILL_PREFIX: &str = "mosaico://skill/";

struct Page {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    content: &'static str,
}

const PAGES: &[Page] = &[
    Page {
        name: "skill",
        title: "mosaico skill",
        description:
            "Agent participation skill: self-organization, channels, identity, coordination.",
        content: include_str!("../../../skills/mosaico/SKILL.md"),
    },
    Page {
        name: "channel-creation",
        title: "Channel creation",
        description: "When and how to create, join, switch, and reorganize channels.",
        content: include_str!("../../../skills/mosaico/references/channel-creation.md"),
    },
    Page {
        name: "coordination-guide",
        title: "Coordination guide",
        description: "Directing attention, handoffs, tags, replies, and dispatch.",
        content: include_str!("../../../skills/mosaico/references/coordination-guide.md"),
    },
    Page {
        name: "cross-workspace",
        title: "Cross-workspace coordination",
        description: "Working across workspaces without losing ownership boundaries.",
        content: include_str!("../../../skills/mosaico/references/cross-workspace.md"),
    },
    Page {
        name: "headless-mode",
        title: "Headless mode",
        description: "Publication cadence and delivery when headless mode is on.",
        content: include_str!("../../../skills/mosaico/references/headless-mode.md"),
    },
    Page {
        name: "identity-and-capabilities",
        title: "Identity and agent capabilities",
        description: "Public identity, CLI vs remote MCP self, inventory, managed delivery.",
        content: include_str!("../../../skills/mosaico/references/identity-and-capabilities.md"),
    },
    Page {
        name: "mcp-chatbot-setup",
        title: "Third-party chatbots through MCP",
        description: "Operator guide for connecting ChatGPT, Grok, or other MCP clients.",
        content: include_str!("../../../skills/mosaico/references/mcp-chatbot-setup.md"),
    },
    Page {
        name: "message-search",
        title: "Message search",
        description: "Local-cache search scope, filters, XML output, and pagination.",
        content: include_str!("../../../skills/mosaico/references/message-search.md"),
    },
    Page {
        name: "multi-instance-setup",
        title: "Multiple isolated instances",
        description: "Set up and operate segregated daemon awareness on one host.",
        content: include_str!("../../../skills/mosaico/references/multi-instance-setup.md"),
    },
    Page {
        name: "public-work-status",
        title: "Public work status",
        description: "Session titles, lifecycle states, and self-only lifecycle commands.",
        content: include_str!("../../../skills/mosaico/references/public-work-status.md"),
    },
    Page {
        name: "unhosted",
        title: "Unhosted sessions",
        description: "Mention delivery boundaries, bounded waits, and durable re-homing.",
        content: include_str!("../../../skills/mosaico/references/unhosted.md"),
    },
];

/// Resolve skill body by tool/resource name. `None` / `"skill"` → entry page.
/// `"list"` / `"index"` → markdown index of pages.
pub(super) fn content(name: Option<&str>) -> Result<(String, String, String)> {
    let key = name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("skill");
    if key == "list" || key == "index" {
        return Ok(("list".into(), SKILL_URI.into(), index_markdown()));
    }
    let page = page(key)?;
    Ok((page.name.into(), page_uri(page.name), page.content.into()))
}

pub(super) fn tool_result(name: Option<&str>) -> Result<Value> {
    let (name, uri, body) = content(name)?;
    Ok(json!({
        "content": [{ "type": "text", "text": body }],
        "structuredContent": {
            "name": name,
            "uri": uri,
            "mimeType": "text/markdown",
            "available": available_names(),
        },
        "isError": false,
    }))
}

pub(super) fn resource_list_entries() -> Vec<Value> {
    PAGES
        .iter()
        .map(|page| {
            json!({
                "uri": page_uri(page.name),
                "name": page.name,
                "title": page.title,
                "description": page.description,
                "mimeType": "text/markdown",
            })
        })
        .collect()
}

pub(super) fn resource_templates() -> Value {
    json!([{
        "uriTemplate": "mosaico://skill/{name}",
        "name": "skill-page",
        "title": "Mosaico skill page",
        "description": "Agent skill entry or named reference. Use name=skill for the entry, list for the index.",
        "mimeType": "text/markdown",
    }])
}

fn page(key: &str) -> Result<&'static Page> {
    if let Some(page) = PAGES.iter().find(|page| page.name == key) {
        return Ok(page);
    }
    bail!(
        "unknown skill page {key:?}; available: {}",
        available_names().join(", ")
    )
}

fn page_uri(name: &str) -> String {
    if name == "skill" {
        SKILL_URI.to_string()
    } else {
        format!("{SKILL_PREFIX}{name}")
    }
}

fn available_names() -> Vec<&'static str> {
    let mut names: Vec<_> = PAGES.iter().map(|p| p.name).collect();
    names.push("list");
    names
}

fn index_markdown() -> String {
    let mut out = String::from("# Mosaico skill index\n\n");
    out.push_str(
        "Call `mosaico.skill` with `name` set to a page below, or read the matching resource URI.\n\n",
    );
    for page in PAGES {
        out.push_str(&format!(
            "- **{}** (`{}`) — {}\n",
            page.name,
            page_uri(page.name),
            page.description
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_and_identity_pages_embed() {
        let (_, _, entry) = content(None).unwrap();
        assert!(entry.contains("Prime Directive"));
        let (_, _, identity) = content(Some("identity-and-capabilities")).unwrap();
        assert!(identity.contains("CLI wins"));
        let (_, _, search) = content(Some("message-search")).unwrap();
        assert!(search.contains("Omitting `--channel`"));
        assert!(search.contains("mosaico.channel_search"));
        let (_, _, instances) = content(Some("multi-instance-setup")).unwrap();
        assert!(instances.contains("MOSAICO=relay1"));
        assert!(instances.contains("never searches or falls back"));
        let (_, _, unhosted) = content(Some("unhosted")).unwrap();
        assert!(unhosted.contains("A wait is a temporary bridge"));
    }

    #[test]
    fn unknown_page_lists_available() {
        let err = content(Some("nope")).unwrap_err().to_string();
        assert!(err.contains("unknown skill page"));
        assert!(err.contains("coordination-guide"));
    }

    #[test]
    fn list_returns_index() {
        let value = tool_result(Some("list")).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("identity-and-capabilities"));
        assert!(text.contains("multi-instance-setup"));
        assert!(text.contains("mosaico://skill"));
    }
}
