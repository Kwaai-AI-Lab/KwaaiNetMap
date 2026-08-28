//! HTTP and WebSocket route handlers.
//!
//! Routes:
//! - `GET /api/stats`  — aggregated network stats
//! - `GET /api/nodes`  — list of peer entries
//! - `WS  /api/live`   — real-time stats pushed every 5 seconds
//! - `GET /health`     — liveness probe

use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
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

// ── GET /api/stats ────────────────────────────────────────────────────────────

pub async fn get_stats(State(state): State<SharedState>) -> impl IntoResponse {
    let stats = state.cache.stats(state.total_blocks);
    Json(stats)
}

// ── GET /api/nodes ────────────────────────────────────────────────────────────

pub async fn get_nodes(State(state): State<SharedState>) -> impl IntoResponse {
    let nodes = state.cache.snapshot();
    Json(nodes)
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
