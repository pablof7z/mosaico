//! Local catalog for `tenex-edge debug validate --targets`.

use serde_json::{json, Value};

#[derive(Clone, Copy)]
struct TargetForm {
    target: &'static str,
    proves: &'static str,
    example: &'static str,
}

const TARGET_FORMS: &[TargetForm] = &[
    TargetForm {
        target: "all",
        proves: "oracle, host seams, resource accounting, and live-session consistency",
        example: "tenex-edge debug validate all",
    },
    TargetForm {
        target: "state:<surface> | status | subscriptions | hook_context | turn_lifecycle | cursor | delivery | session_start | session_watch | outbox",
        proves: "surface state, oracle status, seams, resource drift, and live rows",
        example: "tenex-edge debug validate state:status",
    },
    TargetForm {
        target: "coverage | validation_coverage | inventory",
        proves: "durable table inventory, validation target families, and uncovered ledgers",
        example: "tenex-edge debug validate coverage",
    },
    TargetForm {
        target: "table:<name> | ledger:<name>",
        proves: "one durable table's presence, row count, columns, target family, sample handles, and meaning",
        example: "tenex-edge debug validate table:messages",
    },
    TargetForm {
        target: "lookup:<value> | find:<value> | id:<value> | <raw-id-or-nip19>",
        proves: "durable identifier matches for raw ids, hex pubkeys, and NIP-19 npub/nprofile/note/nevent handles, plus concrete validation handles to run next",
        example: "tenex-edge debug validate <event-id-or-npub>",
    },
    TargetForm {
        target: "status:<session>",
        proves: "status graph, local session row, relay status rows, and channel agreement",
        example: "tenex-edge debug validate status:<session>",
    },
    TargetForm {
        target: "sub:<channel> | sub/<h|d|p>/<id>",
        proves: "subscription resources, owners, refcounts, planner causes, and receipts",
        example: "tenex-edge debug validate sub:<channel>",
    },
    TargetForm {
        target: "hook:<session>[@time] | hook_context:<session>",
        proves: "hook render graph, receipt match, session channel, and roster/channel invariants",
        example: "tenex-edge debug validate hook:<session>",
    },
    TargetForm {
        target: "turn:<session> | turn_lifecycle:<session> | cursor:<session> | cur:<session>",
        proves: "live Trellis projection agrees with the local session row",
        example: "tenex-edge debug validate turn:<session>",
    },
    TargetForm {
        target: "session_start:<session> | watch:<session> | session_watch:<session>",
        proves: "advisory launch/watch state, host-effect intent, failures, pid liveness",
        example: "tenex-edge debug validate session_start:<session>",
    },
    TargetForm {
        target: "outbox:<id>",
        proves: "Trellis outbox projection, durable queue row, relay outcome, and errors",
        example: "tenex-edge debug validate outbox:<id>",
    },
    TargetForm {
        target: "channel:<h> | readiness:<h> | channel_ready:<h>",
        proves: "relay channel metadata, member/admin invariants, readiness attempts, subscriptions",
        example: "tenex-edge debug validate readiness:<channel>",
    },
    TargetForm {
        target: "readiness_attempt:<id> | provider_attempt:<id>",
        proves: "one provider readiness decision, its outcome/reason, and current channel corroboration",
        example: "tenex-edge debug validate readiness_attempt:<id>",
    },
    TargetForm {
        target: "awareness:<h> | who:<h>",
        proves: "confirmed channel roster, active members, sessions, aliases, and identity rows",
        example: "tenex-edge debug validate awareness:<channel>",
    },
    TargetForm {
        target: "session:<session> | joined:<session>[:channel] | session_channel:<session>",
        proves: "local session, current channel binding, joined channel, and active subscriptions",
        example: "tenex-edge debug validate joined:<session>",
    },
    TargetForm {
        target: "alias:<harness>:<kind>:<value> | harness_session:<harness>:<id> | resume:<harness>:<id> | pty_session:<id> | watch_pid:<pid>",
        proves: "session alias resolution, canonical session binding, and live-session surfaces",
        example: "tenex-edge debug validate pty_session:<id>",
    },
    TargetForm {
        target: "identity:<pubkey> | profile:<pubkey> | pubkey:<pubkey> | agent:<slug> | backend:<label>",
        proves: "profile rows, local identities, bound sessions, and membership/admin channels",
        example: "tenex-edge debug validate agent:codex",
    },
    TargetForm {
        target: "workspace:<channel> | work_root:<channel>",
        proves: "channel workspace binding and local filesystem path validity",
        example: "tenex-edge debug validate workspace:<channel>",
    },
    TargetForm {
        target: "member:<channel>:<pubkey> | admin:<channel>:<pubkey>",
        proves: "member/admin row, role, profile/session context, and membership snapshot absence",
        example: "tenex-edge debug validate admin:<channel>:<pubkey>",
    },
    TargetForm {
        target: "membership_snapshot:<channel> | roster:<channel>",
        proves: "admin/member snapshot hydration, high-water marks, roster counts, and admin presence",
        example: "tenex-edge debug validate membership_snapshot:<channel>",
    },
    TargetForm {
        target: "message:<id> | msg:<id> | event:<id>",
        proves: "message/event materialization, receipts, relay event state, and outbox state",
        example: "tenex-edge debug validate event:<event>",
    },
    TargetForm {
        target: "recipient:<event>:<pubkey>[:session] | delivery:<event>:<pubkey>",
        proves: "recipient edge presence, delivery timestamp, profile/session context, and exclusions",
        example: "tenex-edge debug validate recipient:<event>:<pubkey>",
    },
    TargetForm {
        target: "inbox:<event> | quarantine:<event>",
        proves: "inbound processing or quarantine state, errors, and local materialization",
        example: "tenex-edge debug validate inbox:<event>",
    },
    TargetForm {
        target: "llm:<id> | txn:<surface>:<id> | receipt:<id> | commit:<id> | trellis_commit:<id>",
        proves: "durable Trellis receipts, LLM call evidence, transaction commits, and ledger payloads",
        example: "tenex-edge debug validate commit:<id>",
    },
    TargetForm {
        target: "capsule:<id> | planner:<label> | --fact JSON | --cause LABEL",
        proves: "replay capsules, planner-label classification, dry-run simulation, and acid checks",
        example: "tenex-edge debug validate --fact '{\"StatusDrive\":{\"Tick\":{\"session_id\":\"s1\",\"at\":1}}}'",
    },
];

