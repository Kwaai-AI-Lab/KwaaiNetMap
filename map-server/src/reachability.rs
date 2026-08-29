//! Live reachability probes behind `/api/v1/is_reachable/<peer_id>`.
//!
//! v1 dialled the peer through Hivemind and cached the verdict for 300 s. This
//! does the same on the observer's own swarm: dial, then hang up immediately —
//! the question is only whether a connection can be established.

use std::time::{Duration, Instant};

use dashmap::DashMap;
use kwaai_p2p::{NetworkHandle, PeerId};
use serde::Serialize;

/// Matches v1's cache expiry. A probe is a real dial, so an uncached endpoint
/// is a lever for making the observer node dial arbitrary peers on request.
const CACHE_TTL: Duration = Duration::from_secs(300);

const DIAL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct Reachability {
    cache: DashMap<String, (Instant, Verdict)>,
    handle: NetworkHandle,
}

impl Reachability {
    pub fn new(handle: NetworkHandle) -> Self {
        Self {
            cache: DashMap::new(),
            handle,
        }
    }

    /// `addrs` are the multiaddrs the last crawl saw this peer on. Dialling a
    /// known address is far more likely to succeed than a bare `/p2p/<id>`,
    /// which leaves the daemon to find a route on its own.
    pub async fn check(&self, peer_id: &str, addrs: &[String]) -> Verdict {
        if let Some(entry) = self.cache.get(peer_id) {
            if entry.0.elapsed() < CACHE_TTL {
                return entry.1.clone();
            }
        }

        let verdict = match self.dial(peer_id, addrs).await {
            Ok(()) => Verdict {
                ok: true,
                error: None,
            },
            Err(e) => Verdict {
                ok: false,
                error: Some(e),
            },
        };
        self.cache
            .insert(peer_id.to_string(), (Instant::now(), verdict.clone()));
        verdict
    }

    async fn dial(&self, peer_id: &str, addrs: &[String]) -> Result<(), String> {
        let parsed: PeerId = peer_id.parse().map_err(|_| "not a peer id".to_string())?;

        // Every address we know, then the bare id — which is not a fallback so
        // much as the routed path: a dial with no transport component leaves
        // Kademlia to supply an address.
        let mut candidates: Vec<String> = addrs.to_vec();
        candidates.push(format!("/p2p/{peer_id}"));

        let mut last = "no candidate address".to_string();
        for addr in candidates {
            match tokio::time::timeout(DIAL_TIMEOUT, self.handle.connect_peer(&addr)).await {
                Ok(Ok(_)) => {
                    let _ = self.handle.disconnect_peer(parsed).await;
                    return Ok(());
                }
                Ok(Err(e)) => last = e.to_string(),
                Err(_) => {
                    last = format!(
                        "failed to connect in {} sec. Firewall may be blocking connections",
                        DIAL_TIMEOUT.as_secs()
                    )
                }
            }
        }
        Err(last)
    }
}
