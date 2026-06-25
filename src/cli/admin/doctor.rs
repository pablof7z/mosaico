use super::*;

// ── doctor ───────────────────────────────────────────────────────────────────

pub async fn doctor() -> Result<()> {
    // The daemon owns the single relay connection, so it performs the probe.
    let v = daemon_call_async("doctor", serde_json::json!({})).await?;
    if let Some(relays) = v["relays"].as_array() {
        let relays: Vec<&str> = relays.iter().filter_map(|r| r.as_str()).collect();
        println!("relays: {relays:?}");
    }
    if let Some(pk) = v["probe_pubkey"].as_str() {
        println!("probe pubkey: {pk}");
    }
    println!("publish: {}", v["publish"].as_str().unwrap_or("?"));
    println!("read-back: {}", v["readback"].as_str().unwrap_or("?"));
    Ok(())
}
