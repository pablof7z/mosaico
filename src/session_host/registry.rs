use crate::session::Harness;

/// How a harness command is transformed into a one-shot, run-to-exit launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HeadlessShape {
    ClaudePrint,
    CodexExec,
    OpencodeRun,
}

pub(super) fn headless_shape_for_harness(harness: Harness) -> Option<HeadlessShape> {
    match harness {
        Harness::ClaudeCode => Some(HeadlessShape::ClaudePrint),
        Harness::Codex => Some(HeadlessShape::CodexExec),
        Harness::Opencode => Some(HeadlessShape::OpencodeRun),
        Harness::Grok | Harness::Unknown => None,
    }
}

pub(super) fn build_headless_command(
    base: &[String],
    shape: HeadlessShape,
    resume_id: Option<&str>,
    fresh_session_id: Option<&str>,
    prompt: &str,
) -> Vec<String> {
    match shape {
        HeadlessShape::ClaudePrint => {
            let mut out = base.to_vec();
            if let Some(id) = fresh_session_id {
                out.extend(["--session-id".to_string(), id.to_string()]);
            }
            out.push("-p".to_string());
            if let Some(id) = resume_id {
                out.extend(["--resume".to_string(), id.to_string()]);
            }
            out.push(prompt.to_string());
            out
        }
        HeadlessShape::CodexExec => {
            let mut out = Vec::with_capacity(base.len() + 4);
            let mut it = base.iter();
            if let Some(bin) = it.next() {
                out.push(bin.clone());
            }
            out.extend(["exec".to_string(), "--json".to_string()]);
            out.extend(it.cloned());
            if let Some(id) = resume_id {
                out.extend(["resume".to_string(), id.to_string()]);
            }
            out.push(prompt.to_string());
            out
        }
        HeadlessShape::OpencodeRun => {
            let mut out = Vec::with_capacity(base.len() + 5);
            let mut it = base.iter();
            if let Some(bin) = it.next() {
                out.push(bin.clone());
            }
            out.extend([
                "run".to_string(),
                "--format".to_string(),
                "json".to_string(),
            ]);
            if let Some(id) = resume_id {
                out.extend(["--session".to_string(), id.to_string()]);
            }
            out.extend(it.cloned());
            out.push(prompt.to_string());
            out
        }
    }
}

/// Returns `(slug, bundle, byline)` for configured local agents.
pub fn spawnable_agents() -> Vec<(String, String, Option<String>)> {
    crate::identity::list_local_agents(&crate::config::mosaico_home())
        .into_iter()
        .map(|(slug, harness, _profile, byline)| (slug, harness, byline))
        .collect()
}
