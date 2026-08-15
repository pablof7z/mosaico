use super::*;

#[test]
fn opencode_bridge_blocks_denials_and_queues_warnings_for_the_model() {
    let source = config::OPENCODE_PLUGIN_TS;
    assert!(source.contains("\"tool.execute.before\""));
    assert!(source.contains("\"pre-tool-use\""));
    assert!(source.contains("throw new Error(result.message)"));
    assert!(source.contains("pendingBoundaryWarnings.push(result.message)"));
    assert!(source.contains("drainBoundaryWarnings(pendingBoundaryWarnings)"));
}

#[test]
fn opencode_executes_replacement_empty_clear_and_one_shot_warning_helpers() {
    let source = config::OPENCODE_PLUGIN_TS;
    let (_, tail) = source
        .split_once("// MOSAICO_CONTEXT_HELPERS_START")
        .expect("helper start marker");
    let (helpers, _) = tail
        .split_once("// MOSAICO_CONTEXT_HELPERS_END")
        .expect("helper end marker");
    let exercise = r#"
const stale = {parts: [
  {type: "text", text: "old", _mosaicoInjected: true},
  {type: "text", text: "user"}
]};
replaceMosaicoPart(stale);
if (stale.parts.length !== 1 || stale.parts[0].text !== "user") process.exit(10);
replaceMosaicoPart(stale, {type: "text", text: "current", _mosaicoInjected: true});
replaceMosaicoPart(stale, {type: "text", text: "newest", _mosaicoInjected: true});
if (stale.parts.length !== 2 || stale.parts[0].text !== "newest") process.exit(11);
const warnings = ["warn one", "warn two"];
if (drainBoundaryWarnings(warnings) !== "warn one\n\nwarn two") process.exit(12);
if (drainBoundaryWarnings(warnings) !== "") process.exit(13);
"#;
    let script = format!("{helpers}\n{exercise}");
    let node = std::env::var_os("NVM_BIN")
        .map(std::path::PathBuf::from)
        .map(|path| path.join("node"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| std::path::PathBuf::from("node"));
    let output = std::process::Command::new(node)
        .args(["--input-type=module", "--eval", &script])
        .output()
        .expect("Node is required to execute the installed OpenCode helper contract");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(source.contains("replaceMosaicoPart(message)"));
    assert!(source.contains("drainBoundaryWarnings(pendingBoundaryWarnings)"));
}

#[test]
fn pi_extension_uses_native_lifecycle_and_tool_boundaries() {
    // The extension is a direct UDS client of the Mosaico daemon: it registers
    // the exact Pi session, pumps durable inbox deliveries as custom context,
    // and routes native tool calls through `pi_tool_call`. The install path must
    // ship every module the extension imports, and the daemon protocol depends
    // on `caller()` anchoring harness="pi" plus the live session id.
    let entry = config::PI_EXTENSION_TS;
    for contract in [
        "session_start",
        "before_agent_start",
        "tool_call",
        "tool_result",
        "agent_settled",
        "session_shutdown",
        "message_start",
        "MOSAICO_TRANSPORT",
        "MOSAICO_PUBKEY",
        "registerMosaicoTools",
        "DeliveryPump",
        "paintSessionStatus",
        "return { block: true, reason: result.message }",
    ] {
        assert!(
            entry.contains(contract),
            "missing Pi entry contract {contract}"
        );
    }

    let protocol = config::PI_PROTOCOL_TS;
    for contract in [
        "ctx.sessionManager.getSessionId()",
        "harness: \"pi\"",
        "harness_session",
        "watch_pid",
        "MOSAICO_OBSERVED_HARNESS",
        "pi_tool_call",
        "channel_read",
        "isHostedPi",
    ] {
        assert!(
            protocol.contains(contract),
            "missing Pi protocol contract {contract}"
        );
    }

    let tools = config::PI_TOOLS_TS;
    for contract in [
        "registerTool",
        "registerMosaicoTools",
        "mosaico_reply",
        "mosaico_channel_create",
        "mosaico_dispatch",
        "execute",
        "readChannel",
    ] {
        assert!(
            tools.contains(contract),
            "missing Pi tool contract {contract}"
        );
    }

    // The installer must ship every module the extension imports, so a Pi load
    // never fails on a missing sibling file.
    let shipped: std::collections::HashSet<&str> = config::PI_EXTENSION_FILES
        .iter()
        .map(|(name, _)| *name)
        .collect();
    for name in [
        "index.ts",
        "delivery.ts",
        "protocol.ts",
        "status.ts",
        "tools.ts",
    ] {
        assert!(shipped.contains(name), "Pi install missing {name}");
    }
}
