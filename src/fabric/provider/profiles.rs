use super::Nip29Provider;
use nostr_sdk::prelude::{Filter, Kind, PublicKey};
use std::time::Duration;

const PROFILE_FETCH_TIMEOUT: Duration = Duration::from_secs(4);

impl Nip29Provider {
    pub(crate) async fn fetch_and_cache_profile_name(
        &self,
        pubkey: &str,
        now: u64,
    ) -> Option<String> {
        let author = PublicKey::from_hex(pubkey).ok()?;
        let filter = Filter::new().author(author).kind(Kind::from(0u16)).limit(1);
        let events = self
            .transport
            .fetch(filter, PROFILE_FETCH_TIMEOUT)
            .await
            .ok()?;

        let event = events.into_iter().max_by_key(|e| e.created_at)?;
        let display_name = display_name_from_metadata(&event.content)?;
        let host = host_tag(&event).unwrap_or_default();
        let is_backend = backend_tag(&event);
        let agent_slug = agent_slug_tag(&event).unwrap_or_default();
        let (name, slug) =
            profile_cache_fields_with_agent_slug(&display_name, &host, &agent_slug, is_backend);

        self.with_store(|s| {
            s.upsert_profile_with_agent_slug(
                pubkey,
                &name,
                &slug,
                &agent_slug,
                &host,
                is_backend,
                now,
            )
            .ok()
        });
        Some(name)
    }
}

fn display_name_from_metadata(content: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;
    for key in ["display_name", "name"] {
        if let Some(s) = v.get(key).and_then(|n| n.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn host_tag(event: &nostr_sdk::Event) -> Option<String> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.first().map(String::as_str) == Some("host"))
            .then(|| s.get(1).cloned())
            .flatten()
    })
}

fn backend_tag(event: &nostr_sdk::Event) -> bool {
    event
        .tags
        .iter()
        .any(|t| t.as_slice().first().map(String::as_str) == Some("backend"))
}

fn agent_slug_tag(event: &nostr_sdk::Event) -> Option<String> {
    tag_value(event, "agent-slug").or_else(|| tag_value(event, "agentSlug"))
}

fn tag_value(event: &nostr_sdk::Event, name: &str) -> Option<String> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.first().map(String::as_str) == Some(name))
            .then(|| s.get(1).cloned())
            .flatten()
    })
}

fn profile_cache_fields_with_agent_slug(
    display_name: &str,
    host: &str,
    agent_slug: &str,
    is_backend: bool,
) -> (String, String) {
    let name = if is_backend {
        display_name.trim().to_string()
    } else {
        crate::idref::session_handle_from_profile_name(display_name, host, agent_slug)
    };
    let slug = name.clone();
    (name, slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_prefers_display_name_over_name() {
        let c = r#"{"name":"pablo","display_name":"Pablo F"}"#;
        assert_eq!(display_name_from_metadata(c).as_deref(), Some("Pablo F"));
    }

    #[test]
    fn display_name_falls_back_to_name() {
        let c = r#"{"name":"pablo"}"#;
        assert_eq!(display_name_from_metadata(c).as_deref(), Some("pablo"));
    }

    #[test]
    fn empty_or_blank_metadata_yields_none() {
        assert_eq!(display_name_from_metadata("{}"), None);
        assert_eq!(display_name_from_metadata(r#"{"name":"  "}"#), None);
        assert_eq!(display_name_from_metadata("not json"), None);
    }

    #[test]
    fn cache_fields_keep_qualified_name_and_bare_slug() {
        assert_eq!(
            profile_cache_fields_with_agent_slug(
                "developer1@remoteBackend",
                "remoteBackend",
                "",
                false
            ),
            ("developer1".to_string(), "developer1".to_string())
        );
        assert_eq!(
            profile_cache_fields_with_agent_slug("developer1", "remoteBackend", "", false),
            ("developer1".to_string(), "developer1".to_string())
        );
        assert_eq!(
            profile_cache_fields_with_agent_slug(
                "developer1@remoteBackend",
                "remoteBackend",
                "developer",
                false
            ),
            (
                "developer/developer1".to_string(),
                "developer/developer1".to_string()
            )
        );
    }
}
