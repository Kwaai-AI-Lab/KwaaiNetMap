//! IP geolocation for the map view.
//!
//! v1 called ip-api.com once per address behind an unbounded `functools.cache`.
//! This keeps the same provider and field names — the page reads
//! `status`/`city`/`country`/`lat`/`lon` straight off the response — but uses
//! the batch endpoint so one crawl costs one request rather than one per node,
//! which is what keeps a growing network inside the free tier's 45 req/min.

use std::net::IpAddr;
use std::sync::Arc;

use dashmap::DashMap;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::snapshot::Location;

/// ip-api.com's batch endpoint takes at most 100 addresses per request.
const BATCH_LIMIT: usize = 100;

/// Cache ceiling. Entries are never invalidated — an IP's city does not move —
/// so this exists only to stop an unbounded crawl leaking memory.
const CACHE_LIMIT: usize = 50_000;

#[derive(Debug, Deserialize)]
struct ApiEntry {
    #[serde(default)]
    status: String,
    #[serde(default)]
    country: String,
    #[serde(default)]
    city: String,
    #[serde(default)]
    lat: f64,
    #[serde(default)]
    lon: f64,
    #[serde(default)]
    isp: String,
    #[serde(default)]
    query: String,
}

pub struct GeoIp {
    cache: Arc<DashMap<String, Location>>,
    /// Relay DNS name → resolved public IP. Successes only: relays are
    /// stable, a failed lookup is retried next crawl.
    dns: Arc<DashMap<String, String>>,
    client: reqwest::Client,
    endpoint: String,
    enabled: bool,
}

impl GeoIp {
    /// `GEOIP_ENABLED=0` turns lookups off entirely — every node then reports
    /// an unknown location. Sealed or air-gapped deployments want this; so does
    /// anyone unwilling to send operator IPs to a third party.
    pub fn from_env() -> Self {
        let enabled = std::env::var("GEOIP_ENABLED")
            .map(|v| !matches!(v.as_str(), "0" | "false" | "no"))
            .unwrap_or(true);
        let endpoint = std::env::var("GEOIP_BATCH_URL")
            .unwrap_or_else(|_| "http://ip-api.com/batch".to_string());

        Self {
            cache: Arc::new(DashMap::new()),
            dns: Arc::new(DashMap::new()),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
            endpoint,
            enabled,
        }
    }

    /// Resolve every uncached address in one pass, then read results from cache.
    pub async fn warm(&self, ips: &[String]) {
        if !self.enabled || self.cache.len() >= CACHE_LIMIT {
            return;
        }

        let mut pending: Vec<String> = ips
            .iter()
            .filter(|ip| !self.cache.contains_key(*ip))
            .cloned()
            .collect();
        pending.sort();
        pending.dedup();
        if pending.is_empty() {
            return;
        }

        for chunk in pending.chunks(BATCH_LIMIT) {
            match self.fetch_batch(chunk).await {
                Ok(entries) => {
                    for e in entries {
                        let location = Location {
                            status: e.status.clone(),
                            country: e.country,
                            city: e.city,
                            lat: e.lat,
                            lon: e.lon,
                            isp: e.isp,
                        };
                        self.cache.insert(e.query, location);
                    }
                }
                Err(err) => {
                    // A geolocation outage must not fail the crawl; nodes just
                    // render at the page's fallback pin until it recovers.
                    warn!("geoip batch lookup failed: {err}");
                    return;
                }
            }
        }
    }

    async fn fetch_batch(&self, ips: &[String]) -> anyhow::Result<Vec<ApiEntry>> {
        debug!("geoip: resolving {} address(es)", ips.len());
        let entries = self
            .client
            .post(&self.endpoint)
            .json(ips)
            .send()
            .await?
            .error_for_status()?
            .json::<Vec<ApiEntry>>()
            .await?;
        Ok(entries)
    }

    fn cached(&self, ip: &str) -> Location {
        self.cache
            .get(ip)
            .map(|l| l.clone())
            .unwrap_or_else(Location::unknown)
    }

