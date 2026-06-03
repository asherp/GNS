//! HTTP API: name resolution, profile lookup, and config introspection.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::graph::{
    normalize_label, parse_gns_address, resolve, resolve_address, DemoSource, GraphSource,
    NameResolveOptions, ResolveOptions,
};
use crate::nostr::PublicKey;

#[derive(Clone)]
pub struct AppState {
    pub source: Arc<dyn GraphSource>,
    pub default_max_depth: usize,
    pub max_fanout: usize,
    pub max_name_paths: usize,
    pub relays: Vec<String>,
    pub demo: bool,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/resolve", get(resolve_handler))
        .route("/api/resolve-name", get(resolve_name_handler))
        .route("/api/normalize", get(normalize_handler))
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
struct ResolveNameParams {
    /// Resolving namespace (the caller's pubkey), npub or hex.
    from: String,
    /// GNS address, e.g. `barbara@alex.michael.nostr`.
    name: String,
}

async fn resolve_name_handler(
    State(state): State<AppState>,
    Query(params): Query<ResolveNameParams>,
) -> impl IntoResponse {
    let from = match PublicKey::parse(&params.from) {
        Ok(pk) => pk,
        Err(e) => return bad_request(format!("invalid `from`: {e}")).into_response(),
    };
    let parsed = match parse_gns_address(&params.name) {
        Ok(p) => p,
        Err(e) => return bad_request(format!("invalid `name`: {e}")).into_response(),
    };

    let opts = NameResolveOptions {
        max_fanout: state.max_fanout,
        max_paths: state.max_name_paths,
    };

    match resolve_address(state.source.as_ref(), from, &parsed, opts).await {
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
struct NormalizeParams {
    name: String,
}

async fn normalize_handler(Query(params): Query<NormalizeParams>) -> impl IntoResponse {
    let label = normalize_label(&params.name);
    Json(json!({
        "name": params.name,
        "label": label,
        "valid": !label.is_empty(),
    }))
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
    let demo_name = state.demo.then(|| "barbara@alex.michael.nostr".to_string());
    Json(json!({
        "demo": state.demo,
        "relays": state.relays,
        "default_max_depth": state.default_max_depth,
        "demo_from": demo_from,
        "demo_to": demo_to,
        "demo_name": demo_name,
    }))
}
