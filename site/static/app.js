"use strict";

// ===========================================================================
// GNS in-browser demo.
//
// Two modes, one resolver:
//   • "fixture" — the built-in example graph (offline, deterministic).
//   • "live"    — connects from *your browser* straight to public Nostr relays
//                 over WebSockets, fetches real kind-3 follow lists and kind-0
//                 profiles on demand, and resolves whatever you type. Real data,
//                 real nip05, real per-hop relay attribution.
//
// The resolution logic (normalize → membership → ambiguity → address walk) is
// identical to the Rust resolver; only the *source* of the graph differs.
// ===========================================================================

// ---------------------------------------------------------------------------
// Built-in example graph — mirrors src/graph/demo_source.rs.
// Identity is the key (`id` here); `name` is the kind-0 profile name.
// ---------------------------------------------------------------------------
const GRAPH = {
  you:    { name: "You",      follows: ["michael", "carol", "frank1", "frank2"] },
  michael:{ name: "Michael",  follows: ["alex", "dave"] },
  alex:   { name: "Alex",     follows: ["barbara"] },
  barbara:{ name: "Barbara",  follows: [] },
  carol:  { name: "Carol",    follows: ["barbara"] },
  dave:   { name: "Dave",     follows: ["barbara"] },
  frank1: { name: "Frank",    follows: [] },
  frank2: { name: "⚡Frank⚡", follows: [] },
};
const FIXTURE_ROOT = "you";

// ---------------------------------------------------------------------------
// Label normalization + address parsing — mirrors name.rs / address.rs.
// ---------------------------------------------------------------------------
function normalizeLabel(name) {
  return (name || "").toLowerCase().replace(/[^a-z0-9]/g, "");
}

function parseAddress(addr) {
  const trimmed = (addr || "").trim();
  if (!trimmed) throw new Error("address is empty");
  const at = trimmed.indexOf("@");
  const targetPart = at === -1 ? trimmed : trimmed.slice(0, at);
  const domainPart = at === -1 ? "" : trimmed.slice(at + 1);

  const target = normalizeLabel(targetPart);
  if (!target) throw new Error("target name has no valid GNS label");

  let segments = domainPart.split(".").filter((s) => s.length);
  if (segments.length && normalizeLabel(segments[segments.length - 1]) === "nostr") {
    segments.pop();
  }
  const pathLabels = segments.map((s) => {
    const l = normalizeLabel(s);
    if (!l) throw new Error(`path segment "${s}" has no valid GNS label`);
    return l;
  });

  const walk = pathLabels.slice().reverse();
  walk.push(target);
  return { original: trimmed, target, walk };
}

// ---------------------------------------------------------------------------
// Generic resolver — walks the address labels over an abstract source,
// branching on ambiguity. `source.members(id)` returns the followed nodes of
// `id`, each with its normalized label; it may be async.
// ---------------------------------------------------------------------------
async function resolveAddress(source, rootId, walk, onStep) {
  let partials = [[rootId]];
  let ambiguous = false;

  for (const label of walk) {
    const next = [];
    for (const path of partials) {
      const cur = path[path.length - 1];
      if (onStep) await onStep(cur, label);
      const members = await source.members(cur);
      const matches = members.filter((m) => m.label && m.label === label);
      if (matches.length > 1) ambiguous = true;
      for (const m of matches) next.push(path.concat(m.id));
    }
    partials = next;
    if (!partials.length) break;
  }

  const found = partials.length > 0;
  if (partials.length > 1) ambiguous = true;
  const resolved =
    found && !ambiguous && partials.length === 1
      ? partials[0][partials[0].length - 1]
      : null;
  return { found, ambiguous, resolved, paths: partials };
}

// ---------------------------------------------------------------------------
// Fixture source — the built-in graph, wrapped in the async source contract.
// ---------------------------------------------------------------------------
function fixtureSource() {
  return {
    async members(id) {
      return (GRAPH[id]?.follows || []).map((f) => ({
        id: f,
        label: normalizeLabel(GRAPH[f].name),
      }));
    },
    nodeInfo(id) {
      return { name: GRAPH[id]?.name || id, nip05: null, picture: null, npub: null };
    },
    edgeMeta(parentId) {
      return {
        eventId: `demoevent…${parentId}`,
        relays: ["wss://demo.relay.invalid"],
      };
    },
  };
}

