//! The crawl snapshot and the `/api/v1/state` wire types.
//!
//! These structs ARE the v1 API's JSON. The shape is a published contract: a
//! node's health monitor polls `/api/v1/state`, `kwaai-cli`'s `map.rs` reads
//! `model_reports[].short_name` / `.server_rows` to pick which model to serve,
//! and the map page reads `peer_ip_info.location` and `span.server_info`.
//! Renaming a field here breaks a deployed fleet, not a test.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One node's DHT announcement, decoded from the `Ext(64)` record body.
///
/// Field names mirror what Petals servers publish, because both the v1 page
/// and Python Hivemind consumers read them under these names.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerInfo {
    /// `offline` | `joining` | `online` | `unknown`.
    pub state: String,
    pub throughput: f64,
    pub start_block: i64,
    pub end_block: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_rps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forward_rps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference_rps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub torch_dtype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quant_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub using_relay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_tokens_left: Option<i64>,
    /// A block shard is loading. Only sent while `state` is `joining`, and only
    /// by a node new enough to say so — absent means "not known to be loading",
    /// which the page words as idle rather than as progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard_loading: Option<bool>,
    /// VPK capability map, when the node published one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpk: Option<serde_json::Value>,
    /// Count of trust attestations — drives the v2 trust tier.
    #[serde(default)]
    pub trust_attestations: usize,
    /// Only a subkeyed record names its peer externally; a regular value
    /// carries its own id in the field map. Internal — not part of the v1 shape.
    #[serde(skip)]
    pub peer_id: Option<String>,
}

/// Geolocation of one address, in ip-api.com's field names.
///
/// `status` is what the page branches on: anything but `"success"` renders as
/// "Location Unknown" over a Los Angeles fallback pin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub status: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub lat: f64,
    #[serde(default)]
    pub lon: f64,
    #[serde(default)]
    pub isp: String,
}

impl Location {
    pub fn unknown() -> Self {
        Self {
            status: "fail".into(),
            country: String::new(),
            city: String::new(),
            lat: 0.0,
            lon: 0.0,
            isp: String::new(),
        }
    }

    /// The fallback when a relayed peer's relay could not itself be located;
    /// normally geoip pins "Via relay" at the relay's own coordinates.
    ///
    /// `status: "fail"`, deliberately: v1 called this "success" with lat/lon
    /// 0, and the page pins every "success" at its coordinates — which put
    /// relayed peers in the ocean at (0,0). A failed status keeps them in
    /// the page's Location Unknown bucket instead; the Connection Type
    /// column still says Relay.
    pub fn via_relay() -> Self {
        Self {
            status: "fail".into(),
            country: "Unknown".into(),
            city: "Via relay".into(),
            lat: 0.0,
            lon: 0.0,
            isp: "Relay".into(),
        }
    }
}

/// What the page needs to place a node and label its connection type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerIpInfo {
    pub location: Location,
    /// Observed multiaddrs — the ones that justify `reachability`.
    pub multiaddrs: Vec<String>,
    /// How the observer reaches the peer: `direct`, `punched` (DCUtR through
    /// its NAT), `relayed`, or `inbound` (it dialed us; nothing proves anyone
    /// can dial it). The page labels off this; absent on older snapshots, where
    /// it falls back to reading the multiaddrs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reachability: Option<String>,
}

impl Default for PeerIpInfo {
    fn default() -> Self {
        Self {
            location: Location::unknown(),
            multiaddrs: Vec::new(),
            reachability: None,
        }
    }
}

