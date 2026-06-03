"use strict";

const $ = (id) => document.getElementById(id);
const form = $("resolve-form");
const statusEl = $("status");
const addressEl = $("address");
const detailsEl = $("details");
const hopsEl = $("hops");
const modeEl = $("mode");
const demoBtn = $("demo-btn");

let config = { demo: false, relays: [] };

// ---- Bootstrap: learn whether we're in demo mode and pre-fill examples. ----
fetch("/api/config")
  .then((r) => r.json())
  .then((c) => {
    config = c;
    if (c.demo) {
      modeEl.textContent = "DEMO MODE — built-in fixture graph (no relays)";
      modeEl.classList.add("demo");
      demoBtn.hidden = false;
    } else {
      modeEl.textContent = `${(c.relays || []).length} relays configured`;
    }
  })
  .catch(() => {
    modeEl.textContent = "config unavailable";
  });

demoBtn.addEventListener("click", () => {
  if (config.demo_from) $("from").value = config.demo_from;
  if (config.demo_to) $("to").value = config.demo_to;
});

// ---- Resolve ----
form.addEventListener("submit", async (e) => {
  e.preventDefault();
  const from = $("from").value.trim();
  const to = $("to").value.trim();
  const maxDepth = $("max_depth").value;
  if (!from || !to) {
    setStatus("Enter both a from and a to pubkey.", "err");
    return;
  }
  setStatus("Resolving path through the graph…", "");
  addressEl.textContent = "";
  detailsEl.hidden = true;
  $("resolve-btn").disabled = true;

  try {
    const params = new URLSearchParams({ from, to, max_depth: maxDepth });
    const res = await fetch(`/api/resolve?${params}`);
    const data = await res.json();
    if (!res.ok) {
      setStatus(data.error || "Resolution failed.", "err");
      graph.setData([], []);
      return;
    }
    render(data);
  } catch (err) {
    setStatus(String(err), "err");
  } finally {
    $("resolve-btn").disabled = false;
  }
});

function setStatus(msg, cls) {
  statusEl.textContent = msg;
  statusEl.className = "status" + (cls ? " " + cls : "");
}

function shortNpub(npub) {
  return npub.length > 18 ? `${npub.slice(0, 12)}…${npub.slice(-6)}` : npub;
}

function nodeLabel(node) {
  const p = node.profile || {};
  return p.display_name || p.name || shortNpub(node.npub);
}

// Build a human GNS address: target@<reversed middle path>.nostr
function gnsAddress(path) {
  if (path.length < 2) return "";
  const slug = (n) => {
    const p = n.profile || {};
    return (p.name || p.display_name || n.npub.slice(0, 8)).toLowerCase().replace(/\s+/g, "");
  };
  const target = slug(path[path.length - 1]);
  const middle = path.slice(1, path.length - 1).map(slug).reverse();
  const graphPath = middle.length ? middle.join(".") + "." : "";
  return `${target}@${graphPath}nostr`;
}

function render(data) {
  if (!data.found) {
    setStatus(`No path found within depth (${data.visited} pubkeys explored).`, "err");
    addressEl.textContent = "";
    detailsEl.hidden = true;
    graph.setData([], []);
    return;
  }
  setStatus(`Path found — ${data.hops} hop${data.hops === 1 ? "" : "s"}, ${data.visited} pubkeys explored.`, "ok");
  addressEl.textContent = gnsAddress(data.path);

  // Details list: one entry per node, with the inbound edge's provenance.
  hopsEl.innerHTML = "";
  data.path.forEach((node, i) => {
    const li = document.createElement("li");
    const name = document.createElement("div");
    name.className = "hop-name";
    name.textContent = nodeLabel(node);
    const npub = document.createElement("div");
    npub.className = "hop-npub";
    npub.textContent = node.npub;
    li.appendChild(name);
    li.appendChild(npub);

    if (i > 0) {
      const edge = data.edges[i - 1];
      const meta = document.createElement("div");
      meta.className = "edge-meta";
      const ev = document.createElement("div");
      ev.innerHTML = `follow event <code>${edge.follow_event_id.slice(0, 16)}…</code>`;
      meta.appendChild(ev);
      const relays = document.createElement("div");
      (edge.relays || []).forEach((r) => {
        const chip = document.createElement("span");
        chip.className = "relay-chip";
        chip.textContent = r;
        relays.appendChild(chip);
      });
      if (!edge.relays || edge.relays.length === 0) {
        relays.textContent = "(no relay attribution)";
      }
      meta.appendChild(relays);
      li.appendChild(meta);
    }
    hopsEl.appendChild(li);
  });
  detailsEl.hidden = false;

  graph.setData(data.path, data.edges);
}

