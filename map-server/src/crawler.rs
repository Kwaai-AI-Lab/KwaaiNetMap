//! Background DHT crawler.
//!
//! Runs against this process's own rust-libp2p swarm (see [`crate::observer`]):
//! dial the bootstraps, then `DHTProtocol.rpc_find` over `call_unary_handler`.
//! No Go daemon and no control socket are involved.
//!
//! Each pass walks the `_petals.models` registry for the models on the
//! network, then every block key of each model, and folds the results into one
//! [`Snapshot`]. The model dimension is kept rather than flattened because
//! `/api/v1/state` is reported per model.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use kwaai_hivemind_dht::protocol::{FindRequest, FindResponse, NodeInfo, RequestAuthInfo};
use kwaai_hivemind_dht::PROTOCOL_FIND;
use kwaai_p2p::{NetworkHandle, PeerId};
use prost::Message as _;
use sha1::{Digest, Sha1};
use tracing::{debug, info, warn};

use crate::cache::{NodeCache, NodeEntry};
use crate::dht::{decode_model_registry, decode_server_info, dht_key, dictionary_entries};
use crate::geoip::{public_ips, relay_dns_names, relay_peer_id_of, via_relay_at, GeoIp};
use crate::snapshot::{
    short_peer_id, Contributor, Location, ModelReport, PeerIpInfo, PeerSpan, ReachabilityIssue,
    ServerInfo, ServerRow, Snapshot,
};

/// The `_petals.models` registry names the models actually on the network.
/// These are only used when it comes back empty, so a mid-migration DHT still
/// yields a map instead of a blank page.
const FALLBACK_PREFIXES: &[&str] = &[
    "Llama-3-1-8B-Instruct",
    "Meta-Llama-3-1-8B-Instruct",
    "Llama-2-70b-chat-hf",
    "bloom",
];

/// Block count assumed for a fallback prefix, which carries no registration.
const FALLBACK_BLOCKS: i64 = 80;

/// Upper bound on block keys queried for one model — a guard against a
/// malformed registration asking for a million-key `FindRequest`.
const MAX_BLOCKS: i64 = 512;

const CRAWL_INTERVAL: Duration = Duration::from_secs(60);

/// Bound on one bootstrap probe. `connect_peer` asks the daemon for a 60 s
/// dial timeout, so without this a fleet with two dead bootstraps would spend
/// two minutes of every crawl interval waiting to find that out.
const BOOTSTRAP_DIAL_TIMEOUT: Duration = Duration::from_secs(15);

/// Records for nodes that announce a capability rather than a block range.
const CAPABILITY_KEYS: &[&str] = &["_kwaai.vpk.nodes", "_kwaai.inference.nodes"];

/// What one peer publishes for one model, folded across that model's blocks.
struct PeerModelEntry {
    blocks: BTreeSet<i64>,
    info: ServerInfo,
}

pub async fn run_crawler(
    cache: Arc<NodeCache>,
    handle: NetworkHandle,
    bootstrap_peers: Vec<String>,
) {
    let geo = GeoIp::from_env();
    loop {
        match crawl_once(&handle, &bootstrap_peers, &geo).await {
            Ok(Some(snapshot)) => {
                let served = cache.publish(snapshot);
                if served.crawls_held > 0 {
                    // The bootstrap count is the one thing a held pass did
                    // measure, so it belongs on this line too.
                    warn!(
                        "crawl read 0 peers ({} bootstrap online) — holding the snapshot of {} peer(s) from {} (held {}x)",
                        bootstrap_tally(&served),
                        served.num_peers,
                        served.last_updated,
                        served.crawls_held,
                    );
                } else {
                    info!(
                        "crawl complete: {} peer(s), {} model(s), {} bootstrap online",
                        served.num_peers,
                        served.model_reports.len(),
                        bootstrap_tally(&served),
                    );
                }
            }
            // Reserved for a transport failure with nothing to publish.
            Ok(None) => {}
            Err(e) => warn!("DHT crawl error: {e:#}"),
        }
        tokio::time::sleep(CRAWL_INTERVAL).await;
    }
}

/// `online/total` over the bootstrap dots, as both crawl log lines report it.
fn bootstrap_tally(snapshot: &Snapshot) -> String {
    let online = snapshot
        .bootstrap_states
        .iter()
        .filter(|s| *s == "online")
        .count();
    format!("{online}/{}", snapshot.bootstrap_states.len())
}

