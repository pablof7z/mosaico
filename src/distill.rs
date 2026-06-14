//! Activity distillation (M1 §8).
//!
//! The agent's recent **conversation transcript** is distilled into a one-line
//! intent that becomes its `Activity` note and live `Status`. This is **LLM-only**
//! (like `pc`) — intent isn't recoverable from tool calls by rule. There is no
//! heuristic fallback: if no model is configured (or the call fails), nothing is
//! published. The engine decides *when* to distill (30s into a turn, then
//! periodically) by watching turn state; this module only answers *what* the
//! agent is doing given the transcript.
//!
//! Ordering: `$TENEX_EDGE_DISTILL_CMD` (explicit external-command override) →
//! the `edge-distillation` role in `~/.tenex/llms.json` called natively via
//! `rig` (openrouter/ollama) → `None`.

use anyhow::Result;
use std::io::Write;
use std::process::{Command, Stdio};

/// Pipes a context string to an external command's stdin; its stdout (first
/// non-empty line) is the distilled intent. This is the LLM seam — point
/// `$TENEX_EDGE_DISTILL_CMD` at any model CLI.
pub struct CommandDistiller {
    pub command: String,
}

impl CommandDistiller {
    /// Build from `$TENEX_EDGE_DISTILL_CMD`, if set.
    pub fn from_env() -> Option<Self> {
        std::env::var("TENEX_EDGE_DISTILL_CMD")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|command| Self { command })
    }

    /// Resolve the active distiller command: `$TENEX_EDGE_DISTILL_CMD` if set.
    /// This is an **explicit override** — the default distiller is the native rig
    /// path via the `edge-distillation` role (see [`distill_activity`]).
    pub fn resolve() -> Option<Self> {
        Self::from_env()
    }

    /// Summarize an arbitrary context string (e.g. a transcript snippet) into a
    /// one-line intent. Returns None on failure/empty.
    pub fn summarize(&self, context: &str) -> Option<String> {
        match self.run(context) {
            Ok(line) if !line.trim().is_empty() => Some(line),
            _ => None,
        }
    }

    /// Run the command and return its FULL stdout (all lines), so callers that
    /// need a multi-line response (e.g. the combined TITLE/NOW distiller) can
    /// parse it themselves. Returns None on spawn/exec failure.
    pub fn summarize_full(&self, context: &str) -> Option<String> {
        match self.run_all(context) {
            Ok(out) if !out.trim().is_empty() => Some(out),
            _ => None,
        }
    }

    fn run(&self, input: &str) -> Result<String> {
        Ok(self
            .run_all(input)?
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string())
    }

    fn run_all(&self, input: &str) -> Result<String> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input.as_bytes())?;
        }
        let out = child.wait_with_output()?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

/// System prompt for the combined session distiller. ONE model call yields BOTH
/// a stable session TITLE (what the session is about) and a live NOW line (what
/// the agent is doing this moment) — see [`distill_session`]. Giving the model an
/// explicit NOW slot is what keeps step-level mechanics OUT of the title: it has
/// somewhere else to put them. The title is nudged-to-keep (only changes when the
/// objective substantively changes); NOW is regenerated every turn.
const SESSION_SYSTEM_PROMPT: &str = "You maintain two labels for a coding session. Output EXACTLY two lines, nothing else:\n\nTITLE: the session's overall objective — what the agent was asked to accomplish, NOT the step it happens to be doing right now. A stable noun phrase or imperative, at most 8 words, no trailing punctuation. Prefer the user's stated request. It must stay valid for the WHOLE session; if it would go stale in a few messages it is too specific — zoom out to the goal. You may be given the CURRENT title; if it still fits, repeat it verbatim. Only change it when the objective itself has substantively changed.\n\nNOW: what the agent is doing at this moment — the current step or mechanics. At most 8 words, present tense, no trailing punctuation. This is expected to change every turn.\n\nExample:\nTITLE: Fix GitHub issue 1\nNOW: reading the issue tracker";

/// One distilled (title, activity) pair from a single model call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionLabels {
    /// Stable session title — what the session is about.
    pub title: String,
    /// Live "what it's doing now" line; empty when the model omits it.
    pub activity: String,
}

