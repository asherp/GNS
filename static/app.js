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
let mode = "name"; // "name" | "pubkey"

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

// ---- Mode tabs ----
// Switch the active tab and toggle the fields/panels that belong to it.
function activateMode(newMode) {
  mode = newMode;
  document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("active", t.dataset.mode === mode));
  // `data-fields` is a space-separated list of modes the field belongs to.
  document.querySelectorAll(".mode-fields").forEach((f) => {
    f.hidden = !f.dataset.fields.split(/\s+/).includes(mode);
  });
  // Followers has its own result panel and no path canvas.
  const followers = mode === "followers";
  $("viz").hidden = followers;
  $("resolve-btn").textContent = followers ? "Fetch followers" : "Resolve";
}

document.querySelectorAll(".tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    activateMode(tab.dataset.mode);
    setStatus("", "");
  });
});

demoBtn.addEventListener("click", () => {
  if (config.demo_from) $("from").value = config.demo_from;
  if (mode === "name") {
    if (config.demo_name) $("name").value = config.demo_name;
  } else if (mode === "followers") {
    if (config.demo_from) $("follower-pubkey").value = config.demo_from;
  } else if (config.demo_to) {
    $("to").value = config.demo_to;
  }
});

// ---- Resolve ----
form.addEventListener("submit", async (e) => {
  e.preventDefault();

  if (mode === "followers") {
    const pubkey = $("follower-pubkey").value.trim();
    if (!pubkey) {
      setStatus("Enter a pubkey (npub or hex).", "err");
      return;
    }
    // Push a shareable, back-button-friendly URL; the popstate/boot handlers
    // below re-render from it. Skip the push if we're already on this pubkey.
    const params = new URLSearchParams(location.search);
    if (!(params.get("tab") === "followers" && params.get("pubkey") === pubkey)) {
      history.pushState({ tab: "followers", pubkey }, "", followersUrl(pubkey));
    }
    await fetchFollowers(true);
    return;
  }

  const from = $("from").value.trim();
  if (!from) {
    setStatus("Enter your `from` pubkey.", "err");
    return;
  }

  addressEl.textContent = "";
  detailsEl.hidden = true;
  $("resolve-btn").disabled = true;

  try {
    if (mode === "name") {
      const name = $("name").value.trim();
      if (!name) {
        setStatus("Enter a GNS name to resolve.", "err");
        return;
      }
      setStatus("Walking the graph by name…", "");
      const params = new URLSearchParams({ from, name });
      const res = await fetch(`/api/resolve-name?${params}`);
      const data = await res.json();
      if (!res.ok) {
        setStatus(data.error || "Resolution failed.", "err");
        graph.setData([], []);
        return;
      }
      renderName(data);
    } else {
      const to = $("to").value.trim();
      const maxDepth = $("max_depth").value;
      if (!to) {
        setStatus("Enter a `to` pubkey.", "err");
        return;
      }
      setStatus("Resolving path through the graph…", "");
      const params = new URLSearchParams({ from, to, max_depth: maxDepth });
      const res = await fetch(`/api/resolve?${params}`);
      const data = await res.json();
      if (!res.ok) {
        setStatus(data.error || "Resolution failed.", "err");
        graph.setData([], []);
        return;
      }
      render(data);
    }
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

// ---------------------------------------------------------------------------
// Followers tab: page through /api/followers and show each follow's provenance.
// Followers are a reverse `#p` tag lookup on relays — no graph walk, no depth.
// ---------------------------------------------------------------------------
const FOLLOWER_PAGE = 25;
const followersHeadEl = $("followers-head");
const followerListEl = $("follower-list");
const loadMoreBtn = $("load-more");
let followerState = { pubkey: "", offset: 0, count: 0 };

loadMoreBtn.addEventListener("click", () => fetchFollowers(false));

async function fetchFollowers(reset) {
  const pubkey = reset ? $("follower-pubkey").value.trim() : followerState.pubkey;
  if (!pubkey) {
    setStatus("Enter a pubkey (npub or hex).", "err");
    return;
  }
  if (reset) {
    followerState = { pubkey, offset: 0, count: 0 };
    followerListEl.innerHTML = "";
    followersHeadEl.textContent = "";
    loadMoreBtn.hidden = true;
  }

  detailsEl.hidden = true;
  $("followers-result").hidden = false;
  $("resolve-btn").disabled = true;
  loadMoreBtn.disabled = true;
  setStatus(reset ? "Querying relays for followers…" : "Loading more…", "");

  try {
    const params = new URLSearchParams({
      pubkey: followerState.pubkey,
      limit: FOLLOWER_PAGE,
      offset: followerState.offset,
    });
    const res = await fetch(`/api/followers?${params}`);
    const data = await res.json();
    if (!res.ok) {
      setStatus(data.error || "Failed to fetch followers.", "err");
      return;
    }

    followerState.count = data.count;
    renderFollowersHead(data);
    (data.followers || []).forEach((f) => {
      followerListEl.appendChild(followerRow(f));
      enrichFollower(f); // progressively fill in display name + avatar
    });
    followerState.offset += (data.followers || []).length;

    const shown = followerState.offset;
    loadMoreBtn.hidden = shown >= data.count;
    loadMoreBtn.textContent = `Load ${Math.min(FOLLOWER_PAGE, data.count - shown)} more`;
    setStatus(`Showing ${shown} of ${data.count} follower${data.count === 1 ? "" : "s"}.`, "ok");
  } catch (err) {
    setStatus(String(err), "err");
  } finally {
    $("resolve-btn").disabled = false;
    loadMoreBtn.disabled = false;
  }
}

function renderFollowersHead(data) {
  followersHeadEl.innerHTML = "";
  const title = document.createElement("div");
  title.className = "followers-title";
  title.textContent = `Followers of ${shortNpub(data.npub)}`;
  const meta = document.createElement("div");
  meta.className = "followers-meta";
  const relays = (config.relays || []).length;
  meta.textContent = data.best_effort
    ? `${data.count} found · best-effort across ${relays} relay${relays === 1 ? "" : "s"}`
    : `${data.count} found`;
  followersHeadEl.appendChild(title);
  followersHeadEl.appendChild(meta);
}

function followerRow(f) {
  const li = document.createElement("li");
  li.className = "follower";
  li.dataset.pubkey = f.pubkey;

  const avatar = document.createElement("div");
  avatar.className = "follower-avatar";
  avatar.textContent = "·";

  const body = document.createElement("div");
  body.className = "follower-body";

  const name = document.createElement("div");
  name.className = "follower-name";
  // Keep the display text in its own span so async profile enrichment can
  // replace the name without clobbering the mutual pill (a sibling element).
  const nameText = document.createElement("span");
  nameText.className = "follower-name-text";
  nameText.textContent = shortNpub(f.npub);
  name.appendChild(nameText);
  if (f.mutual) {
    const pill = document.createElement("span");
    pill.className = "mutual-pill";
    pill.textContent = "mutual";
    pill.title = "You follow each other";
    name.appendChild(pill);
  }

  const npub = document.createElement("div");
  npub.className = "follower-npub hop-npub";
  npub.textContent = f.npub;

  const meta = document.createElement("div");
  meta.className = "edge-meta";
  const ev = document.createElement("div");
  ev.innerHTML = `follow event <code>${f.follow_event_id.slice(0, 16)}…</code>`;
  if (f.created_at) {
    const when = document.createElement("span");
    when.className = "follow-when";
    when.textContent = ` · ${new Date(f.created_at * 1000).toLocaleDateString()}`;
    ev.appendChild(when);
  }
  meta.appendChild(ev);
  const relays = document.createElement("div");
  if (f.relays && f.relays.length) {
    f.relays.forEach((r) => {
      const chip = document.createElement("span");
      chip.className = "relay-chip";
      chip.textContent = r;
      relays.appendChild(chip);
    });
  } else {
    relays.textContent = "(no relay attribution)";
  }
  meta.appendChild(relays);

  body.appendChild(name);
  body.appendChild(npub);
  body.appendChild(meta);
  li.appendChild(avatar);
  li.appendChild(body);
  return li;
}

// Best-effort, non-blocking profile lookup to swap the short npub for a real
// display name + avatar once the relay round-trip returns. Failures are silent.
async function enrichFollower(f) {
  try {
    const res = await fetch(`/api/profile?pubkey=${encodeURIComponent(f.pubkey)}`);
    if (!res.ok) return;
    const { profile } = await res.json();
    if (!profile) return;
    const row = followerListEl.querySelector(`li[data-pubkey="${f.pubkey}"]`);
    if (!row) return;
    const label = profile.display_name || profile.name;
    if (label) row.querySelector(".follower-name-text").textContent = label;
    if (profile.picture) {
      const avatar = row.querySelector(".follower-avatar");
      const img = new Image();
      img.alt = "";
      img.onload = () => { avatar.textContent = ""; avatar.appendChild(img); };
      img.src = profile.picture;
    } else if (label) {
      row.querySelector(".follower-avatar").textContent = label.slice(0, 1).toUpperCase();
    }
  } catch (_) {
    /* enrichment is best-effort; keep the npub fallback */
  }
}

// ---------------------------------------------------------------------------
// Deep-linking: ?tab=followers&pubkey=<npub|hex> so the view is shareable and
// the back/forward buttons work.
// ---------------------------------------------------------------------------
function followersUrl(pubkey) {
  return `?tab=followers&pubkey=${encodeURIComponent(pubkey)}`;
}

// Render whatever the current URL describes. Called on first load and on
// back/forward navigation; it never pushes history itself.
function applyRoute() {
  const params = new URLSearchParams(location.search);
  const pubkey = params.get("pubkey");
  if (params.get("tab") === "followers" && pubkey) {
    activateMode("followers");
    $("follower-pubkey").value = pubkey;
    // Skip a redundant relay round-trip if this list is already on screen.
    if (followerState.pubkey !== pubkey || $("followers-result").hidden) {
      fetchFollowers(true);
    }
  } else {
    // Not a followers route — fall back to the default path view.
    if (mode === "followers") activateMode("name");
    $("followers-result").hidden = true;
    setStatus("", "");
  }
}

window.addEventListener("popstate", applyRoute);
applyRoute(); // honor a deep link on first page load

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

function renderName(data) {
  hopsEl.innerHTML = "";
  addressEl.textContent = data.query || "";

  if (!data.found) {
    setStatus(data.note || "No path found for that name.", "err");
    detailsEl.hidden = true;
    graph.setData([], []);
    return;
  }

  // Banner summarising resolution outcome.
  const banner = document.createElement("div");
  if (data.ambiguous) {
    banner.className = "banner warn";
    banner.textContent = data.note || "Ambiguous — does not resolve to a single pubkey.";
    setStatus(`Ambiguous label — ${data.paths.length} alternate paths.`, "err");
  } else {
    banner.className = "banner ok";
    banner.textContent = `Resolved to ${data.resolved}`;
    setStatus(`Resolved "${data.target_label}" — ${data.paths[0].edges.length} hop(s).`, "ok");
  }
  hopsEl.appendChild(banner);

  // Render each path (one when unambiguous; alternates when ambiguous).
  data.paths.forEach((path, idx) => {
    const wrap = document.createElement("div");
    wrap.className = "alt-path";
    if (data.paths.length > 1) {
      const h = document.createElement("h3");
      h.textContent = `Alternate ${idx + 1}`;
      wrap.appendChild(h);
    }
    const list = document.createElement("ol");
    path.nodes.forEach((node, i) => {
      const li = document.createElement("li");
      const name = document.createElement("div");
      name.className = "hop-name";
      name.textContent = nodeLabel(node);
      if (node.label) {
        const pill = document.createElement("span");
        pill.className = "label-pill";
        pill.textContent = node.label;
        name.appendChild(pill);
      }
      const npub = document.createElement("div");
      npub.className = "hop-npub";
      npub.textContent = node.npub;
      li.appendChild(name);
      li.appendChild(npub);

      if (i > 0) {
        const edge = path.edges[i - 1];
        const meta = document.createElement("div");
        meta.className = "edge-meta";
        meta.innerHTML = `follow event <code>${edge.follow_event_id.slice(0, 16)}…</code>`;
        const relays = document.createElement("div");
        (edge.relays || []).forEach((r) => {
          const chip = document.createElement("span");
          chip.className = "relay-chip";
          chip.textContent = r;
          relays.appendChild(chip);
        });
        meta.appendChild(relays);
        li.appendChild(meta);
      }
      list.appendChild(li);
    });
    wrap.appendChild(list);
    hopsEl.appendChild(wrap);
  });
  detailsEl.hidden = false;

  // Draw the first path in the canvas.
  const primary = data.paths[0];
  graph.setData(primary.nodes, primary.edges);
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
