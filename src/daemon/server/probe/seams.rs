//! `probe seams`: render the code-owned authority-frontier registrations.

use crate::reconcile::frontier;
use serde_json::{json, Value};

pub(super) fn seams_value() -> Value {
    let surfaces = frontier::registrations()
        .iter()
        .map(|r| {
            json!({
                "surface": r.name,
                "mode": r.mode.as_str(),
                "facts": r.facts,
                "trellis_inputs": r.trellis_inputs,
                "host_effects": r.host_effects,
                "bypass_risks": r.bypass_risks,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "verb": "seams",
        "host_seam_coverage_percent": frontier::host_seam_coverage_percent(),
        "uncovered": frontier::uncovered_bypass_risks(),
        "surfaces": surfaces,
    })
}