/// One full pass. `Ok(None)` is unreachable now that the swarm is in-process;
/// the signature keeps it so a future transport failure has somewhere to go.
async fn crawl_once(
    handle: &NetworkHandle,
    bootstrap_peers: &[String],
    geo: &GeoIp,
) -> Result<Option<Snapshot>> {
    let started = std::time::Instant::now();
    let our_dhtid = Sha1::new()
        .chain_update(handle.peer_id().to_bytes())
        .finalize()
        .to_vec();

    let bootstraps = bootstrap_peers.to_vec();

    // Dial every bootstrap once up front. The dials are what the queries below
    // ride; they are NOT the reported state — see BootstrapSet.
    let mut set = BootstrapSet::new(&bootstraps);
    for entry in &mut set.entries {
        entry.dialed = entry.addr.contains("/p2p/")
            && matches!(
                tokio::time::timeout(BOOTSTRAP_DIAL_TIMEOUT, handle.connect_peer(&entry.addr))
                    .await,
                Ok(Ok(_))
            );
    }
    if !set.has_dialable() {
        warn!("no bootstrap dialable — publishing an empty snapshot");
        return Ok(Some(Snapshot {
            bootstrap_states: set.states(),
            update_period: CRAWL_INTERVAL.as_secs(),
            update_duration: started.elapsed().as_secs_f64(),
            ..Default::default()
        }));
    }
    // Give the dials a moment to settle before the first RPC rides them.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let models = discover_models(handle, &our_dhtid, &mut set).await;
    debug!("crawling {} model(s)", models.len());

    // prefix -> peer -> blocks + info
    let mut per_model: BTreeMap<String, HashMap<String, PeerModelEntry>> = BTreeMap::new();
    for model in &models {
        let entry = per_model.entry(model.dht_prefix.clone()).or_default();
        collect_model(handle, &our_dhtid, &mut set, model, entry).await;
    }

    // Nodes that announce a capability but serve no blocks would otherwise be
    // invisible; they belong in /api/nodes even with no model to sit under.
    let mut capability_peers: HashMap<String, ServerInfo> = HashMap::new();
    for key in CAPABILITY_KEYS {
        collect_capability(handle, &our_dhtid, &mut set, key, &mut capability_peers).await;
    }

    let discovered: BTreeSet<String> = per_model
        .values()
        .flat_map(|peers| peers.keys().cloned())
        .chain(capability_peers.keys().cloned())
        .collect();
    let dial_errors = connect_discovered_peers(handle, &discovered).await;

    let addrs = observed_addrs(handle).await;
    let flat: Vec<String> = addrs.values().flatten().cloned().collect();
    let mut ips = public_ips(&flat);
    ips.extend(geo.resolve_relays(&relay_dns_names(&flat)).await);
    geo.warm(&ips).await;

    let mut snapshot = build_snapshot(
        set.states(),
        &models,
        per_model,
        capability_peers,
        &addrs,
        &dial_errors,
        geo,
    );
    snapshot.update_period = CRAWL_INTERVAL.as_secs();
    snapshot.update_duration = started.elapsed().as_secs_f64();
    Ok(Some(snapshot))
}

// ── Bootstrap health ──────────────────────────────────────────────────────────

/// The bootstraps, and what this crawl has learned about each.
///
/// `bootstrap_states` is the field the debugging docs tell an operator to check
/// when their node cannot connect, so it has to mean something. A successful
/// `connect_peer` does not: the daemon accepts the request and returns Ok
/// before it knows whether a route exists, so an address that is simply dead
/// dials "successfully" and then fails every query with "peer not found in
/// DHT". Only an answered query is evidence, so that is what marks a bootstrap
/// online.
struct BootstrapEntry {
    addr: String,
    /// The initial dial did not error. Necessary to try a query, not evidence.
    dialed: bool,
    /// It answered a query this pass. This is what "online" means.
    answered: bool,
}

struct BootstrapSet {
    entries: Vec<BootstrapEntry>,
}

impl BootstrapSet {
    fn new(addrs: &[String]) -> Self {
        Self {
            entries: addrs
                .iter()
                .map(|addr| BootstrapEntry {
                    addr: addr.clone(),
                    dialed: false,
                    answered: false,
                })
                .collect(),
        }
    }

    fn has_dialable(&self) -> bool {
        self.entries.iter().any(|e| e.dialed)
    }

    /// Addresses still worth asking. A bootstrap that has already failed a
    /// query this pass is dropped: every later query would cost the same dial
    /// timeout to learn the same thing, which is minutes across a crawl.
    fn live(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.dialed)
            .map(|e| e.addr.clone())
            .collect()
    }

    fn record(&mut self, addr: &str, answered: bool) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.addr == addr) {
            if answered {
                entry.answered = true;
            } else {
                entry.dialed = false;
            }
        }
    }

    fn states(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|e| if e.answered { "online" } else { "offline" }.to_string())
            .collect()
    }
}

// ── DHT queries ───────────────────────────────────────────────────────────────

/// Ask one bootstrap for a batch of keys, recording whether it answered.
/// Results are positionally aligned with the keys sent, which is how a response
/// is mapped back to a block index.
async fn rpc_find(
    handle: &NetworkHandle,
    our_dhtid: &[u8],
    set: &mut BootstrapSet,
    bootstrap: &str,
    keys: Vec<Vec<u8>>,
) -> Option<FindResponse> {
    let response = rpc_find_inner(handle, our_dhtid, bootstrap, keys).await;
    set.record(bootstrap, response.is_some());
    response
}

