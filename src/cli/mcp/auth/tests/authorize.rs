//! #766, half one: a code cannot be obtained for a redirect target that
//! no registration ever recorded.

use super::*;

#[tokio::test]
async fn authorize_refuses_a_redirect_uri_that_was_never_registered() {
    let door = Door::new(&["operator"], &["https://client.example"]);
    let client_id = door.register("https://client.example/callback");

    let response = door
        .auth
        .authorize_page(authorize_params(&client_id, "https://evil.example/steal"))
        .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "an unregistered redirect target must never reach the operator login page"
    );
}

#[tokio::test]
async fn authorize_refuses_a_client_id_that_never_registered() {
    let door = Door::new(&["operator"], &["https://client.example"]);
    let response = door
        .auth
        .authorize_page(authorize_params(
            "mcpc_forged",
            "https://client.example/callback",
        ))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn authorize_serves_the_login_page_for_a_registered_pair() {
    let door = Door::new(&["operator"], &["https://client.example"]);
    let client_id = door.register("https://client.example/callback");
    let response = door
        .auth
        .authorize_page(authorize_params(
            &client_id,
            "https://client.example/callback",
        ))
        .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the registered pair is exactly what must still work"
    );
}

#[tokio::test]
async fn a_posted_authorization_cannot_smuggle_an_unregistered_redirect() {
    let door = Door::new(&["operator"], &["https://client.example"]);
    let client_id = door.register("https://client.example/callback");
    let form: AuthorizeForm = serde_json::from_value(json!({
        "login_challenge": "whatever",
        "response_type": "code",
        "client_id": client_id,
        "redirect_uri": "https://evil.example/steal",
        "code_challenge": "requester-picked-challenge",
        "code_challenge_method": "S256",
        "resource": "https://mosaico.example/mcp",
        "nsec": "",
    }))
    .expect("authorize form");
    let response = door.auth.authorize_submit(form).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        door.auth.codes.lock().await.is_empty(),
        "no code may be minted for an unregistered redirect target"
    );
}

#[test]
fn registration_refuses_an_origin_the_operator_never_approved() {
    let door = Door::new(&["operator"], &["https://client.example"]);
    let response = door
        .auth
        .register(json!({ "redirect_uris": ["https://evil.example/steal"] }));
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
