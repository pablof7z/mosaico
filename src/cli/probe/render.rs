//! Human renders for the differentiated probe verbs. Each is scrupulously honest
//! about what the daemon proved (frontier §4.6): the oracle render names what is
//! NOT proven and lists the uncovered surfaces; simulate says plainly that
//! nothing was applied; why prints the latest-per-key footer.

use serde_json::Value;
use std::fmt::Write as _;

#[cfg(test)]
mod tests;

fn str_at<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn i64_at(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}
fn strs(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `probe oracle` — per-surface green/red + the honest correctness caveat.
pub(super) fn render_oracle(v: &Value) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "oracle  (live)");
    let empty = Vec::new();
    for s in v
        .get("surfaces")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
    {
        let name = str_at(s, "surface");
        if s.get("live_graph").and_then(Value::as_bool) == Some(false) {
            let _ = writeln!(out, "  {name:<15} —        not a live graph (advisory)");
            continue;
        }
        let status = str_at(s, "status");
        let rev = i64_at(s, "revision");
        let nodes = i64_at(s, "nodes");
        let _ = writeln!(out, "  {name:<15} {status:<8} (rev {rev}, {nodes} nodes)");
        if status == "red" {
            let _ = writeln!(out, "      ↳ {}", str_at(s, "error"));
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "oracle: {} / surface-correctness: {} / host-seam-coverage: {}% / uncovered: {}",
        str_at(v, "oracle"),
        str_at(v, "surface_correctness"),
        i64_at(v, "host_seam_coverage_percent"),
        strs(v, "uncovered").join(", "),
    );
    let _ = writeln!(
        out,
        "surface-correctness detail: oracle checks graph bookkeeping, not host effects"
    );
    let _ = writeln!(out, "covered: {}", strs(v, "covered").join(", "));
    out
}

/// `probe seams` — static authority-frontier registrations plus coverage.
pub(super) fn render_seams(v: &Value) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "seams  (host-seam-coverage: {}%)",
        i64_at(v, "host_seam_coverage_percent")
    );
    let _ = writeln!(out, "{:<16} {:<17} bypass risks", "surface", "mode");
    let empty = Vec::new();
    for row in v
        .get("surfaces")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
    {
        let risks = strs(row, "bypass_risks").join(", ");
        let _ = writeln!(
            out,
            "{:<16} {:<17} {}",
            str_at(row, "surface"),
            str_at(row, "mode"),
            if risks.is_empty() {
                "-"
            } else {
                risks.as_str()
            },
        );
    }
    out
}

/// `probe simulate status/<id>` — the would-be plan; nothing is applied.
pub(super) fn render_simulate(v: &Value) -> String {
    let mut out = String::new();
    let session = str_at(v, "session");
    let _ = writeln!(
        out,
        "simulate status/{session}  ← what committing this fact would do (nothing is applied)\n"
    );
    let fact = v.get("fact").cloned().unwrap_or(Value::Null);
    let _ = writeln!(out, "  fact:     {}", fact_line(&fact));

    let empty = Vec::new();
    let cmds = v
        .get("commands")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    if v.get("would_publish").and_then(Value::as_bool) == Some(true) {
        for c in cmds {
            let _ = writeln!(
                out,
                "  result:   WOULD PUBLISH  kind:{}  ({} {})",
                i64_at(c, "kind"),
                str_at(c, "op"),
                str_at(c, "resource"),
            );
        }
    } else {
        let _ = writeln!(out, "  result:   NO CHANGE (deduped — no publish)");
    }
    let changed = strs(v, "changed");
    if !changed.is_empty() {
        let _ = writeln!(out, "  changed:  {}", changed.join(", "));
    }
    out
}

fn fact_line(fact: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(a) = fact.get("activity").and_then(Value::as_str) {
        parts.push(format!("activity={a:?}"));
    }
    if let Some(t) = fact.get("title").and_then(Value::as_str) {
        parts.push(format!("title={t:?}"));
    }
    format!(
        "{} → {}",
        fact.get("kind").and_then(Value::as_str).unwrap_or("fact"),
        if parts.is_empty() {
            "(no fields)".to_string()
        } else {
            parts.join(", ")
        }
    )
}

/// `probe why <handle>` — live causality + the latest-per-key footer.
pub(super) fn render_why(v: &Value) -> String {
    let mut out = String::new();
    let handle = str_at(v, "handle");
    let _ = writeln!(out, "why {handle}");

    if v.get("found").and_then(Value::as_bool) != Some(true) {
        let note = v
            .get("note")
            .and_then(Value::as_str)
            .unwrap_or("no live audit for this handle");
        let _ = writeln!(out, "  {note}");
        return out;
    }

    let _ = writeln!(out, "  resource:  {}", str_at(v, "resource_key"));
    if str_at(v, "kind") == "subscription" {
        let owners = strs(v, "owners").join(", ");
        let _ = writeln!(
            out,
            "  owners:    {owners}   (refcount {})",
            i64_at(v, "refcount")
        );
    }
    let _ = writeln!(
        out,
        "  last:      {}  ← {}",
        str_at(v, "last_kind"),
        str_at(v, "cause")
    );
    let causes = strs(v, "input_causes");
    if !causes.is_empty() {
        let _ = writeln!(out, "  caused by: {}", causes.join(", "));
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "(latest change per key; for history use probe stats / the commits ledger)"
    );
    out
}

/// `probe state <surface>` — live per-surface values.
pub(super) fn render_state(v: &Value) -> String {
    let mut out = String::new();
    let surface = str_at(v, "surface");
    let _ = writeln!(out, "state {surface}  (live)");
    let empty = Vec::new();
    let rows = v.get("rows").and_then(Value::as_array).unwrap_or(&empty);
    if rows.is_empty() {
        let _ = writeln!(out, "  (none)");
    }
    for r in rows {
        if surface == "status" {
            let _ = writeln!(
                out,
                "  {:<10} {:<6} title={:?}  activity={:?}  channels={:?}",
                str_at(r, "session"),
                if r.get("busy").and_then(Value::as_bool) == Some(true) {
                    "busy"
                } else {
                    "idle"
                },
                str_at(r, "title"),
                str_at(r, "activity"),
                strs(r, "channels"),
            );
        } else {
            let _ = writeln!(
                out,
                "  {:<18} refcount {}   owners: {}",
                str_at(r, "resource_key"),
                i64_at(r, "refcount"),
                strs(r, "owners").join(", "),
            );
        }
    }
    out
}