/// Trim a distilled label: strip whitespace and trailing sentence punctuation,
/// cap at 80 chars. Returns None when nothing meaningful remains.
fn clean_label(s: &str) -> Option<String> {
    let s = s.trim().trim_end_matches(['.', ' ', '\t']).trim();
    if s.is_empty() {
        return None;
    }
    let s: String = s.chars().take(80).collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Parse the two-line `TITLE:`/`NOW:` distiller output. Tolerant of label
/// casing, surrounding whitespace, and a model that emits only one of the lines.
/// Accepts `ACTIVITY`/`DOING` as synonyms for `NOW`. A bare single line with no
/// recognized label is treated as the title (degrade gracefully).
fn parse_labels(out: &str) -> (Option<String>, Option<String>) {
    let mut title = None;
    let mut activity = None;
    let mut unlabeled = None;
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line.split_once(':') {
            Some((label, value)) => match label.trim().to_ascii_uppercase().as_str() {
                "TITLE" => title = title.or_else(|| clean_label(value)),
                "NOW" | "ACTIVITY" | "DOING" => activity = activity.or_else(|| clean_label(value)),
                _ => unlabeled = unlabeled.or_else(|| clean_label(line)),
            },
            None => unlabeled = unlabeled.or_else(|| clean_label(line)),
        }
    }
    (title.or(unlabeled), activity)
}

