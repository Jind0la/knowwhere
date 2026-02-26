const API = window.location.origin;

async function api(method, path, body) {
  const opts = { method, headers: { "Content-Type": "application/json" } };
  if (body) opts.body = JSON.stringify(body);
  const resp = await fetch(`${API}${path}`, opts);
  if (!resp.ok) throw new Error(`${resp.status} ${await resp.text()}`);
  return resp.json();
}

function $(id) { return document.getElementById(id); }

function timeAgo(iso) {
  if (!iso) return "never";
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  return `${hrs}h ${mins % 60}m ago`;
}

function truncate(s, n) {
  return s && s.length > n ? s.slice(0, n) + "…" : (s || "");
}

function nodeCard(node) {
  const isSession = !!node.content;
  const label = isSession ? truncate(node.content, 120) : `pointer: ${truncate(node.original_pointer, 100)}`;
  const badge = isSession
    ? '<span class="text-xs bg-indigo-900 text-indigo-300 px-2 py-0.5 rounded">session</span>'
    : '<span class="text-xs bg-emerald-900 text-emerald-300 px-2 py-0.5 rounded">external</span>';
  const mm = node.multimodal ? ` <span class="text-xs text-gray-500">[${node.multimodal.type}]</span>` : "";
  const ts = timeAgo(node.created_at);

  return `<div class="bg-kw-700 rounded-lg px-4 py-3 border border-kw-600">
    <div class="flex items-start justify-between gap-2">
      <div class="flex-1 min-w-0">
        <div class="flex items-center gap-2 mb-1">${badge}${mm}<span class="text-xs text-gray-500">${ts}</span></div>
        <p class="text-sm text-gray-300 break-words">${label}</p>
      </div>
      <code class="text-[10px] text-gray-600 shrink-0">${node.id.slice(0, 8)}</code>
    </div>
  </div>`;
}

async function refreshHealth() {
  try {
    const h = await api("GET", "/health");
    $("health-dot").className = "w-2 h-2 rounded-full bg-green-400";
    $("health-text").textContent = "online";
    $("stat-status").textContent = h.status;
    $("stat-nodes").textContent = h.node_count;
  } catch {
    $("health-dot").className = "w-2 h-2 rounded-full bg-red-400";
    $("health-text").textContent = "offline";
    $("stat-status").textContent = "offline";
  }
}

async function refreshDream() {
  try {
    const d = await api("GET", "/dream/status");
    $("stat-dreams").textContent = d.cycle_count;
    $("stat-dream-last").textContent = `last run: ${timeAgo(d.last_run)}`;
  } catch { /* ignore */ }
}

async function refreshRecent() {
  const container = $("recent-nodes");
  try {
    const nodes = await api("GET", "/nodes/recent?limit=20");
    if (!nodes.length) {
      container.innerHTML = '<p class="text-gray-500 text-sm">No nodes yet. Store some data via the SDK!</p>';
      return;
    }
    container.innerHTML = nodes.map(nodeCard).join("");
  } catch {
    container.innerHTML = '<p class="text-gray-500 text-sm">Could not load nodes.</p>';
  }
}

async function doSearch() {
  const query = $("search-input").value.trim();
  if (!query) return;

  const container = $("search-results");
  container.classList.remove("hidden");
  container.innerHTML = '<p class="text-gray-500 text-sm">Searching…</p>';

  try {
    const embed = await api("POST", "/embed", { text: query });
    const results = await api("POST", "/retrieve_fractal", {
      query_vector: embed.vector,
      top_k: 5,
      max_depth: 3,
    });
    if (!results.length) {
      container.innerHTML = '<p class="text-gray-500 text-sm">No results found.</p>';
      return;
    }
    container.innerHTML = results.map(nodeCard).join("");
  } catch (e) {
    container.innerHTML = `<p class="text-red-400 text-sm">Search failed: ${e.message}</p>`;
  }
}

$("search-btn").addEventListener("click", doSearch);
$("search-input").addEventListener("keydown", (e) => { if (e.key === "Enter") doSearch(); });
$("refresh-btn").addEventListener("click", () => { refreshRecent(); refreshHealth(); refreshDream(); });

async function init() {
  await Promise.all([refreshHealth(), refreshDream(), refreshRecent()]);
  setInterval(() => { refreshHealth(); refreshDream(); }, 10000);
}

init();
