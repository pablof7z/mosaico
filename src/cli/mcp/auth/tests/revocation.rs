//! #766, half two: a credential does not outlive the authority it was
//! granted under.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

use super::super::token::ACCESS_TOKEN_TTL_SECS;
use super::*;

#[test]
fn a_token_stops_verifying_once_its_subject_leaves_the_whitelist() {
    let door = Door::new(&["operator"], &[]);
    let token = door.token_for("operator");
    door.auth
        .verify_token(&token)
        .expect("a whitelisted operator's token verifies");

    // The operator is removed from `whitelistedPubkeys`. No restart, no key
    // rotation, no group memberships harmed.
    door.write_trust(&["someone-else"], &[]);

    let error = refusal(
        door.auth.verify_token(&token),
        "a de-whitelisted operator's existing token must stop working",
    );
    assert!(
        format!("{error:#}").contains("whitelist"),
        "the refusal must name the authority that was withdrawn: {error:#}"
    );
}

#[test]
fn a_token_stops_verifying_when_the_config_becomes_unreadable() {
    let door = Door::new(&["operator"], &[]);
    let token = door.token_for("operator");
    std::fs::remove_file(&door.config_path).expect("remove config");
    assert!(
        door.auth.verify_token(&token).is_err(),
        "an authority check that cannot read the whitelist must deny, not admit"
    );
}

#[test]
fn tokens_carry_an_expiry_and_stop_verifying_after_it() {
    let door = Door::new(&["operator"], &[]);
    let token = door.token_for("operator");

    let payload = token.split('.').nth(1).expect("token payload segment");
    let decoded = URL_SAFE_NO_PAD.decode(payload).expect("decode payload");
    let claims: Value = serde_json::from_slice(&decoded).expect("parse claims");
    assert_eq!(
        claims["exp"].as_u64().expect("exp claim") - claims["iat"].as_u64().expect("iat claim"),
        ACCESS_TOKEN_TTL_SECS
    );

    let expired = TokenClaims {
        iss: door.auth.public_url.clone(),
        aud: door.auth.resource_url.clone(),
        sub: "operator".into(),
        scope: default_scope(),
        iat: crate::util::now_secs() - 7200,
        exp: crate::util::now_secs() - 3600,
    };
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&expired).unwrap());
    let sig = sign(&door.auth.token_key, payload.as_bytes());
    let error = refusal(
        door.auth.verify_token(&format!("teo1.{payload}.{sig}")),
        "an expired token must not verify",
    );
    assert!(format!("{error:#}").contains("expired"));
}

#[test]
fn a_forged_signature_is_refused() {
    let door = Door::new(&["operator"], &[]);
    let token = door.token_for("operator");
    let mut parts = token.split('.').collect::<Vec<_>>();
    let signature = parts[2];
    let last = signature.chars().last().expect("signature is not empty");
    let forged = format!(
        "{}{}",
        &signature[..signature.len() - 1],
        if last == 'A' { 'B' } else { 'A' }
    );
    parts[2] = &forged;
    assert!(door.auth.verify_token(&parts.join(".")).is_err());
}
