# KwaaiNetMap

The KwaaiNet network map — a web service that crawls the KwaaiNet DHT and publishes what it finds,
both as a rendered page and as JSON.

The rendered page has three views of the same data: a **map view** plotting nodes geographically,
a **table view** listing every node with its blocks, throughput and reachability, and a **network
topology** view of how peers connect. The JSON is served at `GET /api/v1/state`, which is the
endpoint a KwaaiNet node's health monitor polls to learn whether it is visible to the rest of the
network.

## How it works

The service builds on [health.petals.dev](https://github.com/petals-infra/health.petals.dev), the
Petals swarm health monitor, cloned at image build time and adapted for KwaaiNet. On top of the
upstream app it carries:

- `patches/` — a quilt-style series applied to the upstream clone at build time, in `series` order.
  Each is applied with `git apply` and no fuzz, so a patch that stops matching upstream fails the
  build loudly rather than being skipped silently.
- `p2p_utils.py` — reachability probing and geolocation, replacing the upstream module.
- `kwaainet.html` — the KwaaiNet UI, copied over the upstream template. It is a single static file
  with no build step; it fetches `api/v1/state` client-side and renders all three views from it.
- `entrypoint.sh` — generates `config.py` from `$INITIAL_PEERS`, then serves the app under gunicorn.

A background updater refreshes the crawled state every `UPDATE_PERIOD` seconds (60 by default).

## Running it

```bash
docker compose -f docker/kwaainet_health/docker-compose.yml build
INITIAL_PEERS="/dns/…/tcp/8000/p2p/Qm…,/dns/…/tcp/8000/p2p/Qm…" \
  docker compose -f docker/kwaainet_health/docker-compose.yml up
```

The service listens on port 8000. `INITIAL_PEERS` is a comma-separated list of bootstrap
multiaddrs; the image ships with the public KwaaiNet bootstraps as a default.

Note the build clones upstream and installs Petals from source, so it is slow and the resulting
image is large.

## Endpoints

| Route | Returns |
|---|---|
| `GET /` | the rendered map/table/topology page |
| `GET /api/v1/state` | crawled network state as JSON — bootstrap reachability, per-model server rows, top contributors, reachability issues |
| `GET /api/v1/is_reachable/<peer_id>` | live reachability probe for one peer |
| `GET /metrics`, `GET /api/prometheus` | Prometheus metrics |

## Layout

| Path | Purpose |
|---|---|
| `docker/kwaainet_health/DockerFile` | image build; build context is the repo root |
| `docker/kwaainet_health/patches/` | patch series applied to the upstream clone |
| `docker/kwaainet_health/kwaainet.html` | the UI |
| `docker/kwaainet_health/p2p_utils.py` | reachability and geolocation |
| `docker/kwaainet_health/entrypoint.sh` | config generation and gunicorn launch |
| `docker/kwaainet_health/check_identity.py` | standalone utility for deriving a libp2p PeerID from an RSA key; not used by the image |

## History

This service previously lived at `docker/kwaainet_health/` in
[OpenAI-Petal](https://github.com/Kwaai-AI-Lab/OpenAI-Petal). Its history moved here intact, so the
directory layout is unchanged from that repo.

## Licence

CC0 1.0 Universal — see [LICENSE](LICENSE). The upstream `health.petals.dev` source is fetched at
build time and is not redistributed here.
