//! Agent-facing heads-up for a stalled status/title distiller: streak
//! threshold, throttle window, and success-reset behavior.

use super::*;

/// A persistent status-title generation failure surfaces a throttled,
/// agent-facing heads-up once the failure streak crosses the threshold — but
/// not before.
#[test]
fn distill_failure_streak_triggers_heads_up_once_threshold_hit() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-distill-streak";
    let sid = {
        let s = m.lock().unwrap();
        materialize_channel(&s, ch);
        register(&s, SELF_PK, ch, 100)
    };
    {
        let s = m.lock().unwrap();
        // Below threshold: a couple of failures should not trigger the
        // heads-up yet (absorbs a transient blip).
        s.record_distill_failure(&sid).unwrap();
        s.record_distill_failure(&sid).unwrap();
    }
    let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    let ctx = super::super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        !ctx.contains("status updates aren't working"),
        "heads-up must not fire below the failure-streak threshold; got:\n{ctx}"
    );

    {
        let s = m.lock().unwrap();
        s.record_distill_failure(&sid).unwrap(); // 3rd consecutive failure
    }
    let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    let ctx = super::super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("status updates aren't working"),
        "heads-up must fire once the failure streak crosses the threshold; got:\n{ctx}"
    );
}

/// Once fired, the heads-up is throttled — it does not repeat on the very
/// next turn while the failure streak is ongoing.
#[test]
fn distill_heads_up_does_not_repeat_within_throttle_window() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-distill-throttle";
    let sid = {
        let s = m.lock().unwrap();
        materialize_channel(&s, ch);
        register(&s, SELF_PK, ch, 100)
    };
    {
        let s = m.lock().unwrap();
        for _ in 0..3 {
            s.record_distill_failure(&sid).unwrap();
        }
    }
    let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    let ctx = super::super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("status updates aren't working"),
        "first fire expected; got:\n{ctx}"
    );

    // Another failure lands, but we're still well inside the throttle window.
    {
        let s = m.lock().unwrap();
        s.record_distill_failure(&sid).unwrap();
    }
    let rec2 = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    assert!(
        rec2.distill_notice_at > 0,
        "mark_distill_notice must have stamped notice_at on first fire"
    );
    let ctx2 = super::super::assemble_turn_start_context(&m, &rec2, "", "", 0).unwrap_or_default();
    assert!(
        !ctx2.contains("status updates aren't working"),
        "heads-up must not repeat inside the throttle window; got:\n{ctx2}"
    );
}

/// A successful distill resets the failure streak, so a prior outage doesn't
/// leave a stale streak that fires the heads-up after things recover.
#[test]
fn distill_success_resets_failure_streak() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-distill-recover";
    let sid = {
        let s = m.lock().unwrap();
        materialize_channel(&s, ch);
        register(&s, SELF_PK, ch, 100)
    };
    {
        let s = m.lock().unwrap();
        for _ in 0..3 {
            s.record_distill_failure(&sid).unwrap();
        }
        s.set_session_distill(&sid, "recovered title", "recovered activity", 200)
            .unwrap();
    }
    let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    assert_eq!(rec.distill_fail_streak, 0, "success must reset the streak");
    let ctx = super::super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        !ctx.contains("status updates aren't working"),
        "heads-up must not fire once the streak has been reset by a success; got:\n{ctx}"
    );
}
