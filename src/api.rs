//! HTTP API: name resolution, profile lookup, and config introspection.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::graph::{resolve, DemoSource, GraphSource, ResolveOptions};
use crate::nostr::PublicKey;

#[derive(Clone)]
pub struct AppState {
    pub source: Arc<dyn GraphSource>,
    pub default_max_depth: usize,
    pub relays: Vec<String>,
    pub demo: bool,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/resolve", get(resolve_handler))
        .route("/api/profile", get(profile_handler))
        .route("/api/config", get(config_handler))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct ResolveParams {
    from: String,
    to: String,
    max_depth: Option<usize>,
}

#[derive(Serialize)]
struct ApiError {
    error: String,
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError { error: msg.into() }),
    )
}

async fn resolve_handler(
    State(state): State<AppState>,
    Query(params): Query<ResolveParams>,
) -> impl IntoResponse {
    let from = match PublicKey::parse(&params.from) {
        Ok(pk) => pk,
        Err(e) => return bad_request(format!("invalid `from`: {e}")).into_response(),
    };
    let to = match PublicKey::parse(&params.to) {
        Ok(pk) => pk,
        Err(e) => return bad_request(format!("invalid `to`: {e}")).into_response(),
    };

    let opts = ResolveOptions {
        max_depth: params
            .max_depth
            .unwrap_or(state.default_max_depth)
            .clamp(1, 12),
        ..ResolveOptions::default()
    };

    match resolve(state.source.as_ref(), from, to, opts).await {
        Ok(res) => Json(res).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct ProfileParams {
    pubkey: String,
}

async fn profile_handler(
    State(state): State<AppState>,
    Query(params): Query<ProfileParams>,
) -> impl IntoResponse {
    let pk = match PublicKey::parse(&params.pubkey) {
        Ok(pk) => pk,
        Err(e) => return bad_request(format!("invalid `pubkey`: {e}")).into_response(),
    };
    match state.source.profile(&pk.to_hex()).await {
        Ok(profile) => Json(json!({
            "npub": pk.to_npub(),
            "pubkey": pk.to_hex(),
            "profile": profile,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn config_handler(State(state): State<AppState>) -> impl IntoResponse {
    let (demo_from, demo_to) = if state.demo {
        let to_npub = |hex: String| PublicKey::from_hex(&hex).map(|p| p.to_npub()).ok();
        (to_npub(DemoSource::you()), to_npub(DemoSource::barbara()))
    } else {
        (None, None)
    };
    Json(json!({
        "demo": state.demo,
        "relays": state.relays,
        "default_max_depth": state.default_max_depth,
        "demo_from": demo_from,
        "demo_to": demo_to,
    }))
}
