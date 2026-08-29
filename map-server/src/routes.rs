//! HTTP and WebSocket route handlers.
//!
//! The `/api/v1/` routes are the v1 service's contract, reproduced here so a
//! DNS cutover does not break a fleet: every running node's health monitor
//! polls `/api/v1/state`, and `kwaai-cli` reads it to choose which model to
//! serve. The unversioned routes are this server's own.

use std::fmt::Write as _;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    response::{IntoResponse, Json},
};
use serde_json::json;
use tracing::warn;

use crate::state::SharedState;

// ── /health ───────────────────────────────────────────────────────────────────

pub async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

// ── GET /api/v1/state ─────────────────────────────────────────────────────────

pub async fn get_v1_state(State(state): State<SharedState>) -> impl IntoResponse {
    Json(state.cache.snapshot())
}

// ── GET /api/v1/is_reachable/:peer_id ─────────────────────────────────────────

pub async fn is_reachable(
    State(state): State<SharedState>,
    Path(peer_id): Path<String>,
) -> impl IntoResponse {
    // Reuse the addresses the last crawl saw, rather than dialling blind.
    let snapshot = state.cache.snapshot();
    let addrs = snapshot
        .model_reports
        .iter()
        .flat_map(|m| &m.server_rows)
        .find(|row| row.peer_id == peer_id)
        .map(|row| row.peer_ip_info.multiaddrs.clone())
        .unwrap_or_default();

    Json(state.reachability.check(&peer_id, &addrs).await)
}

// ── GET /api/stats ────────────────────────────────────────────────────────────

pub async fn get_stats(State(state): State<SharedState>) -> impl IntoResponse {
    Json(state.cache.stats(state.total_blocks))
}

// ── GET /api/nodes ────────────────────────────────────────────────────────────

pub async fn get_nodes(State(state): State<SharedState>) -> impl IntoResponse {
    Json(state.cache.nodes())
}

// ── GET /metrics ──────────────────────────────────────────────────────────────

/// Prometheus text exposition. v1 published this at both paths; keep both so a
/// scrape config written against v1 keeps working.
pub async fn metrics(State(state): State<SharedState>) -> impl IntoResponse {
    let stats = state.cache.stats(state.total_blocks);
    let snapshot = state.cache.snapshot();
    let bootstraps_online = snapshot
        .bootstrap_states
        .iter()
        .filter(|s| *s == "online")
        .count();

    let mut out = String::new();
    let mut gauge = |name: &str, help: &str, value: f64| {
        let _ = writeln!(out, "# HELP {name} {help}");
        let _ = writeln!(out, "# TYPE {name} gauge");
        let _ = writeln!(out, "{name} {value}");
    };

    gauge(
        "kwaainet_nodes",
        "Servers seen in the last crawl.",
        stats.node_count as f64,
    );
    gauge(
        "kwaainet_peers",
        "Distinct peers seen in the last crawl.",
        snapshot.num_peers as f64,
    );
    gauge(
        "kwaainet_tokens_per_second",
        "Aggregate announced throughput.",
        stats.tokens_per_sec,
    );
    gauge(
        "kwaainet_block_coverage_percent",
        "Block coverage of the largest model.",
        stats.coverage_pct,
    );
    gauge(
        "kwaainet_blocks_covered",
        "Model blocks with at least one online server.",
        snapshot.num_blocks_covered as f64,
    );
    gauge(
        "kwaainet_models",
        "Models registered in the DHT.",
        snapshot.model_reports.len() as f64,
    );
    gauge(
        "kwaainet_bootstraps_online",
        "Reachable bootstrap peers.",
        bootstraps_online as f64,
    );
    gauge(
        "kwaainet_bootstraps_total",
        "Configured bootstrap peers.",
        snapshot.bootstrap_states.len() as f64,
    );
    gauge(
        "kwaainet_last_crawl_timestamp_seconds",
        "Unix time of the last completed crawl.",
        snapshot.last_updated.timestamp() as f64,
    );

    ([("content-type", "text/plain; version=0.0.4")], out)
}

// ── WS /api/live ──────────────────────────────────────────────────────────────

pub async fn ws_live(ws: WebSocketUpgrade, State(state): State<SharedState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_live(socket, state))
}

async fn handle_live(mut socket: WebSocket, state: SharedState) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        interval.tick().await;
        let stats = state.cache.stats(state.total_blocks);
        let payload = match serde_json::to_string(&stats) {
            Ok(s) => s,
            Err(e) => {
                warn!("stats serialize error: {e}");
                continue;
            }
        };
        if socket.send(Message::Text(payload)).await.is_err() {
            // Client disconnected
            break;
        }
    }
}
