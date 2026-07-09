//! `tenex-edge debug validate` - user-facing validation with explanations.
//!
//! This is intentionally a thin client over the daemon's existing `probe`
//! validation verb, so the visible command and the hidden diagnostic surface
//! cannot drift in meaning.

use anyhow::Result;
use clap::Args;
use serde_json::{json, Value};

mod targets;

#[derive(Args)]
pub(in crate::cli) struct ValidateArgs {
    /// Optional surface, probe handle, Trellis resource path, explain handle,
    /// event/message/recipient target, awareness target,
    /// channel/readiness/readiness_attempt target, commit target, or `capsule:<id>`.
    target: Option<String>,
    /// List supported validation target forms and examples without calling the daemon.
    #[arg(long)]
    targets: bool,
    /// Exact serde JSON for `InputFact`; adds preview and acid evidence.
    #[arg(long, value_name = "JSON")]
    fact: Option<String>,
    /// Replay capsule id to assert; equivalent to target `capsule:<id>`.
    #[arg(long)]
    capsule: Option<String>,
    /// Specific cause label for acid validation.
    #[arg(long)]
    cause: Option<String>,
    /// Only count stats evidence with `created_at` at/after this unix-millis stamp.
    #[arg(long, default_value_t = 0)]
    since: i64,
    /// Emit the raw JSON the daemon returned instead of the human view.
    #[arg(long)]
    json: bool,
}

impl ValidateArgs {
    fn wants_catalog(&self) -> bool {
        self.targets
            || matches!(
                self.target.as_deref(),
                Some("targets" | "target_catalog" | "target-catalog")
            )
    }

    fn to_probe_rpc(&self) -> Result<Value> {
        Ok(super::rpc_params(json!({
            "verb": "validate",
            "target": self.target,
            "fact": self.fact,
            "capsule": self.capsule,
            "cause": self.cause,
            "since": self.since,
        })))
    }
}

pub(in crate::cli) async fn validate(args: ValidateArgs) -> Result<()> {
    if args.wants_catalog() {
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&targets::catalog_json())?
            );
        } else {
            print!("{}", targets::render_catalog());
        }
        return Ok(());
    }

    let json_output = args.json;
    let params = args.to_probe_rpc()?;
    let v = super::daemon_call_async("probe", params).await?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        print!("{}", super::probe::validate_render::render_validate(&v));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn validate_args_workspace_probe_rpc_params() {
        let args = ValidateArgs {
            target: Some("status:s1".into()),
            targets: false,
            fact: Some(r#"{"StatusDrive":{"Tick":{"session_id":"s1","at":1}}}"#.into()),
            capsule: Some("9".into()),
            cause: Some("status/s1/activity".into()),
            since: 42,
            json: true,
        };
        let params = args.to_probe_rpc().unwrap();

        assert_eq!(params["verb"], "validate");
        assert_eq!(params["target"], "status:s1");
        assert_eq!(
            params["fact"],
            r#"{"StatusDrive":{"Tick":{"session_id":"s1","at":1}}}"#
        );
        assert_eq!(params["capsule"], "9");
        assert_eq!(params["cause"], "status/s1/activity");
        assert_eq!(params["since"], 42);
    }

    #[test]
    fn validate_args_preserve_bad_fact_text_for_daemon_evidence() {
        let args = ValidateArgs {
            target: None,
            targets: false,
            fact: Some("not json".into()),
            capsule: None,
            cause: None,
            since: 0,
            json: false,
        };

        let params = args.to_probe_rpc().unwrap();
        assert_eq!(params["fact"], "not json");
    }

    #[test]
    fn validate_command_parses_as_debug_subcommand() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "tenex-edge",
            "debug",
            "validate",
            "status:s1",
            "--since",
            "42",
            "--json",
        ])
        .expect("validate command parses");

        match cli.cmd {
            crate::cli::args::Cmd::Debug {
                action: crate::cli::debug::DebugAction::Validate(args),
            } => {
                assert_eq!(args.target.as_deref(), Some("status:s1"));
                assert!(!args.targets);
                assert_eq!(args.since, 42);
                assert!(args.json);
            }
            _ => panic!("expected validate command"),
        }
    }

    #[test]
    fn validate_targets_alias_requests_local_catalog() {
        let cli =
            crate::cli::args::Cli::try_parse_from(["tenex-edge", "debug", "validate", "targets"])
                .expect("validate targets parses");

        match cli.cmd {
            crate::cli::args::Cmd::Debug {
                action: crate::cli::debug::DebugAction::Validate(args),
            } => {
                assert_eq!(args.target.as_deref(), Some("targets"));
                assert!(args.wants_catalog());
            }
            _ => panic!("expected validate command"),
        }
    }

    #[test]
    fn validate_targets_command_parses_and_renders_catalog() {
        let cli =
            crate::cli::args::Cli::try_parse_from(["tenex-edge", "debug", "validate", "--targets"])
                .expect("validate --targets parses");

        match cli.cmd {
            crate::cli::args::Cmd::Debug {
                action: crate::cli::debug::DebugAction::Validate(args),
            } => {
                assert!(args.targets);
                assert!(args.wants_catalog());
            }
            _ => panic!("expected validate command"),
        }

        let text = targets::render_catalog();
        assert!(text.contains("validate target catalog"));
        assert!(text.contains("status:<session>"));
        assert!(text.contains("readiness:<h>"));
        assert!(text.contains("readiness_attempt:<id>"));
        assert!(text.contains("alias:<harness>:<kind>:<value>"));
        assert!(text.contains("recipient:<event>:<pubkey>[:session]"));
        assert!(text.contains("delivery:<event>:<pubkey>"));
        assert!(text.contains("tenex-edge debug validate commit:<id>"));

        let json = targets::catalog_json();
        assert_eq!(json["verb"], "validate_targets");
        assert!(json["target_forms"].as_array().unwrap().len() > 10);
    }
}
