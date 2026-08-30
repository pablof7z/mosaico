//! Sending a file with a chat message.
//!
//! Fava signs the kind:24242 BUD-11 authorization; this module owns its
//! `t`/`x`/`expiration` rows, the `Nostr <base64url>` header encoding, the
//! `PUT /upload`, the response-size ceiling, the descriptor parse, and the
//! integrity gate that refuses a descriptor whose sha256 does not match the
//! bytes actually sent. Exact-byte identity is local SHA-256.
//!
//! What stays here is product policy: which local files a message carries,
//! what a `[label]` may look like, and the operator assumption that a group
//! relay also serves Blossom at the same host.

use crate::asset_hash::Sha256Hash;
use crate::domain::ChatAttachment;
use anyhow::{bail, Context, Result};
use nostr::key::PublicKey;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use url::Url;

use crate::fava_host::FavaHost;

mod authorization;

#[derive(serde::Deserialize)]
struct BlossomDescriptor {
    url: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Attachment {
    pub(crate) label: String,
    pub(crate) path: PathBuf,
}

pub(crate) fn parse_spec(raw: &str) -> std::result::Result<Attachment, String> {
    if raw.is_empty() {
        return Err("attachment file path must not be empty".to_string());
    }
    if !raw.starts_with("./") && !Path::new(raw).is_absolute() {
        if let Some((label, file)) = raw.split_once('=') {
            crate::attachment_contract::validate_label(label).map_err(|error| error.to_string())?;
            if file.is_empty() {
                return Err("attachment file path must not be empty".to_string());
            }
            return Ok(Attachment {
                label: label.to_string(),
                path: PathBuf::from(file),
            });
        }
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
    fava: &Arc<FavaHost>,
    author: PublicKey,
) -> Result<Vec<ChatAttachment>> {
    validate(attachments)?;
    if attachments.is_empty() {
        return Ok(Vec::new());
    }

    let server = blossom_server(relays)?;
    // Guard local/private-network destinations the same way the relay engine
    // does: only allowed if the operator opted that exact host in.
    let allowed_local = fava.allowed_local_hosts();
    if is_local_url(&server) {
        let host = server.host_str().unwrap_or_default().to_string();
        if !allowed_local.contains(&host) {
            anyhow::bail!(
                "Blossom server {} is a local/private host not in allowed_local_hosts",
                server
            );
        }
    }

    let mut uploaded = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let descriptor = upload(&server, attachment, fava, author).await?;
        uploaded.push(ChatAttachment {
            url: descriptor.url,
            label: attachment.label.clone(),
            // The hash the sender actually computed over the bytes it sent,
            // carried so the receiver can check what it downloaded.
            sha256: descriptor.sha256,
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
    server: &Url,
    attachment: &Attachment,
    fava: &Arc<FavaHost>,
    author: PublicKey,
) -> Result<BlossomDescriptor> {
    let bytes = tokio::fs::read(&attachment.path).await.with_context(|| {
        format!(
            "reading attachment [{}] from {}",
            attachment.label,
            attachment.path.display()
        )
    })?;
    let hash = Sha256Hash::of(&bytes);
    let auth_token = authorization::sign_upload(fava, author, &hash).await?;
    let content_type = mime_guess::from_path(&attachment.path).first_or_octet_stream();
    let upload_url = server
        .join("upload")
        .context("building Blossom upload URL")?;
    let response = reqwest::Client::new()
        .put(upload_url.as_str())
        .header("Authorization", format!("Nostr {auth_token}"))
        .header("Content-Type", content_type.as_ref())
        .header("X-SHA-256", hash.to_hex())
        .body(bytes)
        .send()
        .await
        .with_context(|| {
            format!(
                "uploading attachment [{}] to {}",
                attachment.label,
                server.as_str()
            )
        })?;
    let status = response.status();
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::UNAUTHORIZED {
        let reason = response
            .headers()
            .get("x-reason")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        anyhow::bail!(
            "Blossom server rejected the authorization for [{}]: {} {}",
            attachment.label,
            status,
            reason
        );
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "Blossom upload of [{}] failed with {}: {body}",
            attachment.label,
            status
        );
    }
    let value: serde_json::Value = response
        .json()
        .await
        .context("parsing Blossom upload response")?;
    let url = value["url"]
        .as_str()
        .context("parsing Blossom upload response: missing the mandatory `url`")?
        .to_string();
    let returned_sha256 = value["sha256"]
        .as_str()
        .context("parsing Blossom upload response: missing the mandatory `sha256`")?
        .to_string();
    if returned_sha256 != hash.to_hex() {
        anyhow::bail!(
            "refusing the descriptor from [{}]: server returned sha256 {} but the bytes are hashing to {}",
            attachment.label,
            returned_sha256,
            hash.to_hex()
        );
    }
    Ok(BlossomDescriptor {
        url,
        sha256: returned_sha256,
    })
}

/// Whether `url` targets a loopback or RFC-1918 private address.
fn is_local_url(url: &Url) -> bool {
    use std::net::IpAddr;
    let host = url.host_str().unwrap_or_default();
    if host == "localhost" {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
            IpAddr::V6(v6) => v6.is_loopback(),
        };
    }
    false
}

#[cfg(test)]
mod tests;
