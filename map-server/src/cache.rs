//! The served state: one crawl snapshot, replaced wholesale each pass.
//!
//! v1 of this crate accumulated peers into a map with TTL eviction. A snapshot
//! is a better fit now the API is per model: a peer that stops serving a block
//! disappears from that model's report on the next crawl, rather than lingering
//! until its TTL runs out and reporting coverage the network no longer has.

use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::snapshot::Snapshot;

/// A peer flattened out of the snapshot, for the v2 endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeEntry {
    pub peer_id: String,
    /// Trust tier: Unknown / Known / Verified / Trusted
    pub trust_tier: String,
    pub start_block: usize,
    pub end_block: usize,
    pub throughput: f64,
    pub public_name: String,
    pub version: String,
    /// Whether this node has VPK capability
    pub vpk: bool,
    pub last_seen: DateTime<Utc>,
}

impl NodeEntry {
    pub fn is_active(&self) -> bool {
        self.throughput > 0.0
    }
}

pub struct NodeCache {
    snapshot: RwLock<Arc<Snapshot>>,
    nodes: RwLock<Arc<Vec<NodeEntry>>>,
}

impl Default for NodeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeCache {
    pub fn new() -> Self {
        Self {
            snapshot: RwLock::new(Arc::new(Snapshot::default())),
            nodes: RwLock::new(Arc::new(Vec::new())),
        }
    }

    /// Install a fresh crawl. The flat node list is derived once here rather
    /// than per request, so `/api/nodes` is a clone of an `Arc`.
    pub fn replace(&self, snapshot: Snapshot) {
        let derived = crate::crawler::node_entries(&snapshot);
        *self.snapshot.write().expect("snapshot lock") = Arc::new(snapshot);
        *self.nodes.write().expect("nodes lock") = Arc::new(derived);
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.snapshot.read().expect("snapshot lock"))
    }

    pub fn nodes(&self) -> Arc<Vec<NodeEntry>> {
        Arc::clone(&self.nodes.read().expect("nodes lock"))
    }

    /// Aggregate stats over the current snapshot.
    ///
    /// `total_blocks` of `None` means "ask the network": coverage is measured
    /// against the largest model actually registered in the DHT. The
    /// `TOTAL_BLOCKS` env var overrides it. Getting this wrong is the classic
    /// misreport — an 80-block default against a 32-block model understates
    /// coverage by 2.5×.
    pub fn stats(&self, total_blocks: Option<usize>) -> NetworkStats {
        let snapshot = self.snapshot();
        let nodes = self.nodes();

        let total = total_blocks
            .or_else(|| {
                snapshot
                    .model_reports
                    .iter()
                    .map(|m| m.num_blocks.max(0) as usize)
                    .max()
                    .filter(|n| *n > 0)
            })
            .unwrap_or(80);

        let mut covered = vec![false; total];
        for report in &snapshot.model_reports {
            for row in &report.server_rows {
                if row.state != "online" {
                    continue;
                }
                let start = row.span.start.max(0) as usize;
                let end = (row.span.end.max(0) as usize).min(total.saturating_sub(1));
                for slot in covered.iter_mut().take(end + 1).skip(start) {
                    *slot = true;
                }
            }
        }
        let covered_count = covered.iter().filter(|c| **c).count();

        NetworkStats {
            node_count: nodes.len(),
            tokens_per_sec: nodes.iter().map(|n| n.throughput).sum(),
            coverage_pct: if total == 0 {
                0.0
            } else {
                covered_count as f64 / total as f64 * 100.0
            },
            active_sessions: nodes.iter().filter(|n| n.is_active()).count(),
            total_blocks: total,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    pub node_count: usize,
    pub tokens_per_sec: f64,
    pub coverage_pct: f64,
    pub active_sessions: usize,
    /// What `coverage_pct` was measured against.
    pub total_blocks: usize,
}
