"use strict";

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
const ROOT = "you";

// ---------------------------------------------------------------------------
// Resolver — mirrors normalize_label / membership / ambiguity / address parse.
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

// Members of a namespace: each followed key with its normalized label.
function members(id) {
  return (GRAPH[id]?.follows || []).map((f) => ({ id: f, label: normalizeLabel(GRAPH[f].name) }));
}

// Membership lookup for a single label: None / one / many (ambiguous).
function matchLabel(id, label) {
  return members(id).filter((m) => m.label && m.label === label);
}

// Walk the address labels, branching on ambiguity. Returns paths + flags.
function resolveAddress(rootId, walk) {
  let partials = [[rootId]];
  let ambiguous = false;

  for (const label of walk) {
    const next = [];
    for (const path of partials) {
      const cur = path[path.length - 1];
      const matches = matchLabel(cur, label);
      if (matches.length > 1) ambiguous = true;
      for (const m of matches) next.push(path.concat(m.id));
    }
    partials = next;
    if (!partials.length) break;
  }

  const found = partials.length > 0;
  if (partials.length > 1) ambiguous = true;
  const resolved = found && !ambiguous && partials.length === 1
    ? partials[0][partials[0].length - 1]
    : null;
  return { found, ambiguous, resolved, paths: partials };
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------
const $ = (id) => document.getElementById(id);
const statusEl = $("status");
const detailsEl = $("details");
const hopsEl = $("hops");

function setStatus(msg, cls) {
  statusEl.textContent = msg;
  statusEl.className = "status" + (cls ? " " + cls : "");
}

function run() {
  let parsed;
  try {
    parsed = parseAddress($("addr").value);
  } catch (e) {
    setStatus(String(e.message || e), "err");
    detailsEl.hidden = true;
    draw([]);
    return;
  }

  const res = resolveAddress(ROOT, parsed.walk);
  hopsEl.innerHTML = "";

  if (!res.found) {
    setStatus(`No follow with the required label was found along this path.`, "err");
    detailsEl.hidden = true;
    draw([]);
    return;
  }

  if (res.ambiguous) {
    setStatus(`Ambiguous “${parsed.target}” — ${res.paths.length} candidates; per the GNS ambiguity rule it does not resolve to a single key.`, "warn");
  } else {
    const last = res.paths[0][res.paths[0].length - 1];
    setStatus(`Resolved “${parsed.target}” → ${GRAPH[last].name} (${res.paths[0].length - 1} hop${res.paths[0].length === 2 ? "" : "s"}).`, "ok");
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
      const li = document.createElement("li");
      const name = document.createElement("span");
      name.className = "hop-name";
      name.textContent = GRAPH[id].name;
      const pill = document.createElement("span");
      pill.className = "label-pill";
      pill.textContent = normalizeLabel(GRAPH[id].name) || "—";
      name.appendChild(pill);
      li.appendChild(name);
      if (i > 0) {
        const meta = document.createElement("div");
        meta.className = "edge-meta";
        meta.textContent = `follows — kind 3 event from ${GRAPH[path[i - 1]].name}`;
        li.appendChild(meta);
      }
      ol.appendChild(li);
    });
    wrap.appendChild(ol);
    hopsEl.appendChild(wrap);
  });
  detailsEl.hidden = false;

  draw(res.paths[0]);
}

// ---------------------------------------------------------------------------
// Canvas: render the (primary) path as avatar bubbles.
// ---------------------------------------------------------------------------
const canvas = $("graph");
const ctx = canvas.getContext("2d");
const palette = ["#8a5cf6", "#38bdf8", "#3fb950", "#d29922", "#f778ba", "#a371f7"];
let current = [];

function draw(path) {
  current = path;
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

  const n = path.length;
  const margin = 70;
  const usableW = Math.max(rect.width - margin * 2, 1);
  const midY = rect.height / 2;
  const r = 30;
  const pos = path.map((_, i) => ({
    x: n === 1 ? rect.width / 2 : margin + (usableW * i) / (n - 1),
    y: midY + (i % 2 === 0 ? -1 : 1) * (n > 1 ? 34 : 0),
  }));

  // edges
  for (let i = 0; i < n - 1; i++) {
    const a = pos[i], b = pos[i + 1];
    const ang = Math.atan2(b.y - a.y, b.x - a.x);
    const sx = a.x + Math.cos(ang) * r, sy = a.y + Math.sin(ang) * r;
    const ex = b.x - Math.cos(ang) * r, ey = b.y - Math.sin(ang) * r;
    ctx.strokeStyle = "#3a4554"; ctx.lineWidth = 2;
    ctx.beginPath(); ctx.moveTo(sx, sy); ctx.lineTo(ex, ey); ctx.stroke();
    const h = 9;
    ctx.fillStyle = "#3a4554";
    ctx.beginPath();
    ctx.moveTo(ex, ey);
    ctx.lineTo(ex - h * Math.cos(ang - 0.4), ey - h * Math.sin(ang - 0.4));
    ctx.lineTo(ex - h * Math.cos(ang + 0.4), ey - h * Math.sin(ang + 0.4));
    ctx.closePath(); ctx.fill();
  }

  // nodes
  path.forEach((id, i) => {
    const p = pos[i];
    const color = palette[i % palette.length];
    ctx.beginPath(); ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
    ctx.fillStyle = color; ctx.fill();
    ctx.lineWidth = 2.5; ctx.strokeStyle = color; ctx.stroke();
    ctx.fillStyle = "#0e1116"; ctx.font = "bold 22px system-ui";
    ctx.textAlign = "center"; ctx.textBaseline = "middle";
    ctx.fillText(GRAPH[id].name.replace(/[^A-Za-z]/g, "").slice(0, 1).toUpperCase() || "?", p.x, p.y + 1);
    ctx.fillStyle = "#e6edf3"; ctx.font = "13px system-ui"; ctx.textBaseline = "top";
    ctx.fillText(GRAPH[id].name, p.x, p.y + r + 8);
  });
}

// wire up
$("resolve").addEventListener("click", run);
$("addr").addEventListener("keydown", (e) => { if (e.key === "Enter") run(); });
document.querySelectorAll(".chip").forEach((c) =>
  c.addEventListener("click", () => { $("addr").value = c.dataset.addr; run(); })
);
window.addEventListener("resize", () => draw(current));
run();