// ===========================================================================
// bech32 (npub ⇄ hex) — minimal, dependency-free (BIP-173).
// ===========================================================================
const BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

function bech32Polymod(values) {
  const GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
  let chk = 1;
  for (const v of values) {
    const top = chk >>> 25;
    chk = ((chk & 0x1ffffff) << 5) ^ v;
    for (let i = 0; i < 5; i++) if ((top >>> i) & 1) chk ^= GEN[i];
  }
  return chk;
}
function bech32HrpExpand(hrp) {
  const out = [];
  for (let i = 0; i < hrp.length; i++) out.push(hrp.charCodeAt(i) >> 5);
  out.push(0);
  for (let i = 0; i < hrp.length; i++) out.push(hrp.charCodeAt(i) & 31);
  return out;
}
function bech32VerifyChecksum(hrp, data) {
  return bech32Polymod(bech32HrpExpand(hrp).concat(data)) === 1;
}
function bech32CreateChecksum(hrp, data) {
  const values = bech32HrpExpand(hrp).concat(data).concat([0, 0, 0, 0, 0, 0]);
  const mod = bech32Polymod(values) ^ 1;
  const out = [];
  for (let i = 0; i < 6; i++) out.push((mod >> (5 * (5 - i))) & 31);
  return out;
}
function convertBits(data, from, to, pad) {
  let acc = 0,
    bits = 0;
  const out = [];
  const maxv = (1 << to) - 1;
  for (const value of data) {
    if (value < 0 || value >> from) throw new Error("bad value in convertBits");
    acc = (acc << from) | value;
    bits += from;
    while (bits >= to) {
      bits -= to;
      out.push((acc >> bits) & maxv);
    }
  }
  if (pad) {
    if (bits) out.push((acc << (to - bits)) & maxv);
  } else if (bits >= from || ((acc << (to - bits)) & maxv)) {
    throw new Error("bad padding in convertBits");
  }
  return out;
}
function bech32Decode(str) {
  if (str !== str.toLowerCase() && str !== str.toUpperCase()) {
    throw new Error("mixed-case bech32");
  }
  str = str.toLowerCase();
  const pos = str.lastIndexOf("1");
  if (pos < 1 || pos + 7 > str.length) throw new Error("not a bech32 string");
  const hrp = str.slice(0, pos);
  const data = [];
  for (const ch of str.slice(pos + 1)) {
    const d = BECH32_CHARSET.indexOf(ch);
    if (d === -1) throw new Error(`invalid bech32 character "${ch}"`);
    data.push(d);
  }
  if (!bech32VerifyChecksum(hrp, data)) throw new Error("bad bech32 checksum");
  return { hrp, words: data.slice(0, -6) };
}
function bech32Encode(hrp, words) {
  const combined = words.concat(bech32CreateChecksum(hrp, words));
  let out = hrp + "1";
  for (const w of combined) out += BECH32_CHARSET[w];
  return out;
}
function npubToHex(npub) {
  const { hrp, words } = bech32Decode(npub.trim());
  if (hrp !== "npub") throw new Error(`expected an npub, got "${hrp}"`);
  const bytes = convertBits(words, 5, 8, false);
  if (bytes.length !== 32) throw new Error("npub does not decode to 32 bytes");
  return bytes.map((b) => b.toString(16).padStart(2, "0")).join("");
}
function hexToNpub(hex) {
  const bytes = [];
  for (let i = 0; i < hex.length; i += 2) bytes.push(parseInt(hex.slice(i, i + 2), 16));
  return bech32Encode("npub", convertBits(bytes, 8, 5, true));
}
function shortNpub(npub) {
  return npub && npub.length > 20 ? `${npub.slice(0, 12)}…${npub.slice(-6)}` : npub;
}
function shortId(id) {
  return id && id.length > 16 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;
}

// ===========================================================================
// Root identity resolution — npub / hex / NIP-05.
// ===========================================================================
const HEX64 = /^[0-9a-f]{64}$/i;

async function resolveRootIdentity(input) {
  const s = (input || "").trim();
  if (!s) throw new Error("enter a root npub / hex pubkey / NIP-05 address");
  if (HEX64.test(s)) return s.toLowerCase();
  if (s.startsWith("npub1")) return npubToHex(s);
  if (s.includes("@") || s.includes(".")) return await nip05ToHex(s);
  throw new Error(`unrecognized root identifier: "${s}"`);
}

