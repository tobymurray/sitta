//! Embedded HTML dashboard pages.
//!
//! Each page is rendered by wrapping page-specific content in a shared shell
//! (sidebar, header, Tailwind/htmx CDN scripts). No template engine — just
//! `format!()` with a layout string.

use axum::response::Html;

/// Render a full HTML page with the shared shell.
pub fn page(title: &str, active: &str, content: &str) -> Html<String> {
    Html(format!(
        r##"<!DOCTYPE html>
<html lang="en" class="h-full">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — Sitta</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
<script src="https://cdn.tailwindcss.com"></script>
<script>
tailwind.config = {{
  darkMode: 'class',
  theme: {{
    extend: {{
      fontFamily: {{ sans: ['Inter', 'system-ui', 'sans-serif'] }},
    }},
  }},
}}
</script>
<style>
  @keyframes slideIn {{ from {{ opacity: 0; transform: translateY(-8px); }} to {{ opacity: 1; transform: translateY(0); }} }}
  .slide-in {{ animation: slideIn 0.3s ease-out; }}
</style>
<script>
  if (window.matchMedia('(prefers-color-scheme: dark)').matches) document.documentElement.classList.add('dark');
</script>
</head>
<body class="h-full bg-gray-50 dark:bg-slate-950 font-sans text-gray-900 dark:text-slate-100">
<div class="flex h-full">

  <!-- Sidebar -->
  <nav class="hidden lg:flex lg:flex-col lg:w-60 bg-white dark:bg-slate-900 border-r border-gray-200 dark:border-slate-800">
    <div class="flex items-center gap-2.5 px-5 py-5 border-b border-gray-200 dark:border-slate-800">
      <div class="w-8 h-8 rounded-lg bg-blue-600 flex items-center justify-center">
        <svg class="w-5 h-5 text-white" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M12 3c-1.5 0-3 .5-4 2-1.5 2-1 5 1 7l3 3 3-3c2-2 2.5-5 1-7-1-1.5-2.5-2-4-2z"/></svg>
      </div>
      <span class="text-lg font-bold tracking-tight">Sitta</span>
    </div>
    <div class="flex-1 py-4 px-3 space-y-1">
      {nav_dashboard}
      {nav_species}
      {nav_status}
    </div>
    <div class="px-5 py-4 border-t border-gray-200 dark:border-slate-800 text-xs text-gray-400 dark:text-slate-600">
      Sitta v0.1.0
    </div>
  </nav>

  <!-- Main content -->
  <div class="flex-1 flex flex-col min-w-0">

    <!-- Mobile header -->
    <header class="lg:hidden flex items-center gap-3 px-4 py-3 bg-white dark:bg-slate-900 border-b border-gray-200 dark:border-slate-800">
      <div class="w-7 h-7 rounded-md bg-blue-600 flex items-center justify-center">
        <svg class="w-4 h-4 text-white" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M12 3c-1.5 0-3 .5-4 2-1.5 2-1 5 1 7l3 3 3-3c2-2 2.5-5 1-7-1-1.5-2.5-2-4-2z"/></svg>
      </div>
      <span class="font-bold">Sitta</span>
      <nav class="ml-auto flex gap-1">
        <a href="/" class="px-2.5 py-1.5 text-sm rounded-md {mob_dashboard}">Live</a>
        <a href="/species" class="px-2.5 py-1.5 text-sm rounded-md {mob_species}">Species</a>
        <a href="/status" class="px-2.5 py-1.5 text-sm rounded-md {mob_status}">Status</a>
      </nav>
    </header>

    <!-- Page content -->
    <main class="flex-1 overflow-y-auto">
      <div class="max-w-6xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
        {content}
      </div>
    </main>
  </div>
</div>
</body>
</html>"##,
        title = title,
        content = content,
        nav_dashboard = nav_item("Dashboard", "/", "dashboard", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M3.75 6A2.25 2.25 0 016 3.75h2.25A2.25 2.25 0 0110.5 6v2.25a2.25 2.25 0 01-2.25 2.25H6a2.25 2.25 0 01-2.25-2.25V6zM3.75 15.75A2.25 2.25 0 016 13.5h2.25a2.25 2.25 0 012.25 2.25V18a2.25 2.25 0 01-2.25 2.25H6A2.25 2.25 0 013.75 18v-2.25zM13.5 6a2.25 2.25 0 012.25-2.25H18A2.25 2.25 0 0120.25 6v2.25A2.25 2.25 0 0118 10.5h-2.25a2.25 2.25 0 01-2.25-2.25V6zM13.5 15.75a2.25 2.25 0 012.25-2.25H18a2.25 2.25 0 012.25 2.25V18A2.25 2.25 0 0118 20.25h-2.25A2.25 2.25 0 0113.5 18v-2.25z"/>"#),
        nav_species = nav_item("Species", "/species", "species", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M3.75 12h16.5m-16.5 3.75h16.5M3.75 19.5h16.5M5.625 4.5h12.75a1.875 1.875 0 010 3.75H5.625a1.875 1.875 0 010-3.75z"/>"#),
        nav_status = nav_item("Status", "/status", "status", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M9.348 14.651a3.75 3.75 0 010-5.303m5.304 0a3.75 3.75 0 010 5.303m-7.425 2.122a6.75 6.75 0 010-9.546m9.546 0a6.75 6.75 0 010 9.546M5.106 18.894c-3.808-3.808-3.808-9.98 0-13.788m13.788 0c3.808 3.808 3.808 9.98 0 13.788M12 12h.008v.008H12V12zm.375 0a.375.375 0 11-.75 0 .375.375 0 01.75 0z"/>"#),
        mob_dashboard = if active == "dashboard" { "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400 font-medium" } else { "text-gray-600 dark:text-slate-400" },
        mob_species = if active == "species" { "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400 font-medium" } else { "text-gray-600 dark:text-slate-400" },
        mob_status = if active == "status" { "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400 font-medium" } else { "text-gray-600 dark:text-slate-400" },
    ))
}

fn nav_item(label: &str, href: &str, key: &str, active: &str, icon_path: &str) -> String {
    let (bg, text) = if key == active {
        ("bg-blue-50 dark:bg-blue-900/20", "text-blue-700 dark:text-blue-400 font-medium")
    } else {
        ("hover:bg-gray-100 dark:hover:bg-slate-800", "text-gray-700 dark:text-slate-300")
    };
    format!(
        r#"<a href="{href}" class="flex items-center gap-3 px-3 py-2 rounded-lg text-sm {bg} {text} transition-colors">
        <svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24">{icon_path}</svg>
        {label}
      </a>"#,
    )
}

// ── Page content fragments ──────────────────────────────────────

pub fn dashboard_content(station_name: &str) -> String {
    format!(
        r##"<div class="flex items-center justify-between mb-6">
  <div>
    <h1 class="text-2xl font-bold tracking-tight">{station_name}</h1>
    <p class="text-sm text-gray-500 dark:text-slate-400 mt-0.5">Live detection feed</p>
  </div>
  <div id="connection-status" class="flex items-center gap-2 text-sm text-gray-400 dark:text-slate-500">
    <span class="relative flex h-2.5 w-2.5"><span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-75"></span><span class="relative inline-flex rounded-full h-2.5 w-2.5 bg-amber-500"></span></span>
    Connecting...
  </div>
</div>

<!-- Stats row -->
<div id="stats" class="grid grid-cols-2 sm:grid-cols-4 gap-4 mb-6">
  <div class="bg-white dark:bg-slate-900 rounded-xl border border-gray-200 dark:border-slate-800 px-4 py-3">
    <p class="text-xs font-medium text-gray-500 dark:text-slate-400 uppercase tracking-wider">Today</p>
    <p id="stat-today" class="text-2xl font-bold mt-1">--</p>
  </div>
  <div class="bg-white dark:bg-slate-900 rounded-xl border border-gray-200 dark:border-slate-800 px-4 py-3">
    <p class="text-xs font-medium text-gray-500 dark:text-slate-400 uppercase tracking-wider">Species</p>
    <p id="stat-species" class="text-2xl font-bold mt-1">--</p>
  </div>
  <div class="bg-white dark:bg-slate-900 rounded-xl border border-gray-200 dark:border-slate-800 px-4 py-3">
    <p class="text-xs font-medium text-gray-500 dark:text-slate-400 uppercase tracking-wider">Top Species</p>
    <p id="stat-top" class="text-lg font-semibold mt-1 truncate">--</p>
  </div>
  <div class="bg-white dark:bg-slate-900 rounded-xl border border-gray-200 dark:border-slate-800 px-4 py-3">
    <p class="text-xs font-medium text-gray-500 dark:text-slate-400 uppercase tracking-wider">Avg Confidence</p>
    <p id="stat-conf" class="text-2xl font-bold mt-1">--</p>
  </div>
</div>

<!-- Live detection feed -->
<div class="flex items-center justify-between mb-4">
  <h2 class="text-lg font-semibold">Recent Detections</h2>
  <span id="detection-count" class="text-sm text-gray-400 dark:text-slate-500"></span>
</div>
<div id="live-feed" class="space-y-3">
  <div id="empty-state" class="text-center py-16 text-gray-400 dark:text-slate-500">
    <svg class="w-12 h-12 mx-auto mb-3 opacity-50" fill="none" stroke="currentColor" stroke-width="1" viewBox="0 0 24 24"><path d="M12 3c-1.5 0-3 .5-4 2-1.5 2-1 5 1 7l3 3 3-3c2-2 2.5-5 1-7-1-1.5-2.5-2-4-2z"/></svg>
    <p class="text-sm">Waiting for detections...</p>
    <p class="text-xs mt-1">Detections will appear here as they are identified</p>
  </div>
</div>

<script>
(function() {{
  const feed = document.getElementById('live-feed');
  const emptyState = document.getElementById('empty-state');
  const connStatus = document.getElementById('connection-status');
  let count = 0;

  function setConnected(ok) {{
    connStatus.innerHTML = ok
      ? '<span class="relative flex h-2.5 w-2.5"><span class="relative inline-flex rounded-full h-2.5 w-2.5 bg-emerald-500"></span></span> Connected'
      : '<span class="relative flex h-2.5 w-2.5"><span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-75"></span><span class="relative inline-flex rounded-full h-2.5 w-2.5 bg-amber-500"></span></span> Connecting...';
  }}

  function confColor(c) {{
    if (c >= 0.8) return ['text-emerald-700 bg-emerald-50 ring-emerald-600/20 dark:text-emerald-400 dark:bg-emerald-900/30 dark:ring-emerald-400/20', 'bg-emerald-500'];
    if (c >= 0.5) return ['text-amber-700 bg-amber-50 ring-amber-600/20 dark:text-amber-400 dark:bg-amber-900/30 dark:ring-amber-400/20', 'bg-amber-500'];
    return ['text-red-700 bg-red-50 ring-red-600/20 dark:text-red-400 dark:bg-red-900/30 dark:ring-red-400/20', 'bg-red-500'];
  }}

  function timeAgo(iso) {{
    const s = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
    if (s < 5) return 'just now';
    if (s < 60) return s + 's ago';
    if (s < 3600) return Math.floor(s/60) + 'm ago';
    return new Date(iso).toLocaleTimeString([], {{hour:'2-digit', minute:'2-digit'}});
  }}

  function createCard(d) {{
    const [badge, bar] = confColor(d.confidence);
    const pct = Math.round(d.confidence * 100);
    const card = document.createElement('div');
    card.className = 'slide-in bg-white dark:bg-slate-900 rounded-xl border border-gray-200 dark:border-slate-800 p-4 transition-all';
    card.innerHTML = `
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2 flex-wrap">
            <h3 class="font-semibold text-base truncate">${{d.species.common_name}}</h3>
            <span class="inline-flex items-center rounded-md px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${{badge}}">${{pct}}%</span>
          </div>
          <p class="text-sm text-gray-500 dark:text-slate-400 italic mt-0.5">${{d.species.scientific_name}}</p>
          <div class="flex items-center gap-3 mt-2 text-xs text-gray-400 dark:text-slate-500">
            <span>${{d.model}} ${{d.model_version}}</span>
            ${{d.source_name ? '<span class="before:content-[\\\"\\u00b7\\\"] before:mr-3">' + d.source_name + '</span>' : ''}}
            <span class="before:content-[\\\"\\u00b7\\\"] before:mr-3">${{timeAgo(d.detected_at)}}</span>
          </div>
        </div>
        <div class="flex-shrink-0 w-16 h-16 rounded-lg bg-gray-100 dark:bg-slate-800 flex items-center justify-center">
          <div class="relative w-12 h-12">
            <svg class="w-12 h-12 -rotate-90" viewBox="0 0 36 36">
              <path d="M18 2.0845 a 15.9155 15.9155 0 0 1 0 31.831 a 15.9155 15.9155 0 0 1 0 -31.831"
                fill="none" stroke="currentColor" stroke-opacity="0.1" stroke-width="3"/>
              <path d="M18 2.0845 a 15.9155 15.9155 0 0 1 0 31.831 a 15.9155 15.9155 0 0 1 0 -31.831"
                fill="none" stroke-width="3" stroke-dasharray="${{pct}}, 100"
                class="${{bar.replace('bg-', 'stroke-')}}"/>
            </svg>
            <span class="absolute inset-0 flex items-center justify-center text-xs font-bold">${{pct}}</span>
          </div>
        </div>
      </div>
      ${{d.alternatives && d.alternatives.length > 0 ? `
        <div class="mt-3 pt-3 border-t border-gray-100 dark:border-slate-800">
          <p class="text-xs text-gray-400 dark:text-slate-500 mb-1.5">Alternatives</p>
          <div class="flex flex-wrap gap-2">
            ${{d.alternatives.slice(0, 3).map(a => `<span class="text-xs bg-gray-100 dark:bg-slate-800 px-2 py-0.5 rounded">${{a.common_name}} <span class="text-gray-400">${{Math.round(a.confidence * 100)}}%</span></span>`).join('')}}
          </div>
        </div>` : ''}}`;
    return card;
  }}

  // Load initial detections from REST
  fetch('/api/v1/detections?limit=20')
    .then(r => r.json())
    .then(data => {{
      if (data.length > 0 && emptyState) emptyState.remove();
      data.reverse().forEach(d => {{
        const card = createCard(d);
        card.classList.remove('slide-in');
        feed.prepend(card);
        count++;
      }});
      document.getElementById('detection-count').textContent = count + ' shown';
    }})
    .catch(() => {{}});

  // Load stats
  function loadStats() {{
    fetch('/api/v1/species')
      .then(r => r.json())
      .then(data => {{
        const total = data.reduce((s, d) => s + d.detection_count, 0);
        const avgConf = data.length > 0 ? data.reduce((s, d) => s + d.avg_confidence * d.detection_count, 0) / total : 0;
        document.getElementById('stat-today').textContent = total;
        document.getElementById('stat-species').textContent = data.length;
        document.getElementById('stat-top').textContent = data.length > 0 ? data[0].common_name : '--';
        document.getElementById('stat-conf').textContent = total > 0 ? Math.round(avgConf * 100) + '%' : '--';
      }})
      .catch(() => {{}});
  }}
  loadStats();
  setInterval(loadStats, 30000);

  // SSE live feed
  const sse = new EventSource('/api/v1/stream/events');
  sse.addEventListener('detection', (e) => {{
    const d = JSON.parse(e.data);
    if (emptyState) emptyState.remove();
    const card = createCard(d);
    feed.prepend(card);
    count++;
    if (feed.children.length > 50) feed.lastChild.remove();
    document.getElementById('detection-count').textContent = count + ' shown';
  }});
  sse.onopen = () => setConnected(true);
  sse.onerror = () => setConnected(false);
}})();
</script>"##,
        station_name = station_name,
    )
}

pub fn species_content() -> String {
    r##"<div class="mb-6">
  <h1 class="text-2xl font-bold tracking-tight">Species</h1>
  <p class="text-sm text-gray-500 dark:text-slate-400 mt-0.5">Detected in the last 24 hours</p>
</div>

<div id="species-table" class="bg-white dark:bg-slate-900 rounded-xl border border-gray-200 dark:border-slate-800 overflow-hidden">
  <div class="p-8 text-center text-gray-400 dark:text-slate-500 text-sm">Loading...</div>
</div>

<script>
(function() {
  fetch('/api/v1/species')
    .then(r => r.json())
    .then(data => {
      const el = document.getElementById('species-table');
      if (data.length === 0) {
        el.innerHTML = '<div class="p-8 text-center text-gray-400 dark:text-slate-500 text-sm">No species detected in the last 24 hours</div>';
        return;
      }
      let html = `<table class="w-full"><thead>
        <tr class="border-b border-gray-200 dark:border-slate-800 text-left text-xs font-medium text-gray-500 dark:text-slate-400 uppercase tracking-wider">
          <th class="px-4 py-3">Species</th>
          <th class="px-4 py-3 text-right">Detections</th>
          <th class="px-4 py-3 text-right hidden sm:table-cell">Avg Confidence</th>
          <th class="px-4 py-3 text-right hidden md:table-cell">Last Seen</th>
        </tr></thead><tbody>`;
      data.forEach((s, i) => {
        const bg = i % 2 === 0 ? '' : 'bg-gray-50 dark:bg-slate-800/50';
        const pct = Math.round(s.avg_confidence * 100);
        const confClass = pct >= 80 ? 'text-emerald-600 dark:text-emerald-400' : pct >= 50 ? 'text-amber-600 dark:text-amber-400' : 'text-red-600 dark:text-red-400';
        const time = new Date(s.last_detected_at).toLocaleTimeString([], {hour:'2-digit', minute:'2-digit'});
        html += `<tr class="${bg} border-b border-gray-100 dark:border-slate-800/50 last:border-0">
          <td class="px-4 py-3">
            <p class="font-medium text-sm">${s.common_name}</p>
            <p class="text-xs text-gray-400 dark:text-slate-500 italic">${s.scientific_name}</p>
          </td>
          <td class="px-4 py-3 text-right">
            <span class="inline-flex items-center justify-center min-w-[2rem] rounded-full bg-blue-50 dark:bg-blue-900/20 text-blue-700 dark:text-blue-400 text-sm font-semibold px-2 py-0.5">${s.detection_count}</span>
          </td>
          <td class="px-4 py-3 text-right hidden sm:table-cell">
            <span class="text-sm font-medium ${confClass}">${pct}%</span>
          </td>
          <td class="px-4 py-3 text-right text-sm text-gray-500 dark:text-slate-400 hidden md:table-cell">${time}</td>
        </tr>`;
      });
      html += '</tbody></table>';
      el.innerHTML = html;
    })
    .catch(() => {
      document.getElementById('species-table').innerHTML =
        '<div class="p-8 text-center text-red-400 text-sm">Failed to load species data</div>';
    });
})();
</script>"##
        .to_string()
}

pub fn status_content(station_name: &str) -> String {
    format!(
        r##"<div class="mb-6">
  <h1 class="text-2xl font-bold tracking-tight">System Status</h1>
  <p class="text-sm text-gray-500 dark:text-slate-400 mt-0.5">{station_name}</p>
</div>

<div id="status-cards" class="grid gap-4 sm:grid-cols-2">
  <div class="bg-white dark:bg-slate-900 rounded-xl border border-gray-200 dark:border-slate-800 p-5">
    <h3 class="text-sm font-medium text-gray-500 dark:text-slate-400 uppercase tracking-wider mb-3">Station</h3>
    <dl class="space-y-2 text-sm">
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-slate-400">Name</dt><dd class="font-medium">{station_name}</dd></div>
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-slate-400">Status</dt><dd id="s-status" class="font-medium">--</dd></div>
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-slate-400">Total Detections</dt><dd id="s-count" class="font-medium">--</dd></div>
    </dl>
  </div>
  <div class="bg-white dark:bg-slate-900 rounded-xl border border-gray-200 dark:border-slate-800 p-5">
    <h3 class="text-sm font-medium text-gray-500 dark:text-slate-400 uppercase tracking-wider mb-3">API</h3>
    <dl class="space-y-2 text-sm">
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-slate-400">Detections</dt><dd><code class="text-xs bg-gray-100 dark:bg-slate-800 px-1.5 py-0.5 rounded">/api/v1/detections</code></dd></div>
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-slate-400">Species</dt><dd><code class="text-xs bg-gray-100 dark:bg-slate-800 px-1.5 py-0.5 rounded">/api/v1/species</code></dd></div>
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-slate-400">Live Stream</dt><dd><code class="text-xs bg-gray-100 dark:bg-slate-800 px-1.5 py-0.5 rounded">/api/v1/stream/events</code></dd></div>
    </dl>
  </div>
</div>

<script>
fetch('/api/v1/status')
  .then(r => r.json())
  .then(d => {{
    document.getElementById('s-status').innerHTML = '<span class="text-emerald-600 dark:text-emerald-400">' + d.status + '</span>';
    document.getElementById('s-count').textContent = d.detection_count.toLocaleString();
  }})
  .catch(() => {{
    document.getElementById('s-status').innerHTML = '<span class="text-red-500">unreachable</span>';
  }});
</script>"##,
        station_name = station_name,
    )
}
