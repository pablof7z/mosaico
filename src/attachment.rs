//! Sending a file with a chat message.
//!
//! Blossom itself is `nmp-blossom`'s: the kind:24242 BUD-11 authorization, its
//! `t`/`x`/`expiration` rows, the `Nostr <base64url>` header encoding, the
//! `PUT /upload`, the response-size ceiling, the descriptor parse, and the
//! integrity gate that refuses a descriptor whose sha256 does not match the
//! bytes actually sent. Exact-byte identity is `nmp-asset`'s.
//!
//! What stays here is product policy: which local files a message carries,
//! what a `[label]` may look like, and the operator assumption that a group
//! relay also serves Blossom at the same host.

use crate::domain::ChatAttachment;
use anyhow::{bail, Context, Result};
use nmp_asset::Sha256Hash;
use nmp_blossom::{
    upload_authorization_draft, BlossomClient, BlossomClientConfig, BlossomServerUrl, BlossomVerb,
    ExpectedAuthorization, SignedAuthorization,
};
use nostr::{Keys, Timestamp};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use url::Url;

use crate::nmp_host::NmpHost;

const AUTH_LIFETIME_SECS: u64 = 5 * 60;

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
    nmp: &Arc<NmpHost>,
    keys: &Keys,
) -> Result<Vec<ChatAttachment>> {
    validate(attachments)?;
    if attachments.is_empty() {
        return Ok(Vec::new());
    }

    let server = BlossomServerUrl::parse(blossom_server(relays)?.as_str())
        .map_err(|error| anyhow::anyhow!("invalid Blossom server URL: {error}"))?;
    // A local relay is a local Blossom server. NMP refuses a loopback or
    // private-network destination unless the operator opted that exact host
    // in — the same allowlist the engine uses for relays.
    let client = BlossomClient::new(BlossomClientConfig {
        allowed_local_hosts: nmp.allowed_local_hosts(),
        ..BlossomClientConfig::default()
    })
    .map_err(|error| anyhow::anyhow!("building Blossom client: {}", error.reason))?;

    let mut uploaded = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let verified = upload(&client, &server, attachment, nmp, keys).await?;
        uploaded.push(ChatAttachment {
            url: verified.descriptor().url.clone(),
            label: attachment.label.clone(),
            // The hash the sender actually computed over the bytes it sent,
            // carried so the receiver can check what it downloaded. Before
            // this it was computed, used for the upload header, and thrown
            // away — which is why the download side could not verify anything.
            sha256: verified.asset().sha256().to_hex(),
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
    client: &BlossomClient,
    server: &BlossomServerUrl,
    attachment: &Attachment,
    nmp: &Arc<NmpHost>,
    keys: &Keys,
) -> Result<nmp_blossom::VerifiedUpload> {
    let bytes = tokio::fs::read(&attachment.path).await.with_context(|| {
        format!(
            "reading attachment [{}] from {}",
            attachment.label,
            attachment.path.display()
        )
    })?;
    let hash = Sha256Hash::of(&bytes);
    let auth = authorization(nmp, keys, hash).await?;
    let content_type = mime_guess::from_path(&attachment.path).first_or_octet_stream();
    client
        .upload(server, &bytes, Some(content_type.as_ref()), &auth)
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "uploading attachment [{}] to {}: {error}",
                attachment.label,
                server.as_str()
            )
        })
}

/// Compose the BUD-11 grant, sign it through NMP's signer registry, and prove
/// it authorizes exactly these bytes for exactly this verb.
///
/// The signing hop is the point. This was the one production path in the repo
/// that called `sign_with_keys` directly, so it needed raw `Keys` in hand and
/// no non-local signer could ever produce an attachment. `nmp-blossom` emits an
/// `UnsignedEvent` precisely so the caller signs it through the machinery it
/// already has.
async fn authorization(
    nmp: &Arc<NmpHost>,
    keys: &Keys,
    hash: Sha256Hash,
) -> Result<SignedAuthorization> {
    let now = Timestamp::now();
    let draft = upload_authorization_draft(
        keys.public_key(),
        hash,
        now,
        now + AUTH_LIFETIME_SECS,
        "Upload Blob",
    )
    .map_err(|error| anyhow::anyhow!("composing Blossom upload authorization: {error}"))?;
    let signed = nmp
        .sign_unsigned(draft, keys)
        .await
        .context("signing Blossom upload authorization through NMP")?;
    SignedAuthorization::validate(
        signed,
        &ExpectedAuthorization {
            verb: BlossomVerb::Upload,
            blob: Some(hash),
        },
        now,
    )
    .map_err(|error| anyhow::anyhow!("Blossom upload authorization is not usable: {error}"))
}

#[cfg(test)]
mod tests;
