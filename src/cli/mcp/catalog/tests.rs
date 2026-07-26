use super::*;

fn tool<'a>(tools: &'a [Value], name: &str) -> &'a Value {
    tools
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap_or_else(|| panic!("missing tool {name}"))
}

#[test]
fn catalog_contains_agent_coordination_tools_without_legacy_names() {
    let tools = list();
    for name in [
        "mosaico.skill",
        "mosaico.wait",
        "mosaico.channel_join",
        "mosaico.channel_send",
        "mosaico.dispatch",
        "mosaico.my_session",
    ] {
        tool(&tools, name);
    }
    for name in ["mosaico.who", "mosaico.channels_join", "mosaico.chat_write"] {
        assert!(tools.iter().all(|candidate| candidate["name"] != name));
    }
}

#[test]
fn wait_schema_exposes_ambient_and_correlated_forms() {
    let tools = list();
    let wait = tool(&tools, "mosaico.wait");
    assert_eq!(wait["annotations"]["readOnlyHint"], true);
    assert_eq!(wait["inputSchema"]["required"], json!(["timeout_seconds"]));
    for property in ["timeout_seconds", "channels", "from", "session"] {
        assert!(wait["inputSchema"]["properties"].get(property).is_some());
    }

    let send = tool(&tools, "mosaico.channel_send");
    assert_eq!(
        send["inputSchema"]["properties"]["wait_seconds"]["type"],
        "integer"
    );
}

#[test]
fn channel_list_schema_uses_the_shared_path_projection_modes() {
    let tools = list();
    let list = tool(&tools, "mosaico.channel_list");
    let properties = &list["inputSchema"]["properties"];
    for property in ["workspace", "all", "recursive", "session"] {
        assert!(properties.get(property).is_some(), "missing {property}");
    }
    assert!(properties.get("channel").is_none());
    assert_eq!(list["annotations"]["readOnlyHint"], true);
}

#[test]
fn channel_create_uses_one_absolute_path_contract() {
    let tools = list();
    let create = tool(&tools, "mosaico.channel_create");
    let properties = &create["inputSchema"]["properties"];
    assert!(properties.get("channel").is_some());
    assert!(properties.get("name").is_none());
    assert!(properties.get("parent_channel").is_none());
    assert_eq!(
        create["inputSchema"]["required"],
        json!(["channel", "about"])
    );
}