async fn rpc_find_inner(
    handle: &NetworkHandle,
    our_dhtid: &[u8],
    bootstrap: &str,
    keys: Vec<Vec<u8>>,
) -> Option<FindResponse> {
    let bp: PeerId = bootstrap.split("/p2p/").nth(1)?.parse().ok()?;
    let request = FindRequest {
        auth: Some(RequestAuthInfo::new()),
        keys,
        peer: Some(NodeInfo {
            node_id: our_dhtid.to_vec(),
        }),
    };
    let mut bytes = Vec::new();
    request.encode(&mut bytes).ok()?;

    let e = match handle.call_unary_handler(bp, PROTOCOL_FIND, &bytes).await {
        Ok(resp) => return FindResponse::decode(&resp[..]).ok(),
        Err(e) => e,
    };
    warn!("rpc_find failed against {bootstrap}: {e}");

    // Measured on production: 48 of 234 crawls lost an rpc_find to
    // `P2PError::Abandoned` — the connection carrying it closed, which the
    // bootstraps do after ~30s idle. One such loss drops the bootstrap for the
    // whole pass and paints it offline, so it is worth one retry.
    //
    // The redial is not reliably a redial: `Command::ConnectPeer` answers Ok
    // from the swarm's own connection record (`DialPeerConditionFalse` maps to
    // `AlreadyConnected`, which `ConnectPeer` reports as success), so if the
    // swarm has not yet seen the close it dials nothing. Hence the log line —
    // redial=ok with rpc=err is the shape that says exactly that.
    let redial =
        match tokio::time::timeout(BOOTSTRAP_DIAL_TIMEOUT, handle.connect_peer(bootstrap)).await {
            Ok(Ok(_)) => "ok".to_string(),
            Ok(Err(e)) => format!("err({e})"),
            Err(_) => "timeout".to_string(),
        };
    let retried = handle.call_unary_handler(bp, PROTOCOL_FIND, &bytes).await;
    let rpc = match &retried {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("err({e})"),
    };
    info!("rpc_find retry against {bootstrap}: redial={redial} rpc={rpc}");

    // Only this outcome reaches `set.record`; the first failure is not recorded.
    retried
        .ok()
        .and_then(|resp| FindResponse::decode(&resp[..]).ok())
}

struct Model {
    dht_prefix: String,
    repository: String,
    num_blocks: i64,
}

/// Read `_petals.models` for the models on the network, falling back to the
/// known prefixes when the registry is empty or unreachable.
async fn discover_models(
    handle: &NetworkHandle,
    our_dhtid: &[u8],
    set: &mut BootstrapSet,
) -> Vec<Model> {
    let mut found: BTreeMap<String, Model> = BTreeMap::new();

    for addr in set.live() {
        let Some(resp) = rpc_find(
            handle,
            our_dhtid,
            set,
            &addr,
            vec![dht_key("_petals.models")],
        )
        .await
        else {
            continue;
        };
        for result in &resp.results {
            if result.value.is_empty() {
                continue;
            }
            for reg in decode_model_registry(&result.value) {
                let num_blocks = reg.num_blocks.clamp(1, MAX_BLOCKS);
                found.entry(reg.dht_prefix.clone()).or_insert(Model {
                    dht_prefix: reg.dht_prefix,
                    repository: reg.repository,
                    num_blocks,
                });
            }
        }
        if !found.is_empty() {
            break;
        }
    }

    if found.is_empty() {
        warn!("_petals.models registry empty — falling back to known prefixes");
        return FALLBACK_PREFIXES
            .iter()
            .map(|p| Model {
                dht_prefix: p.to_string(),
                repository: String::new(),
                num_blocks: FALLBACK_BLOCKS,
            })
            .collect();
    }
    found.into_values().collect()
}

/// Query every block key of one model and fold the servers found under each.
async fn collect_model(
    handle: &NetworkHandle,
    our_dhtid: &[u8],
    set: &mut BootstrapSet,
    model: &Model,
    out: &mut HashMap<String, PeerModelEntry>,
) {
    let keys: Vec<Vec<u8>> = (0..model.num_blocks)
        .map(|b| dht_key(&format!("{}.{}", model.dht_prefix, b)))
        .collect();

    for addr in set.live() {
        let Some(resp) = rpc_find(handle, our_dhtid, set, &addr, keys.clone()).await else {
            continue;
        };
        for (index, result) in resp.results.iter().enumerate() {
            if result.value.is_empty() {
                continue;
            }
            let block = index as i64;
            for (peer_id, info) in servers_in(result.result_type, &result.value) {
                let entry = out.entry(peer_id).or_insert_with(|| PeerModelEntry {
                    blocks: BTreeSet::new(),
                    info: info.clone(),
                });
                entry.blocks.insert(block);
                // Later bootstraps can hold a fresher copy of the same record.
                if info.state == "online" {
                    entry.info = info;
                }
            }
        }
    }
}

