# Map Server v2 — Requirements & Design

> **Status:** Design phase
> **Replaces:** OpenAI-Petal `docker/kwaainet_health/` (Python/Flask + vanilla JS + Leaflet)
> **Current KwaaiNet stub:** `core/crates/map-server/` (Axum, functional but minimal)
> **Target URL:** `map.kwaai.ai`

---

## 1. Why Rewrite?

The existing map (health.petals.dev fork) is a maintenance-mode Python service with a static HTML page. It:

- Polls every 60 s — zero live feel
- Shows 2D pin-drop nodes on a world map — no network topology
- Has no trust tier visualisation
- Treats every node identically regardless of throughput, VPK, or VC status
- Exposes no self-serve benchmarking or install funnel
- Cannot sustain the KwaaiNet brand as the network grows toward thousands of nodes

The new map must feel like **mission control for a living network** — compelling enough that a visitor who has never heard of KwaaiNet installs a node within minutes.

---

## 2. Goals

| Goal | Metric |
|------|--------|
| Visceral "live" feeling | Node updates pushed <2 s after DHT change |
| Funnel conversion | Visitor → node install CTA click |
| Trust transparency | Every node shows tier + VC count at a glance |
| Performance | First contentful paint <1.5 s on 4G |
| Scalability | Handles 10 000 nodes without frame drops |
| Privacy | No visitor PII collected; node operators opt-in to public name |

---

## 3. Audience Personas

**Explorer** — Found KwaaiNet via social/blog; curious but not yet sold. Needs a "wow" moment within 10 seconds.

**Prospective Node Operator** — Has spare GPU/storage; wants to know if it's worth running. Needs earnings estimate + proof the network is real.

**Researcher / Journalist** — Writing about decentralised AI. Needs sharp stats and a quotable headline number.

**Active Node Operator** — Checks the map to confirm their node is visible. Needs a fast "find my node" search.

---

## 4. Functional Requirements

### 4.1 Live Network Globe

- **3D globe** (Three.js + globe.gl or CesiumJS lite) as the hero element — rotating, glowing, with node arcs
- Nodes rendered as luminous dots, sized by throughput, coloured by trust tier
- **Inference arcs**: animated geodesic lines connecting coordinator → shard nodes during active sessions
- Nodes pulse on DHT activity (new announcement or throughput change)
- Smooth flyTo animation when a user searches for a node
- Falls back to flat Mercator canvas for browsers without WebGL

**Trust tier colour coding** (consistent with KwaaiNet design tokens):

| Tier | Colour | Condition |
|------|--------|-----------|
| Unknown | `#EF4444` red | 0 VCs |
| Known | `#F59E0B` amber | 1–2 VCs |
| Verified | `#8B5CF6` purple | 3–4 VCs |
| Trusted | `#10B981` green | 5+ VCs |

### 4.2 Hero Stats Bar

Real-time animated counters (smooth lerp, not jump):

- Active nodes
- Tokens / second (network aggregate)
- Block coverage % (0–80 colour-coded by gap severity)
- Active inference sessions
- Total VPK storage pledged (GB)

### 4.3 Node Detail Panel

Click any globe node → side panel slides in:

```
┌─────────────────────────────────────────────┐
│  metro@kwaai                     ● Trusted   │
│  Peer: QmAbc...5bZ2                          │
│                                              │
│  Blocks  16 – 19  (4 blocks)                │
│  Throughput  ~24 tok/s                       │
│  Version  v0.3.23                            │
│  VPK  500 GB pledged                         │
│  VCs  PeerEndorsementVC × 3                 │
│  First seen  14 days ago                     │
│  Last seen   2 s ago                         │
│                                              │
│  Block coverage bar ████░░░░░░░░  16–19/80  │
│  24 h throughput sparkline   ~~~∧~~~         │
│                                              │
│  [  Endorse this node  ]                    │
└─────────────────────────────────────────────┘
```

### 4.4 Coverage Heatmap

- Horizontal bar below the stats chips showing blocks 0–79
- Green = ≥2 nodes cover this block, amber = 1 node, red = uncovered gap
- Animates as nodes join/leave
- Clicking a block highlights all nodes serving it on the globe