/// The contiguous block range one peer serves for one model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerSpan {
    pub peer_id: String,
    pub start: i64,
    pub end: i64,
    pub server_info: ServerInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerRow {
    pub peer_id: String,
    /// `...` plus the last six characters — the v1 table's abbreviation.
    pub short_peer_id: String,
    pub show_public_name: bool,
    pub state: String,
    pub peer_ip_info: PeerIpInfo,
    pub span: PeerSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelReport {
    /// Full repository path, e.g. `unsloth/Llama-3.1-8B-Instruct`.
    pub name: String,
    /// Last path segment, e.g. `Llama-3.1-8B-Instruct`.
    pub short_name: String,
    pub dht_prefix: String,
    pub repository: String,
    pub num_blocks: i64,
    /// `healthy` when every block has at least one online server, else `broken`.
    pub state: String,
    pub server_rows: Vec<ServerRow>,
}

/// A node with an unusually large share of the network's throughput.
///
/// No known consumer reads this yet; v1 published it, so v2 does too.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contributor {
    pub peer_id: String,
    pub public_name: Option<String>,
    pub throughput: f64,
    pub blocks: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReachabilityIssue {
    pub peer_id: String,
    pub err: String,
}

/// A block range as a row carries it: half-open, the node's own declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockSpan {
    pub start: i64,
    pub end: i64,
}

/// One peer of the network, whatever it serves.
///
/// The page draws its rows from this list. A peer is here because the
/// observer identified it speaking `/kwaai/kad` this pass, or because a kwaai
/// DHT record names it — a node is a peer for being up, not for serving
/// blocks. `model_reports` stays the per-model view the node health monitors
/// and `kwaai-cli` read; nothing there changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRow {
    pub peer_id: String,
    pub short_peer_id: String,
    pub public_name: Option<String>,
    /// What the node announces (`kwaai-0.7.0`), else identify's agent string
    /// (`kwaainet/0.7.0`) for a peer that announced nothing.
    pub version: Option<String>,
    /// The announced `state` (`online`, `joining`, `offline`); `unannounced`
    /// for a peer reached over kad that holds no record in the DHT — up, but
    /// either not announcing or failing to.
    pub state: String,
    pub shard_loading: bool,
    /// The model of its block record, if it has one, and that record's range.
    pub model: Option<String>,
    pub span: Option<BlockSpan>,
    pub throughput: f64,
    pub using_relay: Option<bool>,
    pub vpk: Option<serde_json::Value>,
    pub trust_attestations: usize,
    pub peer_ip_info: PeerIpInfo,
    /// Identified over a live connection speaking `/kwaai/kad` this pass.
    pub kad: bool,
    /// Named by at least one kwaai DHT record this pass.
    pub announced: bool,
}

/// The whole `/api/v1/state` document, and the cache's unit of replacement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// One `online`/`offline` per configured bootstrap, in the order given.
    pub bootstrap_states: Vec<String>,
    pub model_reports: Vec<ModelReport>,
    pub num_peers: usize,
    pub num_blocks_covered: usize,
    pub top_contributors: Vec<Contributor>,
    pub reachability_issues: Vec<ReachabilityIssue>,
    pub last_updated: DateTime<Utc>,
    /// Seconds between crawls, and how long the last one took. v1 publishes
    /// both; a dashboard reading them should not lose them at the cutover.
    pub update_period: u64,
    pub update_duration: f64,
    /// When the peer data above was last actually read, if the crawls since
    /// then read nothing and this document is being held. `None` while fresh.
    ///
    /// Both fields are skipped when they carry their fresh value, so a normal
    /// pass serializes byte-for-byte the document v1 serves and `make
    /// map-compare` stays a real test; they appear on the wire only while a
    /// snapshot is held. `serde(default)` is the reading half of that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_since: Option<DateTime<Utc>>,
    /// How many consecutive empty crawls have been held off.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub crawls_held: u32,
    /// Every peer, serving or not — see [`PeerRow`]. Additive: absent from
    /// v1's document, ignored by its readers.
    #[serde(default)]
    pub peers: Vec<PeerRow>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            bootstrap_states: Vec::new(),
            model_reports: Vec::new(),
            num_peers: 0,
            num_blocks_covered: 0,
            top_contributors: Vec::new(),
            reachability_issues: Vec::new(),
            last_updated: Utc::now(),
            update_period: 0,
            update_duration: 0.0,
            stale_since: None,
            crawls_held: 0,
            peers: Vec::new(),
        }
    }
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// `...` plus the last six characters, matching v1's table column.
pub fn short_peer_id(peer_id: &str) -> String {
    if peer_id.len() > 10 {
        format!("...{}", &peer_id[peer_id.len() - 6..])
    } else {
        peer_id.to_string()
    }
}
