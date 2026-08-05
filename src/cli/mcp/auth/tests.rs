use super::token::TokenClaims;
use super::*;

/// A door standing on its own temp home: its own config document (the live
/// trust input) and its own client registry. Nothing here reads the developer's
/// real `~/.mosaico`, and no test mutates process-wide environment.
struct Door {
    auth: AuthState,
    config_path: PathBuf,
    _home: tempfile::TempDir,
}

impl Door {
    fn new(operators: &[&str], redirect_origins: &[&str]) -> Self {
        let home = tempfile::tempdir().expect("temp home");
        let config_path = home.path().join("config.json");
        let door = Self {
            auth: AuthState {
                public_url: "https://mosaico.example".into(),
                resource_url: "https://mosaico.example/mcp".into(),
                actor_secret: b"test-secret".to_vec(),
                token_key: derive_key(b"test-secret", TOKEN_KEY_LABEL),
                config_path: config_path.clone(),
                clients: ClientRegistry::open(home.path().join(CLIENT_REGISTRY_FILE)),
                codes: Arc::new(Mutex::new(HashMap::new())),
                challenges: Arc::new(Mutex::new(HashMap::new())),
            },
            config_path,
            _home: home,
        };
        door.write_trust(operators, redirect_origins);
        door
    }

    fn write_trust(&self, operators: &[&str], redirect_origins: &[&str]) {
        std::fs::write(
            &self.config_path,
            json!({
                "whitelistedPubkeys": operators,
                "mcpRedirectOrigins": redirect_origins,
            })
            .to_string(),
        )
        .expect("write config");
    }

    fn register(&self, redirect_uri: &str) -> String {
        let origins = self.auth.trust().expect("trust").redirect_origins;
        let response = self
            .auth
            .clients
            .register(&json!({ "redirect_uris": [redirect_uri] }), &origins)
            .expect("registration accepted");
        response["client_id"]
            .as_str()
            .expect("client_id")
            .to_string()
    }

    fn token_for(&self, pubkey: &str) -> String {
        self.auth
            .issue_token(&self.code_for(pubkey))
            .expect("issue token")
    }

    fn code_for(&self, pubkey: &str) -> AuthCode {
        AuthCode {
            client_id: "client".into(),
            redirect_uri: "https://client.example/callback".into(),
            code_challenge: "challenge".into(),
            resource: self.auth.resource_url.clone(),
            scope: default_scope(),
            pubkey: pubkey.into(),
            expires_at: crate::util::now_secs() + 60,
        }
    }
}

fn auth() -> AuthState {
    Door::new(&[], &[]).auth
}

/// `TokenClaims` deliberately has no `Debug`, so `expect_err` is unavailable.
fn refusal(result: Result<TokenClaims>, expectation: &str) -> anyhow::Error {
    match result {
        Ok(_) => panic!("{expectation}"),
        Err(error) => error,
    }
}

fn authorize_params(client_id: &str, redirect_uri: &str) -> AuthorizeParams {
    serde_json::from_value(json!({
        "response_type": "code",
        "client_id": client_id,
        "redirect_uri": redirect_uri,
        "code_challenge": "requester-picked-challenge",
        "code_challenge_method": "S256",
        "resource": "https://mosaico.example/mcp",
        "scope": "mosaico:read",
    }))
    .expect("authorize params")
}

#[test]
fn actor_correlation_is_keyed_stable_and_contains_no_raw_identifiers() {
    let auth = auth();
    let first = auth.redact_actor_key(&["openai-v1", "subject", "conversation-one"]);
    let repeat = auth.redact_actor_key(&["openai-v1", "subject", "conversation-one"]);
    let second = auth.redact_actor_key(&["openai-v1", "subject", "conversation-two"]);
    assert_eq!(first, repeat);
    assert_ne!(first, second);
    assert!(!first.contains("subject"));
    assert!(!first.contains("conversation"));
}

#[test]
fn the_token_key_is_not_the_actor_correlation_key() {
    let auth = auth();
    assert_ne!(
        auth.token_key, auth.actor_secret,
        "one secret must not both sign tokens and pseudonymise actor keys"
    );
}

#[test]
fn metadata_separates_authorization_server_from_mcp_resource() {
    let auth = auth();
    assert_eq!(
        auth.protected_resource(),
        json!({
            "resource": "https://mosaico.example/mcp",
            "authorization_servers": ["https://mosaico.example"],
            "scopes_supported": SCOPES,
            "resource_documentation": "https://github.com/pablof7z/mosaico",
        })
    );
    assert_eq!(
        auth.authorization_server()["issuer"],
        "https://mosaico.example"
    );
}

#[test]
fn metadata_advertises_registration_as_the_only_way_to_become_a_client() {
    let auth = auth();
    let metadata = auth.authorization_server();
    assert_eq!(
        metadata["registration_endpoint"],
        "https://mosaico.example/oauth/register"
    );
    assert!(
        metadata
            .get("client_id_metadata_document_supported")
            .is_none(),
        "mosaico never fetches a client_id metadata document; advertising it invites \
         unregistered client_ids"
    );
}

#[test]
fn tokens_are_audience_bound_to_mcp_resource() {
    let door = Door::new(&["operator"], &[]);
    let code = door.code_for("operator");
    let token = door.auth.issue_token(&code).expect("issue token");
    let claims = door.auth.verify_token(&token).expect("verify token");
    assert_eq!(claims.iss, "https://mosaico.example");
    assert_eq!(claims.aud, "https://mosaico.example/mcp");

    let origin_token = door
        .auth
        .issue_token(&AuthCode {
            resource: door.auth.public_url.clone(),
            ..code
        })
        .expect("issue origin-audience token");
    assert!(door.auth.verify_token(&origin_token).is_err());
}

#[path = "tests/authorize.rs"]
mod authorize;
#[path = "tests/revocation.rs"]
mod revocation;
