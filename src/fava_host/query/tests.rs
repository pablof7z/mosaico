use super::*;
use nostr::key::Keys;

#[tokio::test]
async fn broad_group_activity_query_has_one_fixed_result_ceiling() {
    let host = FavaHost::open(
        &["wss://relay.example.com".into()],
        None,
        None,
        &Keys::generate(),
    )
    .unwrap();
    let query = host
        .fava_query(&SubscriptionQuery::GroupContentsMulti {
            groups: BTreeSet::from(["group".into()]),
            kinds: BTreeSet::from([9]),
        })
        .unwrap();

    assert_eq!(
        query.result_limit().map(std::num::NonZeroUsize::get),
        Some(BROAD_H_RESULT_LIMIT)
    );
}
