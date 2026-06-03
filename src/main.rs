//! GNS — Graph Name System.
//!
//! A lightweight server that resolves names through the Nostr social graph:
//! given two pubkeys it returns the shortest follow-chain between them, with
//! the follow event ids and the relays that served them.

mod api;
mod config;
mod graph;
mod nostr;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::api::AppState;
use crate::config::Config;
use crate::graph::{CachedSource, DemoSource, GraphSource, RelaySource};

#[derive(Parser, Debug)]
#[command(name = "gns", version, about = "Graph Name System resolver")]
struct Cli {
    /// Path to the TOML config file.
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    /// Run against a built-in demo graph instead of live relays (no network).
    #[arg(long)]
    demo: bool,

    /// Override the bind address (e.g. 0.0.0.0:8080).
    #[arg(long)]
    bind: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,gns=debug".into()),
        )
        .init();

    let cli = Cli::parse();

    let mut cfg = if cli.config.exists() {
        Config::load(&cli.config)?
    } else {
        info!(path = %cli.config.display(), "config not found, using defaults");
        Config::default()
    };
    if let Some(bind) = cli.bind {
        cfg.bind = bind;
    }

    let source: Arc<dyn GraphSource> = if cli.demo {
        info!("running in DEMO mode with a built-in fixture graph");
        Arc::new(DemoSource::new())
    } else {
        let relay_src = RelaySource::new(
            cfg.relays.clone(),
            cfg.relay_timeout(),
            cfg.verify_signatures,
        );
        Arc::new(CachedSource::new(
            Arc::new(relay_src),
            cfg.cache_ttl(),
            cfg.cache_capacity,
        ))
    };

    let state = AppState {
        source,
        default_max_depth: cfg.max_depth,
        relays: if cli.demo {
            vec!["demo://built-in".to_string()]
        } else {
            cfg.relays.clone()
        },
        demo: cli.demo,
    };

    let app = api::router(state)
        .fallback_service(ServeDir::new(&cfg.static_dir))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr = cfg.bind_addr()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "GNS resolver listening — open http://{addr}/ for the dashboard");
    axum::serve(listener, app).await?;

    Ok(())
}
