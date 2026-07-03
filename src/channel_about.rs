use anyhow::Result;

pub(crate) const CHANNEL_ABOUT_MAX_CHARS: usize = 80;

pub(crate) fn parse_channel_about(value: &str) -> std::result::Result<String, String> {
    validate_channel_about_string(value)?;
    Ok(value.to_string())
}

pub(crate) fn validate_channel_about(about: &str) -> Result<()> {
    validate_channel_about_string(about).map_err(|msg| anyhow::anyhow!(msg))
}

fn validate_channel_about_string(value: &str) -> std::result::Result<(), String> {
    let len = value.chars().count();
    if len > CHANNEL_ABOUT_MAX_CHARS {
        return Err(format!(
            "--about must be {CHANNEL_ABOUT_MAX_CHARS} characters or fewer (got {len})"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_limit_counts_chars() {
        let ok = "x".repeat(CHANNEL_ABOUT_MAX_CHARS);
        validate_channel_about(&ok).expect("limit length is accepted");

        let too_long = "x".repeat(CHANNEL_ABOUT_MAX_CHARS + 1);
        let err = validate_channel_about(&too_long).expect_err("over limit rejects");
        assert!(format!("{err:#}").contains("80 characters or fewer"));
    }
}