/// Query one capability registry (`_kwaai.vpk.nodes`, `_kwaai.inference.nodes`).
///
/// These two do not agree on a value shape: the inference registry holds an
/// `Ext(64)` server record, the VPK one a plain msgpack capability map. The
/// subkey is a peer id in both, so the peer is counted either way and the
/// record is decoded only if it happens to be one.
async fn collect_capability(
    handle: &NetworkHandle,
    our_dhtid: &[u8],
    set: &mut BootstrapSet,
    key: &str,
    out: &mut HashMap<String, ServerInfo>,
) {
    for addr in set.live() {
        let Some(resp) = rpc_find(handle, our_dhtid, set, &addr, vec![dht_key(key)]).await else {
            continue;
        };
        for result in &resp.results {
            if result.value.is_empty() {
                continue;
            }
            for (peer_id, raw) in dictionary_entries(&result.value) {
                let info = decode_server_info(&raw).unwrap_or(ServerInfo {
                    state: "unknown".to_string(),
                    ..Default::default()
                });
                out.entry(peer_id).or_insert(info);
            }
        }
    }
}

/// Servers carried by one `FindResult`, whichever result type it came back as.
///
/// A dictionary is the normal case — many servers subkeyed by peer id under one
/// block key. A regular value only ever holds one, and identifies its peer from
/// inside the record.
fn servers_in(result_type: i32, value: &[u8]) -> Vec<(String, ServerInfo)> {
    const REGULAR: i32 = 1;
    const DICTIONARY: i32 = 2;

    match result_type {
        DICTIONARY => dictionary_entries(value)
            .into_iter()
            .filter_map(|(peer_id, raw)| Some((peer_id, decode_server_info(&raw)?)))
            .collect(),
        REGULAR => decode_server_info(value)
            .and_then(|info| Some(vec![(info.peer_id.clone()?, info)]))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Bound on one peer dial during the location pass.
/// 15s, not v1's 5: a relay handshake through a loaded bootstrap can take
/// longer than a plain TCP connect, and a missed dial blanks the peer's
/// location for a whole crawl. Worst case this pass costs
/// ceil(peers / PEER_DIAL_CONCURRENCY) x 15s of the 60s crawl interval.
const PEER_DIAL_TIMEOUT: Duration = Duration::from_secs(15);

/// Dials run together; the pass costs one timeout, not one per unreachable peer.
const PEER_DIAL_CONCURRENCY: usize = 8;

/// Dial every discovered peer, so `observed_addrs` below has connections to
/// read addresses from, and return why each failed dial failed.
///
/// The DHT yields peer ids, never addresses, so a location can only come off
/// a live connection. v1 dialled every server each pass as its reachability
/// probe and read `list_peers` afterwards — its locations were a side effect
/// of that probe, and without an equivalent every row is "Location Unknown".
/// The bare `/p2p/<id>` dial is the routed path: Kademlia supplies whatever
/// addresses the walk above just taught it. Failures leave the peer unlocated,
/// as on v1, and connections are left for the swarm's idle timeout to reap.
///
/// The failures are also the only real connectivity signal in the crawl, which
/// is what `reachability_issues` reports — hence the return value rather than
/// inferring it from a peer's absence from `list_peers`, where an idle-timeout
/// reap between the dial and the listing would read as unreachable.
async fn connect_discovered_peers(
    handle: &NetworkHandle,
    peer_ids: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    use futures::StreamExt as _;

    let ours = handle.local_peer_id();
    // Owned ids, not borrows of the set: an async block capturing `&String`
    // from the iterator cannot satisfy the higher-ranked bound `buffer_unordered`
    // needs.
    let targets: Vec<String> = peer_ids.iter().filter(|id| **id != ours).cloned().collect();
    futures::stream::iter(targets)
        .map(|peer_id| async move {
            let addr = format!("/p2p/{peer_id}");
            let err =
                match tokio::time::timeout(PEER_DIAL_TIMEOUT, handle.connect_peer(&addr)).await {
                    Ok(Ok(_)) => return None,
                    Ok(Err(e)) => e.to_string(),
                    Err(_) => format!("dial timed out after {}s", PEER_DIAL_TIMEOUT.as_secs()),
                };
            debug!("location dial {peer_id}: {err}");
            Some((peer_id, err))
        })
        .buffer_unordered(PEER_DIAL_CONCURRENCY)
        .filter_map(|r| async move { r })
        .collect()
        .await
}

/// Addresses of every peer the observer currently holds a connection to.
///
/// This is the only source of peer addresses: DHT records carry peer ids, never
/// addresses — which is why the crawl dials every discovered peer first.
///
/// `list_peers` reports one entry per *connection*, so a peer reached more than
/// one way appears more than once; the addresses are grouped back per peer.
async fn observed_addrs(handle: &NetworkHandle) -> HashMap<String, Vec<String>> {
    let peers = match handle.list_peers().await {
        Ok(p) => p,
        Err(e) => {
            warn!("list_peers failed: {e}");
            return HashMap::new();
        }
    };

    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for peer in peers {
        out.entry(peer.peer_id.to_base58())
            .or_default()
            .push(peer.addr.to_string());
    }
    out
}

// ── Snapshot assembly ─────────────────────────────────────────────────────────

/// A row's location, with one fallback geoip alone cannot provide: a peer
/// relayed through another *node* often records the relay hop by that node's
/// LAN address (a node relaying from behind its own NAT), which geolocates to
/// nothing — but the relay is usually a peer this crawl already located, so
/// the row borrows its relay's coordinates by peer id.
fn locate_row(geo: &GeoIp, peer_addrs: &[String], known: &HashMap<String, Location>) -> Location {
    let loc = geo.locate(peer_addrs);
    if loc.status == "success" || !peer_addrs.iter().any(|a| a.contains("/p2p-circuit")) {
        return loc;
    }
    peer_addrs
        .iter()
        .find_map(|a| {
            let relay = known.get(&relay_peer_id_of(a)?)?;
            (relay.status == "success" && (relay.lat, relay.lon) != (0.0, 0.0))
                .then(|| via_relay_at(relay.clone()))
        })
        .unwrap_or(loc)
}

fn build_snapshot(
    bootstrap_states: Vec<String>,
    models: &[Model],
    per_model: BTreeMap<String, HashMap<String, PeerModelEntry>>,
    capability_peers: HashMap<String, ServerInfo>,
    addrs: &HashMap<String, Vec<String>>,
    dial_errors: &BTreeMap<String, String>,
    geo: &GeoIp,
) -> Snapshot {
    let mut model_reports = Vec::new();
    let mut all_peers: BTreeSet<String> = BTreeSet::new();
    let mut blocks_covered = 0usize;
    // Keyed by peer so a server carrying several models is reported once.
    let mut issues: BTreeMap<String, String> = BTreeMap::new();

    // Every connected peer's own location, keyed by peer id, so a peer
    // relayed through another *node* can borrow its relay's pin.
    let known: HashMap<String, Location> = addrs
        .iter()
        .map(|(id, a)| (id.clone(), geo.locate(a)))
        .collect();

    for model in models {
        let Some(peers) = per_model.get(&model.dht_prefix) else {
            continue;
        };
        if peers.is_empty() {
            continue;
        }

        let mut covered: BTreeSet<i64> = BTreeSet::new();
        let mut server_rows = Vec::new();
        for (peer_id, entry) in peers {
            all_peers.insert(peer_id.clone());
            // v1 probes only the servers claiming to serve, and reports the
            // ones that would not answer. A peer that never claimed to serve
            // is not an issue, and a state that is merely not ONLINE is not a
            // reachability fact at all — it was the announcement all along.
            if entry.info.state == "online" {
                covered.extend(entry.blocks.iter().copied());
                if let Some(err) = dial_errors.get(peer_id) {
                    issues.insert(peer_id.clone(), err.clone());
                }
            }

            let peer_addrs = addrs.get(peer_id).cloned().unwrap_or_default();
            server_rows.push(ServerRow {
                short_peer_id: short_peer_id(peer_id),
                peer_id: peer_id.clone(),
                show_public_name: entry.info.public_name.is_some(),
                state: entry.info.state.clone(),
                peer_ip_info: PeerIpInfo {
                    location: locate_row(geo, &peer_addrs, &known),
                    multiaddrs: peer_addrs,
                },
                span: PeerSpan {
                    peer_id: peer_id.clone(),
                    // The node's own declared range, verbatim, which is what v1
                    // puts here and what the page renders as "start-end". Note
                    // `end` is EXCLUSIVE: a node serving only block 0 announces
                    // 0..1, and the DHT holds one key for it. Deriving this from
                    // the block keys actually seen would read 0-0 instead and
                    // silently disagree with v1 on every row.
                    start: entry.info.start_block,
                    end: entry.info.end_block,
                    server_info: entry.info.clone(),
                },
            });
        }
        server_rows.sort_by(|a, b| {
            a.span
                .start
                .cmp(&b.span.start)
                .then(a.peer_id.cmp(&b.peer_id))
        });
        blocks_covered += covered.len();

        let name = model_name(&model.repository, &model.dht_prefix);
        model_reports.push(ModelReport {
            short_name: name.rsplit('/').next().unwrap_or(&name).to_string(),
            name,
            dht_prefix: model.dht_prefix.clone(),
            repository: model.repository.clone(),
            num_blocks: model.num_blocks,
            state: if covered.len() as i64 == model.num_blocks {
                "healthy".into()
            } else {
                "broken".into()
            },
            server_rows,
        });
    }

    // Most servers first — the page has no sort of its own on this list.
    model_reports.sort_by_key(|m| std::cmp::Reverse(m.server_rows.len()));
    all_peers.extend(capability_peers.keys().cloned());

    Snapshot {
        top_contributors: top_contributors(&model_reports),
        // Filled in by the caller, which is what times the pass.
        update_period: 0,
        update_duration: 0.0,
        bootstrap_states,
        num_peers: all_peers.len(),
        num_blocks_covered: blocks_covered,
        model_reports,
        reachability_issues: issues
            .into_iter()
            .map(|(peer_id, err)| ReachabilityIssue { peer_id, err })
            .collect(),
        last_updated: Utc::now(),
        // A crawl that produced a snapshot is fresh by construction; the hold
        // logic sets these if it ends up being held instead.
        stale_since: None,
        crawls_held: 0,
    }
}

/// `unsloth/Llama-3.1-8B-Instruct` from a HuggingFace repository URL, or the
/// DHT prefix when the model published no repository.
fn model_name(repository: &str, dht_prefix: &str) -> String {
    let path = repository
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(repository);
    match path.split_once('/') {
        Some((_host, rest)) if !rest.is_empty() => rest.trim_end_matches('/').to_string(),
        _ => dht_prefix.to_string(),
    }
}

/// The five highest-throughput servers on the network.
fn top_contributors(reports: &[ModelReport]) -> Vec<Contributor> {
    let mut best: HashMap<String, Contributor> = HashMap::new();
    for report in reports {
        for row in &report.server_rows {
            let candidate = Contributor {
                peer_id: row.peer_id.clone(),
                public_name: row.span.server_info.public_name.clone(),
                throughput: row.span.server_info.throughput,
                blocks: row.span.end - row.span.start + 1,
            };
            best.entry(row.peer_id.clone())
                .and_modify(|c| {
                    if candidate.throughput > c.throughput {
                        *c = candidate.clone();
                    }
                })
                .or_insert(candidate);
        }
    }
    let mut out: Vec<_> = best.into_values().collect();
    out.sort_by(|a, b| b.throughput.total_cmp(&a.throughput));
    out.truncate(5);
    out
}

/// What to serve, given a fresh crawl and what is currently served.
///
/// A pass that read zero peers is far more often both bootstraps having lost
/// their connection mid-crawl than the network having emptied — measured: 4 of
/// 234 crawls blanked the public map for a minute with nothing having changed.
/// So an empty crawl over populated data holds the data and takes only the
/// bootstrap dots and timings from this pass. Deliberately no partial rule: a
/// crawl that read *some* peers is real churn and replaces wholesale.
pub fn merge_crawl(served: &Snapshot, fresh: Snapshot) -> Snapshot {
    // `served.num_peers == 0` is the startup case — nothing to hold.
    if fresh.num_peers > 0 || served.num_peers == 0 {
        return fresh;
    }
    Snapshot {
        bootstrap_states: fresh.bootstrap_states,
        update_period: fresh.update_period,
        update_duration: fresh.update_duration,
        // When the held data was read, so it survives further held passes.
        stale_since: served.stale_since.or(Some(served.last_updated)),
        crawls_held: served.crawls_held + 1,
        // `last_updated` means "when this was read" and must not advance.
        ..served.clone()
    }
}

/// The flat per-peer view `/api/nodes` and `/api/stats` are built from.
pub fn node_entries(snapshot: &Snapshot) -> Vec<NodeEntry> {
    let mut best: HashMap<String, NodeEntry> = HashMap::new();
    for report in &snapshot.model_reports {
        for row in &report.server_rows {
            let info = &row.span.server_info;
            let entry = NodeEntry {
                peer_id: row.peer_id.clone(),
                trust_tier: tier_from_vc_count(info.trust_attestations).to_string(),
                start_block: row.span.start.max(0) as usize,
                end_block: row.span.end.max(0) as usize,
                throughput: info.throughput,
                public_name: info.public_name.clone().unwrap_or_default(),
                version: info.version.clone().unwrap_or_default(),
                vpk: info.vpk.is_some(),
                last_seen: snapshot.last_updated,
            };
            // A peer serving several models appears once, under its best one.
            best.entry(row.peer_id.clone())
                .and_modify(|e| {
                    if entry.throughput > e.throughput {
                        *e = entry.clone();
                    }
                })
                .or_insert(entry);
        }
    }
    best.into_values().collect()
}

fn tier_from_vc_count(count: usize) -> &'static str {
    match count {
        0 => "Unknown",
        1..=2 => "Known",
        3..=4 => "Verified",
        _ => "Trusted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peer_relayed_through_a_known_node_borrows_its_pin() {
        // Seen live: a node relaying from behind its own NAT records the
        // relay hop by LAN address, which geolocates to nothing — but the
        // relay itself is a located peer in the same crawl.
        let geo = GeoIp::from_env();
        let relay_id = "12D3KooWRelay".to_string();
        let circuit = vec![format!(
            "/ip4/192.168.0.198/tcp/8080/p2p/{relay_id}/p2p-circuit/p2p/12D3KooWPeer"
        )];

        let mut known = HashMap::new();
        known.insert(
            relay_id,
            Location {
                status: "success".into(),
                country: "United States".into(),
                city: "Phoenix".into(),
                lat: 33.4,
                lon: -112.1,
                isp: "Cox".into(),
            },
        );
        let loc = locate_row(&geo, &circuit, &known);
        assert_eq!(loc.status, "success");
        assert_eq!(loc.city, "Via relay");
        assert_eq!((loc.lat, loc.lon), (33.4, -112.1));

        // No known relay: the sentinel must NOT be a "success" at (0,0) —
        // that is the page's cue to pin a peer in the ocean.
        let loc = locate_row(&geo, &circuit, &HashMap::new());
        assert_eq!(loc.status, "fail");
    }

    #[test]
    fn a_dialable_bootstrap_is_not_online_until_it_answers() {
        let mut set = BootstrapSet::new(&["/ip4/1.2.3.4/tcp/8000/p2p/QmA".to_string()]);
        set.entries[0].dialed = true;
        // A dial that did not error proves nothing on its own.
        assert_eq!(set.states(), vec!["offline"]);

        set.record("/ip4/1.2.3.4/tcp/8000/p2p/QmA", true);
        assert_eq!(set.states(), vec!["online"]);
    }

    #[test]
    fn a_failed_query_drops_the_bootstrap_for_the_rest_of_the_pass() {
        let mut set = BootstrapSet::new(&[
            "/ip4/1.2.3.4/tcp/8000/p2p/QmA".to_string(),
            "/ip4/5.6.7.8/tcp/8000/p2p/QmB".to_string(),
        ]);
        set.entries.iter_mut().for_each(|e| e.dialed = true);

        set.record("/ip4/1.2.3.4/tcp/8000/p2p/QmA", false);
        assert_eq!(
            set.live(),
            vec!["/ip4/5.6.7.8/tcp/8000/p2p/QmB".to_string()]
        );
        assert_eq!(set.states(), vec!["offline", "offline"]);
    }

    /// A node serving exactly one block announces 0..1 and puts one key in the
    /// DHT. v1 renders that as "0-1"; reading the span off the observed keys
    /// would render "0-0" and disagree with v1 on every row of every model.
    #[test]
    fn a_single_block_server_spans_zero_to_one() {
        let model = Model {
            dht_prefix: "M".into(),
            repository: String::new(),
            num_blocks: 32,
        };
        let mut peers = HashMap::new();
        peers.insert(
            "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG".to_string(),
            PeerModelEntry {
                blocks: [0].into_iter().collect(),
                info: ServerInfo {
                    state: "online".into(),
                    start_block: 0,
                    end_block: 1,
                    ..Default::default()
                },
            },
        );
        let snapshot = build_snapshot(
            vec!["online".into()],
            &[model],
            BTreeMap::from([("M".to_string(), peers)]),
            HashMap::new(),
            &HashMap::new(),
            &BTreeMap::new(),
            &GeoIp::from_env(),
        );

        let span = &snapshot.model_reports[0].server_rows[0].span;
        assert_eq!((span.start, span.end), (0, 1));
        assert_eq!(snapshot.num_blocks_covered, 1);
    }

    #[test]
    fn model_name_comes_from_the_repository_url() {
        assert_eq!(
            model_name("https://huggingface.co/unsloth/Llama-3.1-8B-Instruct", "x"),
            "unsloth/Llama-3.1-8B-Instruct"
        );
    }

    #[test]
    fn model_name_falls_back_to_the_prefix() {
        assert_eq!(
            model_name("", "Llama-3-1-8B-Instruct"),
            "Llama-3-1-8B-Instruct"
        );
    }

    #[test]
    fn a_model_is_broken_until_every_block_is_served() {
        let model = Model {
            dht_prefix: "M".into(),
            repository: String::new(),
            num_blocks: 4,
        };
        let mut peers = HashMap::new();
        peers.insert(
            "QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG".to_string(),
            PeerModelEntry {
                blocks: [0, 1, 2].into_iter().collect(),
                info: ServerInfo {
                    state: "online".into(),
                    // Half-open, as announced: blocks 0, 1 and 2.
                    start_block: 0,
                    end_block: 3,
                    ..Default::default()
                },
            },
        );
        let snapshot = build_snapshot(
            vec!["online".into()],
            &[model],
            BTreeMap::from([("M".to_string(), peers)]),
            HashMap::new(),
            &HashMap::new(),
            &BTreeMap::new(),
            &GeoIp::from_env(),
        );

        let report = &snapshot.model_reports[0];
        assert_eq!(report.state, "broken");
        assert_eq!(snapshot.num_blocks_covered, 3);
        // The span is the node's announced range, half-open — NOT the highest
        // block key observed, which would read 2 here and disagree with v1.
        assert_eq!(report.server_rows[0].span.start, 0);
        assert_eq!(report.server_rows[0].span.end, 3);
        assert_eq!(report.server_rows[0].short_peer_id, "...WnPbdG");
    }

    #[test]
    fn an_offline_server_covers_nothing_and_is_not_a_reachability_issue() {
        let model = Model {
            dht_prefix: "M".into(),
            repository: String::new(),
            num_blocks: 2,
        };
        let mut peers = HashMap::new();
        peers.insert(
            "QmGone".to_string(),
            PeerModelEntry {
                blocks: [0, 1].into_iter().collect(),
                info: ServerInfo {
                    state: "offline".into(),
                    start_block: 0,
                    end_block: 2,
                    ..Default::default()
                },
            },
        );
        let snapshot = build_snapshot(
            vec!["online".into()],
            &[model],
            BTreeMap::from([("M".to_string(), peers)]),
            HashMap::new(),
            &HashMap::new(),
            &BTreeMap::new(),
            &GeoIp::from_env(),
        );

        assert_eq!(snapshot.num_blocks_covered, 0);
        assert_eq!(snapshot.model_reports[0].state, "broken");
        // A departed server is an announcement, not a failed dial. v1 probes
        // only the servers claiming to serve and reports what would not answer.
        assert!(snapshot.reachability_issues.is_empty());
    }

    /// A server claiming ONLINE that the crawl could not dial is the one thing
    /// `reachability_issues` is for, and it is reported once however many
    /// models it serves.
    #[test]
    fn an_undialable_online_server_is_reported_once() {
        let models = [
            Model {
                dht_prefix: "M".into(),
                repository: String::new(),
                num_blocks: 1,
            },
            Model {
                dht_prefix: "N".into(),
                repository: String::new(),
                num_blocks: 1,
            },
        ];
        let entry = || {
            let mut peers = HashMap::new();
            peers.insert(
                "QmDark".to_string(),
                PeerModelEntry {
                    blocks: [0].into_iter().collect(),
                    info: ServerInfo {
                        state: "online".into(),
                        start_block: 0,
                        end_block: 1,
                        ..Default::default()
                    },
                },
            );
            peers
        };
        let snapshot = build_snapshot(
            vec!["online".into()],
            &models,
            BTreeMap::from([("M".to_string(), entry()), ("N".to_string(), entry())]),
            HashMap::new(),
            &HashMap::new(),
            &BTreeMap::from([("QmDark".to_string(), "dial timed out after 15s".to_string())]),
            &GeoIp::from_env(),
        );

        assert_eq!(snapshot.reachability_issues.len(), 1);
        assert_eq!(snapshot.reachability_issues[0].peer_id, "QmDark");
        assert_eq!(
            snapshot.reachability_issues[0].err,
            "dial timed out after 15s"
        );
        // The row still says what the node announced; only the issue list is
        // about connectivity.
        assert_eq!(snapshot.model_reports[0].server_rows[0].state, "online");
    }

    fn snapshot_of(num_peers: usize, bootstrap_states: &[&str]) -> Snapshot {
        Snapshot {
            num_peers,
            num_blocks_covered: num_peers * 4,
            bootstrap_states: bootstrap_states.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    /// The measured failure: both bootstraps lost their connection mid-crawl,
    /// the pass read nothing, and the map went blank for a minute.
    #[test]
    fn an_empty_crawl_holds_the_served_peer_data() {
        let mut served = snapshot_of(7, &["online", "online"]);
        served.last_updated = Utc::now() - chrono::Duration::seconds(60);
        let read_at = served.last_updated;

        let merged = merge_crawl(&served, snapshot_of(0, &["offline", "offline"]));

        assert_eq!(merged.num_peers, 7);
        assert_eq!(merged.num_blocks_covered, 28);
        // "when this data was read" — it must not advance while held.
        assert_eq!(merged.last_updated, read_at);
        // The dots are about this pass, not the held data.
        assert_eq!(merged.bootstrap_states, vec!["offline", "offline"]);
        assert_eq!(merged.stale_since, Some(read_at));
        assert_eq!(merged.crawls_held, 1);
    }

    #[test]
    fn a_second_held_pass_keeps_the_original_stale_since() {
        let mut served = snapshot_of(7, &["offline", "offline"]);
        served.last_updated = Utc::now() - chrono::Duration::seconds(120);
        let first_stale = Utc::now() - chrono::Duration::seconds(180);
        served.stale_since = Some(first_stale);
        served.crawls_held = 1;

        let merged = merge_crawl(&served, snapshot_of(0, &["offline", "online"]));

        assert_eq!(merged.stale_since, Some(first_stale));
        assert_eq!(merged.crawls_held, 2);
    }

    #[test]
    fn a_crawl_that_read_peers_replaces_and_clears_the_stale_marks() {
        let mut served = snapshot_of(7, &["offline", "offline"]);
        served.stale_since = Some(Utc::now());
        served.crawls_held = 3;

        let merged = merge_crawl(&served, snapshot_of(2, &["online", "online"]));

        // Real churn, even downward: 2 peers replaces 7.
        assert_eq!(merged.num_peers, 2);
        assert_eq!(merged.stale_since, None);
        assert_eq!(merged.crawls_held, 0);
    }

    /// The hold fields must not widen the v1 document: `make map-compare`
    /// diffs this JSON against v1's on every pass, and a `"stale_since": null`
    /// on every fresh snapshot would make it disagree forever.
    #[test]
    fn the_hold_fields_reach_the_wire_only_while_held() {
        let fresh = serde_json::to_value(snapshot_of(7, &["online"])).unwrap();
        assert!(fresh.get("stale_since").is_none());
        assert!(fresh.get("crawls_held").is_none());

        let held = merge_crawl(&snapshot_of(7, &["online"]), snapshot_of(0, &["offline"]));
        let held = serde_json::to_value(held).unwrap();
        assert!(held.get("stale_since").is_some());
        assert_eq!(held.get("crawls_held").and_then(|v| v.as_u64()), Some(1));
    }

    /// Startup: the cache holds `Snapshot::default()`, so there is nothing to
    /// hold and an empty first crawl must be published as-is.
    #[test]
    fn an_empty_crawl_over_an_empty_cache_replaces() {
        let merged = merge_crawl(&Snapshot::default(), snapshot_of(0, &["offline"]));

        assert_eq!(merged.bootstrap_states, vec!["offline"]);
        assert_eq!(merged.crawls_held, 0);
        assert_eq!(merged.stale_since, None);
    }
}
