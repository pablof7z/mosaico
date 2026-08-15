//! Shared `since` time parsing for chat read/search params: a unix timestamp
//! or a duration string (`2h`, `30m`, ...) resolved against now.
use super::*;

/// Parse a unix timestamp or a duration string (`s`/`m`/`h`/`d`) into a unix
/// timestamp. Durations are subtracted from the current time.
pub(in crate::daemon::server) fn parse_time(raw: &str) -> Result<u64> {
    if let Ok(timestamp) = raw.parse::<u64>() {
        return Ok(timestamp);
    }
    let value = raw.trim();
    let (number, unit) = value.split_at(value.len().saturating_sub(1));
    let amount = number
        .parse::<u64>()
        .with_context(|| format!("invalid time {raw:?}; use a timestamp or duration like 2h"))?;
    let factor = match unit {
        "s" | "S" => 1,
        "m" | "M" => 60,
        "h" | "H" => 60 * 60,
        "d" | "D" => 24 * 60 * 60,
        _ => anyhow::bail!("invalid time {raw:?}; duration units are s, m, h, or d"),
    };
    Ok(now_secs().saturating_sub(
        amount
            .checked_mul(factor)
            .with_context(|| format!("duration {raw:?} is too large"))?,
    ))
}

/// Deserialize a `since` field that may be null, a unix timestamp, or a
/// duration string accepted by [`parse_time`]. Used by `channel_read`.
pub(in crate::daemon::server) fn optional_time<'de, D>(
    d: D,
) -> std::result::Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    const MSG: &str = "since must be a non-negative timestamp or duration";
    match Option::<serde_json::Value>::deserialize(d)? {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom(MSG)),
        Some(serde_json::Value::String(s)) => {
            parse_time(&s).map(Some).map_err(serde::de::Error::custom)
        }
        Some(_) => Err(serde::de::Error::custom(MSG)),
    }
}
