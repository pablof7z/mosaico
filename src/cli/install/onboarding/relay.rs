//! Relay reachability + NIP-29 capability probing for onboarding.
//!
//! A relay is "usable" only when it is reachable and announces NIP-29 support
//! in its NIP-11 document. We fetch NIP-11 over HTTP (`Accept:
//! application/nostr+json`), which proves both reachability and the announced
//! capability in a single request.

use serde::Deserialize;

/// Outcome of probing a single relay URL.
#[derive(Debug, Clone)]
pub(super) enum Probe {
    /// Reachable and its NIP-11 document lists NIP-29.
    Usable,
    /// Reachable but NIP-29 is absent from the announced `supported_nips`.
    MissingNip29,
    /// Could not reach or parse the relay's NIP-11 document.
    Unreachable(String),
}

#[derive(Deserialize)]
struct Nip11 {
    #[serde(default)]
    supported_nips: Vec<serde_json::Value>,
}

/// Translate a `ws://`/`wss://` relay URL to the `http(s)://` origin used to
/// fetch its NIP-11 document.
fn nip11_http_url(relay: &str) -> Result<String, String> {
    let parsed = url::Url::parse(relay).map_err(|e| format!("invalid relay URL: {e}"))?;
    let scheme = match parsed.scheme() {
        "ws" | "http" => "http",
        "wss" | "https" => "https",
        other => {
            return Err(format!(
                "relay URL must be ws:// or wss:// (got {other}://)"
            ))
        }
    };
    let host = parsed
        .host_str()
        .ok_or_else(|| "relay URL has no host".to_string())?;
    let mut out = format!("{scheme}://{host}");
    if let Some(port) = parsed.port() {
        out.push_str(&format!(":{port}"));
    }
    let path = parsed.path().trim_end_matches('/');
    out.push_str(path);
    Ok(out)
}

/// Probe a relay URL once. Never panics; all failures fold into `Unreachable`.
pub(super) async fn probe(relay: &str) -> Probe {
    let http = match nip11_http_url(relay) {
        Ok(u) => u,
        Err(e) => return Probe::Unreachable(e),
    };
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Probe::Unreachable(format!("http client: {e}")),
    };
    let resp = client
        .get(&http)
        .header(reqwest::header::ACCEPT, "application/nostr+json")
        .send()
        .await;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Probe::Unreachable(short_error(&e.to_string())),
    };
    let doc: Nip11 = match resp.json().await {
        Ok(d) => d,
        Err(_) => return Probe::Unreachable("relay did not return a NIP-11 document".into()),
    };
    if announces_nip29(&doc.supported_nips) {
        Probe::Usable
    } else {
        Probe::MissingNip29
    }
}

/// NIP-29 may appear as the number `29` or a string like `"29"`.
fn announces_nip29(nips: &[serde_json::Value]) -> bool {
    nips.iter()
        .any(|n| n.as_i64() == Some(29) || n.as_str().is_some_and(|s| s.trim() == "29"))
}

fn short_error(msg: &str) -> String {
    let trimmed = msg.trim();
    if trimmed.len() > 120 {
        format!("{}…", &trimmed[..120])
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_ws_schemes_to_http_origins() {
        assert_eq!(
            nip11_http_url("ws://127.0.0.1:9888").unwrap(),
            "http://127.0.0.1:9888"
        );
        assert_eq!(
            nip11_http_url("wss://relay.example/").unwrap(),
            "https://relay.example"
        );
        assert_eq!(
            nip11_http_url("wss://relay.example/nostr").unwrap(),
            "https://relay.example/nostr"
        );
    }

    #[test]
    fn rejects_non_ws_schemes() {
        assert!(nip11_http_url("ftp://relay.example").is_err());
    }

    #[test]
    fn detects_nip29_as_number_or_string() {
        assert!(announces_nip29(&[
            serde_json::json!(1),
            serde_json::json!(29)
        ]));
        assert!(announces_nip29(&[serde_json::json!("29")]));
        assert!(!announces_nip29(&[
            serde_json::json!(1),
            serde_json::json!(11)
        ]));
    }
}
