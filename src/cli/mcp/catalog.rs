use serde_json::{json, Value};

struct ToolSpec {
    name: &'static str,
    description: &'static str,
    props: &'static [Prop],
    required: &'static [&'static str],
    read_only: bool,
    destructive: bool,
}

struct Prop {
    name: &'static str,
    ty: &'static str,
    description: &'static str,
}

pub(super) fn list() -> Vec<Value> {
    SPECS.iter().map(def).collect()
}

pub(super) fn requires_write(name: &str) -> bool {
    SPECS
        .iter()
        .find(|spec| spec.name == name)
        .is_some_and(|spec| !spec.read_only)
}

const SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "tenex_edge.who",
        description: "Read current tenex-edge awareness.",
        props: &[
            Prop::new("channel", "string", "Channel or channel id to inspect."),
            Prop::new("all_roots", "boolean", "Return all channel awareness."),
        ],
        required: &[],
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "tenex_edge.channels_list",
        description: "List channels under a channel.",
        props: &[Prop::new(
            "channel",
            "string",
            "Channel slug. Defaults to current directory channel.",
        )],
        required: &[],
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "tenex_edge.chat_read",
        description: "Read recent messages from a channel.",
        props: &[
            Prop::new("channel", "string", "Optional channel destination."),
            Prop::new("session", "string", "Explicit tenex-edge session id."),
            Prop::new("limit", "integer", "Maximum messages to return."),
            Prop::new("since", "string", "Unix timestamp or duration like 2h."),
            Prop::new("id", "string", "Read one message by id prefix."),
        ],
        required: &[],
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "tenex_edge.chat_write",
        description: "Send a message to a channel.",
        props: &[
            Prop::new("message", "string", "Message body."),
            Prop::new("channel", "string", "Optional destination channel."),
            Prop::new("session", "string", "Explicit tenex-edge session id."),
            Prop::new("long_message", "boolean", "Allow long messages."),
        ],
        required: &["message"],
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "tenex_edge.channels_create",
        description: "Create and join a task channel.",
        props: &[
            Prop::new("name", "string", "Human channel name."),
            Prop::new("about", "string", "Short stable channel description."),
            Prop::new("parent_channel", "string", "Parent channel reference."),
            Prop::new("agents", "array", "Agent targets as slug@backend strings."),
            Prop::new("session", "string", "Explicit tenex-edge session id."),
        ],
        required: &["name", "about"],
        read_only: false,
        destructive: false,
    },
    channel_tool(
        "tenex_edge.channels_join",
        "Join a channel for passive context.",
        false,
    ),
    channel_tool(
        "tenex_edge.channels_leave",
        "Leave a passively joined channel.",
        true,
    ),
    channel_tool(
        "tenex_edge.channels_switch",
        "Switch the active session channel.",
        true,
    ),
];

const CHANNEL_PROPS: &[Prop] = &[
    Prop::new("channel", "string", "Channel name, path, or opaque id."),
    Prop::new("session", "string", "Explicit tenex-edge session id."),
];

const fn channel_tool(
    name: &'static str,
    description: &'static str,
    destructive: bool,
) -> ToolSpec {
    ToolSpec {
        name,
        description,
        props: CHANNEL_PROPS,
        required: &["channel"],
        read_only: false,
        destructive,
    }
}

impl Prop {
    const fn new(name: &'static str, ty: &'static str, description: &'static str) -> Self {
        Self {
            name,
            ty,
            description,
        }
    }
}

fn def(spec: &ToolSpec) -> Value {
    let schemes = security_schemes(spec);
    json!({
        "name": spec.name,
        "title": spec.name,
        "description": spec.description,
        "inputSchema": schema(spec.props, spec.required),
        "securitySchemes": schemes,
        "_meta": {
            "securitySchemes": schemes,
        },
        "annotations": {
            "readOnlyHint": spec.read_only,
            "destructiveHint": spec.destructive,
        },
    })
}

fn security_schemes(spec: &ToolSpec) -> Value {
    let scopes = if spec.read_only {
        json!(["tenex:read"])
    } else {
        json!(["tenex:read", "tenex:write"])
    };
    json!([{ "type": "oauth2", "scopes": scopes }])
}

fn schema(props: &[Prop], required: &[&str]) -> Value {
    let properties = props
        .iter()
        .map(|prop| {
            let value = if prop.ty == "array" {
                json!({
                    "type": "array",
                    "items": { "type": "string" },
                    "description": prop.description,
                })
            } else {
                json!({ "type": prop.ty, "description": prop.description })
            };
            (prop.name.to_string(), value)
        })
        .collect::<serde_json::Map<_, _>>();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn catalog_contains_channel_basics() {
        let names = super::list()
            .into_iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_string))
            .collect::<Vec<_>>();

        assert!(names.contains(&"tenex_edge.channels_join".to_string()));
        assert!(names.contains(&"tenex_edge.chat_write".to_string()));
    }
}
