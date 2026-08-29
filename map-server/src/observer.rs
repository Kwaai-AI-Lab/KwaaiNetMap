//! The observer: this process's own rust-libp2p node.
//!
//! The swarm runs in this process via `kwaai-p2p`, and the crawler issues its
//! DHT calls straight through a [`NetworkHandle`].
//!
//! Deliberately **not** `kwaai-p2p-daemon`: that crate speaks the control
//! protocol of the standalone Go libp2p daemon, which KwaaiNet has replaced
//! with rust-libp2p, and its build script needs a Go toolchain to compile that
//! Go binary. Going through it would also mean a second process to run and a
//! unix control socket between the two.
//!
//! # Why this cannot announce itself
//!
//! An observer must read the DHT and publish nothing, or the map counts itself
//! as a serving node and claims blocks it does not have. Here that is
//! structural rather than configured: announcing is an explicit call this
//! crate never makes, and `dht_server: false` keeps Kademlia a client that
//! answers no one else's queries. There is no `announce_self` flag to be
//! silently ignored by the wrong binary version.

use anyhow::{Context, Result};
use kwaai_p2p::{Multiaddr, NetworkConfig, NetworkHandle, NetworkService};
use libp2p::identity;
use tokio::task::JoinHandle;
use tracing::{info, warn};

pub struct Observer {
    pub handle: NetworkHandle,
    /// The swarm task. Held so the swarm outlives this struct's owner rather
    /// than by accident of tokio not cancelling detached tasks.
    pub swarm: JoinHandle<()>,
}

/// Start the swarm and dial the bootstrap set.
///
/// The identity is generated per process and never persisted: nothing dials
/// the map, so a stable peer id would buy nothing and a key file would be one
/// more thing to mount.
pub async fn start(bootstrap_peers: &[String]) -> Result<Observer> {
    let keypair = identity::Keypair::generate_ed25519();
    let peer_id = keypair.public().to_peer_id();

    let config = NetworkConfig {
        initial_peers: bootstrap_peers.to_vec(),
        // Kademlia stays a client: this node answers no rpc_find/rpc_store for
        // anyone else. Together with never announcing, that is what makes it
        // an observer.
        dht_server: false,
        ..NetworkConfig::default()
    };

    let (handle, swarm) =
        NetworkService::spawn(config, keypair).context("starting the libp2p swarm")?;
    info!("observer peer id: {peer_id}");

    let addrs: Vec<Multiaddr> = bootstrap_peers
        .iter()
        .filter_map(|p| p.parse().ok())
        .collect();

    // Succeeds if any one peer was reachable. A total failure is not fatal:
    // the crawler retries every pass, and a map that starts before its
    // bootstraps is better than one that refuses to.
    if let Err(e) = handle.bootstrap(addrs).await {
        warn!("initial bootstrap failed, will retry each crawl: {e}");
    }

    Ok(Observer { handle, swarm })
}
