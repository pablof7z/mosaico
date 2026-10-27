//! Per-REQ subscription ids: fresh on every replacement, bounded completion.
use super::relay::SubscriptionFrame;
use super::*;

/// Fresh per-REQ ids must not let completion state grow without bound: a
/// late EOSE for an already closed id is pruned at the next close.
#[tokio::test]
async fn closing_a_subscription_prunes_completion_for_ids_no_longer_open() {
    let relay = TestRelay::new(AckMode::Accept, []);
    let runtime = runtime(&relay, Duration::from_millis(100)).await;
    let filter = || vec![Filter::new().kind(Kind::TextNote).limit(8)];
    runtime.replace_demand("content", filter()).await.unwrap();
    runtime
        .wait_for_eose("content", Duration::from_secs(2))
        .await
        .unwrap();
    let completed = |runtime: &NostrRuntime| runtime.completed.lock().unwrap().clone();
    runtime
        .completed
        .lock()
        .unwrap()
        .insert(nostr::prelude::SubscriptionId::new("content:late"));

    runtime.replace_demand("content", filter()).await.unwrap();
    runtime
        .wait_for_eose("content", Duration::from_secs(2))
        .await
        .unwrap();
    let live = runtime.demands.lock().unwrap()["content"].id.clone();
    assert_eq!(completed(&runtime), BTreeSet::from([live]));

    runtime.close_demand("content").await.unwrap();
    assert!(completed(&runtime).is_empty());
}

/// A relay may apply one connection's frames out of order, so a replacement
/// REQ must never reuse a subscription id a CLOSE can still target.
#[tokio::test]
async fn replacing_a_demand_opens_a_fresh_subscription_before_closing_the_old_one() {
    let relay = TestRelay::new(AckMode::Accept, []);
    let runtime = runtime(&relay, Duration::from_millis(100)).await;
    let filter = || vec![Filter::new().kind(Kind::TextNote).limit(8)];
    runtime.replace_demand("content", filter()).await.unwrap();
    runtime.replace_demand("content", filter()).await.unwrap();
    runtime.close_demand("content").await.unwrap();

    let mut frames = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            frames = relay.subscription_frames();
            if frames.len() >= 4 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("relay saw only {frames:?}"));
    let [SubscriptionFrame::Req(first), SubscriptionFrame::Req(second), SubscriptionFrame::Close(closed_first), SubscriptionFrame::Close(closed_second)] =
        frames.as_slice()
    else {
        panic!("unexpected subscription frames: {frames:?}");
    };
    assert_ne!(first, second, "a replacement must use a fresh id");
    assert_eq!(closed_first, first);
    assert_eq!(closed_second, second);
}
