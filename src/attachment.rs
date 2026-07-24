use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, TagKind, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use url::Url;

const AUTH_LIFETIME_SECS: u64 = 5 * 60;
pub(crate) const AGENT_HINT: &str =
    "Attachments: add `--attach label=/path/to/file` and reference `[label]` in the message.";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Attachment {
    pub(crate) label: String,
    pub(crate) path: PathBuf,
}

#[derive(Deserialize)]
struct BlobDescriptor {
    url: String,
    sha256: String,
    size: u64,
    #[serde(rename = "type")]
    mime_type: String,
    uploaded: u64,
}

pub(crate) fn parse_spec(raw: &str) -> std::result::Result<Attachment, String> {
    let (label, path) = raw
        .split_once('=')
        .ok_or_else(|| "attachment must use LABEL=FILE".to_string())?;
    validate_label(label)?;
    if path.is_empty() {
        return Err("attachment file path must not be empty".to_string());
    }
    Ok(Attachment {
        label: label.to_string(),
        path: PathBuf::from(path),
    })
}

fn validate_label(label: &str) -> std::result::Result<(), String> {
    if label.is_empty()
        || !label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(
            "attachment label must contain only ASCII letters, digits, '.', '-', or '_'"
                .to_string(),
        );
    }
    Ok(())
}

pub(crate) fn canonicalize(mut attachments: Vec<Attachment>) -> Result<Vec<Attachment>> {
    for attachment in &mut attachments {
        attachment.path = std::fs::canonicalize(&attachment.path).with_context(|| {
            format!(
                "reading attachment [{}] from {}",
                attachment.label,
                attachment.path.display()
            )
        })?;
        if !attachment.path.is_file() {
            bail!(
                "attachment [{}] is not a regular file: {}",
                attachment.label,
                attachment.path.display()
            );
        }
    }
    Ok(attachments)
}

pub(crate) async fn upload_and_expand(
    message: &str,
    attachments: &[Attachment],
    relays: &[String],
    keys: &Keys,
) -> Result<String> {
    validate(message, attachments)?;
    if attachments.is_empty() {
        return Ok(message.to_string());
    }

    let server = blossom_server(relays)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(5 * 60))
        .build()
        .context("building Blossom HTTP client")?;
    let mut expanded = message.to_string();
    for attachment in attachments {
        let public_url = upload(&client, &server, attachment, keys).await?;
        expanded = expanded.replace(&format!("[{}]", attachment.label), public_url.as_str());
    }
    Ok(expanded)
}

fn validate(message: &str, attachments: &[Attachment]) -> Result<()> {
    let mut labels = HashSet::new();
    for attachment in attachments {
        validate_label(&attachment.label).map_err(anyhow::Error::msg)?;
        if attachment.path.as_os_str().is_empty() {
            bail!("attachment [{}] has an empty file path", attachment.label);
        }
        if !labels.insert(attachment.label.as_str()) {
            bail!("duplicate attachment label [{}]", attachment.label);
        }
        let marker = format!("[{}]", attachment.label);
        if !message.contains(&marker) {
            bail!("message does not reference attachment {marker}");
        }
    }
    Ok(())
}

fn blossom_server(relays: &[String]) -> Result<Url> {
    let relay = relays
        .first()
        .context("cannot upload attachments without a configured relay")?;
    let mut url =
        Url::parse(relay).with_context(|| format!("invalid configured relay URL {relay:?}"))?;
    let scheme = match url.scheme() {
        "wss" => "https",
        "ws" => "http",
        "https" => "https",
        "http" => "http",
        other => bail!("configured relay uses unsupported URL scheme {other:?}"),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("failed to convert relay URL to Blossom HTTP URL"))?;
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

async fn upload(
    client: &reqwest::Client,
    server: &Url,
    attachment: &Attachment,
    keys: &Keys,
) -> Result<Url> {
    let bytes = tokio::fs::read(&attachment.path).await.with_context(|| {
        format!(
            "reading attachment [{}] from {}",
            attachment.label,
            attachment.path.display()
        )
    })?;
    if bytes.is_empty() {
        bail!("attachment [{}] is empty", attachment.label);
    }
    let size = bytes.len();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let auth = authorization(keys, server, &hash)?;
    let upload_url = server
        .join("upload")
        .context("building Blossom upload URL")?;
    let content_type = mime_guess::from_path(&attachment.path).first_or_octet_stream();
    let response = client
        .put(upload_url.clone())
        .header(reqwest::header::AUTHORIZATION, auth)
        .header(reqwest::header::CONTENT_TYPE, content_type.as_ref())
        .header("X-SHA-256", &hash)
        .body(bytes)
        .send()
        .await
        .with_context(|| {
            format!(
                "uploading attachment [{}] to {upload_url}",
                attachment.label
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let reason = response.text().await.unwrap_or_default();
        let reason: String = reason.chars().take(500).collect();
        bail!(
            "Blossom upload for attachment [{}] failed with HTTP {status}: {}",
            attachment.label,
            reason.trim()
        );
    }
    let descriptor: BlobDescriptor = response.json().await.with_context(|| {
        format!(
            "parsing Blossom response for attachment [{}]",
            attachment.label
        )
    })?;
    if !descriptor.sha256.eq_ignore_ascii_case(&hash) {
        bail!(
            "Blossom response hash mismatch for attachment [{}]",
            attachment.label
        );
    }
    if descriptor.size != size as u64 {
        bail!(
            "Blossom response size mismatch for attachment [{}]",
            attachment.label
        );
    }
    if descriptor.mime_type.trim().is_empty() || descriptor.uploaded == 0 {
        bail!(
            "Blossom returned an incomplete descriptor for attachment [{}]",
            attachment.label
        );
    }
    let public_url = Url::parse(&descriptor.url)
        .with_context(|| format!("invalid Blossom URL for attachment [{}]", attachment.label))?;
    if !matches!(public_url.scheme(), "http" | "https") {
        bail!(
            "Blossom returned a non-HTTP URL for attachment [{}]",
            attachment.label
        );
    }
    if !public_url.path().to_ascii_lowercase().contains(&hash) {
        bail!(
            "Blossom URL does not contain the blob hash for attachment [{}]",
            attachment.label
        );
    }
    Ok(public_url)
}

fn authorization(keys: &Keys, server: &Url, hash: &str) -> Result<String> {
    let expires = Timestamp::now() + AUTH_LIFETIME_SECS;
    let host = server
        .host_str()
        .context("Blossom server URL has no domain")?
        .to_ascii_lowercase();
    let event = EventBuilder::new(Kind::Custom(24242), "Upload Blob")
        .tags([
            Tag::custom(TagKind::t(), ["upload"]),
            Tag::expiration(expires),
            Tag::custom(TagKind::x(), [hash]),
            Tag::custom(TagKind::custom("server"), [host]),
        ])
        .sign_with_keys(keys)
        .context("signing Blossom upload authorization")?;
    Ok(format!("Nostr {}", STANDARD.encode(event.as_json())))
}

#[cfg(test)]
mod tests;
