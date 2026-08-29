# patches/

Build-time patched dependencies. Nothing here is vendored source: only the
patch file and a checksum-pinned fetch script are committed; the expanded crate
is produced locally and gitignored.

## multistream-select (slash-less protocol IDs)

`multistream-select 0.13.0` (from rust-libp2p, MIT) with one behavioural
change, applied via `[patch.crates-io]` in `map-server/Cargo.toml`.

Upstream rejects any protocol name that does not start with `/`. The hivemind
network negotiates **bare handler names** — `DHTProtocol.rpc_find` — as libp2p
protocol IDs, which go-libp2p accepts: the restriction is local to rust-libp2p,
not part of the wire protocol. The patch relaxes validation to what the message
framing actually requires: non-empty UTF-8 with no newline.

**Without it this crawler cannot read the DHT at all.** Every `rpc_find` is
rejected dialer-side, before anything reaches the wire, as
`Transport error: A protocol (name) is invalid` — and because a crawl that
finds nothing looks exactly like a network with nothing in it, the map serves a
plausible empty document rather than failing. This is the same patch and the
same reason as KwaaiNet's `core/patches/`; it has to be repeated here because
`[patch.crates-io]` applies only to the root manifest of a build, and this
crate is built standalone rather than inside that workspace.

### Fresh checkout

Cargo cannot parse the manifest until the patched source exists:

```sh
bash map-server/patches/fetch-multistream-select.sh
```

`Dockerfile.map-server` and the CI workflow run it automatically. The script
pins the crates.io tarball by sha256 and is a no-op once the source is present
and matches the patch.

### Keeping it in step

If KwaaiNet's copy changes, copy it over — the two must not diverge, since they
have to negotiate with each other.
