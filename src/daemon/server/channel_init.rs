use super::*;

const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

pub(in crate::daemon::server) async fn rpc_channel_init(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
        path: String,
    }

    let p: P = serde_json::from_value(params.clone()).context("channel_init params")?;
    let channel = root_slug(&p.channel)?;
    let path = std::path::Path::new(&p.path);
    if !path.is_absolute() {
        anyhow::bail!("workspace path must be absolute");
    }
    let canonical = path
        .canonicalize()
        .with_context(|| format!("canonicalizing workspace path {}", path.display()))?;

    state.with_store(|store| {
        store.upsert_channel(&channel, &channel, "", "", now_secs())?;
        crate::daemon::workspace_path::WorkspacePathResolver::new(store).bind_root_path(
            &channel,
            &canonical,
            now_secs(),
        )
    })?;

    let management = state.management_keys()?;
    let management_pubkey = management.public_key().to_hex();
    let provider = state.provider();
    let readiness = provider.ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
        channel: &channel,
        expect_member: &management_pubkey,
        parent_hint: None,
        name: None,
    });
    let ready = tokio::time::timeout(READY_TIMEOUT, readiness)
        .await
        .context("root channel readiness timed out")?;
    ready.require_ready(format!(
        "workspace root #{channel} was registered locally but was not provisioned"
    ))?;

    ensure_subscription(state, &channel).await?;
    publish_backend_profile(state).await?;
    Ok(serde_json::json!({
        "channel": crate::channel_ref::format_channel_ref(&channel, &[]),
        "path": canonical,
    }))
}

fn root_slug(reference: &str) -> Result<String> {
    let reference = reference.trim();
    let Some(slug) = reference.strip_prefix(crate::channel_ref::CHANNEL_PATH_PREFIX) else {
        anyhow::bail!("workspace root must be a full path such as #workspace");
    };
    if slug.is_empty() || slug.contains(['/', '.']) || slug.chars().any(char::is_whitespace) {
        anyhow::bail!("workspace root must contain exactly one non-empty path segment");
    }
    Ok(slug.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_slug_accepts_only_one_absolute_segment() {
        assert_eq!(root_slug("#mosaico").unwrap(), "mosaico");
        for invalid in ["mosaico", "#", "#a/b", "#a.b", "#a b", "/mosaico"] {
            assert!(root_slug(invalid).is_err(), "{invalid:?}");
        }
    }
}
