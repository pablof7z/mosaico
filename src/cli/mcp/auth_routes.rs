use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use serde_json::Value;

use super::http::HttpState;

pub(super) fn routes() -> Router<HttpState> {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth_protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_authorization_server),
        )
        .route(
            "/.well-known/openid-configuration",
            get(oauth_authorization_server),
        )
        .route(
            "/oauth/authorize",
            get(oauth_authorize).post(oauth_authorize_submit),
        )
        .route("/oauth/token", post(oauth_token))
        .route("/oauth/register", post(oauth_register))
}

async fn oauth_protected_resource(State(state): State<HttpState>) -> Response {
    match state.auth {
        Some(auth) => Json(auth.protected_resource()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn oauth_authorization_server(State(state): State<HttpState>) -> Response {
    match state.auth {
        Some(auth) => Json(auth.authorization_server()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn oauth_authorize(
    State(state): State<HttpState>,
    Query(params): Query<super::auth::AuthorizeParams>,
) -> Response {
    match state.auth {
        Some(auth) => auth.authorize_page(params).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn oauth_authorize_submit(
    State(state): State<HttpState>,
    Form(form): Form<super::auth::AuthorizeForm>,
) -> Response {
    match state.auth {
        Some(auth) => auth.authorize_submit(form).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn oauth_token(
    State(state): State<HttpState>,
    Form(form): Form<super::auth::TokenForm>,
) -> Response {
    match state.auth {
        Some(auth) => auth.token(form).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn oauth_register(State(state): State<HttpState>, Json(body): Json<Value>) -> Response {
    match state.auth {
        Some(auth) => auth.register(body),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