// Best-effort: fetch the domain's /.well-known/nostr.json. Many domains don't
// send CORS headers, in which case the browser blocks it — reported clearly.
async function nip05ToHex(addr) {
  let name, domain;
  if (addr.includes("@")) [name, domain] = addr.split("@");
  else [name, domain] = ["_", addr];
  name = (name || "_").toLowerCase();
  domain = (domain || "").trim();
  if (!domain) throw new Error("NIP-05 address has no domain");
  const url = `https://${domain}/.well-known/nostr.json?name=${encodeURIComponent(name)}`;
  let res;
  try {
    res = await fetch(url, { redirect: "follow" });
  } catch (e) {
    throw new Error(
      `NIP-05 lookup for ${name}@${domain} failed — the domain likely doesn't allow cross-origin requests (CORS). Try an npub instead.`
    );
  }
  if (!res.ok) throw new Error(`NIP-05 lookup for ${name}@${domain} returned HTTP ${res.status}`);
  const json = await res.json();
  const hex = json && json.names && json.names[name];
  if (!hex) throw new Error(`no NIP-05 entry for "${name}" at ${domain}`);
  return hex.toLowerCase();
}

// ===========================================================================
// Relay pool — minimal browser-side Nostr client over WebSockets.
// Tracks which relays served each event (for real provenance).
// ===========================================================================
class RelayPool {
  constructor(urls) {
    this.urls = urls;
    this.conns = new Map(); // url -> { ws, ready, subs: Map }
  }

  conn(url) {
    let c = this.conns.get(url);
    if (c && c.ws.readyState <= 1) return c;
    const ws = new WebSocket(url);
    c = { ws, subs: new Map(), ready: null };
    c.ready = new Promise((resolve, reject) => {
      ws.onopen = () => resolve(ws);
      ws.onerror = () => reject(new Error(`could not connect to ${url}`));
    });
    ws.onmessage = (ev) => {
      let msg;
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }
      const [type, subId] = msg;
      const sub = c.subs.get(subId);
      if (!sub) return;
      if (type === "EVENT") sub.onEvent(msg[2], url);
      else if (type === "EOSE" || type === "CLOSED") sub.onEose();
    };
    ws.onclose = () => {
      for (const s of c.subs.values()) s.onEose();
      c.subs.clear();
    };
    this.conns.set(url, c);
    return c;
  }

  // Run one filter across every relay; return deduped events + per-id relay set.
  async query(filter, timeout = 5000) {
    const byId = new Map();
    const relaysById = new Map();
    const subId = "gns" + Math.random().toString(36).slice(2, 10);

    const perRelay = this.urls.map(
      (url) =>
        new Promise((resolve) => {
          let done = false;
          let c;
          const finish = () => {
            if (done) return;
            done = true;
            if (c) {
              try {
                if (c.ws.readyState === 1) c.ws.send(JSON.stringify(["CLOSE", subId]));
              } catch {}
              c.subs.delete(subId);
            }
            resolve();
          };
          c = this.conn(url);
          c.ready
            .then((ws) => {
              c.subs.set(subId, {
                onEvent: (e, u) => {
                  if (!e || !e.id) return;
                  byId.set(e.id, e);
                  if (!relaysById.has(e.id)) relaysById.set(e.id, new Set());
                  relaysById.get(e.id).add(u);
                },
                onEose: finish,
              });
              ws.send(JSON.stringify(["REQ", subId, filter]));
              setTimeout(finish, timeout);
            })
            .catch(finish);
        })
    );

    await Promise.all(perRelay);
    return { events: [...byId.values()], relaysById };
  }

  close() {
    for (const c of this.conns.values()) {
      try {
        c.ws.close();
      } catch {}
    }
    this.conns.clear();
  }
}

function pickNewest(events) {
  let best = null;
  for (const ev of events) if (!best || ev.created_at > best.created_at) best = ev;
  return best;
}
function chunk(arr, n) {
  const out = [];
  for (let i = 0; i < arr.length; i += n) out.push(arr.slice(i, i + n));
  return out;
}
function parseProfileContent(ev) {
  let c = {};
  try {
    c = JSON.parse(ev.content) || {};
  } catch {}
  return {
    name: c.name,
    display_name: c.display_name || c.displayName,
    picture: c.picture,
    nip05: c.nip05,
  };
}

