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
    let source = config::PI_EXTENSION_TS;
    for contract in [
        "session_start",
        "before_agent_start",
        "tool_call",
        "tool_result",
        "agent_settled",
        "MOSAICO_TRANSPORT",
        "MOSAICO_ENDPOINT_ID",
        "session_shutdown",
        "ctx.sessionManager.getSessionId()",
        "return { block: true, reason: result.message }",
    ] {
        assert!(source.contains(contract), "missing Pi contract {contract}");
    }
    let tools = config::PI_TOOLS_TS;
    for contract in ["registerTool", "mosaico_reply", "mosaico_channel_create"] {
        assert!(
            tools.contains(contract),
            "missing Pi tool contract {contract}"
        );
    }
    let protocol = config::PI_PROTOCOL_TS;
    for contract in ["[\"harness\", \"pi\"]", "ctx.sessionManager.getSessionId()"] {
        assert!(
            protocol.contains(contract),
            "missing Pi protocol contract {contract}"
        );
    }
}