### 4.5 Benchmark + Earnings Estimator

Identical to current BenchmarkSection (WebGPU/CPU GEMM) with improvements:

- **WebGL GEMM fallback** (twgl.js) for Safari/Firefox — currently missing
- **Persist results** in `localStorage` — re-running shows delta vs prior result
- **Share button** — encodes tps/storage in URL params, no server needed
- **Calibrated conversion factor** — hardware-matched against known GPUs (M2, RTX 4090, A100) using empirical data from opted-in operators
- Storage: show `navigator.storage.estimate().quota` (browser quota) vs actual disk quota, explain the 60% cap

### 4.6 Trust Graph

D3 force-directed graph (existing `TrustGraphSection`), enhanced:

- **Real endorsement edges** — decode `PeerEndorsementVC` subject/issuer DIDs from `/api/nodes`, draw directed arrows
- **Tier filter** — checkbox row for Unknown/Known/Verified/Trusted
- **Node size** proportional to throughput
- **Animated edge routing** — edges pulse when an endorsement VC was issued in the last 24 h
- Collapsible "How trust works" explainer (ToIP 4-layer stack)

### 4.7 Install Funnel

- Detect platform via `navigator.userAgentData.platform`
- Fetch latest release tag from GitHub API (`/repos/Kwaai-AI-Lab/KwaaiNet/releases/latest`) — inject real version, not hardcoded `main`
- **Gamification state machine** (localStorage): `teaser` → `benchmarked` → `installed` → `node_live`
  - Achievement chips animate in at each transition
  - `node_live`: detected by polling `/api/nodes` for a peer ID stored post-install
- **Windows PowerShell copy** — fixes current bug where curl command is copied regardless of tab
- Step 3 "Your node appears on the map" links to the globe pre-filtered to that node's location

### 4.8 Operator Search

- Search bar: accepts `peer_id`, `public_name`, or IP fragment
- Results highlight matching node on globe (flyTo + pulse)
- URL: `map.kwaai.ai/?node=QmAbc...` — shareable deep link

---

## 5. Non-Functional Requirements

### 5.1 Performance

| Constraint | Target |
|-----------|--------|
| Time to interactive | < 2 s on 10 Mbps |
| Bundle size (gzip) | < 300 KB JS, < 50 KB CSS |
| Globe framerate | ≥ 60 fps @ 10 000 nodes |
| WebSocket reconnect | Automatic, ≤ 3 s |
| API response | `/api/nodes` < 100 ms at p99 |

- Three.js globe rendered in an offscreen canvas, transferred via `OffscreenCanvas` to keep main thread free
- Node data delivered as compact binary (MessagePack over WebSocket) not JSON
- Virtual DOM diffing only for the side panel; globe is imperative (Three.js scene graph)

### 5.2 Accessibility

- WCAG 2.1 AA contrast on all text
- `aria-label` on every icon-only button
- Keyboard navigation for globe (arrow keys pan, +/- zoom)
- Reduced motion: disable arc animations and globe rotation when `prefers-reduced-motion: reduce`

### 5.3 Scalability

- The backend must handle 10 000 nodes without degradation
- `/api/nodes` response cached as pre-serialised bytes for 5 s; only one goroutine reserialises at a time
- WebSocket fan-out uses a single broadcast channel — no per-connection cache
- Globe culls off-screen nodes below zoom threshold (LOD)

---

## 6. Backend API (v2)

### New endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/nodes` | Full node list (JSON or MessagePack via `Accept` header) |
| `GET` | `/api/nodes/:peer_id` | Single node detail, uptime history, VC list |
| `GET` | `/api/coverage` | Block 0–79 coverage bitmap as `u8[80]` (count per block) |
| `GET` | `/api/stats` | Aggregate stats snapshot |
| `WS` | `/api/live` | Push diffs every 2 s (nodes added/removed/changed) |
| `POST` | `/api/v1/state` | Heartbeat ping from running nodes — upsert into cache immediately |
| `GET` | `/api/trust/:did` | TRQP-lite: resolve trust tier for a DID (v3) |

### WebSocket diff protocol

Instead of pushing the full node list every 5 s, push a compact diff:

