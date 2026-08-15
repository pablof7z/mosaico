use anyhow::{Context as _, Result};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

pub(super) fn validate_caller(params: &Value) -> Result<()> {
    anyhow::ensure!(
        params.get("harness").and_then(Value::as_str) == Some("pi"),
        "Pi tool calls require harness=pi"
    );
    for key in ["harness_session", "cwd"] {
        anyhow::ensure!(
            params
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty()),
            "Pi tool calls require {key}"
        );
    }
    anyhow::ensure!(
        params.get("session").is_none(),
        "Pi tools cannot override session identity"
    );
    Ok(())
}

pub(super) fn caller(params: &Value) -> Result<Map<String, Value>> {
    super::super::channel_send::caller_params(params)
        .as_object()
        .cloned()
        .context("Pi caller identity must be an object")
}

pub(super) fn allow_only(args: &Value, allowed: &[&str]) -> Result<()> {
    let object = args
        .as_object()
        .context("Pi tool arguments must be an object")?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        anyhow::bail!("unsupported Pi tool argument {key:?}");
    }
    Ok(())
}

pub(super) fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{key} is required"))
}

pub(super) fn required_u64(args: &Value, key: &str) -> Result<u64> {
    args.get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .with_context(|| format!("{key} must be a positive integer"))
}

pub(super) fn optional(args: &Value, key: &str) -> Value {
    args.get(key).cloned().unwrap_or(Value::Null)
}

pub(super) fn array(args: &Value, key: &str) -> Result<Value> {
    let value = optional(args, key);
    if value.is_null() {
        return Ok(json!([]));
    }
    let values = value
        .as_array()
        .with_context(|| format!("{key} must be an array"))?;
    anyhow::ensure!(
        values.iter().all(Value::is_string),
        "{key} entries must be strings"
    );
    Ok(value)
}

pub(super) fn bool_or(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
}

pub(super) fn enabled(args: &Value, key: &str) -> bool {
    args.get(key).is_some_and(|value| match value {
        Value::Bool(value) => *value,
        Value::String(value) => !value.trim().is_empty(),
        _ => false,
    })
}

pub(super) fn insert(map: &mut Map<String, Value>, key: &str, value: impl serde::Serialize) {
    map.insert(
        key.to_string(),
        serde_json::to_value(value).expect("serializable RPC value"),
    );
}

pub(super) fn attachments(args: &Value, caller: &Map<String, Value>) -> Result<Value> {
    let specs = array(args, "attachments")?;
    let cwd = caller
        .get("cwd")
        .and_then(Value::as_str)
        .context("Pi tool call has no cwd")?;
    let parsed = specs
        .as_array()
        .expect("validated attachment array")
        .iter()
        .map(|value| crate::attachment::parse_spec(value.as_str().expect("validated string")))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(anyhow::Error::msg)?;
    let parsed = parsed
        .into_iter()
        .map(|mut attachment| {
            if attachment.path.is_relative() {
                attachment.path = PathBuf::from(cwd).join(attachment.path);
            }
            attachment
        })
        .collect();
    Ok(serde_json::to_value(crate::attachment::canonicalize(
        parsed,
    )?)?)
}

pub(super) fn agent_specs(args: &Value) -> Result<Value> {
    let agents = array(args, "agents")?;
    let specs = agents
        .as_array()
        .expect("validated agent array")
        .iter()
        .map(|value| {
            let raw = value.as_str().expect("validated string");
            let parsed = crate::idref::parse_agent_backend_ref(raw)
                .with_context(|| format!("malformed agent {raw:?}"))?;
            let backend = parsed
                .backend
                .context("channel agents require slug@backend")?;
            Ok(json!({"slug": parsed.slug, "backend": backend}))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!(specs))
}

pub(super) fn search_time(args: &Value, key: &str) -> Result<Value> {
    let Some(value) = args.get(key) else {
        return Ok(Value::Null);
    };
    if value.is_u64() {
        return Ok(value.clone());
    }
    let raw = value
        .as_str()
        .with_context(|| format!("{key} must be a timestamp or duration"))?;
    Ok(json!(super::super::chat_time::parse_time(raw)?))
}
