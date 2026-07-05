use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[test]
fn frontier_modes_match_epic_baseline() {
    let modes = registrations()
        .iter()
        .map(|r| (r.name, r.mode.as_str()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(modes["status"], "authoritative");
    assert_eq!(modes["subscriptions"], "authoritative");
    assert_eq!(modes["hook_context"], "authoritative");
    assert_eq!(modes["turn_lifecycle"], "authoritative");
    assert_eq!(modes["cursor"], "authoritative");
    assert_eq!(modes["outbox"], "authoritative");
    assert_eq!(modes["session_start"], "imperative");
    assert_eq!(host_seam_coverage_percent(), 85);
}

#[test]
fn frontier_is_only_surface_mode_assignment_site() {
    let hits = scan(["SurfaceMode::"]);
    let unexpected = hits
        .into_iter()
        .filter(|hit| !hit.starts_with("src/reconcile/frontier.rs:"))
        .filter(|hit| !hit.starts_with("src/reconcile/frontier/tests.rs:"))
        .collect::<Vec<_>>();
    assert!(
        unexpected.is_empty(),
        "unexpected mode assignments: {unexpected:#?}"
    );
}

#[test]
fn no_direct_status_publish_outside_status_seam() {
    assert_only_allowed(
        scan(["DomainEvent::Status("]),
        [
            "src/daemon/server/demux.rs",
            "src/domain.rs",
            "src/fabric/mod.rs",
            "src/fabric/nip29/wire.rs",
            "src/status_seam.rs",
        ],
    );
}

#[test]
fn no_direct_subscribe_unsubscribe_outside_subscription_executor() {
    assert_only_allowed(
        scan([
            ".subscribe_with_id(",
            ".subscribe_with_id_to(",
            ".unsubscribe(",
        ]),
        ["src/daemon/server/subscriptions.rs", "src/transport.rs"],
    );
}

#[test]
fn authoritative_effect_executors_require_preview_evidence() {
    let status = source("src/status_seam.rs");
    assert!(status.contains("_preview: &TransactionResult<StatusCommand>"));
    assert!(status.contains("preview_matches(preview.as_ref(), &outcome.result)"));

    let subscriptions = source("src/daemon/server/subscriptions.rs");
    assert!(subscriptions.contains("_preview: &trellis_core::TransactionResult"));
    assert!(subscriptions.contains("preview_matches(&preview, &result)"));

    let turn_lifecycle = source("src/daemon/server/turn_lifecycle.rs");
    assert!(turn_lifecycle.contains("preview_turn_started"));
    assert!(turn_lifecycle.contains("command_plans_match"));

    let cursor = source("src/daemon/server/cursor.rs");
    assert!(cursor.contains("preview_request"));
    assert!(cursor.contains("command_plans_match"));

    let outbox = source("src/outbox_seam.rs");
    assert!(outbox.contains("preview_fact"));
    assert!(outbox.contains("command_plans_match"));
}

#[test]
fn no_throwaway_hook_context_reconciler_on_render_path() {
    assert_only_allowed(
        scan(["HookContextReconciler::new()"]),
        [
            "src/reconcile/hook_context/replay.rs",
            "src/turn_context.rs",
        ],
    );
    for path in ["src/turn_context/start.rs", "src/turn_context/check.rs"] {
        assert!(
            !source(path).contains("HookContextReconciler"),
            "{path} must use the daemon-held hook-context graph helper"
        );
    }
}

#[test]
fn no_direct_set_working_outside_declared_lifecycle_seam() {
    assert_only_allowed(scan(["set_working("]), ["src/state/sessions.rs"]);
}

#[test]
fn no_direct_set_session_transcript_outside_declared_lifecycle_seam() {
    assert_only_allowed(scan(["set_session_transcript("]), ["src/state/sessions.rs"]);
}

#[test]
fn no_direct_cursor_mutation_outside_declared_cursor_seam() {
    assert!(scan(["set_seen_cursor("]).is_empty());
    assert!(scan(["try_advance_seen_cursor("]).is_empty());
    assert_only_allowed(
        scan(["apply_cursor_projection("]),
        [
            "src/daemon/server/cursor.rs",
            "src/state/turn_projection.rs",
        ],
    );
}

#[test]
fn no_direct_outbox_result_mutation_outside_declared_outbox_seam() {
    assert!(scan(["mark_published("]).is_empty());
    assert!(scan(["mark_failed("]).is_empty());
    assert_only_allowed(
        scan(["apply_outbox_projection("]),
        ["src/outbox_seam.rs", "src/state/outbox.rs"],
    );
}

fn assert_only_allowed<const N: usize>(hits: BTreeSet<String>, allowed: [&str; N]) {
    let unexpected = hits
        .into_iter()
        .filter(|hit| {
            !allowed
                .iter()
                .any(|path| hit.starts_with(&format!("{path}:")))
        })
        .collect::<Vec<_>>();
    assert!(
        unexpected.is_empty(),
        "unexpected bypass sites: {unexpected:#?}"
    );
}

fn scan<const N: usize>(needles: [&str; N]) -> BTreeSet<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    source_files(&root.join("src"))
        .into_iter()
        .flat_map(|path| hits_in_file(&root, &path, &needles))
        .collect()
}

fn source(path: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path))
        .expect("read source file")
}

fn source_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read source dir") {
        let path = entry.expect("read source entry").path();
        if path.is_dir() {
            out.extend(source_files(&path));
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") && !is_test_path(&path) {
            out.push(path);
        }
    }
    out
}

fn is_test_path(path: &Path) -> bool {
    path.file_name().and_then(|s| s.to_str()) == Some("tests.rs")
        || path.components().any(|c| c.as_os_str() == "tests")
}

fn hits_in_file<const N: usize>(root: &Path, path: &Path, needles: &[&str; N]) -> Vec<String> {
    let rel = path.strip_prefix(root).unwrap().to_string_lossy();
    std::fs::read_to_string(path)
        .expect("read source file")
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//") && needles.iter().any(|needle| line.contains(*needle))
        })
        .map(|(idx, _)| format!("{rel}:{}", idx + 1))
        .collect()
}
