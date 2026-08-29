//! The KwaaiNet map API.
//!
//! Serves both generations of the API from one crawl:
//!
//! - `GET /api/v1/state`               — the v1 document a node's health
//!   monitor polls and `kwaai-cli` reads to pick a model
//! - `GET /api/v1/is_reachable/<peer>` — live dial probe
//! - `GET /metrics`, `/api/prometheus` — Prometheus text
//! - `GET /api/stats`, `/api/nodes`    — aggregate stats and the flat peer list
//! - `WS  /api/live`                   — stats pushed every 5 s
//!
//! A background task crawls the DHT through a running `kwaainet` node every
//! 60 s and replaces the served snapshot.

use std::sync::Arc;

use anyhow::Result;
use axum::{
    routing::{any, get},
    Router,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod cache;
mod crawler;
mod dht;
mod geoip;
mod observer;
mod reachability;
mod routes;
mod snapshot;
mod state;

use state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "map_server=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3030".to_string());
    // Unset means "measure against the largest model the DHT registers".
    let total_blocks: Option<usize> = std::env::var("TOTAL_BLOCKS")
        .ok()
        .and_then(|v| v.parse().ok());

    // Bootstrap peers from env (space-separated multiaddrs) or use defaults.
    let bootstrap_peers: Vec<String> = std::env::var("BOOTSTRAP_PEERS")
        .unwrap_or_default()
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    if bootstrap_peers.is_empty() {
        anyhow::bail!(
            "BOOTSTRAP_PEERS is empty. Expected space-separated multiaddrs, each \
             including /p2p/<PeerId> — the crawler reads the peer id out of the \
             multiaddr and skips entries without one."
        );
    }

    // This process's own rust-libp2p swarm. Both the crawler and the
    // reachability probes drive it; there is no separate node process.
    let observer = observer::start(&bootstrap_peers).await?;
    let handle = observer.handle.clone();

    let node_cache = Arc::new(cache::NodeCache::new());
    let shared = Arc::new(AppState {
        cache: Arc::clone(&node_cache),
        total_blocks,
        reachability: reachability::Reachability::new(handle.clone()),
    });

    // Spawn background DHT crawler
    let crawler_cache = Arc::clone(&node_cache);
    tokio::spawn(async move {
        crawler::run_crawler(crawler_cache, handle, bootstrap_peers).await;
    });

    // CORS: allow the deployed origin in prod, everything in dev
    let allowed_origins = std::env::var("ALLOWED_ORIGINS").unwrap_or_else(|_| "*".to_string());
    let cors = if allowed_origins == "*" {
        CorsLayer::permissive()
    } else {
        let origins: Vec<_> = allowed_origins
            .split(',')
            .filter_map(|o| o.trim().parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    };

    let api = Router::new()
        .route("/stats", get(routes::get_stats))
        .route("/nodes", get(routes::get_nodes))
        .route("/live", any(routes::ws_live))
        .route("/prometheus", get(routes::metrics))
        .route("/v1/state", get(routes::get_v1_state))
        .route("/v1/is_reachable/:peer_id", get(routes::is_reachable))
        .with_state(Arc::clone(&shared));

    let app = Router::new()
        .nest("/api", api)
        .route("/health", get(routes::health))
        .route("/metrics", get(routes::metrics))
        .with_state(shared)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!("map-server listening on {bind_addr}");

    // A dead swarm means the crawler can never refresh, so serving on would
    // hand out a frozen snapshot indefinitely. Exit instead and let the
    // restart policy deal with it — the same reason the two-process container
    // this replaces took itself down when either half died.
    tokio::select! {
        result = axum::serve(listener, app) => result?,
        _ = observer.swarm => anyhow::bail!("libp2p swarm task ended"),
    }

    Ok(())
}
