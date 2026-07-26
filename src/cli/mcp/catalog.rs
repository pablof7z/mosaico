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
        name: "mosaico.skill",
        description: "Load the mosaico agent skill (or a named reference page). \
                      Use before coordinating on the fabric when you lack a local \
                      skill install. Omit name for the entry; name=list for the index; \
                      name=identity-and-capabilities|coordination-guide|… for a page.",
        props: &[Prop::new(
            "name",
            "string",
            "Skill page: omit or \"skill\" for entry; \"list\" for index; or a reference stem.",
        )],
        required: &[],
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "mosaico.my_session",
        description: "Read the current agent session and full mosaico awareness.",
        props: &[],
        required: &[],
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "mosaico.wait",
        description: "Wait for the next matching message without polling. Returns a message or \
                      timeout outcome.",
        props: &[
            Prop::new("timeout_seconds", "integer", "Maximum seconds to wait."),
            Prop::new("channels", "array", "Optional joined channels to watch."),
            Prop::new("from", "string", "Optional human or agent author filter."),
            SESSION_PROP,
        ],
        required: &["timeout_seconds"],
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "mosaico.channel_list",
        description: "List the caller-aware workspace/channel forest with public paths. By \
                      default, own and joined workspaces are expanded and other workspaces are \
                      compact.",
        props: &[
            Prop::new(
                "workspace",
                "string",
                "Expand only this workspace root. Mutually exclusive with all and recursive.",
            ),
            Prop::new(
                "all",
                "boolean",
                "Return every workspace root as a compact inventory.",
            ),
            Prop::new(
                "recursive",
                "boolean",
                "Expand every known workspace and channel, including unjoined ones.",
            ),
            SESSION_PROP,
        ],
        required: &[],
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "mosaico.channel_read",
        description: "Read recent messages from a channel.",
        props: &[
            Prop::new(
                "channel",
                "string",
                "Full channel path (/workspace/child). Must already be joined.",
            ),
            SESSION_PROP,
            Prop::new("limit", "integer", "Maximum messages to return."),
            Prop::new("since", "string", "Unix timestamp or duration like 2h."),
            Prop::new("id", "string", "Read one message by id prefix."),
        ],
        required: &[],
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "mosaico.channel_send",
        description: "Send a message to a channel.",
        props: &[
            Prop::new("message", "string", "Message body."),
            Prop::new("tags", "array", "Agent names to tag."),
            Prop::new(
                "force",
                "boolean",
                "Allow literal mention-like text without tags.",
            ),
            Prop::new(
                "channel",
                "string",
                "Full channel path (/workspace/child). Must already be joined.",
            ),
            SESSION_PROP,
            Prop::new("long_message", "boolean", "Allow long messages."),
            Prop::new(
                "wait_seconds",
                "integer",
                "After sending, wait this many seconds for a correlated reply.",
            ),
            Prop::new(
                "reply_to",
                "string",
                "Reply to this message id (short prefix from channel_read). \
                 Threads the reply onto the original message and routes it to \
                 that channel; tags/force/channel are ignored when set.",
            ),
        ],
        required: &["message"],
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "mosaico.channel_create",
        description: "Create and join a task channel.",
        props: &[
            Prop::new(
                "channel",
                "string",
                "Full absolute path for the new leaf (/workspace/epic/child).",
            ),
            Prop::new("about", "string", "Short stable channel description."),
            Prop::new("agents", "array", "Agent targets as slug@backend strings."),
            SESSION_PROP,
        ],
        required: &["channel", "about"],
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "mosaico.react",
        description: "React to a specific message with an emoji — a non-disruptive \
                      acknowledgement that never interrupts the target's turn. Prefer \
                      this over a chat reply for a bare ack (\"got it\", 👍, ✅).",
        props: &[
            Prop::new("message_id", "string", "Target message id or short prefix."),
            Prop::new("emoji", "string", "Reaction emoji (e.g. 👍 ✅ 👀) or +/-."),
            SESSION_PROP,
        ],
        required: &["message_id", "emoji"],
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "mosaico.dispatch",
        description: "Start a new fabric agent session and join it to channels. \
                      Use to bring a capability online that is not already present; \
                      prefer messaging an existing session that already owns the work.",
        props: &[
            Prop::new(
                "target",
                "string",
                "Agent target as agent or agent@backend-label.",
            ),
            Prop::new("workspace", "string", "Workspace/root channel to run in."),
            Prop::new(
                "channels",
                "array",
                "Fully-qualified channels to join. Defaults to the workspace root.",
            ),
            Prop::new(
                "message",
                "string",
                "Opening message delivered after the new session ACKs.",
            ),
            SESSION_PROP,
        ],
        required: &["target", "workspace", "message"],
        read_only: false,
        destructive: false,
    },
    channel_tool(
        "mosaico.channel_join",
        "Join a channel for passive context.",
        false,
    ),
    channel_tool(
        "mosaico.channel_leave",
        "Leave a passively joined channel.",
        true,
    ),
];

const SESSION_PROP: Prop = Prop::new(
    "session",
    "string",
    "Public session npub, hex pubkey, or handle.",
);
const CHANNEL_PROPS: &[Prop] = &[
    Prop::new("channel", "string", "Full channel path (/workspace/child)."),
    SESSION_PROP,
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
        json!(["mosaico:read"])
    } else {
        json!(["mosaico:read", "mosaico:write"])
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
#[path = "catalog/tests.rs"]
mod tests;
