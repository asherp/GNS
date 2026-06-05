# GNS Naming: changes, spoofing, and ordering

This note captures the design reasoning behind how GNS treats names over time:
what a name *is*, what happens when someone renames, how spoofing is handled,
and what role (if any) timestamps and external anchors play. It is a design
rationale, not an implemented spec — the parts that are implemented today are
called out explicitly.

## TL;DR

- **Identity is the pubkey, not the name.** A follow points at a key, so a
  legitimate rename is inherently clean and needs no special machinery.
- **The follow requirement + the ambiguity rule already defeat the spoofs that
  matter.** Both are implemented today, and neither needs stored history.
- **`created_at` is self-reported and forgeable** — never use it as a security
  signal, only as a soft display hint.
- The only gap is the **silent swap** (a name changes hands between two keys).
  Catching it needs a tiny continuity memo, not profile history.
- For **trustless cross-key seniority** you need an external clock
  (OpenTimestamps / Bitcoin). That layer is optional and adversarial-only.
- **NIP-05 is one optional signal, not the root.** Identity is the key; NIP-05 is
  a DNS domain co-signing it — corroboration, weighted like any other edge, never
  a gate (§9). The cost model follows from this: you pay for *discovery*
  (traversal, freshness, proofs), not for *registering* names (§10).

## 1. What a name is

A GNS label is derived from a profile's kind-0 `name`, normalized to
`[a-z0-9]` (see `src/graph/name.rs`). A label `x` belongs to namespace `y`
**iff `y`'s kind-3 follow list contains a pubkey whose normalized label is `x`**.
The scarce resource is not the name — anyone can call themselves "alice" — it is
the **follow edge**. You only appear in Bob's namespace if Bob follows you.

This single fact does most of the security work: a random attacker who names
themselves "alice" is simply not in `*.bob`.

## 2. Renames are clean because identity is the key

A follow references a **pubkey**, not a name. So when Alice renames herself
Alice → Carol:

- Bob still follows the same key, so his vouching carries over automatically —
  **no refollow required**.
- `carol.bob` resolves to her immediately (her current name is "carol").
- There is no question of "is Carol the same person as Alice" — it is literally
  the same key signing a new kind-0.

The common case therefore needs **nothing**: no gating, no history, no anchoring.

### Rejected alternative: gating renames

We considered a rule "a follow only confirms a name if
`kind0.created_at <= kind3.created_at`," so a post-follow rename would stop
resolving until the follower re-followed. We rejected it:

- It punishes the *legitimate* renamer (every follower must re-vouch).
- kind-3 carries a single, list-wide `created_at` that bumps on *any* list
  edit, so "refollow" can't be distinguished from "edited the list."
- `created_at` is forgeable, so the gate is bypassable anyway.

Gating solves a problem that key-based identity doesn't have.

## 3. Spoofing

"Spoof" can only mean: **an account Bob already follows renames itself to
collide with a name.** (Injection is impossible without a follow.) Three cases,
analyzed assuming we store no profile history and do not gate:

### A. Legit rename, no collision

```
t1 Alice kind-0 "Alice"
t2 Bob follows Alice
t3 Alice kind-0 "Carol"
```

