//! Materialize received chat attachments in one shared, collision-safe tree.
//!
//! # The HTTP here is a gap, not a preference
//!
//! `nmp-blossom` has no blob RETRIEVAL door at the pinned revision. It ships
//! `upload`, `mirror`, `delete` and `list`; `BlossomVerb::Get` exists as
//! vocabulary with no draft builder and no client method, and the crate's own
//! doc says the `get`/`media` endpoints are follow-up work (upstream NMP #749,
//! blocked on #748). Its wired `reqwest::Client` — hickory DNS behind a
//! post-resolution local-IP admission filter, no redirects, no retries, no
//! proxy, one deadline — is private, so a consumer cannot borrow the transport
//! either.
//!
//! So the fetch is ours until NMP #749 lands, and it is the only part that is.
//! The thing that made the old code a hole was not the `reqwest` call: it was
//! that nothing checked the bytes. `nmp_asset::Sha256Hash` — the same exact-byte
//! identity `nmp-blossom` mints an upload witness from — does that here.

use crate::domain::ChatAttachment;
use anyhow::{bail, Context, Result};
use nmp_asset::Sha256Hash;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::attachment_contract::EVENT_MARKER;

/// A hostile or misconfigured server must not be able to spend the daemon's
/// memory on one attachment. `nmp-blossom` bounds its own response bodies for
/// the same reason; this is that ceiling, applied to the one verb it does not
/// yet own.
const MAX_ATTACHMENT_BYTES: usize = 64 * 1024 * 1024;

pub(crate) async fn download(
    root: &Path,
    event_id: &str,
    attachments: &[ChatAttachment],
) -> Result<Option<PathBuf>> {
    if attachments.is_empty() {
        return Ok(None);
    }
    crate::attachment_contract::validate_attachments(attachments)?;
    let directory = event_directory(root, event_id)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("building attachment download client")?;
    for attachment in attachments {
        let expected = Sha256Hash::from_hex(&attachment.sha256).map_err(|error| {
            anyhow::anyhow!(
                "attachment [{}] declares an unusable sha256: {error}",
                attachment.label
            )
        })?;
        let response = client
            .get(&attachment.url)
            .send()
            .await
            .with_context(|| format!("downloading attachment [{}]", attachment.label))?;
        let status = response.status();
        if !status.is_success() {
            bail!(
                "downloading attachment [{}] failed with HTTP {status}",
                attachment.label
            );
        }
        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("reading attachment [{}]", attachment.label))?;
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            bail!(
                "attachment [{}] is {} bytes, over the {MAX_ATTACHMENT_BYTES}-byte ceiling",
                attachment.label,
                bytes.len()
            );
        }
        // Content-addressed, so this is the whole trust story: the bytes are
        // what the sender uploaded, or they do not reach the disk. Checked
        // BEFORE the write, under a label the remote side chose.
        let observed = Sha256Hash::of(&bytes);
        if observed != expected {
            bail!(
                "attachment [{}] does not match its declared sha256: expected {}, got {}",
                attachment.label,
                expected.to_hex(),
                observed.to_hex()
            );
        }
        write_new(&directory.join(&attachment.label), &bytes)?;
    }
    Ok(Some(directory))
}

pub(crate) fn copy_local(
    root: &Path,
    event_id: &str,
    attachments: &[crate::attachment::Attachment],
) -> Result<Option<PathBuf>> {
    if attachments.is_empty() {
        return Ok(None);
    }
    crate::attachment_contract::validate_labels(
        attachments
            .iter()
            .map(|attachment| attachment.label.as_str()),
    )?;
    let directory = event_directory(root, event_id)?;
    for attachment in attachments {
        let bytes = std::fs::read(&attachment.path).with_context(|| {
            format!(
                "copying attachment [{}] from {}",
                attachment.label,
                attachment.path.display()
            )
        })?;
        write_new(&directory.join(&attachment.label), &bytes)?;
    }
    Ok(Some(directory))
}

fn event_directory(root: &Path, event_id: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        !event_id.is_empty(),
        "attachment event id must not be empty"
    );
    std::fs::create_dir_all(root)
        .with_context(|| format!("creating attachment root {}", root.display()))?;
    let first_len = event_id.len().min(6);
    for length in first_len..=event_id.len() {
        let directory = root.join(&event_id[..length]);
        match std::fs::create_dir(&directory) {
            Ok(()) => {
                write_marker(&directory, event_id)?;
                return Ok(directory);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if marker_matches(&directory, event_id) {
                    return Ok(directory);
                }
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("creating attachment directory {}", directory.display())
                })
            }
        }
    }
    bail!("attachment directory collision for event {event_id}")
}

fn write_marker(directory: &Path, event_id: &str) -> Result<()> {
    write_new(&directory.join(EVENT_MARKER), event_id.as_bytes())
}

fn marker_matches(directory: &Path, event_id: &str) -> bool {
    std::fs::read_to_string(directory.join(EVENT_MARKER)).is_ok_and(|stored| stored == event_id)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating attachment directory {}", parent.display()))?;
    }
    match OpenOptions::new().create_new(true).write(true).open(path) {
        Ok(mut file) => file
            .write_all(bytes)
            .with_context(|| format!("writing attachment {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error).with_context(|| format!("creating attachment {}", path.display())),
    }
}

#[cfg(test)]
#[path = "attachment_receive/tests.rs"]
mod tests;
