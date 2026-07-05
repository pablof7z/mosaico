use super::*;
use serde_json::json;

#[test]
fn oracle_render_is_honest_about_correctness() {
    let v = json!({
        "verb": "oracle", "ok": true,
        "surfaces": [
            {"surface":"status","live_graph":true,"status":"green","revision":812,"nodes":6},
            {"surface":"subscriptions","live_graph":true,"status":"green","revision":44,"nodes":5},
            {"surface":"hook_context","live_graph":true,"status":"green","revision":2,"nodes":7},
            {"surface":"turn_lifecycle","live_graph":true,"status":"green","revision":3,"nodes":8},
            {"surface":"cursor","live_graph":true,"status":"green","revision":4,"nodes":9},
            {"surface":"outbox","live_graph":true,"status":"green","revision":5,"nodes":10}
        ],
        "surface_correctness_proven": false,
        "surface_correctness": "NOT PROVEN",
        "host_seam_coverage_percent": 85,
        "oracle": "green",
        "covered": ["status","subscriptions","hook_context","turn_lifecycle","cursor","outbox"],
        "uncovered": ["rpc_session_start"]
    });
    let text = render_oracle(&v);
    assert!(text.contains("status          green    (rev 812, 6 nodes)"));
    assert!(text.contains("hook_context    green    (rev 2, 7 nodes)"));
    assert!(text.contains("oracle: green / surface-correctness: NOT PROVEN"));
    assert!(text.contains("host-seam-coverage: 85%"));
    assert!(text.contains("uncovered: rpc_session_start"));
}

#[test]
fn seams_render_lists_modes_and_risks() {
    let v = json!({
        "verb": "seams",
        "host_seam_coverage_percent": 85,
        "surfaces": [
            {"surface":"status","mode":"authoritative","bypass_risks":[]},
            {"surface":"cursor","mode":"authoritative","bypass_risks":[]},
            {"surface":"outbox","mode":"authoritative","bypass_risks":[]}
        ]
    });
    let text = render_seams(&v);
    assert!(text.contains("host-seam-coverage: 85%"));
    assert!(text.contains("status"));
    assert!(text.contains("authoritative"));
    assert!(text.contains("cursor"));
    assert!(text.contains("outbox"));
    assert!(text.contains("authoritative"));
}

#[test]
fn simulate_render_would_publish() {
    let v = json!({
        "verb":"simulate","surface":"status",
        "fact":{"kind":"distill","activity":"reviewing the PR","title":null},
        "would_publish": true, "would_effect": true,
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
        "verb":"simulate","surface":"status",
        "fact":{"kind":"distill","activity":"reading","title":null},
        "would_publish": false, "would_effect": false, "commands": [], "changed": [],
    });
    let text = render_simulate(&v);
    assert!(text.contains("NO CHANGE (deduped)"));
}

#[test]
fn simulate_render_subscription_effect() {
    let v = json!({
        "verb":"simulate","surface":"subscriptions",
        "fact":{"SubscriptionSync":{"snapshot":{"daemon_channels":["room"],"addressed_pubkeys":[],"archived_channels":[],"sessions":{}},"at":1}},
        "would_effect": true,
        "commands":[{"op":"Open","resource":"sub/h/room","effect":true}],
        "changed":["subscriptions/daemon/channels"],
    });
    let text = render_simulate(&v);
    assert!(text.contains("simulate subscriptions"));
    assert!(text.contains("WOULD APPLY    (Open sub/h/room)"));
}

#[test]
fn diff_render_shows_hashes_and_fields() {
    let v = json!({
        "verb":"diff","surface":"status","mode":"live-preview",
        "artifact_changed":true,"before_hash":"sha256:before","after_hash":"sha256:after",
        "field_diff":[{"field":"commands"}]
    });
    let text = render_diff(&v);
    assert!(text.contains("artifact: CHANGED"));
    assert!(text.contains("before:   sha256:before"));
    assert!(text.contains("changed:  commands"));
}

#[test]
fn acid_render_shows_verdicts() {
    let v = json!({
        "verb":"acid","handle":"status:s1","surface":"status",
        "cause":"status/s1/activity","necessary":true,"unrelated_stable":true,"ok":true,
        "original_hash":"sha256:o","removed_hash":"sha256:r","unrelated_hash":"sha256:u"
    });
    let text = render_acid(&v);
    assert!(text.contains("acid status:s1"));
    assert!(text.contains("necessary=true"));
    assert!(text.contains("ok=true"));
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

#[test]
fn state_hook_context_render_lists_graph_details() {
    let v = json!({"verb":"state","surface":"hook_context","rows":[
        {"session":"s1","revision":2,"nodes":7,"render_count":2,
         "input_labels":["hook/s1/cursor"],"why_input_causes":["hook/s1/presence"],
         "text":"Fabric context\nmore"}
    ]});
    let text = render_state(&v);
    assert!(text.contains("state hook_context  (live)"));
    assert!(text.contains("s1                 rev 2  nodes 7  renders 2"));
    assert!(text.contains("caused by: hook/s1/presence"));
    assert!(text.contains("inputs:    hook/s1/cursor"));
    assert!(text.contains("text:      \"Fabric context\""));
}
