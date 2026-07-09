//! `tenex-edge debug explain <handle>` — point at an artifact, see what produced it.
//!
//! The store is daemon-owned, so this thin verb forwards the handle to the
//! `explain` RPC (like `who`) and renders the joined record it returns. For a
//! published kind:30315 (`event:<id>`) the headline is the exact LLM inputs —
//! system prompt, transcript slice, model, raw response — that produced its
//! activity text. `--redact` swaps the bulky user-content bodies for
//! `sha256:<hash> (<n> bytes)` placeholders; `--json` emits the raw joined record.

use anyhow::Result;
use clap::Args;
use serde_json::Value;

#[derive(Args)]
pub(super) struct ExplainArgs {
    /// The artifact handle: `event:<id>`, `llm:<id>`, `session:<id>[@<ts>]`,
    /// `hook:<id>[@<ts>]`, `txn:<surface>:<id>[@<ts>]`, or `sub:<channel>`.
    pub(super) handle: String,
    /// Emit the raw joined record as JSON instead of the human view.
    #[arg(long)]
    pub(super) json: bool,
    /// Replace prompt/transcript/response bodies with `sha256:<hash> (<n> bytes)`.
    #[arg(long)]
    pub(super) redact: bool,
}

pub(super) fn explain(args: ExplainArgs) -> Result<()> {
    // Validate the handle locally for a crisp error before hitting the daemon.
    crate::explain::parse_handle(&args.handle)?;
    let params = serde_json::json!({ "handle": args.handle });
    let mut v = crate::daemon::blocking::call("explain", params)?;
    if args.redact {
        redact(&mut v);
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        print!("{}", render(&args.handle, &v));
    }
    Ok(())
}

/// Replace the three bulky user-content bodies in the llm_call with hashed
/// placeholders. Never touches receipts (already Trellis-free metadata).
fn redact(v: &mut Value) {
    if let Some(call) = v.get_mut("llm_call").and_then(Value::as_object_mut) {
        for field in ["system_prompt", "transcript_slice", "raw_response"] {
            if let Some(body) = call.get(field).and_then(Value::as_str) {
                let placeholder = format!(
                    "{} ({} bytes)",
                    crate::instrument::window_hash(body),
                    body.len()
                );
                call.insert(field.to_string(), Value::String(placeholder));
            }
        }
    }
}

/// Human view: the receipts that explain the artifact, then — the headline — the
/// exact LLM inputs it rejoined to.
fn render(handle: &str, v: &Value) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let kind = v.get("kind").and_then(Value::as_str).unwrap_or("?");
    let _ = writeln!(out, "explain {handle}  ({kind})\n");

    let receipts = v.get("receipts").and_then(Value::as_array);
    match receipts.filter(|r| !r.is_empty()) {
        None => {
            let _ = writeln!(out, "receipts: (none found for this handle)");
        }
        Some(rows) => {
            let _ = writeln!(out, "receipts:");
            for r in rows {
                let s = |k| r.get(k).and_then(Value::as_str).unwrap_or("");
                let n = |k| r.get(k).and_then(Value::as_i64).unwrap_or(0);
                let aref = r
                    .get("artifact_ref")
                    .and_then(Value::as_str)
                    .unwrap_or("(none)");
                let _ = writeln!(
                    out,
                    "  [{}] txn {} rev {} → {}  (at {})",
                    s("surface"),
                    n("transaction_id"),
                    n("revision"),
                    aref,
                    n("created_at"),
                );
                let _ = writeln!(out, "      changed: {}", s("changed_summary"));
                let _ = writeln!(out, "      commands: {}", s("commands"));
            }
        }
    }

    if let Some(call) = v.get("llm_call").filter(|c| !c.is_null()) {
        let s = |k| call.get(k).and_then(Value::as_str).unwrap_or("");
        let _ = writeln!(
            out,
            "\nLLM inputs — what was fed to the model so it said this:"
        );
        let _ = writeln!(out, "  provider/model: {} / {}", s("provider"), s("model"));
        let _ = writeln!(out, "  window_hash:    {}", s("window_hash"));
        let _ = writeln!(
            out,
            "  parsed:         title={:?} activity={:?}",
            s("parsed_title"),
            s("parsed_activity")
        );
        let _ = writeln!(out, "\n  system prompt:\n{}", indent(s("system_prompt")));
        let _ = writeln!(
            out,
            "  transcript slice (fed to the model):\n{}",
            indent(s("transcript_slice"))
        );
        let _ = writeln!(out, "  raw response:\n{}", indent(s("raw_response")));
    } else if kind == "event" {
        let _ = writeln!(
            out,
            "\nLLM inputs: (none — this publish was not distill-driven)"
        );
    }
    out
}

/// Indent a multi-line body by four spaces for the human view.
fn indent(body: &str) -> String {
    body.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The joined record shape the `explain` engine returns for a distill-driven
    /// kind:30315 (see `crate::explain`), used to exercise the human renderer.
    fn event_fixture() -> Value {
        serde_json::json!({
            "kind": "event",
            "receipts": [{
                "id": 1, "surface": "status", "transaction_id": 5, "revision": 2,
                "changed_summary": "{\"inputs\":[],\"session_id\":\"te-a1b2\",\"window_hash\":\"sha256:9f2c\"}",
                "commands": "[{\"kind\":\"replace\",\"key\":\"status/te-a1b2\",\"reason\":\"replace\"}]",
                "artifact_ref": "e3f9…30315", "created_at": 1_720_000_000_000i64,
            }],
            "llm_call": {
                "provider": "claude-cli", "model": "claude-haiku",
                "window_hash": "sha256:9f2c", "parsed_title": "Fix flaky auth test",
                "parsed_activity": "reading the failing assertion",
                "system_prompt": "You maintain two labels for a coding session...",
                "transcript_slice": "CURRENT TITLE: Fix flaky auth test\n\nTRANSCRIPT:\nuser: the login test fails intermittently",
                "raw_response": "TITLE: Fix flaky auth test\nNOW: reading the failing assertion",
            },
        })
    }

    #[test]
    fn event_render_surfaces_the_llm_inputs() {
        let text = render("event:e3f9…30315", &event_fixture());
        assert!(text.contains("[status] txn 5 rev 2 → e3f9…30315"));
        assert!(text.contains("LLM inputs — what was fed to the model"));
        assert!(text.contains("claude-cli / claude-haiku"));
        assert!(text.contains("You maintain two labels"));
        assert!(text.contains("the login test fails intermittently"));
    }

    #[test]
    fn redact_swaps_bodies_for_hashed_placeholders() {
        let mut v = event_fixture();
        redact(&mut v);
        let sp = v["llm_call"]["system_prompt"].as_str().unwrap();
        assert!(sp.starts_with("sha256:") && sp.ends_with(" bytes)"));
        // A redacted body no longer contains the raw user content.
        assert!(!v["llm_call"]["transcript_slice"]
            .as_str()
            .unwrap()
            .contains("login test"));
    }
}
