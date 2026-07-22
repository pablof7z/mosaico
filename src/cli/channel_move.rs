use super::*;

pub(super) async fn accept(name: String, about: String) -> Result<()> {
    let response = daemon_call_async(
        "channel_move_accept",
        crate::cli::rpc_params(serde_json::json!({ "name": name, "about": about })),
    )
    .await?;
    let name = response["name"].as_str().unwrap_or("channel");
    let created = response["created"].as_bool().unwrap_or(false);
    let added = response["added"].as_array().map(Vec::len).unwrap_or(0);
    let requested = response["requested"].as_array().map(Vec::len).unwrap_or(0);
    let verb = if created { "created" } else { "reused" };
    println!("#{name} {verb}; added {added} agent(s)");
    if requested > 0 {
        println!("  requested {requested} remote move(s)");
    }
    if let Some(skipped) = response["skipped"]
        .as_array()
        .filter(|rows| !rows.is_empty())
    {
        eprintln!(
            "  skipped {} participant(s) that were no longer eligible",
            skipped.len()
        );
    }
    Ok(())
}
