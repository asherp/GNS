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

## 8. Layered conclusion

| Concern | Mechanism | Cost | Status |
| --- | --- | --- | --- |
| Inject a name into a namespace | Follow requirement (membership rule) | none | **implemented** |
| Rename yourself | Key-based identity (no gating) | none | **implemented** (renames just work) |
| Two followed keys collide on a name | Ambiguity rule (don't resolve, return alternates) | none | **implemented** |
| Migrate keys (resolution) | Followers re-point follows (votes) | none | **implemented** (re-follow just works) |
| Old-name still routes | `(namespace,label)→pubkey` redirect memo | tiny | proposed |
| Name silently changes hands | Same memo, TOFU "changed hands" flag | tiny | proposed |
| Migrate keys (safely) | Signed `K_old`↔`K_new` attestation (downgrades the flag, collapses ambiguity) | small | proposed |
| Who held a name first (across keys) | OTS / Bitcoin anchoring + archiving | heavy, optional | future |
| Equivocation / forks | Per-author chains + anchoring | heavy, optional | future |

The guiding principle: **keep the common path (resolve + rename + re-follow)
free and key-based; add state only to *detect* (not gate) name hand-offs and to
*verify* migrations; reserve external anchoring for trustless cross-key
disputes.**
