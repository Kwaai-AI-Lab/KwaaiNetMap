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
    /// A circuit address carries the relay's IP, so it never contributes a
    /// location: a peer reachable only through a relay is "Via relay" rather
    /// than pinned at whichever bootstrap is relaying it.
    pub fn locate(&self, addrs: &[String]) -> Location {
        match first_public_ip(addrs) {
            Some(ip) => self.cached(&ip),
            None if addrs.iter().any(|a| a.contains("/p2p-circuit")) => Location::via_relay(),
            None => Location::unknown(),
        }
    }
}

/// The public IPs worth asking about, across every peer's addresses.
pub fn public_ips(addrs: &[String]) -> Vec<String> {
    addrs.iter().filter_map(|a| public_ip_of(a)).collect()
}

fn first_public_ip(addrs: &[String]) -> Option<String> {
    addrs.iter().find_map(|a| public_ip_of(a))
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
    fn relay_only_peers_are_labelled_not_mislocated() {
        let geo = GeoIp::from_env();
        let circuit = vec!["/ip4/93.184.216.34/tcp/8000/p2p/QmR/p2p-circuit/p2p/QmP".to_string()];
        assert_eq!(geo.locate(&circuit).city, "Via relay");
        assert_eq!(geo.locate(&[]).status, "fail");
    }
}
