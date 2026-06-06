# CLAUDE.md

Guidance for working in this repository.

## What this is

GNS (Graph Name System) — a lightweight Rust resolver that turns the Nostr
social graph into a naming/discovery layer. It resolves `(name, graph position)`
to a pubkey by walking kind-3 follow lists, carrying full provenance (follow
event ids + the relays they were seen on). See `README.md` for the vision and
`docs/naming.md` for the naming spec.

## Common commands

```bash
cargo run -- --demo        # run against the built-in fixture graph (no network)
cargo run --release        # live mode — reads relays from config.toml
cargo test                 # unit tests (npub round-trip, BFS, normalization, …)
```

The dashboard is served at `http://127.0.0.1:8080/` in both modes.

## Layout

- `src/` — the resolver. Key modules: `graph/resolver.rs` (BFS pubkey→pubkey),
  `graph/name.rs` (label normalization + ambiguity), `graph/address.rs` (GNS
  address parsing), `graph/name_resolver.rs` (address walk), `api.rs` (HTTP API).
  `graph/demo_source.rs` is the offline fixture graph.
- `static/` — the Rust server's dashboard (vanilla JS, talks to `/api/*`).
- `site/` — the standalone marketing/landing site (Zola). Its in-browser demo in
  `site/static/app.js` reimplements the resolver in JS and can also resolve
  **live** by connecting from the browser straight to relays over WebSockets.

When you change resolution rules, keep the Rust resolver and the JS
reimplementation in `site/static/app.js` in sync — the latter intentionally
mirrors `demo_source.rs`, `name.rs`, and `address.rs`.

## Website / deployment

**The landing site (`site/`) is published to GitHub Pages only from the `docs`
branch.** The deploy workflow (`.github/workflows/pages.yml`) triggers on pushes
to `docs` that touch `site/**` (or the workflow file).

`main` is *not* deployed. So the publish flow is:

1. Develop and merge changes into `main` as usual.
2. **To update the live website, merge `main` into `docs`** (e.g. open a PR with
   `head: main`, `base: docs`, or fast-forward `docs` to `main`).
3. The Pages workflow then builds the Zola site and deploys it.

If a change to `site/` has landed on `main` but the website hasn't updated, it's
almost always because `main` hasn't been merged into `docs` yet.

One-time setup (already done, noted for reference): Settings → Pages → Source
must be set to "GitHub Actions" — the Actions token can deploy Pages but cannot
enable it.