- `carol.bob` → resolves to Alice (rename propagates; we don't gate). ✓
- `alice.bob` → not found (we don't keep the old name). Acceptable.
- Nothing to flag.

### B. Rename into a live collision (the real spoof) — handled today

```
Bob follows A ("Alice") and M ("Mallory"); then M renames to "Alice".
```

- `alice.bob` → A and M both normalize to "alice" → **ambiguous → does not
  resolve**, returns both candidates with provenance (npub, picture). The spoof
  is surfaced, not silently successful.
- This is the **ambiguity rule** (implemented in `resolve_label` /
  `resolve_address`). It needs **no timestamps and no history**.
- `created_at` could *hint* which profile adopted the name more recently, but M
  controls her own `created_at` and can backdate it — so treat it as a UI hint
  only, never a decision.

### C. The silent swap (the one gap)

```
t1 A "Alice"; Bob follows A
t2 A renames to "Carol"        (legit; A vacates "alice")
t3 M (Bob already follows) renames "Mallory" -> "Alice"
```

- `alice.bob` at t3 → only M is "alice" now → resolves to M, **unambiguous,
  looks clean.** The spoof succeeds, and with no history GNS can't know "alice"
  used to mean A.

## 4. Catching the swap: a continuity memo (TOFU), not history

Catching case C does **not** need old kind-0 events. It needs the indexer to
remember the *binding it last served*:

```
(namespace, label) -> last_seen_pubkey
```

When `alice.bob` resolves to a **different** pubkey than last time, flag
"this name changed hands" — **annotate, do not block** (matches "detect
spoofing, don't gate renames"). This is **TOFU** (Trust On First Use), the SSH
`known_hosts` model: pin what you first saw, alarm on change.

It cleanly separates legit from hostile:

- Legit rename (A: Alice→Carol): the label "alice" goes *not-found* and "carol"
  appears fresh — **same label / different key never happens → no flag.**
- Swap (M takes the "alice" A held): **same label, different key → flagged.**

Limits: TOFU only protects *after* a trusted first sighting (a first-ever lookup
has no prior), and the memo is per-indexer (indexers reconcile only after both
observe the swap).

Optionally, the same memo enables a graceful **old-name redirect**: `alice.bob`
→ "Alice → now Carol, npub…" instead of *not-found* — an HTTP-301 for names.

## 5. Why `created_at` can't be trusted, and what ordering is possible

`created_at` is set by the signer and is trivially forgeable, so it cannot
establish who held a name first. Stronger options:

### Per-author hash chains (references)

Each event can reference the previous one (`["e", <prev_id>]` or a custom
`["prev", …]` tag). Because an event's `id` hashes its content *including* that
reference, you get a tamper-evident **causal order** of one author's events —
like git commits. This kills self-backdating *within* Alice's own history.

Three catches:

1. **Partial order only.** A chain orders one author's events; it says nothing
   about Alice's rename relative to Bob's follow. Cross-author total order needs
   a shared anchor.
2. **Forkable.** Alice holds her key and can sign two successors to the same
   prev event (equivocation). Chains are tamper-*evident*, not fork-*proof*;
   detecting a fork requires observing both branches.
3. **Replaceable events don't retain the links.** kind-0/kind-3 are replaceable;
   relays keep only the latest, so prior chain links are discarded and a fresh
   resolver sees a dangling `prev`. Chaining over profiles requires either
   publishing changes as **non-replaceable** events or having the indexer
   **archive** each version it observes.

### External anchor: OpenTimestamps / Bitcoin

For **trustless cross-key seniority**, anchor an event id into Bitcoin via OTS:

```
event id ──► OTS calendar (free) ──► Merkle-batched ──► root committed to Bitcoin
         ──► small .ots proof: "this exact signed event existed before block N"
```

Key properties:

- **Permissionless.** OTS timestamps a *hash*; the event is already
  self-certifying (`id = sha256(serialization)`), so **the GNS server can anchor
  any event it sees without the author's cooperation.**
- **Composable.** Everything settles to one chain, so multiple GNS servers
  anchoring independently reconcile via Bitcoin — the earliest anchor for an id
  wins. No single trusted timestamper. (Fits the "public good" economics: anchors
  are portable across indexers.)
- **Defeats backdating.** Nobody can manufacture an *earlier* Bitcoin commitment,
  so once A's "Alice" is anchored at height H₁, M cannot fake seniority.
- **Makes equivocation provable.** Anchoring both forks gives Bitcoin-ordered
  evidence of two conflicting same-key events.

Caveats:

- **Upper bound, from first anchor on.** OTS proves "existed by T," not "didn't
  exist before." It orders events only from when someone first anchors them;
  the first sighting is still TOFU.
- **~hours latency.** Good for seniority/dispute resolution, not real-time.
- **You must keep what you anchor.** The proof binds a hash; if you discard the
  old kind-0 you have a proof for content you no longer hold. OTS pairs with
  archiving.
- **Only anchored events get the global order.**

## 6. The indexer as timestamping attestor (pragmatic middle)

Between "trust `created_at`" (insecure) and "anchor everything to Bitcoin"
(heavy), the indexer's own **observed-time log** is a lightweight timestamping
authority: it records the order it first saw events, which an attacker cannot
rewrite. Several indexers gossiping observations approximate a decentralized
timestamp without Bitcoin's latency — the cheap 80% of what OTS provides.

So the indexer can grow into: **resolver + archiver + (optional) OTS attestor.**

## 7. Key migration

Key migration (Alice moves from `K_old` to `K_new`, by rotation or after loss)
falls out of the same primitives. Crucially, **on the wire a migration looks
identical to the "silent swap" spoof of §3C** — `alice.bob` used to point at
`K_old`, now points at `K_new`. The whole design problem is letting the
legitimate migration through while still catching the hostile takeover.

### The resolution half is automatic (follows are votes)

Membership is "Bob follows a key named alice." So the moment Bob unfollows
`K_old` and follows `K_new` (named "alice"), `alice.bob` → `K_new`. No rename
operation, no registrar, no protocol step — **each follower re-pointing their
edge is a vote**, and the name tracks the namespace owner's own follows.

It is **gradual / eventually-consistent**, which is correct rather than a bug:
migrated followers resolve to `K_new`, un-migrated ones to `K_old`, and the graph
heals as people move. No flag day; each namespace honestly reflects what its
owner currently believes.

### The hard half is *authorizing* the move

"Resolves automatically once Bob follows `K_new`" just relocates the question to
*why* Bob would follow `K_new` and how he knows it is really Alice.

- **Planned rotation (`K_old` still controlled).** Alice publishes a **signed
  migration pointer** from `K_old`: "my successor is `K_new`." Clients verify it
  against the key they already trust (`K_old`) and offer one-click re-follow. The
  strong form is **bidirectional cross-signing**: `K_old` signs "successor =
  `K_new`" *and* `K_new` signs "predecessor = `K_old`," so both keys provably
  consent — defeating an attacker who controls only one. (NIP-26 delegated
  signing is a related primitive but addresses delegation, not succession; there
  is no finalized migration NIP at time of writing, so this is a small convention
  GNS would adopt or define.)
- **Compromise (`K_old` also held by an attacker).** Nothing `K_old` says can be
  trusted, so migration cannot be authorized cryptographically from `K_old`. It
  falls back to out-of-band re-establishment (other channels, or proving control
  of other identities Alice holds). GNS softens this — each follower decides
  individually, so there is no single binding to hijack — but it does **not**
  solve key compromise, and we should not pretend it does.

### Migrate *both* events

A migrating key must republish **kind-0 (name) and kind-3 (follows)** to
`K_new`. If Michael ports only his profile, `…@michael.nostr` reaches `K_new` but
`K_new` follows nobody, so everything *downstream* of Michael (`alex`, `barbara`)
breaks. Migration means carrying your outgoing namespace with you.

### How it plugs into the other layers

The signed migration attestation is exactly the discriminator the earlier layers
were missing:

1. **Migration vs silent-swap spoof.** The TOFU continuity memo (§4) flags
   `alice.bob` changing `K_old`→`K_new` as "⚠ changed hands." A verified
   `K_old`↔`K_new` link downgrades that to "✓ migrated (attested by prior key)";
   no link → it correctly stays a warning.
2. **Collapses the transient double-follow ambiguity.** Mid-migration, if Bob
   follows *both* keys and both are named "alice," the ambiguity rule (§3B) would
   refuse to resolve. If the pair is attested, the indexer treats them as one
   identity and **prefers the successor** instead of failing.
3. **Old-key redirect.** Symmetric to the old-*name* redirect: lookups landing on
   `K_old` can annotate "migrated to `K_new`," nudging stragglers to re-follow.
4. **OTS anchoring** (§5) gives the attestation a trustless timestamp, so an
   attacker cannot forge an *earlier* "migration" to retroactively hijack a name.

### Summary

The resolution is automatic and needs no new code — migration is just everyone
re-pointing their follow. What makes it *safe and smooth* is a
**(bi)directionally-signed migration attestation linking `K_old` and `K_new`**,
which lets clients re-follow with confidence, tells the spoofing layer the swap
is legitimate, collapses the double-follow ambiguity toward the successor, and
powers a straggler redirect — with key *compromise* as the residual hard case
the graph can only soften, not solve.

## 8. Weighted resolution

Everything above — shortest path, the ambiguity rule, "prefer the successor,"
"prefer mutual follows" — is a special case of one mechanism: **weighted
best-path search**.

### The algebra

Give each edge a weight `w ∈ [0,1]` (confidence that this hop is the correct
reading of the label). A path's score is the **product** of its edge weights,
and resolution picks the **maximum-score** path. Work in negative-log space —
`cost = -ln(w)`, minimize the **sum** — and it is plain **Dijkstra**, with the
useful boundary `w = 0 → cost = +∞ → path excluded`. One algorithm subsumes:

```
edge weight w        cost = -ln(w)        effect
1.0  (certain)        0                    free hop
δ < 1 (per-hop decay) -ln δ > 0            longer paths score lower → shortest-path preference
0    (name conflict)  +∞                   path dropped
```

Today's behaviour is exactly the special case **weights ∈ {0,1} with hop decay**:
unit weight + decay recovers shortest path; `Ambiguous ⇒ weight 0` recovers
"must not resolve." Nothing regresses. But per-*edge* zeroing also improves on
the current global failure: an ambiguous hop no longer fails the whole
resolution — a *different* unambiguous route to the same target survives and
wins, which is precisely the spec's "SHOULD return alternate paths even if they
are longer." (A *strict* named address with a forced ambiguous hop still
correctly dies — there is no other reading of that hop.)

### Edge weight = a product of independent factors

Keep the weight factored, so each signal is its own tunable knob:

```
w(A → B) = δ_hop  ×  recip(A,B)  ×  select(A)  ×  conflict  ×  (continuity, attestation, …)
              ↑          ↑             ↑            ↑
        per-hop decay  reciprocity   selectivity  0 if name conflict
```

- **`δ_hop` (< 1)** — distance decay, so a chain of strong follows is never
  "free" and closer identities win (this is also where *path compression* lives:
  learned shortcuts are simply closer/cheaper).
- **`conflict`** — `0` on an ambiguous label; kills the path.
- **`recip(A,B)`** — reciprocity (below).
- **`select(A)`** — selectivity (below).
- Later signals (TOFU continuity penalty, migration attestation restore,
  seniority) slot in as further multipliers without changing the search.

### Reciprocity: mutual follows beat one-way

A one-way follow is nearly free to manufacture (follow a celebrity, follow your
target); a follow-*back* needs the other party's cooperation. So mutual edges
carry more confidence: `recip = 1` if B also follows A, `r < 1` if one-way.

Choose `r` as a **"hops" question**, not a decimal. Because `-ln(m^k) =
k·(-ln m)`, setting

> **one-way weight = (mutual weight)^k**

makes a one-way hop cost exactly `k` mutual hops, *independent* of the decay:

| intent | k | example (mutual = 0.9) |
| --- | --- | --- |
| reachability — mild preference | 2 | one-way 0.81 |
| naming / discovery — clear preference | 3 | one-way 0.73 |
| high-assurance | 4–5 | one-way only as a last resort |

Default **k = 2** generally, **k = 3** for naming/discovery (where "is this the
alice *I* mean" deserves the bias). Avoid large `k` — it sends the search on
absurd mutual detours to dodge a single one-way edge.

Detecting mutuality costs the *reverse* edge: to weight `A → B` you need B's
kind-3 to check whether `A ∈ B.follows`. That roughly doubles contact-list
fetches, amortized by the freshness cache (and you often need B's list next
anyway).

### Selectivity: a follow from a choosy account counts more

A follow from someone who follows 50,000 accounts is weak evidence; a follow
from someone who follows 50 is strong. The follower's out-degree is just their
contact-list size, which we already have, so fold in an inverse-out-degree
factor, e.g.

```
select(A) = 1 / (1 + ln(out_degree(A)))
```

Reciprocity says "they follow back"; selectivity says "and they are choosy about
it" — together a decent proxy for a real relationship. (Tune the shape; `ln`
keeps it gentle so that following a few thousand accounts still leaves usable
weight, while follow-everything accounts are strongly discounted.)

### Corroboration is a *different* combine

Max-product picks the single best path. "This target is reached by many
independent paths, so I trust it more" is product *within* a path but **sum
*across* paths** (trust-flow / conductance — the forward algorithm vs Viterbi).
Treat it as a later confidence-aggregation layer, separate from the single
best-path search.

### Policy knobs

- **Conflict = hard 0 (default)** keeps the clash *surfaced* rather than
  silently auto-picked — the property we wanted in §3/§4. Weighting conflicts by
  seniority/closeness instead of zeroing (auto-pick the senior claim) is an
  opt-in policy, not the default.
- The meaningful quantities are all *ratios of logs*; pick `δ`, `k`, and the
  selectivity shape by intent, then the absolute scale only matters once
  multiple non-uniform factors coexist.

### Mapping to the code

`PathEdge` gains `weight: f64`; the resolver core becomes a Dijkstra over
`-ln(weight)`; `Membership::Ambiguous` emits its candidates with `weight = 0`
(or seniority/closeness weights under a policy flag); each returned path carries
a `score`. `resolved = Some` iff exactly one maximum-score path with score > 0;
all-zero → `resolved = None` + alternates, identical to today.

## 9. NIP-05: one optional signal, not the root

Traditional **NIP-05** proves `name@example.com` by having a DNS-controlled web
server publish a `.well-known/nostr.json` that maps the name to a pubkey. It is
useful, but its *authority* is **domain ownership**, which re-imports exactly the
DNS problems GNS set out to avoid: domains expire, registrars can seize names,
*ownership* (not relationship) decides identity, and you pay rent to keep a
namespace alive.

GNS inverts the dependency. Identity is the **pubkey**; the name is kind-0
**metadata**; resolution is **graph position**. In that world NIP-05 doesn't
disappear — it **demotes** from *the* identity binding to **one attestation among
many**: a domain owner vouching "this pubkey is mine," exactly as useful (and as
limited) as any other voucher in the graph.

### The stack, most authoritative first

```
1. Public key          identity itself — everything else is a claim about a key
2. Signed kind-0 name   the key's current *claimed* name (self-asserted, mutable, not unique)
3. Social graph (kind-3) trust & resolution — who vouches; mutual/selective weight; shortest path
4. NIP-05               optional external attestation — a DNS domain co-signing the key
5. OpenTimestamps/BTC   historical proof & permanence — trustless cross-key seniority (§5)
```

Read top-down this is a **fallback order, not a dependency chain**: each lower
layer is optional and *adds confidence* to the layer above without being required
by it. Strip NIP-05 and OTS away and GNS still resolves — they only sharpen
disputes. (The layer that does the real naming work is **3**, the graph; **1** is
what makes renames and migration clean, per §2 and §7.)

### Where NIP-05 still earns its place

- **Interop, for free.** Existing Nostr clients already display and verify
  NIP-05, so a GNS key that *also* publishes one gets a verified badge everywhere
  today — GNS coexists with NIP-05 rather than replacing it.
- **A weight, not a gate.** In the §8 algebra a verified NIP-05 is just another
  multiplicative factor on confidence — "this key is also vouched by domain X" —
  never a precondition for resolution. Treat it like reciprocity/selectivity: a
  knob, not a router.
- **An out-of-band anchor for migration.** §7's key-*compromise* case falls back
  to "other identities Alice controls"; a NIP-05 domain is precisely such an
  identity, useful for re-establishing a key the graph alone can't authorize.

### What it is explicitly *not* in GNS

- not **globally unique** — the `@alice` in `alice.bob` is graph-relative;
- not a name **registry** — there is no scarce reserved string to own;
- not **required** to resolve; and
- not **authoritative over the graph** — if NIP-05 says one thing and the follows
  say another, the follows win. NIP-05 is corroboration.

So NIP-05 is the same shape as everything else in §8 — a signed claim with a
provenance (here, a domain) — and gets the same treatment: **surfaced, weighted,
never trusted as the root.**

## 10. Economics: pay for discovery, not registration

Demoting the name from "the asset" to "metadata" also moves *where the money is*.

**Registration model (DNS / ENS / NFT namespaces).** Scarcity is manufactured on
the *name*: you pay to reserve a string, pay to renew, and the name can lapse or
be seized. Identity becomes a **rented asset** — exactly what GNS is trying not
to be.

**Discovery model (GNS).** Names aren't scarce — anyone can call themselves
"alice" — so there is nothing to *sell* at the name. The real, recurring cost is
**resolution**: traversing the graph, indexing kind-3 lists, keeping caches
fresh, fetching reverse edges for reciprocity, and maintaining/re-verifying
proofs. That cost recurs whether or not any name ever changes, and it is where a
sustainable business sits. The chargeable surfaces:

| surface | what you're paying for |
| --- | --- |
| lookups / traversal | answering `barbara@alex.michael.nostr` |
| indexing & path compression | precomputed shortcuts so hot paths stay cheap (§8) |
| freshness / caching | the cache TTL is literally a "how stale will you tolerate" knob — the public-good dynamic |
| proof maintenance | archiving observed kind-0/kind-3 and keeping OTS proofs (§5, "you must keep what you anchor") |

The slogan: **businesses subsidize path freshness; they don't collect rent on
names.** A company that wants `*.acme` to resolve crisply pays to keep *its*
corner of the graph indexed and fresh — improving a shared public good — rather
than paying a registrar to *own* the string "acme." Stop paying and resolution
gets **staler, not revoked**: the names still exist in the graph, and any other
indexer can rebuild from relay data. Compare registration, where non-payment
*deletes* your identity.

Two consequences worth stating:

- **Anchors are portable, so timestamping is a public good, not a moat.** Because
  OTS settles to one chain (§5), a proof one indexer pays to create is verifiable
  by all — you can't lock customers in by hoarding seniority proofs; you compete
  on freshness and coverage.
- **Incentives point at keeping the graph live, not at gatekeeping.** Revenue
  scales with *use* (queries, freshness SLAs) rather than with *exclusion* (who is
  forbidden a name) — the alignment you want for a naming commons.

## 11. Layered conclusion

| Concern | Mechanism | Cost | Status |
| --- | --- | --- | --- |
| Inject a name into a namespace | Follow requirement (membership rule) | none | **implemented** |
| Rename yourself | Key-based identity (no gating) | none | **implemented** (renames just work) |
| Two followed keys collide on a name | Ambiguity rule (don't resolve, return alternates) | none | **implemented** |
| Migrate keys (resolution) | Followers re-point follows (votes) | none | **implemented** (re-follow just works) |
| Shortest / best path | Weighted Dijkstra over `-ln(weight)` (generalizes BFS) | small | proposed |
| Prefer mutual & selective follows | Reciprocity (one-way = mutual^k) × selectivity (inverse out-degree) factors | small | proposed |
| Old-name still routes | `(namespace,label)→pubkey` redirect memo | tiny | proposed |
| Name silently changes hands | Same memo, TOFU "changed hands" flag | tiny | proposed |
| Migrate keys (safely) | Signed `K_old`↔`K_new` attestation (downgrades the flag, collapses ambiguity) | small | proposed |
| Who held a name first (across keys) | OTS / Bitcoin anchoring + archiving | heavy, optional | future |
| Equivocation / forks | Per-author chains + anchoring | heavy, optional | future |
| External domain attestation | NIP-05 as a confidence *weight* (corroboration), not a gate | none (interop) | optional |
| Sustainable operation | Charge for *discovery* (traversal / freshness / proofs), not name *registration* | — | economic model |

The guiding principle: **keep the common path (resolve + rename + re-follow)
free and key-based; treat resolution as weighted best-path search (conflict =
weight 0); add state only to *detect* (not gate) name hand-offs and to *verify*
migrations; fold external attestations (NIP-05) in as weights, never roots; and
reserve external anchoring for trustless cross-key disputes.** The identity stack
is layered (§9): *pubkey → signed name → social graph → NIP-05 → OTS/Bitcoin*,
each lower layer optional and only *adding* confidence — which is why the costs
land on **discovery, not registration** (§10).