// ---------------------------------------------------------------------------
// Live source — relay-backed, with caching + provenance.
// ---------------------------------------------------------------------------
function liveSource(relayUrls, onStatus) {
  const pool = new RelayPool(relayUrls);
  const contactsCache = new Map(); // id -> [followHex]
  const profiles = new Map(); // id -> {name, nip05, picture, ...}
  const edge = new Map(); // followerId -> {eventId, relays}

  async function ensureProfiles(ids) {
    const need = ids.filter((id) => !profiles.has(id));
    if (!need.length) return;
    for (const part of chunk(need, 300)) {
      onStatus && onStatus(`Fetching ${part.length} profile${part.length === 1 ? "" : "s"}…`);
      const { events } = await pool.query({ kinds: [0], authors: part });
      const newestByAuthor = new Map();
      for (const ev of events) {
        const prev = newestByAuthor.get(ev.pubkey);
        if (!prev || ev.created_at > prev.created_at) newestByAuthor.set(ev.pubkey, ev);
      }
      for (const id of part) {
        const ev = newestByAuthor.get(id);
        profiles.set(id, ev ? parseProfileContent(ev) : {});
      }
    }
  }

  async function contacts(id) {
    if (contactsCache.has(id)) return contactsCache.get(id);
    const { events, relaysById } = await pool.query({ kinds: [3], authors: [id] });
    const newest = pickNewest(events);
    let follows = [];
    if (newest) {
      follows = [
        ...new Set(
          newest.tags
            .filter((t) => t[0] === "p" && /^[0-9a-f]{64}$/i.test(t[1] || ""))
            .map((t) => t[1].toLowerCase())
        ),
      ];
      edge.set(id, {
        eventId: newest.id,
        relays: [...(relaysById.get(newest.id) || [])],
      });
    }
    contactsCache.set(id, follows);
    return follows;
  }

  return {
    pool,
    async warm(rootId) {
      await ensureProfiles([rootId]);
    },
    async members(id) {
      const nm = profiles.get(id)?.name || shortId(id);
      onStatus && onStatus(`Fetching follow list of ${nm}…`);
      const follows = await contacts(id);
      await ensureProfiles(follows);
      return follows.map((f) => ({ id: f, label: normalizeLabel(profiles.get(f)?.name) }));
    },
    nodeInfo(id) {
      const p = profiles.get(id) || {};
      let npub = null;
      try {
        npub = hexToNpub(id);
      } catch {}
      return {
        name: p.name || p.display_name || shortId(id),
        nip05: p.nip05 || null,
        picture: p.picture || null,
        npub,
      };
    },
    edgeMeta(parentId) {
      return edge.get(parentId) || { eventId: null, relays: [] };
    },
  };
}

// ===========================================================================
// View model + rendering (shared by both modes).
// ===========================================================================
const $ = (id) => document.getElementById(id);
const statusEl = $("status");
const detailsEl = $("details");
const hopsEl = $("hops");

function setStatus(msg, cls) {
  statusEl.textContent = msg;
  statusEl.className = "status" + (cls ? " " + cls : "");
}

// Build {nodes, edges} maps for the resolved paths from the active source.
function buildView(source, paths) {
  const nodes = {};
  const edges = {};
  for (const path of paths) {
    for (const id of path) if (!nodes[id]) nodes[id] = source.nodeInfo(id);
    for (let i = 1; i < path.length; i++) {
      const parent = path[i - 1];
      if (!edges[parent]) edges[parent] = source.edgeMeta(parent);
    }
  }
  return { nodes, edges };
}

