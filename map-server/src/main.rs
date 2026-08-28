//! map.kwaai.ai public API server.
//!
//! Serves:
//! - `GET /api/stats` — aggregated network stats (node count, tps, coverage)
//! - `GET /api/nodes` — list of all known peers with trust tier + shard info
//! - `WS  /api/live`  — real-time stats stream (5 s deltas)
//!
//! A background task crawls the DHT via the running p2pd every 60 s, refreshing
//! the in-memory [`NodeCache`].

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
mod routes;
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
    let total_blocks: usize = std::env::var("TOTAL_BLOCKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(80);

    // Bootstrap peers from env (space-separated multiaddrs) or use defaults.
    let bootstrap_peers: Vec<String> = std::env::var("BOOTSTRAP_PEERS")
        .unwrap_or_default()
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    let node_cache = Arc::new(cache::NodeCache::new(120));
    let shared = Arc::new(AppState {
        cache: Arc::clone(&node_cache),
        total_blocks,
    });

    // Spawn background DHT crawler
    let crawler_cache = Arc::clone(&node_cache);
    tokio::spawn(async move {
        crawler::run_crawler(crawler_cache, bootstrap_peers).await;
    });

    // CORS: allow map.kwaai.ai in prod, everything in dev
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
        .with_state(shared);

    let app = Router::new()
        .nest("/api", api)
        .route("/health", get(routes::health))
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!("map-server listening on {bind_addr}");
    axum::serve(listener, app).await?;

    Ok(())
}
