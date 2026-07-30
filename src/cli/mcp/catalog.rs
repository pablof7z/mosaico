use serde_json::{json, Value};

mod specs;

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
    specs::SPECS.iter().map(def).collect()
}

pub(super) fn requires_write(name: &str) -> bool {
    specs::SPECS
        .iter()
        .find(|spec| spec.name == name)
        .is_some_and(|spec| !spec.read_only)
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