    /// Where to pin a peer, given every address we have seen it on.
    ///
    /// A circuit address carries the relay's IP, so it never yields the
    /// peer's own location. It does place a pin: a relay-only peer borrows
    /// the relay's coordinates so it appears near its relay on the map
    /// instead of at the (0,0) sentinel, while the "Via relay" label keeps
    /// the popup honest about whose city that is.
    pub fn locate(&self, addrs: &[String]) -> Location {
        if let Some(ip) = first_public_ip(addrs) {
            return self.cached(&ip);
        }
        let relay_ip = first_relay_ip(addrs).or_else(|| {
            addrs
                .iter()
                .find_map(|a| relay_dns_of(a).and_then(|n| self.dns.get(&n).map(|e| e.clone())))
        });
        match relay_ip {
            Some(ip) => via_relay_at(self.cached(&ip)),
            None if addrs.iter().any(|a| a.contains("/p2p-circuit")) => Location::via_relay(),
            None => Location::unknown(),
        }
    }

    /// Resolve relay DNS names to IPs, returning every IP now known for
    /// `names` so the caller can warm them. Disabled deployments do no
    /// lookups at all — the sealed test bed stays silent.
    pub async fn resolve_relays(&self, names: &[String]) -> Vec<String> {
        let mut pending: Vec<&String> = names.iter().collect();
        pending.sort();
        pending.dedup();

        let mut out = Vec::new();
        for name in pending {
            if let Some(ip) = self.dns.get(name) {
                out.push(ip.clone());
                continue;
            }
            if !self.enabled {
                continue;
            }
            match tokio::net::lookup_host((name.as_str(), 0)).await {
                Ok(resolved) => {
                    if let Some(ip) = resolved.map(|sa| sa.ip()).find(is_public) {
                        let ip = ip.to_string();
                        self.dns.insert(name.clone(), ip.clone());
                        out.push(ip);
                    }
                }
                Err(e) => debug!("relay dns {name}: {e}"),
            }
        }
        out
    }
}

/// "Via relay", pinned at the relay's coordinates. An unresolved relay falls
/// back to the plain sentinel rather than claiming (0,0) was looked up.
fn via_relay_at(relay: Location) -> Location {
    if relay.status != "success" {
        return Location::via_relay();
    }
    Location {
        city: "Via relay".to_string(),
        isp: format!("Relay: {}", relay.isp),
        ..relay
    }
}

/// The public IPs worth asking about, across every peer's addresses — the
/// peers' own, plus relay IPs so a relayed peer's borrowed pin resolves too.
pub fn public_ips(addrs: &[String]) -> Vec<String> {
    addrs
        .iter()
        .filter_map(|a| public_ip_of(a))
        .chain(addrs.iter().filter_map(|a| relay_ip_of(a)))
        .collect()
}

fn first_public_ip(addrs: &[String]) -> Option<String> {
    addrs.iter().find_map(|a| public_ip_of(a))
}

fn first_relay_ip(addrs: &[String]) -> Option<String> {
    addrs.iter().find_map(|a| relay_ip_of(a))
}

/// DNS names of relays across every peer's addresses — relays are as often
/// named as numbered (kubo's AutoTLS names under libp2p.direct, our
/// bootstraps' /dns/ forms), and a name geolocates only once resolved.
pub fn relay_dns_names(addrs: &[String]) -> Vec<String> {
    addrs.iter().filter_map(|a| relay_dns_of(a)).collect()
}

/// The relay's IP out of a circuit address: the transport before
/// `/p2p-circuit` is how we reach the relay, and its IP locates the relay.
fn relay_ip_of(addr: &str) -> Option<String> {
    let (relay_part, _) = addr.split_once("/p2p-circuit")?;
    public_ip_of(relay_part)
}

/// The relay's DNS name out of a circuit address, when it is named not
/// numbered.
fn relay_dns_of(addr: &str) -> Option<String> {
    let (relay_part, _) = addr.split_once("/p2p-circuit")?;
    let mut parts = relay_part.split('/').skip(1);
    while let Some(proto) = parts.next() {
        if matches!(proto, "dns" | "dns4" | "dns6" | "dnsaddr") {
            return parts.next().map(str::to_string);
        }
    }
    None
}

/// The IP of one multiaddr, if it is a directly dialable public address.
fn public_ip_of(addr: &str) -> Option<String> {
    if addr.contains("/p2p-circuit") {
        return None;
    }
    let mut parts = addr.split('/').skip(1);
    while let Some(proto) = parts.next() {
        if matches!(proto, "ip4" | "ip6") {
            let raw = parts.next()?;
            let ip: IpAddr = raw.parse().ok()?;
            return is_public(&ip).then(|| raw.to_string());
        }
    }
    None
}