pub(super) fn catalog_json() -> Value {
    json!({
        "verb": "validate_targets",
        "target_forms": TARGET_FORMS.iter().map(|form| {
            json!({
                "target": form.target,
                "proves": form.proves,
                "example": form.example,
            })
        }).collect::<Vec<_>>(),
    })
}

pub(super) fn render_catalog() -> String {
    let mut out = String::new();
    out.push_str("validate target catalog\n");
    out.push_str("  validate returns a verdict, named checks, limitations, and evidence tails.\n");
    out.push_str(
        "  Add --json to any normal validation command for the full machine-readable envelope.\n\n",
    );
    for form in TARGET_FORMS {
        out.push_str(&format!("  {}\n", form.target));
        out.push_str(&format!("    proves:  {}\n", form.proves));
        out.push_str(&format!("    example: {}\n", form.example));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_specific_supported_target_forms() {
        let text = render_catalog();
        assert!(text.contains("validate target catalog"));
        assert!(text.contains("state:<surface>"));
        assert!(text.contains("status | subscriptions | hook_context"));
        assert!(text.contains("coverage | validation_coverage | inventory"));
        assert!(text.contains("table:<name> | ledger:<name>"));
        assert!(text.contains("lookup:<value> | find:<value> | id:<value> | <raw-id-or-nip19>"));
        assert!(text.contains("sample handles"));
        assert!(text.contains("readiness:<h>"));
        assert!(text.contains("readiness_attempt:<id>"));
        assert!(text.contains("provider_attempt:<id>"));
        assert!(text.contains("alias:<harness>:<kind>:<value>"));
        assert!(text.contains("harness_session:<harness>:<id>"));
        assert!(text.contains("pty_session:<id>"));
        assert!(text.contains("watch_pid:<pid>"));
        assert!(text.contains("profile:<pubkey>"));
        assert!(text.contains("admin:<channel>:<pubkey>"));
        assert!(text.contains("membership_snapshot:<channel>"));
        assert!(text.contains("roster:<channel>"));
        assert!(text.contains("recipient:<event>:<pubkey>[:session]"));
        assert!(text.contains("delivery:<event>:<pubkey>"));
        assert!(text.contains("trellis_commit:<id>"));
        assert!(text.contains("planner:<label>"));
    }

    #[test]
    fn catalog_json_matches_rendered_forms() {
        let json = catalog_json();
        let rows = json["target_forms"].as_array().unwrap();
        assert_eq!(json["verb"], "validate_targets");
        assert_eq!(rows.len(), TARGET_FORMS.len());
        assert!(rows
            .iter()
            .any(|row| row["target"] == "channel:<h> | readiness:<h> | channel_ready:<h>"));
        assert!(rows
            .iter()
            .any(|row| row["target"] == "coverage | validation_coverage | inventory"));
        assert!(rows
            .iter()
            .any(|row| row["target"] == "table:<name> | ledger:<name>"));
        assert!(rows
            .iter()
            .any(|row| row["target"]
                == "lookup:<value> | find:<value> | id:<value> | <raw-id-or-nip19>"));
        assert!(rows
            .iter()
            .any(|row| row["target"] == "readiness_attempt:<id> | provider_attempt:<id>"));
        assert!(rows
            .iter()
            .any(|row| row["target"] == "membership_snapshot:<channel> | roster:<channel>"));
    }
}
