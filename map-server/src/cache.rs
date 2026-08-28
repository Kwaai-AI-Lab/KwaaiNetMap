//! In-memory node cache with TTL eviction.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A single peer's info snapshot from the DHT.
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
    inner: Arc<DashMap<String, NodeEntry>>,
    ttl_secs: u64,
}

impl NodeCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            ttl_secs,
        }
    }

    pub fn upsert(&self, entry: NodeEntry) {
        self.inner.insert(entry.peer_id.clone(), entry);
    }

    /// Evict entries older than TTL and return the remaining snapshot.
    pub fn snapshot(&self) -> Vec<NodeEntry> {
        let cutoff = Utc::now() - chrono::Duration::seconds(self.ttl_secs as i64);
        // Remove stale entries
        self.inner.retain(|_, v| v.last_seen > cutoff);
        self.inner.iter().map(|r| r.value().clone()).collect()
    }

    /// Aggregate stats from current (non-stale) snapshot.
    pub fn stats(&self, total_blocks: usize) -> NetworkStats {
        let nodes = self.snapshot();
        let node_count = nodes.len();
        let tokens_per_sec: f64 = nodes.iter().map(|n| n.throughput).sum();
        let active_sessions = nodes.iter().filter(|n| n.is_active()).count();

        // Build coverage bitmap
        let mut covered = vec![false; total_blocks.max(1)];
        for node in &nodes {
            let end = node.end_block.min(total_blocks.saturating_sub(1));
            for b in node.start_block..=end {
                if b < covered.len() {
                    covered[b] = true;
                }
            }
        }
        let covered_count = covered.iter().filter(|&&c| c).count();
        let coverage_pct = if total_blocks == 0 {
            0.0
        } else {
            covered_count as f64 / total_blocks as f64 * 100.0
        };

        NetworkStats {
            node_count,
            tokens_per_sec,
            coverage_pct,
            active_sessions,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    pub node_count: usize,
    pub tokens_per_sec: f64,
    pub coverage_pct: f64,
    pub active_sessions: usize,
}