// ---------------------------------------------------------------------------
// Canvas graph: avatar bubbles connected along the resolved path.
// ---------------------------------------------------------------------------
const graph = (() => {
  const canvas = $("graph");
  const ctx = canvas.getContext("2d");
  let nodes = [];
  let edges = [];
  const images = new Map();

  const palette = ["#8a5cf6", "#38bdf8", "#3fb950", "#d29922", "#f778ba", "#f85149", "#a371f7"];

  function resize() {
    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    layout();
    draw();
  }

  function layout() {
    const rect = canvas.getBoundingClientRect();
    const n = nodes.length;
    if (n === 0) return;
    const margin = 90;
    const usableW = Math.max(rect.width - margin * 2, 1);
    const midY = rect.height / 2;
    nodes.forEach((node, i) => {
      node.x = n === 1 ? rect.width / 2 : margin + (usableW * i) / (n - 1);
      // gentle vertical zig-zag so edges read as a walk, not a flat line
      node.y = midY + (i % 2 === 0 ? -1 : 1) * Math.min(60, rect.height / 6) * (n > 1 ? 1 : 0);
      node.r = 34;
      node.color = palette[i % palette.length];
    });
  }

  function loadImage(url) {
    if (!url || images.has(url)) return;
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => { images.set(url, img); draw(); };
    img.onerror = () => { images.set(url, null); };
    images.set(url, "loading");
    img.src = url;
  }

  function draw() {
    const rect = canvas.getBoundingClientRect();
    ctx.clearRect(0, 0, rect.width, rect.height);
    if (nodes.length === 0) {
      ctx.fillStyle = "#5b6675";
      ctx.font = "15px system-ui";
      ctx.textAlign = "center";
      ctx.fillText("Resolve a path to see the graph", rect.width / 2, rect.height / 2);
      return;
    }

    // edges with arrowheads
    edges.forEach((_edge, i) => {
      const a = nodes[i];
      const b = nodes[i + 1];
      if (!a || !b) return;
      drawArrow(a, b);
    });

    // nodes
    nodes.forEach((node) => drawNode(node));
  }

  function drawArrow(a, b) {
    const ang = Math.atan2(b.y - a.y, b.x - a.x);
    const sx = a.x + Math.cos(ang) * a.r;
    const sy = a.y + Math.sin(ang) * a.r;
    const ex = b.x - Math.cos(ang) * b.r;
    const ey = b.y - Math.sin(ang) * b.r;
    ctx.strokeStyle = "#3a4554";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(sx, sy);
    ctx.lineTo(ex, ey);
    ctx.stroke();
    // arrowhead
    const head = 9;
    ctx.fillStyle = "#3a4554";
    ctx.beginPath();
    ctx.moveTo(ex, ey);
    ctx.lineTo(ex - head * Math.cos(ang - 0.4), ey - head * Math.sin(ang - 0.4));
    ctx.lineTo(ex - head * Math.cos(ang + 0.4), ey - head * Math.sin(ang + 0.4));
    ctx.closePath();
    ctx.fill();
  }

  function drawNode(node) {
    const p = node.profile || {};
    const img = p.picture ? images.get(p.picture) : null;

    ctx.save();
    ctx.beginPath();
    ctx.arc(node.x, node.y, node.r, 0, Math.PI * 2);
    ctx.closePath();

    // glow ring
    ctx.shadowColor = node.color;
    ctx.shadowBlur = 16;
    ctx.fillStyle = "#0e1116";
    ctx.fill();
    ctx.shadowBlur = 0;

    if (img && img !== "loading") {
      ctx.save();
      ctx.clip();
      ctx.drawImage(img, node.x - node.r, node.y - node.r, node.r * 2, node.r * 2);
      ctx.restore();
    } else {
      ctx.fillStyle = node.color;
      ctx.fill();
      ctx.fillStyle = "#0e1116";
      ctx.font = "bold 26px system-ui";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      const label = nodeLabel(node);
      ctx.fillText(label.slice(0, 1).toUpperCase(), node.x, node.y + 1);
    }

    ctx.lineWidth = 2.5;
    ctx.strokeStyle = node.color;
    ctx.stroke();
    ctx.restore();

    // caption
    ctx.fillStyle = "#e6edf3";
    ctx.font = "13px system-ui";
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    ctx.fillText(nodeLabel(node), node.x, node.y + node.r + 8);
    ctx.fillStyle = "#8b97a7";
    ctx.font = "11px ui-monospace, monospace";
    ctx.fillText(shortNpub(node.npub), node.x, node.y + node.r + 26);
  }

  function setData(path, e) {
    nodes = (path || []).map((n) => ({ ...n }));
    edges = e || [];
    nodes.forEach((n) => { if (n.profile && n.profile.picture) loadImage(n.profile.picture); });
    layout();
    draw();
  }

  window.addEventListener("resize", resize);
  // initial sizing once layout settles
  requestAnimationFrame(resize);

  return { setData };
})();
