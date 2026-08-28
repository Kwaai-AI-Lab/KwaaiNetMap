//! Background DHT crawler.
//!
//! Every 60 seconds this task dials each bootstrap peer via p2pd and sends a
//! `FindRequest` for all known DHT key prefixes. Results are decoded from the
//! Hivemind Ext(64) wire format and stored in the [`NodeCache`].
//!
//! Wire format (from node.rs / shard_cmd.rs):
//! ```text
//! Ext(64, msgpack([state_i32, throughput_f64, {fields_map}]))
//! fields_map keys: start_block, end_block, public_name, version,
//!                  torch_dtype, using_relay, cache_tokens_left,
//!                  adapters, next_pings, peer_id, trust_attestations, vpk
//! ```

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::Result;
use chrono::Utc;
use kwaai_hivemind_dht::protocol::{FindRequest, FindResponse, NodeInfo, RequestAuthInfo};
use kwaai_p2p::NetworkConfig;
use kwaai_p2p_daemon::{P2PClient, DEFAULT_SOCKET_NAME};
use libp2p::PeerId;
use prost::Message as _;
use sha1::{Digest, Sha1};
use tracing::{info, warn};

use crate::cache::{NodeCache, NodeEntry};

/// Fallback DHT key prefixes in effective_dht_prefix format (org stripped, dots→dashes).
/// These cover known KwaaiNet model prefixes. The crawler also auto-discovers prefixes
/// from the `_petals.models` registry at runtime.
const FALLBACK_PREFIXES: &[&str] = &[
    "Llama-3-1-8B-Instruct",
    "Llama-2-70b-chat-hf",
    "Meta-Llama-3-1-8B-Instruct",
    "bloom",
];

/// Total blocks to scan per prefix (upper bound; missing keys return empty).
const SCAN_BLOCKS: usize = 80;

/// How often to re-crawl the DHT.
const CRAWL_INTERVAL_SECS: u64 = 60;

pub async fn run_crawler(cache: Arc<NodeCache>, bootstrap_peers: Vec<String>) {
    loop {
        if let Err(e) = crawl_once(&cache, &bootstrap_peers).await {
            warn!("DHT crawl error: {e:#}");
        }
        tokio::time::sleep(Duration::from_secs(CRAWL_INTERVAL_SECS)).await;
    }
}

async fn crawl_once(cache: &NodeCache, bootstrap_peers: &[String]) -> Result<()> {
    let raw_sock =
        std::env::var("KWAAINET_SOCKET").unwrap_or_else(|_| DEFAULT_SOCKET_NAME.to_string());
    let socket = if cfg!(unix) {
        format!("/unix/{}", raw_sock)
    } else {
        "/ip4/127.0.0.1/tcp/5005".to_string()
    };
    let mut client = match P2PClient::connect(&socket).await {
        Ok(c) => c,
        Err(e) => {
            warn!("Cannot connect to p2pd at {socket}: {e}");
            return Ok(());
        }
    };

    let peer_id_hex = client.identify().await?;
    tracing::debug!("identify ok, peer_id_hex len={}", peer_id_hex.len());
    let our_peer_id =
        PeerId::from_bytes(&hex::decode(&peer_id_hex)?).unwrap_or_else(|_| PeerId::random());
    let our_dhtid = Sha1::new()
        .chain_update(our_peer_id.to_bytes())
        .finalize()
        .to_vec();

    let effective_bootstrap: Vec<String> = if bootstrap_peers.is_empty() {
        NetworkConfig::with_petals_bootstrap().bootstrap_peers
    } else {
        bootstrap_peers.to_vec()
    };

    let mut discovered: HashMap<String, NodeEntry> = HashMap::new();

    // Step 1: discover registered model prefixes from _petals.models registry
    let mut active_prefixes: Vec<String> =
        FALLBACK_PREFIXES.iter().map(|s| s.to_string()).collect();
    if let Ok(extra) = fetch_model_prefixes(&mut client, &our_dhtid, &effective_bootstrap).await {
        for p in extra {
            if !active_prefixes.contains(&p) {
                active_prefixes.push(p);
            }
        }
    }
    tracing::debug!(
        "crawling {} prefix(es): {:?}",
        active_prefixes.len(),
        active_prefixes
    );

    // Step 2: also crawl VPK nodes registry
    active_prefixes.push("_kwaai.vpk.nodes".to_string());

    for prefix in &active_prefixes {
        let keys: Vec<Vec<u8>> = if prefix.starts_with("_kwaai") {
            vec![dht_key(prefix)]
        } else {
            (0..SCAN_BLOCKS)
                .map(|b| dht_key(&format!("{}.{}", prefix, b)))
                .collect()
        };

        let find_req = FindRequest {
            auth: Some(RequestAuthInfo::new()),
            keys,
            peer: Some(NodeInfo {
                node_id: our_dhtid.clone(),
            }),
        };
        let mut req_bytes = Vec::new();
        find_req.encode(&mut req_bytes)?;

        for addr in &effective_bootstrap {
            let Some(peer_str) = addr.split("/p2p/").nth(1) else {
                continue;
            };
            let bp = match peer_str.parse::<PeerId>() {
                Ok(p) => p,
                Err(_) => continue,
            };

            if client.connect_peer(addr).await.is_err() {
                continue;
            }
            tokio::time::sleep(Duration::from_millis(400)).await;

            let resp_bytes = match client
                .call_unary_handler(&bp.to_bytes(), "DHTProtocol.rpc_find", &req_bytes)
                .await
            {
                Ok(b) => b,
                Err(e) => {
                    warn!("rpc_find failed for {addr}: {e}");
                    continue;
                }
            };

            tracing::debug!("rpc_find response {} bytes from {addr}", resp_bytes.len());

            let Ok(resp) = FindResponse::decode(&resp_bytes[..]) else {
                warn!("FindResponse decode failed for {addr}");
                continue;
            };

            tracing::debug!("{} results in FindResponse from {addr}", resp.results.len());

            for result in &resp.results {
                if result.value.is_empty() {
                    continue;
                }
                tracing::debug!(
                    "  result rt={} value_len={}",
                    result.result_type,
                    result.value.len()
                );
                match result.result_type {
                    1 => {
                        if let Some(entry) = decode_regular(&result.value) {
                            tracing::debug!("  → decoded peer {}", entry.peer_id);
                            discovered.entry(entry.peer_id.clone()).or_insert(entry);
                        } else {
                            tracing::debug!("  → decode_regular returned None");
                        }
                    }
                    2 => decode_dictionary(&result.value, &mut discovered),
                    _ => {}
                }
            }
        }
    }

    let count = discovered.len();
    for entry in discovered.into_values() {
        cache.upsert(entry);
    }
    info!("DHT crawl complete: {count} peer(s) found");
    Ok(())
}

