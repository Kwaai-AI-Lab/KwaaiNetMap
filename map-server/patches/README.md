# patches/

Build-time patched dependencies. Nothing here is vendored source: only the
patch files and checksum-pinned fetch scripts are committed; the expanded
crates are produced locally and gitignored.

`fetch-patches.sh` is the single entry point — it runs every per-crate fetch
script and is what the Dockerfile and CI call.

## libp2p-kad (peerstore answers, temporary)

`libp2p-kad 0.48.0` with KwaaiNet's `core/patches/libp2p-kad.patch` at
v0.7.0, byte for byte. It is here because kwaai-p2p 0.7.0 calls
`Behaviour::set_peerstore_addresses`, which only that patch defines — the
crate's own publish failed on exactly that line, so 0.7.0 exists as a git tag
and not on crates.io, and this manifest takes it from the tag. `[patch.crates-io]`
never crosses into a dependency, so the patch has to be repeated here, as
multistream-select is.

Temporary: KwaaiNet#216 drops the peerstore call, at which point kwaai-p2p
publishes again, the dependency goes back to a crates.io version, and this
patch and its fetch script go with it. It had already been removed once —
the `set_protocol_names` setter it also restores sits behind kwaai-p2p's
`kad-multi-protocol` feature, which the map does not take.

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
bash map-server/patches/fetch-patches.sh
```

`Dockerfile.map-server` and the CI workflow run it automatically. The script
pins its crates.io tarball by sha256 and is a no-op once the source is present
and matches the patch.

### Keeping it in step

If KwaaiNet's copy changes, copy it over — the two must not diverge, since they
have to negotiate with each other.
