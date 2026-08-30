fn should_publish_eye_reaction(hosted: &[String], mentioned_pk: &str) -> bool {
    hosted.contains(&mentioned_pk.to_string())
}

#[test]
fn eye_reaction_fires_for_hosted_agent_pubkey() {
    let hosted = vec!["pk-ord-1".to_string(), "pk-ord-2".to_string()];
    assert!(should_publish_eye_reaction(&hosted, "pk-ord-1"));
}

#[test]
fn eye_reaction_fires_for_identity_derived_pubkey() {
    let hosted = vec!["base-pk".to_string(), "pk-ord-1".to_string()];
    assert!(should_publish_eye_reaction(&hosted, "pk-ord-1"));
}

#[test]
fn eye_reaction_does_not_fire_for_foreign_peer() {
    assert!(!should_publish_eye_reaction(
        &["pk-ord-1".to_string()],
        "foreign-pk"
    ));
}

#[test]
fn eye_reaction_does_not_fire_for_empty_mentioned_pk() {
    assert!(!should_publish_eye_reaction(&["pk-ord-1".to_string()], ""));
}