```json
{
  "ts": 1733961600,
  "add": [{ "peer_id": "Qm...", "trust_tier": "Trusted", ... }],
  "update": [{ "peer_id": "Qm...", "throughput": 31.2 }],
  "remove": ["Qm...oldpeer"]
}
```

This reduces bandwidth by ~95% for established connections.

### Persistence (v2)

SQLite via `rusqlite` for:
- Node cache across restarts (no cold-start blank map)
- 24 h throughput history per node (for sparklines)
- Active session log (inference arcs)

---

## 7. Frontend Technology Stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| Framework | React 18 + Vite | Already in use; fast HMR |
| Globe | `globe.gl` (Three.js wrapper) | Simplest API for animated arcs + dots at scale |
| Trust graph | D3 v7 force simulation | Fine-grained control |
| WebSocket | Native + reconnect wrapper | No lib needed |
| Benchmark worker | WebGPU → WebGL → CPU cascade | Already implemented |
| State | Zustand | Minimal, no boilerplate |
| Styling | Tailwind CSS | Already in use |
| Animations | Framer Motion (UI) + GSAP (globe) | Separate layers for perf |
| Charts | Recharts (sparklines) | Lightweight |

---

## 8. Design Language

Extend current tokens:

```css
/* Existing */
--bg-deep:    #0A0F1E;
--bg-card:    #0F1F3D;
--blue:       #3B82F6;
--green:      #10B981;
--purple:     #8B5CF6;
--amber:      #F59E0B;
--red:        #EF4444;

/* New for globe */
--globe-ocean:   #0D1B2A;
--globe-land:    #1A2F4A;
--globe-border:  #1E3A5F;
--arc-active:    rgba(59,130,246,0.6);
--arc-inference: rgba(16,185,129,0.8);
--node-pulse:    rgba(255,255,255,0.15);
```

Globe atmosphere: subtle blue-white rim glow (`THREE.Sprite` with additive blending). Stars: static particle field (`THREE.Points`, 2000 vertices). Day/night terminator line (optional — based on `Date.now()` and latitude).

---

## 9. Implementation Phases

### Phase 1 — Backend hardening (2 weeks)
1. WebSocket diff protocol (replace full-snapshot push)
2. `/api/nodes/:peer_id` endpoint
3. `/api/coverage` bitmap endpoint
4. `POST /api/v1/state` heartbeat ingest
5. SQLite persistence for cache + history
6. Rate-limit `/api/nodes` (5 s cache, `tower` middleware)
7. CORS lockdown to `https://map.kwaai.ai`

### Phase 2 — Globe hero (2 weeks)
1. Replace NetworkGraph flat canvas with `globe.gl` 3D globe
2. Node dots: size = throughput, colour = trust tier
3. Inference arc animation (synthesised from active sessions)
4. Coverage heatmap bar (from `/api/coverage`)
5. Node detail side panel
6. Operator search + deep-link URL

### Phase 3 — Engagement features (1 week)
1. Trust graph: real VC endorsement edges
2. Benchmark: WebGL fallback, localStorage persist, share URL
3. Install funnel: GitHub Release API version, gamification state machine
4. WebSocket reconnect indicator in hero bar

### Phase 4 — Polish & accessibility (1 week)
1. Framer Motion entrance animations for panels
2. Reduced-motion support
3. WCAG audit + aria-labels
4. Lighthouse performance pass (target score ≥ 90)
5. Mobile responsive nav + hamburger menu
6. Favicon (Kwaai tree SVG)

---

## 10. Success Metrics

| Metric | Baseline (current) | Target |
|--------|-------------------|--------|
| Bounce rate | Unknown | < 40% |
| Time on page | ~20 s | > 90 s |
| Node install CTA clicks | Not tracked | Track via `data-event` |
| Lighthouse perf | Not measured | ≥ 90 |
| WebSocket uptime | N/A | ≥ 99.9% |
| `/api/nodes` p99 latency | ~300 ms | < 100 ms |

---

## 11. Out of Scope (v2 → v3)

- Passkey/WebAuthn operator auth
- Earnings ledger / payment integration
- TRQP full trust registry query
- EigenTrust propagation graph
- Mobile app
