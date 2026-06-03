# GNS — Graph Name System

A decentralized naming and discovery layer built on the **Nostr social graph**.
Instead of mapping names to servers (DNS) or directly to public keys (NIP-05),
GNS resolves identities through *graph relationships*:

```
DNS:     name -> server
NIP-05:  name -> pubkey
GNS:     (name, graph position) -> pubkey
```

The scarce resource is not the name — it's your **position in the graph**. The
same person is reachable through many paths, and each path is *provenance*: it
tells you *how* you discovered them.

```
barbara@alex.michael.nostr
   │       │       │
   │       │       └─ graph path (you → michael → alex)
   │       └───────── walked via follow lists
   └───────────────── target name
```

See the [full vision](#vision) below.

## What this server does

This repository is a **lightweight resolver**: a caller provides two pubkeys and
the server returns the shortest follow-chain between them, with full provenance.

> Given `from` and `to` pubkeys, return the sequence of npubs connecting them,
> plus — for every hop — the **follow event id** and the **relays** it was
> observed on.

Resolution is a breadth-first walk over **kind-3 contact lists**, using only
existing Nostr events. No new NIP is required.

## Architecture

```
Nostr Relays  ──►  GNS Indexer (this server)  ──►  Clients / Dashboard
 (source of truth)   graph traversal + caching       verify against signed events
```

| Module | Responsibility |
| --- | --- |
| `src/nostr/key.rs` | `npub` ⇄ hex (NIP-19 bech32) |
| `src/nostr/event.rs` | Event model, NIP-01 id computation, schnorr verification |
| `src/nostr/client.rs` | Minimal per-relay websocket client (for relay attribution) |
| `src/graph/relay_source.rs` | Fetch follow lists / profiles, fanning out across relays |
| `src/graph/cache.rs` | TTL "freshness window" cache (the public-good dynamic) |
| `src/graph/resolver.rs` | Breadth-first shortest-path resolution |
| `src/api.rs` | HTTP API (`/api/resolve`, `/api/profile`, `/api/config`) |
| `static/` | Self-contained dashboard with avatar-bubble graph |

The relay client is implemented directly on websockets (rather than via a higher
level SDK) specifically so each follow event can be attributed to the relays
that served it — a core requirement.

## Quick start

### Demo mode (no network)

Runs against a built-in fixture graph (`You → Michael → Alex → Barbara`, plus
alternate routes via Carol/Dave), so you can see resolution and the dashboard
without any relay access:

```bash
cargo run -- --demo
# open http://127.0.0.1:8080/  and click "Load demo example" → Resolve
```

### Live mode

Edit `config.toml` to set your relays, then:

```bash
cargo run --release
# open http://127.0.0.1:8080/
```

## HTTP API

### `GET /api/resolve?from=<npub|hex>&to=<npub|hex>&max_depth=<n>`

```jsonc
{
  "from": "npub1…",
  "to":   "npub1…",
  "found": true,
  "hops": 2,
  "path": [
    { "npub": "npub1…", "pubkey": "01…", "profile": { "name": "You", "picture": null } },
    { "npub": "npub1…", "pubkey": "05…", "profile": { "name": "Carol" } },
    { "npub": "npub1…", "pubkey": "04…", "profile": { "name": "Barbara" } }
  ],
  "edges": [
    { "from": "npub1…", "to": "npub1…", "follow_event_id": "…", "relays": ["wss://relay.damus.io"] },
    { "from": "npub1…", "to": "npub1…", "follow_event_id": "…", "relays": ["wss://nos.lol"] }
  ],
  "visited": 6
}
```

* `path` — npubs ordered `from → to`, each with kind-0 profile (name + avatar).
* `edges` — one per hop; `follow_event_id` is the **follower's** kind-3 event,
  `relays` are where that event was observed.

### `GET /api/profile?pubkey=<npub|hex>`

Returns kind-0 metadata (name, display name, picture, nip05, about).

### `GET /api/config`

Reports demo mode, configured relays, and (in demo mode) example pubkeys.

### `GET /healthz`

Liveness probe.

## Configuration

`config.toml` (CLI flags `--config`, `--demo`, `--bind` override it):

```toml
bind = "127.0.0.1:8080"
relays = ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.nostr.band"]
relay_timeout_ms = 5000
cache_ttl_secs = 300        # freshness window
cache_capacity = 100000
verify_signatures = true    # drop events that fail schnorr verification
max_depth = 6
static_dir = "static"
```

## Dashboard

The dashboard (`static/`) is dependency-free (vanilla JS + `<canvas>`): it draws
the resolved path as **avatar bubbles** connected by directed edges, renders the
derived GNS address (`barbara@alex.michael.nostr`), and lists per-hop follow
event ids and relays.

## Tests

```bash
cargo test
```

Covers npub round-tripping (NIP-19 vector), NIP-01 id computation + schnorr
verification (genuine sign/verify round-trip, plus tamper detection), `p`-tag
extraction, and BFS resolution (shortest path, zero-hop, and unreachable cases).

## Status & roadmap

This is a working prototype of the core resolver. Natural next steps that the
architecture already accommodates:

* **Multiple / alternate shortest paths** (path redundancy) — the resolver
  currently returns one shortest path; the `GraphSource` + BFS structure extends
  to returning alternates.
* **Path compression** — caching learned short-cuts (`You → Barbara`) as the
  graph is traversed.
* **Persistent index** — the in-memory cache can be backed by a store for warm
  starts and cross-process sharing.
* **Streaming freshness** — subscribe to relays for live kind-3 updates.

---

## Vision

GNS turns the Nostr social graph into a decentralized naming layer. Names are
interpreted *relative to a location in the graph*, so they need not be globally
unique:

```
john.company
john.friend
john.bitcoin     # all can coexist
```

Trust may emerge from the graph, but the primary purpose is **discovery and
naming** — a *Web of Names*, not a Web of Trust. Relays remain the source of
truth; indexers provide traversal, caching, freshness, and shortest-path
discovery; clients verify everything against signed Nostr events. If an indexer
disappears, another can rebuild entirely from relay data.

GNS fits naturally into NostrMail: a client resolves `barbara@alex.michael.nostr`
to a pubkey, encrypts to it, and delivers — the user never sees a public key.
