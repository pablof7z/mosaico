use crate::domain::ChatAttachment;
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, TagKind, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use url::Url;

const AUTH_LIFETIME_SECS: u64 = 5 * 60;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Attachment {
    pub(crate) label: String,
    pub(crate) path: PathBuf,
}

#[derive(Deserialize)]
struct BlobDescriptor {
    url: String,
}

pub(crate) fn parse_spec(raw: &str) -> std::result::Result<Attachment, String> {
    if raw.is_empty() {
        return Err("attachment file path must not be empty".to_string());
    }
    let path = PathBuf::from(raw);
    let label = infer_label(&path)?;
    Ok(Attachment { label, path })
}

fn infer_label(path: &Path) -> std::result::Result<String, String> {
    let label_path = if path.is_absolute() {
        PathBuf::from(
            path.file_name()
                .ok_or_else(|| "attachment path must name a file".to_string())?,
        )
    } else {
        let mut clean = PathBuf::new();
        for part in path.components() {
            match part {
                Component::CurDir => {}
                Component::Normal(value) => clean.push(value),
                Component::ParentDir => {
                    return Err("attachment path must not contain '..'".to_string())
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err("attachment label must be a safe relative path".to_string())
                }
            }
        }
        clean
    };
    let label = label_path
        .to_str()
        .ok_or_else(|| "attachment path must have a UTF-8 label".to_string())?
        .to_string();
    crate::attachment_contract::validate_label(&label).map_err(|error| error.to_string())?;
    Ok(label)
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

pub(crate) fn prepare_message(message: &str, attachments: &[Attachment]) -> Result<String> {
    validate(attachments)?;
    let mut prepared = message.to_string();
    let missing = attachments
        .iter()
        .filter(|attachment| !message.contains(&format!("[{}]", attachment.label)))
        .map(|attachment| format!("[{}]", attachment.label))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        if !prepared.is_empty() {
            prepared.push_str("\n\n");
        }
        prepared.push_str(&missing.join("\n"));
    }
    Ok(prepared)
}

pub(crate) async fn upload_all(
    attachments: &[Attachment],
    relays: &[String],
    keys: &Keys,
) -> Result<Vec<ChatAttachment>> {
    validate(attachments)?;
    if attachments.is_empty() {
        return Ok(Vec::new());
    }

    let server = blossom_server(relays)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(5 * 60))
        .build()
        .context("building Blossom HTTP client")?;
    let mut uploaded = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let public_url = upload(&client, &server, attachment, keys).await?;
        uploaded.push(ChatAttachment {
            url: public_url.into(),
            label: attachment.label.clone(),
        });
    }
    crate::attachment_contract::validate_attachments(&uploaded)?;
    Ok(uploaded)
}

fn validate(attachments: &[Attachment]) -> Result<()> {
    for attachment in attachments {
        if attachment.path.as_os_str().is_empty() {
            bail!("attachment [{}] has an empty file path", attachment.label);
        }
    }
    crate::attachment_contract::validate_labels(
        attachments
            .iter()
            .map(|attachment| attachment.label.as_str()),
    )
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
    let public_url = Url::parse(&descriptor.url)
        .with_context(|| format!("invalid Blossom URL for attachment [{}]", attachment.label))?;
    if !matches!(public_url.scheme(), "http" | "https") {
        bail!(
            "Blossom returned a non-HTTP URL for attachment [{}]",
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