/// Complete the combined session prompt natively via rig (openrouter/ollama),
/// returning the raw multi-line text. `None` on any error so the caller falls back.
async fn complete_via_rig(
    resolved: &crate::llmconfig::ResolvedModel,
    context: &str,
) -> Option<String> {
    use rig::client::CompletionClient;
    use rig::completion::Prompt;

    let text: String = match resolved.provider.as_str() {
        "openrouter" => {
            let client = rig::providers::openrouter::Client::new(&resolved.api_key).ok()?;
            let agent = client
                .agent(&resolved.model)
                .preamble(SESSION_SYSTEM_PROMPT)
                .temperature(0.2)
                .max_tokens(96)
                .build();
            agent.prompt(context).await.ok()?
        }
        "ollama" => {
            use rig::providers::ollama::OllamaApiKey;
            // Ollama needs no auth by default; supply the empty key marker so the
            // builder's api-key type-state is satisfied (mirrors rig's own
            // `from_env`), then point it at the base URL from providers.json.
            let mut builder =
                rig::providers::ollama::Client::builder().api_key(OllamaApiKey::default());
            if !resolved.base_url.is_empty() {
                builder = builder.base_url(&resolved.base_url);
            }
            let client = builder.build().ok()?;
            let agent = client
                .agent(&resolved.model)
                .preamble(SESSION_SYSTEM_PROMPT)
                .temperature(0.2)
                .max_tokens(96)
                .build();
            agent.prompt(context).await.ok()?
        }
        _ => return None,
    };
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Assemble a [`SessionLabels`] from parsed lines, applying nudge-to-keep: a
/// missing/empty title falls back to `current_title`. Returns None only when
/// there is no title at all (no parse, no current) — so the caller can try the
/// next provider.
fn assemble(
    parsed: (Option<String>, Option<String>),
    current_title: Option<&str>,
) -> Option<SessionLabels> {
    let (title, activity) = parsed;
    let title = title.or_else(|| {
        current_title
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    })?;
    Some(SessionLabels {
        title,
        activity: activity.unwrap_or_default(),
    })
}

/// Distill BOTH the persistent session title and the live activity line from the
/// recent transcript in a **single** model call (see [`SESSION_SYSTEM_PROMPT`]).
/// The `current_title` is fed back so the model keeps a still-accurate title
/// (nudge-to-keep). Ordering:
///   (a) `$TENEX_EDGE_DISTILL_CMD` set → external command (explicit override);
///   (b) else the `edge-distillation` role resolves → native rig (openrouter/ollama);
///   (c) else **nudge-to-keep**: retain the current title with an empty activity.
pub async fn distill_session(
    transcript: &str,
    current_title: Option<&str>,
) -> Option<SessionLabels> {
    let ctx = transcript.trim();
    if ctx.is_empty() {
        // Nothing new to read: keep the title, no live activity.
        return current_title
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| SessionLabels {
                title: t.to_string(),
                activity: String::new(),
            });
    }
    // Give the model the current title as context so it can choose to keep it.
    let input = match current_title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => format!("CURRENT TITLE: {t}\n\nTRANSCRIPT:\n{ctx}"),
        None => ctx.to_string(),
    };
    // (a) explicit external-command override. The command sees SESSION_SYSTEM_PROMPT
    // semantics by convention (it is the LLM seam); we parse its two-line output.
    if let Some(cmd) = CommandDistiller::resolve() {
        if let Some(out) = cmd.summarize_full(&input) {
            if let Some(labels) = assemble(parse_labels(&out), current_title) {
                return Some(labels);
            }
        }
    }
    // (b) native rig via the edge-distillation role, with the combined preamble.
    if let Some(resolved) = crate::llmconfig::resolve_role("edge-distillation") {
        if let Some(out) = complete_via_rig(&resolved, &input).await {
            if let Some(labels) = assemble(parse_labels(&out), current_title) {
                return Some(labels);
            }
        }
    }
    // (c) no model / empty output → keep the existing title (nudge-to-keep).
    current_title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| SessionLabels {
            title: t.to_string(),
            activity: String::new(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_distiller_uses_stdout_first_line() {
        let d = CommandDistiller {
            command: "cat >/dev/null; printf 'Fixing the auth bug\\nignored second line'".into(),
        };
        assert_eq!(
            d.summarize("User: fix the auth bug").unwrap(),
            "Fixing the auth bug"
        );
    }

    #[test]
    fn command_distiller_none_on_failure() {
        let d = CommandDistiller {
            command: "exit 1".into(),
        };
        assert!(d.summarize("anything").is_none());
    }

    #[test]
    fn command_distiller_none_on_empty_output() {
        let d = CommandDistiller {
            command: "cat >/dev/null; true".into(),
        };
        assert!(d.summarize("anything").is_none());
    }

    #[test]
    fn parse_labels_reads_both_lines() {
        let (title, activity) =
            parse_labels("TITLE: Fix GitHub issue 1\nNOW: reading the issue tracker");
        assert_eq!(title.as_deref(), Some("Fix GitHub issue 1"));
        assert_eq!(activity.as_deref(), Some("reading the issue tracker"));
    }

    #[test]
    fn parse_labels_is_case_and_synonym_tolerant() {
        let (title, activity) = parse_labels("title:  Refactor parser  \nActivity: writing tests.");
        assert_eq!(title.as_deref(), Some("Refactor parser"));
        // Trailing punctuation is stripped.
        assert_eq!(activity.as_deref(), Some("writing tests"));
    }

    #[test]
    fn parse_labels_bare_line_is_title() {
        let (title, activity) = parse_labels("Fixing the auth bug");
        assert_eq!(title.as_deref(), Some("Fixing the auth bug"));
        assert_eq!(activity, None);
    }

    /// Drive `distill_session` through the external-command seam. Both scenarios
    /// live in ONE test: `TENEX_EDGE_DISTILL_CMD` is process-global, so parallel
    /// env-mutating tests would race.
    #[tokio::test]
    async fn distill_session_via_command() {
        // (1) A distiller emitting both lines populates title and activity.
        std::env::set_var(
            "TENEX_EDGE_DISTILL_CMD",
            "cat >/dev/null; printf 'TITLE: Fix GitHub issue 1\\nNOW: reading the issue tracker\\n'",
        );
        let got = distill_session("user: fix github issue 1", None)
            .await
            .unwrap();
        assert_eq!(got.title, "Fix GitHub issue 1");
        assert_eq!(got.activity, "reading the issue tracker");

        // (2) Echoing only the prior title back keeps it (nudge-to-keep), no NOW.
        std::env::set_var(
            "TENEX_EDGE_DISTILL_CMD",
            "sed -n 's/^CURRENT TITLE: /TITLE: /p' | head -n1",
        );
        let got = distill_session(
            "TRANSCRIPT:\nuser: keep going",
            Some("refactoring the auth flow"),
        )
        .await
        .unwrap();
        std::env::remove_var("TENEX_EDGE_DISTILL_CMD");
        assert_eq!(got.title, "refactoring the auth flow");
        assert_eq!(got.activity, "");
    }

    /// Empty transcript returns the current title (no activity) rather than re-distilling.
    #[tokio::test]
    async fn distill_session_empty_transcript_returns_current() {
        let got = distill_session("   ", Some("writing the parser"))
            .await
            .unwrap();
        assert_eq!(got.title, "writing the parser");
        assert_eq!(got.activity, "");
    }
}
