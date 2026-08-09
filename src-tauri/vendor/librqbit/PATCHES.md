# Local patches to librqbit 8.1.1

This is a vendored copy of the crates.io release of `librqbit` 8.1.1, with the
changes listed below. Everything else is byte-identical to upstream.

Re-vendoring from scratch: copy the crate out of the cargo registry
(`~/.cargo/registry/src/*/librqbit-8.1.1`), delete `.cargo-ok`, then re-apply
these three edits. Each is marked in the source with a `LOCAL PATCH:` comment,
so `grep -rn "LOCAL PATCH" src/` finds all of them.

## Why

The status bar needs qBittorrent's three-state connection indicator:
**Online / Firewalled / Offline**. "Firewalled" means the client is working but
has never received an *inbound* peer connection, which indicates the listen port
isn't reachable from the internet. qBittorrent gets this from libtorrent's
`session_status::has_incoming_connections`.

librqbit already tracks inbound peers internally, but only in
`PeerStates::live_outgoing_peers`, and both `PeerStates` and
`TorrentStateLive::peers` are private to the crate. So there is no way to
derive it from the public API — hence the patch.

## Patch set A — incoming connection counter

1. **`src/session_stats/atomic.rs`** — added
   `pub incoming_connections: AtomicU64` to `AtomicSessionStats`.

2. **`src/session_stats/snapshot.rs`** — added
   `pub incoming_connections: u64` to `SessionStatsSnapshot` and populated it
   from the atomic counter.

3. **`src/session.rs`** (`task_tcp_listener`) — increment the counter when
   `live.add_incoming_peer(checked)` returns `Ok`. Deliberately counts
   *successful peer handoffs* rather than raw `TcpListener::accept` calls, so
   port scanners and failed handshakes don't produce a false "reachable".

Additive only. Dropping this patch costs the firewalled state, nothing else.

## Patch set B — bind to a single network interface

`SessionOptions::bind_ip: Option<IpAddr>` pins every socket to one local
address, so a VPN user's traffic cannot fall back to their real interface.
`None` reproduces upstream behaviour exactly.

**This only works because all five egress paths are covered.** Binding four of
five is worse than binding none: the UI would claim protection while one socket
quietly announces the real IP. If you re-vendor, verify every one of these.

| # | Path | File | Upstream behaviour |
|---|------|------|--------------------|
| 1 | Inbound listener | `src/session.rs` `create_tcp_listener` | hardcoded `("0.0.0.0", port)` |
| 2 | Outbound peers | `src/stream_connect.rs` | `TcpStream::connect`, OS picks source |
| 3 | DHT (UDP) | `src/session.rs` DHT init | `listen_addr` never plumbed |
| 4 | HTTP trackers | `src/session.rs` reqwest builder | no `local_address` |
| 5 | UDP trackers | **`librqbit-tracker-comms`** | hardcoded `0.0.0.0:0` |

Path 2 builds the socket via `tokio::net::TcpSocket` so the source address is
bound *before* connecting, and errors on an IPv4/IPv6 family mismatch rather
than silently falling back to an unbound socket.

Path 5 lives in a second crate, vendored alongside this one at
`../librqbit-tracker-comms`, with `UdpTrackerClient::new` taking an extra
`bind_ip` argument. `Cargo.toml` points at it by path.

## Patch set C — output folder accessor

`ManagedTorrent::output_folder()`. `shared.options` is `pub(crate)`, so "open
containing folder" is otherwise impossible from outside the crate.

## Patch set D — force recheck

`Session::force_recheck(&handle)` plus `ManagedTorrent::reset_to_initializing()`.
Every torrent client offers this; upstream has no public equivalent.

It works by dropping the torrent back to `Initializing` and letting the normal
`start()` path run `check()`. The important detail is the `previously_errored`
flag passed to `TorrentStateInitializing::new`: when true, `check()` **clears
the stored fastresume bitfield** before validating, so it can't skip hashing.
That flag is named for its only upstream caller (the error-recovery branch), but
its behaviour is exactly "force a full recheck".

`reset_to_initializing` tears down the running state *before* recreating the
storage — on Windows the old state's open file handles would otherwise block
reopening them.

## Patch set E — force reannounce

`Session::force_reannounce(&handle)`, plus `ManagedTorrentShared::reannounce`
(an `Arc<Notify>`) and matching changes in **librqbit-tracker-comms**.

Upstream's announce loops end in a bare `tokio::time::sleep(interval).await`,
so there is nothing to interrupt. The patch adds
`TrackerComms::sleep_or_reannounce`, which `select!`s the sleep against the
Notify, and replaces all three sleep sites (HTTP interval, HTTP error backoff,
UDP interval). `TrackerComms::start` takes the Notify; `Session::make_peer_rx`
threads it from `ManagedTorrentShared`.

Best-effort by design: `notify_waiters()` only wakes loops currently sleeping.
A tracker mid-request misses the poke and announces on its normal schedule,
which is the correct outcome anyway.

The magnet-resolution call site in `add_torrent` passes a throwaway Notify —
no `ManagedTorrent` exists at that point.

## Patch set F — peer detail

`PeerStats` gains `peer_id`, `progress`, `interested` and `inflight`, populated
from `LivePeerState` when the peer is live.

All of it was already tracked and simply never surfaced — `LivePeerState::peer_id`
was even annotated `#[allow(dead_code)]`. Without this a peer list can show
only an address and a state name, which is not worth having.

- `peer_id` is hex; decode with `librqbit_core::peer_id::try_decode_peer_id`
  (upstream, unused) to get a client name like "qBittorrent 4.6.0".
- `progress` is `bitfield.count_ones() / bitfield.len()` — how much of the
  torrent that peer holds.
- `LivePeerState::peer_id_hex()` was added because the field is private.

Per-peer **speed** is deliberately not patched in: librqbit only counts
cumulative `fetched_bytes`, and the app derives a rate by sampling that across
its one-second polls.

### Known trade-off

Setting `bind_ip` disables **DHT routing-table persistence**.
`PersistentDht::create` accepts no listen address (`PersistentDhtConfig` only
carries `dump_interval` / `config_filename`), so binding the DHT socket requires
the non-persistent `DhtBuilder::with_config` path. Cost: slower DHT bootstrap
after a restart. Fixing this properly means vendoring `librqbit-dht` as well.

## Upstreaming

This is worth sending to https://github.com/ikatson/rqbit — it's a small
additive change and the "am I firewalled" question is common to every torrent
client. If it lands upstream, delete `vendor/` and point `librqbit` back at
crates.io.