/// Private, loopback and reserved ranges are never sent to the geolocation
/// provider: it cannot resolve them, and a LAN address is not ours to publish.
fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                // 198.18.0.0/15 — benchmarking range, and the kwaaiai-env test bed.
                || (v4.octets()[0] == 198 && (v4.octets()[1] & 0xfe) == 18)
                // 100.64.0.0/10 — carrier-grade NAT.
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1])))
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback() || v6.is_unspecified() || v6.octets()[0] & 0xfe == 0xfc)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_public_ip_out_of_a_multiaddr() {
        assert_eq!(
            public_ip_of("/ip4/93.184.216.34/tcp/8000/p2p/QmAbc"),
            Some("93.184.216.34".to_string())
        );
    }

    #[test]
    fn skips_private_and_test_bed_ranges() {
        for addr in [
            "/ip4/192.168.1.10/tcp/8000",
            "/ip4/10.0.0.5/tcp/8000",
            "/ip4/127.0.0.1/tcp/8000",
            "/ip4/198.18.0.40/tcp/8000",
            "/ip4/100.100.0.1/tcp/8000",
        ] {
            assert_eq!(public_ip_of(addr), None, "{addr} should not be geolocated");
        }
    }

    #[test]
    fn a_circuit_address_yields_no_ip() {
        let circuit = "/ip4/93.184.216.34/tcp/8000/p2p/QmRelay/p2p-circuit/p2p/QmPeer";
        assert_eq!(public_ip_of(circuit), None);
    }

    #[test]
    fn a_named_relay_yields_its_dns_name() {
        // kubo's AutoTLS shape, seen live: peers relayed through named relays
        // rendered at (0,0) because only ip4 relay parts were recognised.
        let autotls =
            "/dns4/140-99-164-63.k51abc.libp2p.direct/tcp/4001/tls/ws/p2p-circuit/p2p/12D3KooWPeer";
        assert_eq!(
            relay_dns_of(autotls),
            Some("140-99-164-63.k51abc.libp2p.direct".to_string())
        );
        let bootstrap = "/dns/bootstrap-1.kwaai.ai/tcp/8000/p2p/QmRelay/p2p-circuit/p2p/QmPeer";
        assert_eq!(
            relay_dns_of(bootstrap),
            Some("bootstrap-1.kwaai.ai".to_string())
        );
        // Not a circuit: the name is the peer's own, not a relay's.
        assert_eq!(relay_dns_of("/dns4/node.example.com/tcp/8000"), None);
    }

    #[test]
    fn the_relay_ip_comes_out_of_a_circuit_address() {
        let circuit = "/ip4/93.184.216.34/tcp/8000/p2p/QmRelay/p2p-circuit/p2p/QmPeer";
        assert_eq!(relay_ip_of(circuit), Some("93.184.216.34".to_string()));
        assert_eq!(
            relay_ip_of("/ip4/93.184.216.34/tcp/8000/p2p/QmDirect"),
            None
        );
    }

    #[test]
    fn a_located_relay_lends_its_coordinates_but_not_its_name() {
        let relay = Location {
            status: "success".into(),
            country: "United States".into(),
            city: "Ashburn".into(),
            lat: 39.0,
            lon: -77.5,
            isp: "AWS".into(),
        };
        let loc = via_relay_at(relay);
        assert_eq!(loc.city, "Via relay");
        assert_eq!(loc.country, "United States");
        assert_eq!(loc.lat, 39.0);
        assert_eq!(loc.isp, "Relay: AWS");
        // An unresolved relay keeps the plain sentinel, not a fake (0,0) fix.
        assert_eq!(via_relay_at(Location::unknown()).country, "Unknown");
    }

    #[test]
    fn relay_only_peers_are_labelled_not_mislocated() {
        let geo = GeoIp::from_env();
        let circuit = vec!["/ip4/93.184.216.34/tcp/8000/p2p/QmR/p2p-circuit/p2p/QmP".to_string()];
        assert_eq!(geo.locate(&circuit).city, "Via relay");
        assert_eq!(geo.locate(&[]).status, "fail");
    }
}