// ── DHT key helpers ────────────────────────────────────────────────────────────

/// SHA1(msgpack(raw_key)) — matches node.rs `dht_id()` / Hivemind DHTID.generate().
fn dht_key(raw_key: &str) -> Vec<u8> {
    let packed = rmp_serde::to_vec(raw_key).expect("msgpack key");
    Sha1::new().chain_update(&packed).finalize().to_vec()
}

/// Fetch registered model prefixes from the `_petals.models` DHT registry.
/// Returns effective_dht_prefix strings (e.g. "Llama-3-1-8B-Instruct").
async fn fetch_model_prefixes(
    client: &mut P2PClient,
    our_dhtid: &[u8],
    bootstrap_peers: &[String],
) -> anyhow::Result<Vec<String>> {
    let find_req = FindRequest {
        auth: Some(RequestAuthInfo::new()),
        keys: vec![dht_key("_petals.models")],
        peer: Some(NodeInfo {
            node_id: our_dhtid.to_vec(),
        }),
    };
    let mut req_bytes = Vec::new();
    find_req.encode(&mut req_bytes)?;

    let mut prefixes = Vec::new();
    for addr in bootstrap_peers {
        let Some(peer_str) = addr.split("/p2p/").nth(1) else {
            continue;
        };
        let Ok(bp) = peer_str.parse::<PeerId>() else {
            continue;
        };
        if client.connect_peer(addr).await.is_err() {
            continue;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let Ok(resp_bytes) = client
            .call_unary_handler(&bp.to_bytes(), "DHTProtocol.rpc_find", &req_bytes)
            .await
        else {
            continue;
        };
        let Ok(resp) = FindResponse::decode(&resp_bytes[..]) else {
            continue;
        };

        for result in resp.results {
            if result.value.is_empty() {
                continue;
            }
            if result.result_type == 2 {
                // FoundDictionary: subkeys are msgpack(prefix_string)
                if let Some(subs) = extract_dict_subkeys(&result.value) {
                    prefixes.extend(subs);
                }
            }
        }
        if !prefixes.is_empty() {
            break;
        }
    }
    Ok(prefixes)
}

fn extract_dict_subkeys(bytes: &[u8]) -> Option<Vec<String>> {
    let outer = rmpv::decode::read_value(&mut &bytes[..]).ok()?;
    let inner_bytes = match &outer {
        rmpv::Value::Ext(80, b) => b.as_slice(),
        _ => return None,
    };
    let inner = rmpv::decode::read_value(&mut &inner_bytes[..]).ok()?;
    let arr = inner.as_array()?;
    if arr.len() < 3 {
        return None;
    }
    let entries = arr[2].as_array()?;
    let mut result = Vec::new();
    for entry in entries {
        let arr = entry.as_array()?;
        if arr.is_empty() {
            continue;
        }
        let prefix = match &arr[0] {
            rmpv::Value::String(s) => s.as_str().unwrap_or("").to_string(),
            rmpv::Value::Binary(b) => match rmpv::decode::read_value(&mut b.as_slice()) {
                Ok(rmpv::Value::String(s)) => s.as_str().unwrap_or("").to_string(),
                _ => continue,
            },
            _ => continue,
        };
        if !prefix.is_empty() {
            result.push(prefix)
        }
    }
    Some(result)
}

// ── Decoder: FoundRegular (rt=1) ──────────────────────────────────────────────

fn decode_regular(bytes: &[u8]) -> Option<NodeEntry> {
    let val = rmpv::decode::read_value(&mut &bytes[..]).ok()?;
    let inner_bytes = match &val {
        rmpv::Value::Ext(64, b) => b.as_slice(),
        _ => return None,
    };
    let inner = rmpv::decode::read_value(&mut &inner_bytes[..]).ok()?;
    let arr = inner.as_array()?;
    if arr.len() < 3 {
        return None;
    }

    let throughput = arr[1].as_f64().unwrap_or(0.0);
    let map = arr[2].as_map()?;

    let get_i = |k: &str| -> Option<i64> {
        map.iter()
            .find(|(ky, _)| ky.as_str() == Some(k))
            .and_then(|(_, v)| v.as_i64())
    };
    let get_s = |k: &str| -> String {
        map.iter()
            .find(|(ky, _)| ky.as_str() == Some(k))
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let _get_bool = |k: &str| -> bool {
        map.iter()
            .find(|(ky, _)| ky.as_str() == Some(k))
            .and_then(|(_, v)| v.as_bool())
            .unwrap_or(false)
    };

    let start_block = get_i("start_block")? as usize;
    let end_block = get_i("end_block")? as usize;
    let public_name = get_s("public_name");
    let peer_id_b58 = get_s("peer_id");
    let version = get_s("version");
    let vpk = map.iter().any(|(k, _)| k.as_str() == Some("vpk"));

    // Derive trust tier from trust_attestations count (VC-backed scoring TBD)
    let ta_count = map
        .iter()
        .find(|(k, _)| k.as_str() == Some("trust_attestations"))
        .and_then(|(_, v)| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let trust_tier = tier_from_vc_count(ta_count).to_string();

    Some(NodeEntry {
        peer_id: if peer_id_b58.is_empty() {
            format!("unknown:{}", bs58::encode(rand_bytes(8)).into_string())
        } else {
            peer_id_b58
        },
        trust_tier,
        start_block,
        end_block,
        throughput,
        public_name,
        version,
        vpk,
        last_seen: Utc::now(),
    })
}

// ── Decoder: FoundDictionary (rt=2, Python Hivemind) ─────────────────────────

fn decode_dictionary(bytes: &[u8], out: &mut HashMap<String, NodeEntry>) {
    let outer = match rmpv::decode::read_value(&mut &bytes[..]) {
        Ok(v) => v,
        Err(_) => return,
    };
    let inner_bytes = match &outer {
        rmpv::Value::Ext(80, b) => b.as_slice(),
        _ => return,
    };
    let inner = match rmpv::decode::read_value(&mut &inner_bytes[..]) {
        Ok(v) => v,
        Err(_) => return,
    };
    let outer_arr = match inner.as_array() {
        Some(a) if a.len() >= 3 => a,
        _ => return,
    };
    let entries = match outer_arr[2].as_array() {
        Some(e) => e,
        None => return,
    };

    for entry in entries {
        let arr = match entry.as_array() {
            Some(a) if a.len() >= 2 => a,
            _ => continue,
        };

        // Subkey is rmp_serde::to_vec(&peer_id_base58) = msgpack(string)
        let peer_id_b58 = match &arr[0] {
            rmpv::Value::String(s) => s.as_str().unwrap_or("").to_string(),
            rmpv::Value::Binary(b) => match rmpv::decode::read_value(&mut b.as_slice()) {
                Ok(rmpv::Value::String(s)) => s.as_str().unwrap_or("").to_string(),
                _ => continue,
            },
            _ => continue,
        };
        if peer_id_b58.is_empty() {
            continue;
        }

        // Value bytes: rmp_serde encoded NodeEntry map
        let val_bytes = match &arr[1] {
            rmpv::Value::Binary(b) => b.as_slice(),
            _ => continue,
        };
        if let Some(entry) = decode_regular(val_bytes) {
            out.entry(peer_id_b58.clone()).or_insert(NodeEntry {
                peer_id: peer_id_b58,
                ..entry
            });
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tier_from_vc_count(count: usize) -> &'static str {
    match count {
        0 => "Unknown",
        1..=2 => "Known",
        3..=4 => "Verified",
        _ => "Trusted",
    }
}

fn rand_bytes(n: usize) -> Vec<u8> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::SystemTime;
    let mut h = DefaultHasher::new();
    SystemTime::now().hash(&mut h);
    let v = h.finish().to_le_bytes();
    v[..n.min(8)].to_vec()
}
