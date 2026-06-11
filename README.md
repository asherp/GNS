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

This repository is a **lightweight resolver** with two layers:

1. **Pubkey resolution** — given `from` and `to` pubkeys, return the shortest
   follow-chain between them, with full provenance: for every hop, the **follow
   event id** and the **relays** it was observed on.
2. **Name resolution** — given a `from` pubkey and a GNS address such as
   `barbara@alex.michael.nostr`, walk the address's labels through follow lists
   and return the resolved pubkey (or alternate paths when ambiguous).
3. **Reverse-edge (follower) lookup** — given a pubkey, return the set (and
   count) of pubkeys that follow it: the inverse of the follow-graph above.
   Nostr follow lists are one-directional and have no native follower index;
   GNS already runs relay clients with attribution, so reconstructing reverse
   edges from kind-3 events is a natural extension (best-effort given relay
   coverage).

Both resolution layers run as a breadth-first walk over **kind-3 contact
lists**, using only existing Nostr events. No new NIP is required.

## GNS names

A Nostr profile is eligible for GNS resolution only if its `name` (from the
kind-0 event) **normalizes to a non-empty label**:

1. take the profile's `name`,
2. lowercase it,
3. keep only ASCII letters `a-z` and digits `0-9`,
4. the result is the label; if empty, the profile has no valid GNS label.

```text
"Michael"        -> "michael"
"Michael Saylor" -> "michaelsaylor"
"MICHAEL-SAYLOR" -> "michaelsaylor"
"⚡Michael⚡"     -> "michael"
"FiatJaf"        -> "fiatjaf"
```

**Membership.** A label `x` belongs to namespace `y` iff `y`'s kind-3 follow
list contains a pubkey whose normalized label is `x`.

**Ambiguity.** If multiple followed pubkeys in a namespace normalize to the same
label, that label is **ambiguous and must not resolve**. The resolver returns
the alternate paths (each fully labelled) so a client can disambiguate.

**Address walk.** `barbara@alex.michael.nostr` reads right-to-left from the
resolving namespace — walk labels `[michael, alex, barbara]`: find `michael`
among your follows, `alex` among michael's follows, `barbara` among alex's.
A trailing `.nostr` namespace TLD is optional; a bare `barbara` is a compressed
direct lookup.

**Renames, spoofing, ordering, migration & weighted resolution.** How GNS
treats names over time — why renames are clean and ungated (identity is the
pubkey, not the name), how the follow requirement and ambiguity rule defeat
spoofing, the "silent swap" gap and a TOFU continuity memo to detect it, how key
migration works (followers re-point their follows; a signed old↔new attestation
keeps it safe), how resolution generalizes to a weighted best-path search
(conflict = weight 0, with mutual-follow and selectivity weighting), the role of
hash chains / OpenTimestamps for trustless cross-key seniority, how **NIP-05
becomes one optional attestation weight rather than the root of identity** (the
stack is *pubkey → signed name → social graph → NIP-05 → OTS/Bitcoin*), and why
the costs land on **discovery rather than name registration** — is written up in
[`docs/naming.md`](docs/naming.md).

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
| `src/graph/resolver.rs` | Breadth-first shortest-path resolution (pubkey → pubkey) |
| `src/graph/name.rs` | Label normalization + namespace membership / ambiguity |
| `src/graph/address.rs` | GNS address parsing (`barbara@alex.michael.nostr`) |
| `src/graph/name_resolver.rs` | Walk an address's labels through the graph |
| `src/api.rs` | HTTP API (`/api/resolve`, `/api/resolve-name`, `/api/followers`, `/api/normalize`, …) |
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

### `GET /api/resolve-name?from=<npub|hex>&name=<address>`

Resolves a GNS address from the caller's namespace.

```jsonc
{
  "query": "barbara@alex.michael.nostr",
  "from": "npub1…",
  "target_label": "barbara",
  "walk_labels": ["michael", "alex", "barbara"],
  "found": true,
  "ambiguous": false,
  "resolved": "npub1…",            // present only when unambiguous
  "paths": [
    { "nodes": [ { "npub": "…", "label": "you", "profile": {…} }, … ],
      "edges": [ { "follow_event_id": "…", "relays": ["wss://…"] }, … ] }
  ]
}
```

When a label is ambiguous, `resolved` is omitted, `ambiguous` is `true`, and
`paths` contains every alternate route with a `note` explaining why.

### `GET /api/followers?pubkey=<npub|hex>&limit=<n>&offset=<n>`

The **reverse edge** of resolution: given a pubkey, return the pubkeys observed
**following** it — every kind-3 list that carries it in a `p` tag. This is the
inverse of the follow-graph GNS already walks, filling the gap left by
nostr.band as a follower index.

```jsonc
{
  "pubkey": "04…",
  "npub":   "npub1…",
  "count":  3,                  // total followers known (pre-pagination)
  "limit":  50,
  "offset": 0,
  "followers": [
    { "npub": "npub1…", "pubkey": "03…",
      "follow_event_id": "…",   // the follower's kind-3 event
      "relays": ["wss://relay.damus.io"],
      "created_at": 1700000000 }
  ],
  "best_effort": true
}
```

* `followers` — newest-first, one entry per distinct follower (their newest
  kind-3 event wins), each with the same relay-attribution/provenance GNS tracks
  for forward edges, so a caller can show *how* a follower is connected.
* `count` — total followers known before pagination; page with `limit`
  (default 50, max 500) and `offset`.
* **Best-effort.** Results are reconstructed from whatever kind-3 events the
  configured relays return for a `{kinds:[3], #p:[pubkey]}` query, so the set is
  a lower bound (a census, not the truth) and is eventually-consistent: coverage
  depends on relay reach, and a follower who has unfollowed may linger until
  their newest list propagates. Served from the same TTL freshness cache as the
  rest of GNS.

### `GET /api/normalize?name=<string>`

Returns the normalized GNS label for a name and whether it is valid (non-empty):
`{ "name": "⚡Michael⚡", "label": "michael", "valid": true }`.

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
follower_query_limit = 500  # max kind-3 events per relay for /api/followers
static_dir = "static"
```

## Dashboard

The dashboard (`static/`) is dependency-free (vanilla JS + `<canvas>`). It has
two modes — **By name** (resolve a GNS address) and **By pubkey** (shortest
chain between two keys). It draws the resolved path as **avatar bubbles**
connected by directed edges, shows each node's GNS label, and lists per-hop
follow event ids and relays. Ambiguous names show every alternate path.

## Tests

```bash
cargo test
```

Covers npub round-tripping (NIP-19 vector), NIP-01 id computation + schnorr
verification (genuine sign/verify round-trip, plus tamper detection), `p`-tag
extraction, BFS resolution (shortest path, zero-hop, unreachable), label
normalization (all spec examples), namespace membership + ambiguity, GNS address
parsing, and named-path resolution (unique, single-hop, ambiguous, not-found).

## Status & roadmap

This is a working prototype of the core resolver and the name layer
(normalization, membership, ambiguity, address resolution). Natural next steps
the architecture already accommodates:

* **Alternate shortest paths for pubkey resolution** — the pubkey resolver
  returns one shortest path; named resolution already returns alternates on
  ambiguity, and the same branching extends to the BFS resolver.
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
