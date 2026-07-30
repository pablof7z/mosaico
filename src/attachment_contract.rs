use crate::domain::ChatAttachment;
use anyhow::{bail, Result};
use std::path::{Component, Path};
use url::Url;

pub(crate) fn validate_label(label: &str) -> Result<()> {
    if label.is_empty() {
        bail!("attachment label must not be empty");
    }
    if label.ends_with('/')
        || label
            .chars()
            .any(|c| c.is_control() || matches!(c, '[' | ']' | '\\'))
    {
        bail!("attachment label contains an unsafe character");
    }
    let path = Path::new(label);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("attachment label must be a safe relative path");
    }
    Ok(())
}

pub(crate) fn validate_labels<'a>(labels: impl IntoIterator<Item = &'a str>) -> Result<()> {
    let labels = labels.into_iter().collect::<Vec<_>>();
    for (index, label) in labels.iter().enumerate() {
        validate_label(label)?;
        for earlier in &labels[..index] {
            if label == earlier {
                bail!("duplicate attachment label [{label}]");
            }
            if overlaps(label, earlier) {
                bail!("attachment labels [{earlier}] and [{label}] would overwrite the same path");
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_attachments(attachments: &[ChatAttachment]) -> Result<()> {
    validate_labels(
        attachments
            .iter()
            .map(|attachment| attachment.label.as_str()),
    )?;
    for attachment in attachments {
        let url = Url::parse(&attachment.url)?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("attachment URL must use http or https");
        }
    }
    Ok(())
}

pub(crate) fn try_push(accepted: &mut Vec<ChatAttachment>, candidate: ChatAttachment) -> bool {
    let valid_url = Url::parse(&candidate.url)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), "http" | "https"));
    let valid_label = validate_label(&candidate.label).is_ok();
    let collides = accepted.iter().any(|existing| {
        candidate.label == existing.label || overlaps(&candidate.label, &existing.label)
    });
    if !valid_url || !valid_label || collides {
        return false;
    }
    accepted.push(candidate);
    true
}

fn overlaps(left: &str, right: &str) -> bool {
    Path::new(left).starts_with(Path::new(right)) || Path::new(right).starts_with(Path::new(left))
}
