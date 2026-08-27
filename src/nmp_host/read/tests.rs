use super::filter;

/// Regression: building a filter with multiple indexed tag constraints must
/// retain every tag entry, not silently drop later ones.
#[test]
fn filter_preserves_multiple_indexed_constraints() {
    let filter = filter(
        &[1],
        &["ab".repeat(32)],
        &[('h', "group".into()), ('t', "marker".into())],
    )
    .unwrap();
    assert_eq!(filter.kinds, vec![1u16]);
    assert!(!filter.authors.is_empty());
    assert_eq!(filter.tags.len(), 2);
}