function renderResult(parsed, res, view) {
  hopsEl.innerHTML = "";

  if (!res.found) {
    setStatus("No follow with the required label was found along this path.", "err");
    detailsEl.hidden = true;
    draw([], {});
    return;
  }

  if (res.ambiguous) {
    setStatus(
      `Ambiguous “${parsed.target}” — ${res.paths.length} candidates; per the GNS ambiguity rule it does not resolve to a single key.`,
      "warn"
    );
  } else {
    const last = res.paths[0][res.paths[0].length - 1];
    const hops = res.paths[0].length - 1;
    setStatus(
      `Resolved “${parsed.target}” → ${view.nodes[last].name} (${hops} hop${hops === 1 ? "" : "s"}).`,
      "ok"
    );
  }

  res.paths.forEach((path, idx) => {
    const wrap = document.createElement("div");
    wrap.className = "alt-path";
    if (res.paths.length > 1) {
      const h = document.createElement("h3");
      h.textContent = `Candidate ${idx + 1}`;
      wrap.appendChild(h);
    }
    const ol = document.createElement("ol");
    path.forEach((id, i) => {
      const info = view.nodes[id] || { name: id };
      const li = document.createElement("li");

      const name = document.createElement("span");
      name.className = "hop-name";
      name.textContent = info.name;
      const pill = document.createElement("span");
      pill.className = "label-pill";
      pill.textContent = normalizeLabel(info.name) || "—";
      name.appendChild(pill);
      li.appendChild(name);

      // Identity line: npub + nip05 when present.
      if (info.npub || info.nip05) {
        const idline = document.createElement("div");
        idline.className = "hop-id";
        const bits = [];
        if (info.npub) bits.push(shortNpub(info.npub));
        if (info.nip05) bits.push(`nip05: ${info.nip05}`);
        idline.textContent = bits.join("  ·  ");
        li.appendChild(idline);
      }

      // Edge provenance from the *follower* (previous hop).
      if (i > 0) {
        const parent = path[i - 1];
        const meta = view.edges[parent] || {};
        const m = document.createElement("div");
        m.className = "edge-meta";
        const relays = meta.relays && meta.relays.length ? meta.relays.join(", ") : "—";
        const evid = meta.eventId ? shortId(meta.eventId) : "—";
        m.textContent = `follows — kind 3 event ${evid} from ${view.nodes[parent].name} · seen on ${relays}`;
        li.appendChild(m);
      }
      ol.appendChild(li);
    });
    wrap.appendChild(ol);
    hopsEl.appendChild(wrap);
  });
  detailsEl.hidden = false;
  draw(res.paths[0], view.nodes);
}

// ---------------------------------------------------------------------------
// Canvas: render the (primary) path as avatar bubbles.
// ---------------------------------------------------------------------------
const canvas = $("graph");
const ctx = canvas.getContext("2d");
const palette = ["#8a5cf6", "#38bdf8", "#3fb950", "#d29922", "#f778ba", "#a371f7"];
let current = [];
let currentNodes = {};

function draw(path, nodes) {
  current = path;
  currentNodes = nodes || currentNodes;
  const dpr = window.devicePixelRatio || 1;
  const rect = canvas.getBoundingClientRect();
  canvas.width = rect.width * dpr;
  canvas.height = rect.height * dpr;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, rect.width, rect.height);

  if (!path.length) {
    ctx.fillStyle = "#5b6675";
    ctx.font = "15px system-ui";
    ctx.textAlign = "center";
    ctx.fillText("No path", rect.width / 2, rect.height / 2);
    return;
  }

  const nameOf = (id) => currentNodes[id]?.name || id;
  const n = path.length;
  const margin = 70;
  const usableW = Math.max(rect.width - margin * 2, 1);
  const midY = rect.height / 2;
  const r = 30;
  const pos = path.map((_, i) => ({
    x: n === 1 ? rect.width / 2 : margin + (usableW * i) / (n - 1),
    y: midY + (i % 2 === 0 ? -1 : 1) * (n > 1 ? 34 : 0),
  }));

  for (let i = 0; i < n - 1; i++) {
    const a = pos[i],
      b = pos[i + 1];
    const ang = Math.atan2(b.y - a.y, b.x - a.x);
    const sx = a.x + Math.cos(ang) * r,
      sy = a.y + Math.sin(ang) * r;
    const ex = b.x - Math.cos(ang) * r,
      ey = b.y - Math.sin(ang) * r;
    ctx.strokeStyle = "#3a4554";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(sx, sy);
    ctx.lineTo(ex, ey);
    ctx.stroke();
    const h = 9;
    ctx.fillStyle = "#3a4554";
    ctx.beginPath();
    ctx.moveTo(ex, ey);
    ctx.lineTo(ex - h * Math.cos(ang - 0.4), ey - h * Math.sin(ang - 0.4));
    ctx.lineTo(ex - h * Math.cos(ang + 0.4), ey - h * Math.sin(ang + 0.4));
    ctx.closePath();
    ctx.fill();
  }

  path.forEach((id, i) => {
    const p = pos[i];
    const color = palette[i % palette.length];
    ctx.beginPath();
    ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
    ctx.fillStyle = color;
    ctx.fill();
    ctx.lineWidth = 2.5;
    ctx.strokeStyle = color;
    ctx.stroke();
    ctx.fillStyle = "#0e1116";
    ctx.font = "bold 22px system-ui";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(nameOf(id).replace(/[^A-Za-z0-9]/g, "").slice(0, 1).toUpperCase() || "?", p.x, p.y + 1);
    ctx.fillStyle = "#e6edf3";
    ctx.font = "13px system-ui";
    ctx.textBaseline = "top";
    const label = nameOf(id);
    ctx.fillText(label.length > 16 ? label.slice(0, 15) + "…" : label, p.x, p.y + r + 8);
  });
}

