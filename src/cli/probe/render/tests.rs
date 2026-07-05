use super::*;
use serde_json::json;

#[test]
fn oracle_render_is_honest_about_correctness() {
    let v = json!({
        "verb": "oracle", "ok": true,
        "surfaces": [
            {"surface":"status","live_graph":true,"status":"green","revision":812,"nodes":6},
            {"surface":"subscriptions","live_graph":true,"status":"green","revision":44,"nodes":5},
            {"surface":"hook_context","live_graph":false,"note":"advisory"}
        ],
        "surface_correctness_proven": false,
        "surface_correctness": "NOT PROVEN",
        "host_seam_coverage_percent": 28,
        "oracle": "green",
        "covered": ["status","subscriptions"],
        "uncovered": ["rpc_turn_start","cursor CAS","rpc_session_start","outbox publish"]
    });
    let text = render_oracle(&v);
    assert!(text.contains("status          green    (rev 812, 6 nodes)"));
    assert!(text.contains("hook_context    —        not a live graph (advisory)"));
    assert!(text.contains("oracle: green / surface-correctness: NOT PROVEN"));
    assert!(text.contains("host-seam-coverage: 28%"));
    assert!(text.contains("uncovered: rpc_turn_start, cursor CAS"));
}

#[test]
fn seams_render_lists_modes_and_risks() {
    let v = json!({
        "verb": "seams",
        "host_seam_coverage_percent": 28,
        "surfaces": [
            {"surface":"status","mode":"authoritative","bypass_risks":[]},
            {"surface":"cursor","mode":"imperative","bypass_risks":["cursor CAS"]}
        ]
    });
    let text = render_seams(&v);
    assert!(text.contains("host-seam-coverage: 28%"));
    assert!(text.contains("status"));
    assert!(text.contains("authoritative"));
    assert!(text.contains("cursor"));
    assert!(text.contains("imperative"));
    assert!(text.contains("cursor CAS"));
}

#[test]
fn simulate_render_would_publish() {
    let v = json!({
        "verb":"simulate","session":"s1",
        "fact":{"kind":"distill","activity":"reviewing the PR","title":null},
        "would_publish": true,
        "commands":[{"op":"Replace","resource":"status/s1","kind":30315,"publish":true}],
        "changed":["status/s1/activity"],
    });
    let text = render_simulate(&v);
    assert!(text.contains("nothing is applied"));
    assert!(text.contains("activity=\"reviewing the PR\""));
    assert!(text.contains("WOULD PUBLISH  kind:30315  (Replace status/s1)"));
    assert!(text.contains("changed:  status/s1/activity"));
}

#[test]
fn simulate_render_no_change() {
    let v = json!({
        "verb":"simulate","session":"s1",
        "fact":{"kind":"distill","activity":"reading","title":null},
        "would_publish": false, "commands": [], "changed": [],
    });
    let text = render_simulate(&v);
    assert!(text.contains("NO CHANGE (deduped — no publish)"));
}

#[test]
fn why_sub_render_shows_owners_and_footer() {
    let v = json!({
        "verb":"why","handle":"sub:general","kind":"subscription","found":true,
        "resource_key":"sub/h/general","refcount":2,
        "owners":["daemon-subs","session-s1"],
        "last_kind":"Open","cause":"planner: subscriptions/daemon/subs",
    });
    let text = render_why(&v);
    assert!(text.contains("resource:  sub/h/general"));
    assert!(text.contains("owners:    daemon-subs, session-s1   (refcount 2)"));
    assert!(text.contains("last:      Open  ← planner: subscriptions/daemon/subs"));
    assert!(text.contains("(latest change per key;"));
}

#[test]
fn why_status_render_shows_cause() {
    let v = json!({
        "verb":"why","handle":"status:s1","kind":"status","found":true,
        "resource_key":"status/s1","last_kind":"Replace",
        "cause":"planner: status/s1/coll","input_causes":["status/s1/activity"],
    });
    let text = render_why(&v);
    assert!(text.contains("caused by: status/s1/activity"));
}

#[test]
fn why_not_found_is_clean() {
    let v = json!({"verb":"why","handle":"status:ghost","found":false,
                   "note":"no command emitted yet on this daemon graph"});
    let text = render_why(&v);
    assert!(text.contains("no command emitted yet"));
}

#[test]
fn state_status_render_lists_sessions() {
    let v = json!({"verb":"state","surface":"status","rows":[
        {"session":"s1","title":"T","activity":"reading","busy":true,"channels":["room"]}
    ]});
    let text = render_state(&v);
    assert!(text.contains("state status  (live)"));
    assert!(text.contains("s1"));
    assert!(text.contains("busy"));
    assert!(text.contains("activity=\"reading\""));
}