// ===========================================================================
// Mode wiring.
// ===========================================================================
let mode = "fixture";

function switchMode(next) {
  mode = next;
  document.querySelectorAll(".demo-mode").forEach((b) =>
    b.classList.toggle("active", b.dataset.dmode === next)
  );
  document.querySelectorAll(".demo-pane").forEach((p) => {
    p.hidden = p.dataset.pane !== next;
  });
  setStatus("");
  detailsEl.hidden = true;
  draw([], {});
}

// ---- Fixture mode ----
async function runFixture() {
  let parsed;
  try {
    parsed = parseAddress($("addr").value);
  } catch (e) {
    setStatus(String(e.message || e), "err");
    detailsEl.hidden = true;
    draw([], {});
    return;
  }
  const source = fixtureSource();
  const res = await resolveAddress(source, FIXTURE_ROOT, parsed.walk);
  renderResult(parsed, res, buildView(source, res.paths));
}

// ---- Live mode ----
let livePool = null;

async function runLive() {
  const btn = $("live-resolve");
  btn.disabled = true;
  if (livePool) {
    livePool.close();
    livePool = null;
  }
  try {
    const relayUrls = $("live-relays")
      .value.split(",")
      .map((s) => s.trim())
      .filter((s) => /^wss?:\/\//.test(s));
    if (!relayUrls.length) throw new Error("enter at least one wss:// relay");

    const parsed = parseAddress($("live-addr").value);

    setStatus("Resolving root identity…");
    const rootId = await resolveRootIdentity($("live-root").value);

    const source = liveSource(relayUrls, (msg) => setStatus(msg));
    livePool = source.pool;
    setStatus(`Connecting to ${relayUrls.length} relay${relayUrls.length === 1 ? "" : "s"}…`);
    await source.warm(rootId);

    const res = await resolveAddress(source, rootId, parsed.walk);
    renderResult(parsed, res, buildView(source, res.paths));
  } catch (e) {
    setStatus(String(e.message || e), "err");
    detailsEl.hidden = true;
    draw([], {});
  } finally {
    btn.disabled = false;
  }
}

// ===========================================================================
// Bootstrap.
// ===========================================================================
document.querySelectorAll(".demo-mode").forEach((b) =>
  b.addEventListener("click", () => switchMode(b.dataset.dmode))
);

// Fixture controls
$("resolve").addEventListener("click", runFixture);
$("addr").addEventListener("keydown", (e) => {
  if (e.key === "Enter") runFixture();
});
document.querySelectorAll(".chip[data-addr]:not(.live-chip)").forEach((c) =>
  c.addEventListener("click", () => {
    $("addr").value = c.dataset.addr;
    runFixture();
  })
);

// Live controls
$("live-resolve").addEventListener("click", runLive);
$("live-addr").addEventListener("keydown", (e) => {
  if (e.key === "Enter") runLive();
});
document.querySelectorAll(".live-chip").forEach((c) =>
  c.addEventListener("click", () => {
    if (c.dataset.root) $("live-root").value = c.dataset.root;
    if (c.dataset.addr) $("live-addr").value = c.dataset.addr;
    runLive();
  })
);

window.addEventListener("resize", () => draw(current, currentNodes));

runFixture();
