//! Embedded HTML dashboard pages.
//!
//! Each page is rendered by wrapping page-specific content in a shared shell
//! (sidebar, header, Tailwind/htmx CDN scripts). No template engine — just
//! `format!()` with a layout string.

use axum::response::Html;

use crate::settings::{InitialConfig, RuntimeSettings};
use crate::visualization;

/// Render a full HTML page with the shared shell.
pub fn page(title: &str, active: &str, content: &str, timezone: &str) -> Html<String> {
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
      colors: {{
        nuthatch: {{
          50: '#fdf5ef', 100: '#fae8d6', 200: '#f4ceac', 300: '#ecad78',
          400: '#e38a47', 500: '#d97226', 600: '#c45c1c', 700: '#a34619',
          800: '#84391b', 900: '#6c3019', 950: '#3a160b',
        }},
        plumage: {{
          50: '#f2f6f9', 100: '#e2eaf0', 200: '#c9d8e3', 300: '#a3bccf',
          400: '#7799b4', 500: '#5b7d9b', 600: '#496683', 700: '#3d536b',
          800: '#36475a', 900: '#2f3d4d', 950: '#1c2733',
        }},
      }},
    }},
  }},
}}
</script>
<style>
  @keyframes slideIn {{ from {{ opacity: 0; transform: translateY(-8px); }} to {{ opacity: 1; transform: translateY(0); }} }}
  .slide-in {{ animation: slideIn 0.3s ease-out; }}
</style>
<script>
  (function() {{
    var s = localStorage.getItem('sitta-theme');
    if (s === 'dark' || (!s && window.matchMedia('(prefers-color-scheme: dark)').matches)) document.documentElement.classList.add('dark');
  }})();
</script>
</head>
<body class="h-full bg-stone-50 dark:bg-plumage-950 font-sans text-stone-900 dark:text-stone-100" data-tz="{timezone}">
<div class="flex h-full">

  <!-- Sidebar -->
  <nav class="hidden lg:flex lg:flex-col lg:w-60 bg-white dark:bg-plumage-900 border-r border-stone-200 dark:border-plumage-800">
    <div class="flex items-center gap-2.5 px-5 py-5 border-b border-stone-200 dark:border-plumage-800">
      <div class="w-8 h-8 rounded-lg bg-gradient-to-br from-plumage-500 to-nuthatch-500 flex items-center justify-center">
        <svg class="w-5 h-5 text-white" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M12 3c-1.5 0-3 .5-4 2-1.5 2-1 5 1 7l3 3 3-3c2-2 2.5-5 1-7-1-1.5-2.5-2-4-2z"/></svg>
      </div>
      <span class="text-lg font-bold tracking-tight bg-gradient-to-r from-plumage-300 to-nuthatch-400 bg-clip-text text-transparent dark:from-plumage-300 dark:to-nuthatch-400">Sitta</span>
    </div>
    <div class="flex-1 py-4 px-3 space-y-1">
      {nav_dashboard}
      {nav_species}
      {nav_status}
      {nav_individuals}
      {nav_settings}
    </div>
    <div id="audio-sources" class="px-3 pb-2"></div>
    <div class="px-5 py-3 border-t border-stone-200 dark:border-plumage-800">
      <button onclick="toggleTheme()" class="flex items-center gap-2 text-xs text-stone-400 dark:text-plumage-500 hover:text-stone-600 dark:hover:text-plumage-300 transition-colors">
        <svg class="w-4 h-4 hidden dark:block" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M12 3v2.25m6.364.386l-1.591 1.591M21 12h-2.25m-.386 6.364l-1.591-1.591M12 18.75V21m-4.773-4.227l-1.591 1.591M5.25 12H3m4.227-4.773L5.636 5.636M15.75 12a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0z"/></svg>
        <svg class="w-4 h-4 block dark:hidden" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M21.752 15.002A9.718 9.718 0 0118 15.75c-5.385 0-9.75-4.365-9.75-9.75 0-1.33.266-2.597.748-3.752A9.753 9.753 0 003 11.25C3 16.635 7.365 21 12.75 21a9.753 9.753 0 009.002-5.998z"/></svg>
        <span class="dark:hidden">Dark mode</span><span class="hidden dark:inline">Light mode</span>
      </button>
      <p class="text-xs text-stone-400 dark:text-plumage-600 mt-2">Sitta v0.1.0</p>
    </div>
  </nav>

  <!-- Main content -->
  <div class="flex-1 flex flex-col min-w-0">

    <!-- Mobile header -->
    <header class="lg:hidden flex items-center gap-3 px-4 py-3 bg-white dark:bg-plumage-900 border-b border-stone-200 dark:border-plumage-800">
      <div class="w-7 h-7 rounded-md bg-gradient-to-br from-plumage-500 to-nuthatch-500 flex items-center justify-center">
        <svg class="w-4 h-4 text-white" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M12 3c-1.5 0-3 .5-4 2-1.5 2-1 5 1 7l3 3 3-3c2-2 2.5-5 1-7-1-1.5-2.5-2-4-2z"/></svg>
      </div>
      <span class="font-bold bg-gradient-to-r from-plumage-300 to-nuthatch-400 bg-clip-text text-transparent">Sitta</span>
      <nav class="ml-auto flex gap-1">
        <a href="/" class="px-2.5 py-1.5 text-sm rounded-md {mob_dashboard}">Live</a>
        <a href="/species" class="px-2.5 py-1.5 text-sm rounded-md {mob_species}">Species</a>
        <a href="/status" class="px-2.5 py-1.5 text-sm rounded-md {mob_status}">Status</a>
        <a href="/individuals" class="px-2.5 py-1.5 text-sm rounded-md {mob_individuals}">Individuals</a>
        <a href="/settings" class="px-2.5 py-1.5 text-sm rounded-md {mob_settings}">Settings</a>
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
<!-- Audio player modal (global) -->
<div id="audio-player" class="hidden fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/50">
  <div class="bg-white dark:bg-plumage-900 rounded-t-xl sm:rounded-xl border border-stone-200 dark:border-plumage-800 shadow-xl w-full sm:max-w-sm p-5">
    <div class="flex items-center justify-between mb-3">
      <h3 id="player-title" class="font-semibold text-sm"></h3>
      <button onclick="stopPlayer()" class="text-stone-400 hover:text-stone-600 dark:text-plumage-500 dark:hover:text-plumage-300">
        <svg class="w-5 h-5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/></svg>
      </button>
    </div>
    <div class="flex items-center gap-3">
      <div class="flex-1 h-3 bg-stone-100 dark:bg-plumage-800 rounded-full overflow-hidden">
        <div id="player-bar" class="h-full bg-nuthatch-500 rounded-full transition-all duration-300" style="width:0%"></div>
      </div>
      <span id="player-dbfs" class="text-xs text-stone-400 dark:text-plumage-500 font-mono w-16 text-right">-- dBFS</span>
    </div>
    <p class="text-xs text-stone-400 dark:text-plumage-500 mt-2">Streaming live audio</p>
  </div>
</div>
<script>
function toggleTheme() {{
  var d = document.documentElement.classList;
  if (d.contains('dark')) {{ d.remove('dark'); localStorage.setItem('sitta-theme','light'); }}
  else {{ d.add('dark'); localStorage.setItem('sitta-theme','dark'); }}
}}

// ── Audio waveforms + player (global, runs on every page) ──────
(function() {{
  const srcEl = document.getElementById('audio-sources');
  if (!srcEl) return;
  const WAVE_COLS = 30;
  const waveData = {{}};
  let activePlayer = null, audioCtx = null, playerAbort = null;

  fetch('/api/v1/sources').then(r => r.json()).then(sources => {{
    if (sources.length === 0) return;
    srcEl.innerHTML = sources.map(s => {{
      waveData[s.name] = new Array(WAVE_COLS).fill(0);
      return `<div class="mb-2">
        <div class="flex items-center justify-between mb-0.5">
          <button onclick="startPlayer('${{s.name}}')" title="Listen to ${{s.name}}"
            class="flex items-center gap-1.5 text-[11px] text-stone-500 dark:text-plumage-400 hover:text-nuthatch-500 dark:hover:text-nuthatch-400 transition-colors truncate">
            <svg class="w-3 h-3 flex-shrink-0" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M19.114 5.636a9 9 0 010 12.728M16.463 8.288a5.25 5.25 0 010 7.424M6.75 8.25l4.72-4.72a.75.75 0 011.28.53v15.88a.75.75 0 01-1.28.53l-4.72-4.72H4.51c-.88 0-1.704-.507-1.938-1.354A9.01 9.01 0 012.25 12c0-.83.112-1.633.322-2.396C2.806 8.756 3.63 8.25 4.51 8.25H6.75z"/></svg>
            ${{s.name}}
          </button>
          <span id="dbfs-${{s.name}}" class="text-[9px] text-stone-400 dark:text-plumage-600 font-mono">--</span>
        </div>
        <canvas id="wave-${{s.name}}" height="16" class="w-full rounded-sm" style="image-rendering:pixelated"></canvas>
      </div>`;
    }}).join('');
    sources.forEach(s => {{ const c=document.getElementById('wave-'+s.name); if(c) c.width=c.offsetWidth; drawWave(s.name); }});
  }}).catch(() => {{}});

  function drawWave(name) {{
    const cvs = document.getElementById('wave-' + name);
    if (!cvs) return;
    const ctx = cvs.getContext('2d'), w = cvs.width, h = cvs.height, data = waveData[name];
    if (!data) return;
    const barW = Math.max(2, w / WAVE_COLS);
    const isDark = document.documentElement.classList.contains('dark');
    ctx.clearRect(0, 0, w, h);
    for (let i = 0; i < WAVE_COLS; i++) {{
      const val = Math.sqrt(data[i]);
      const barH = Math.max(1, val * h * 0.95);
      const opacity = 0.15 + 0.85 * (i + 1) / WAVE_COLS;
      const r = isDark ? 227 : 217, g = isDark ? 138 : 114, b = isDark ? 71 : 38;
      ctx.fillStyle = `rgba(${{r}},${{g}},${{b}},${{opacity}})`;
      ctx.fillRect(i * barW, (h - barH) / 2, barW - 1, barH);
    }}
  }}

  const lvl = new EventSource('/api/v1/audio/levels');
  window.addEventListener('beforeunload', () => lvl.close());
  lvl.addEventListener('level', (e) => {{
    const d = JSON.parse(e.data);
    const val = Math.max(0, Math.min(1, (d.rms_dbfs + 72) / 60));
    if (waveData[d.source]) {{ waveData[d.source].shift(); waveData[d.source].push(val); drawWave(d.source); }}
    const el = document.getElementById('dbfs-' + d.source);
    if (el) el.textContent = d.rms_dbfs > -100 ? d.rms_dbfs.toFixed(0) + ' dB' : '--';
    if (activePlayer === d.source) {{
      const pb = document.getElementById('player-bar'), pd = document.getElementById('player-dbfs');
      if (pb) pb.style.width = (val * 100) + '%';
      if (pd) pd.textContent = (d.rms_dbfs > -100 ? d.rms_dbfs.toFixed(1) : '--') + ' dBFS';
    }}
  }});

  window.startPlayer = async function(source) {{
    stopPlayer(); activePlayer = source;
    document.getElementById('player-title').textContent = source;
    document.getElementById('audio-player').classList.remove('hidden');
    try {{
      playerAbort = new AbortController();
      const resp = await fetch('/api/v1/audio/stream/' + encodeURIComponent(source), {{ signal: playerAbort.signal }});
      const reader = resp.body.getReader();
      let hdrBuf = new Uint8Array(0);
      while (hdrBuf.length < 20) {{ const {{ done, value }} = await reader.read(); if (done) return; const t = new Uint8Array(hdrBuf.length + value.length); t.set(hdrBuf); t.set(value, hdrBuf.length); hdrBuf = t; }}
      const hdr = new DataView(hdrBuf.buffer);
      const sampleRate = hdr.getUint32(0, true), channels = hdr.getUint16(4, true), chunkSamples = hdr.getUint32(8, true), chunkBytes = chunkSamples * 4;
      let remainder = hdrBuf.slice(20);
      if (!audioCtx || audioCtx.sampleRate !== sampleRate) {{ if (audioCtx) audioCtx.close(); audioCtx = new AudioContext({{ sampleRate }}); }}
      if (audioCtx.state === 'suspended') await audioCtx.resume();
      let nextTime = audioCtx.currentTime + 0.1, buf = remainder;
      while (true) {{
        const {{ done, value }} = await reader.read(); if (done) break;
        const t = new Uint8Array(buf.length + value.length); t.set(buf); t.set(value, buf.length); buf = t;
        while (buf.length >= chunkBytes) {{
          const samples = new Float32Array(buf.buffer.slice(buf.byteOffset, buf.byteOffset + chunkBytes)); buf = buf.slice(chunkBytes);
          const ab = audioCtx.createBuffer(channels || 1, chunkSamples, sampleRate); ab.getChannelData(0).set(samples);
          const s = audioCtx.createBufferSource(); s.buffer = ab; s.connect(audioCtx.destination);
          const at = Math.max(nextTime, audioCtx.currentTime); s.start(at); nextTime = at + ab.duration;
        }}
      }}
    }} catch (e) {{ if (e.name !== 'AbortError') console.error('Player error:', e); }}
  }};
  window.stopPlayer = function() {{
    if (playerAbort) {{ playerAbort.abort(); playerAbort = null; }}
    activePlayer = null;
    document.getElementById('audio-player').classList.add('hidden');
  }};
}})();
</script>
</body>
</html>"##,
        title = title,
        content = content,
        timezone = timezone,
        nav_dashboard = nav_item("Dashboard", "/", "dashboard", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M3.75 6A2.25 2.25 0 016 3.75h2.25A2.25 2.25 0 0110.5 6v2.25a2.25 2.25 0 01-2.25 2.25H6a2.25 2.25 0 01-2.25-2.25V6zM3.75 15.75A2.25 2.25 0 016 13.5h2.25a2.25 2.25 0 012.25 2.25V18a2.25 2.25 0 01-2.25 2.25H6A2.25 2.25 0 013.75 18v-2.25zM13.5 6a2.25 2.25 0 012.25-2.25H18A2.25 2.25 0 0120.25 6v2.25A2.25 2.25 0 0118 10.5h-2.25a2.25 2.25 0 01-2.25-2.25V6zM13.5 15.75a2.25 2.25 0 012.25-2.25H18a2.25 2.25 0 012.25 2.25V18A2.25 2.25 0 0118 20.25h-2.25A2.25 2.25 0 0113.5 18v-2.25z"/>"#),
        nav_species = nav_item("Species", "/species", "species", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M3.75 12h16.5m-16.5 3.75h16.5M3.75 19.5h16.5M5.625 4.5h12.75a1.875 1.875 0 010 3.75H5.625a1.875 1.875 0 010-3.75z"/>"#),
        nav_status = nav_item("Status", "/status", "status", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M9.348 14.651a3.75 3.75 0 010-5.303m5.304 0a3.75 3.75 0 010 5.303m-7.425 2.122a6.75 6.75 0 010-9.546m9.546 0a6.75 6.75 0 010 9.546M5.106 18.894c-3.808-3.808-3.808-9.98 0-13.788m13.788 0c3.808 3.808 3.808 9.98 0 13.788M12 12h.008v.008H12V12zm.375 0a.375.375 0 11-.75 0 .375.375 0 01.75 0z"/>"#),
        nav_individuals = nav_item("Individuals", "/individuals", "individuals", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M15 19.128a9.38 9.38 0 002.625.372 9.337 9.337 0 004.121-.952 4.125 4.125 0 00-7.533-2.493M15 19.128v-.003c0-1.113-.285-2.16-.786-3.07M15 19.128v.106A12.318 12.318 0 018.624 21c-2.331 0-4.512-.645-6.374-1.766l-.001-.109a6.375 6.375 0 0111.964-3.07M12 6.375a3.375 3.375 0 11-6.75 0 3.375 3.375 0 016.75 0zm8.25 2.25a2.625 2.625 0 11-5.25 0 2.625 2.625 0 015.25 0z"/>"#),
        nav_settings = nav_item("Settings", "/settings", "settings", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.324.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 011.37.49l1.296 2.247a1.125 1.125 0 01-.26 1.431l-1.003.827c-.293.24-.438.613-.431.992a6.759 6.759 0 010 .255c-.007.378.138.75.43.99l1.005.828c.424.35.534.954.26 1.43l-1.298 2.247a1.125 1.125 0 01-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.57 6.57 0 01-.22.128c-.331.183-.581.495-.644.869l-.213 1.28c-.09.543-.56.941-1.11.941h-2.594c-.55 0-1.02-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 01-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 01-1.369-.49l-1.297-2.247a1.125 1.125 0 01.26-1.431l1.004-.827c.292-.24.437-.613.43-.992a6.932 6.932 0 010-.255c.007-.378-.138-.75-.43-.99l-1.004-.828a1.125 1.125 0 01-.26-1.43l1.297-2.247a1.125 1.125 0 011.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.087.22-.128.332-.183.582-.495.644-.869l.214-1.281z"/><path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>"#),
        mob_dashboard = if active == "dashboard" { "bg-nuthatch-50 text-nuthatch-800 dark:bg-nuthatch-900/30 dark:text-nuthatch-400 font-medium" } else { "text-stone-500 dark:text-plumage-300" },
        mob_species = if active == "species" { "bg-nuthatch-50 text-nuthatch-800 dark:bg-nuthatch-900/30 dark:text-nuthatch-400 font-medium" } else { "text-stone-500 dark:text-plumage-300" },
        mob_status = if active == "status" { "bg-nuthatch-50 text-nuthatch-800 dark:bg-nuthatch-900/30 dark:text-nuthatch-400 font-medium" } else { "text-stone-500 dark:text-plumage-300" },
        mob_individuals = if active == "individuals" { "bg-nuthatch-50 text-nuthatch-800 dark:bg-nuthatch-900/30 dark:text-nuthatch-400 font-medium" } else { "text-stone-500 dark:text-plumage-300" },
        mob_settings = if active == "settings" { "bg-nuthatch-50 text-nuthatch-800 dark:bg-nuthatch-900/30 dark:text-nuthatch-400 font-medium" } else { "text-stone-500 dark:text-plumage-300" },
    ))
}

fn nav_item(label: &str, href: &str, key: &str, active: &str, icon_path: &str) -> String {
    let (bg, text, icon_cls) = if key == active {
        (
            "bg-nuthatch-50 dark:bg-nuthatch-900/20 border-l-2 border-nuthatch-500",
            "text-nuthatch-800 dark:text-nuthatch-400 font-medium",
            "text-nuthatch-600 dark:text-nuthatch-400",
        )
    } else {
        (
            "hover:bg-stone-100 dark:hover:bg-plumage-800/50 border-l-2 border-transparent",
            "text-stone-700 dark:text-plumage-200",
            "text-stone-400 dark:text-plumage-400",
        )
    };
    format!(
        r#"<a href="{href}" class="flex items-center gap-3 px-3 py-2 rounded-lg text-sm {bg} {text} transition-colors">
        <svg class="w-5 h-5 flex-shrink-0 {icon_cls}" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24">{icon_path}</svg>
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
    <p class="text-sm text-gray-500 dark:text-plumage-400 mt-0.5">Live detection feed</p>
  </div>
  <div id="connection-status" class="flex items-center gap-2 text-sm text-gray-400 dark:text-plumage-500">
    <span class="relative flex h-2.5 w-2.5"><span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-75"></span><span class="relative inline-flex rounded-full h-2.5 w-2.5 bg-amber-500"></span></span>
    Connecting...
  </div>
</div>

<!-- Stats row -->
<div id="stats" class="grid grid-cols-2 sm:grid-cols-4 gap-4 mb-6">
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 px-4 py-3 border-t-2 border-t-nuthatch-500">
    <p class="text-xs font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider">Today</p>
    <p id="stat-today" class="text-2xl font-bold mt-1 text-nuthatch-700 dark:text-nuthatch-400">--</p>
  </div>
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 px-4 py-3 border-t-2 border-t-plumage-500">
    <p class="text-xs font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider">Species</p>
    <p id="stat-species" class="text-2xl font-bold mt-1 text-plumage-700 dark:text-plumage-300">--</p>
  </div>
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 px-4 py-3 border-t-2 border-t-nuthatch-400">
    <p class="text-xs font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider">Top Species</p>
    <p id="stat-top" class="text-lg font-semibold mt-1 truncate">--</p>
  </div>
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 px-4 py-3 border-t-2 border-t-plumage-400">
    <p class="text-xs font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider">Avg Confidence</p>
    <p id="stat-conf" class="text-2xl font-bold mt-1">--</p>
  </div>
</div>

ACTIVITY_PANEL_PLACEHOLDER

<!-- Live detection feed -->
<div class="flex items-center justify-between mb-4">
  <h2 class="text-lg font-semibold">Recent Detections</h2>
  <span id="detection-count" class="text-sm text-gray-400 dark:text-plumage-500"></span>
</div>
<div id="live-feed" class="space-y-3">
  <div id="empty-state" class="text-center py-16 text-gray-400 dark:text-plumage-500">
    <svg class="w-12 h-12 mx-auto mb-3 text-nuthatch-400/50 dark:text-nuthatch-600/50" fill="none" stroke="currentColor" stroke-width="1" viewBox="0 0 24 24"><path d="M12 3c-1.5 0-3 .5-4 2-1.5 2-1 5 1 7l3 3 3-3c2-2 2.5-5 1-7-1-1.5-2.5-2-4-2z"/></svg>
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

  const _tz = document.body.dataset.tz || 'UTC';
  const _tf = {{ hour: '2-digit', minute: '2-digit', hour12: false, timeZone: _tz }};

  function timeAgo(iso) {{
    const s = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
    if (s < 5) return 'just now';
    if (s < 60) return s + 's ago';
    if (s < 3600) return Math.floor(s/60) + 'm ago';
    return new Date(iso).toLocaleTimeString('en-GB', _tf);
  }}

  // Audio playback for detection clips.
  let clipAudio = null;
  window.playClip = function(id, btn) {{
    if (clipAudio) {{ clipAudio.pause(); clipAudio = null; document.querySelectorAll('.clip-btn-playing').forEach(b => {{ b.classList.remove('clip-btn-playing'); b.innerHTML = playSvg; }}); }}
    clipAudio = new Audio('/api/v1/detections/' + id + '/audio');
    clipAudio.play();
    btn.classList.add('clip-btn-playing');
    btn.innerHTML = stopSvg;
    clipAudio.onended = () => {{ clipAudio = null; btn.classList.remove('clip-btn-playing'); btn.innerHTML = playSvg; }};
  }}
  const playSvg = '<svg class="w-4 h-4" fill="currentColor" viewBox="0 0 20 20"><path d="M6.3 2.84A1.5 1.5 0 004 4.11v11.78a1.5 1.5 0 002.3 1.27l9.344-5.891a1.5 1.5 0 000-2.538L6.3 2.841z"/></svg>';
  const stopSvg = '<svg class="w-4 h-4" fill="currentColor" viewBox="0 0 20 20"><rect x="4" y="4" width="12" height="12" rx="1.5"/></svg>';

  // Review detection.
  window.reviewDetection = function(id, status, card) {{
    fetch('/api/v1/detections/' + id + '/review', {{
      method: 'PUT',
      headers: {{ 'Content-Type': 'application/json' }},
      body: JSON.stringify({{ status }})
    }}).then(r => {{
      if (!r.ok) return;
      const strip = card.querySelector('.review-strip');
      if (strip) {{
        if (status === 'correct') {{
          strip.innerHTML = '<span class="text-xs text-emerald-600 dark:text-emerald-400 font-medium flex items-center gap-1"><svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M4.5 12.75l6 6 9-13.5"/></svg>Correct</span>';
          card.classList.remove('opacity-50');
          card.classList.add('ring-1', 'ring-emerald-200', 'dark:ring-emerald-800');
        }} else {{
          strip.innerHTML = '<span class="text-xs text-red-500 dark:text-red-400 font-medium">False positive</span>';
          card.classList.add('opacity-50');
          card.classList.remove('ring-1', 'ring-emerald-200', 'dark:ring-emerald-800');
        }}
      }}
    }});
  }}

  function createCard(d) {{
    const [badge, bar] = confColor(d.confidence);
    const pct = Math.round(d.confidence * 100);
    const card = document.createElement('div');
    card.className = 'slide-in bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4 transition-all';
    card.dataset.id = d.id;
    card.innerHTML = `
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2 flex-wrap">
            <h3 class="font-semibold text-base truncate">${{d.species.common_name}}</h3>
            <span class="inline-flex items-center rounded-md px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${{badge}}">${{pct}}%</span>
          </div>
          <p class="text-sm text-gray-500 dark:text-plumage-400 italic mt-0.5">${{d.species.scientific_name}}</p>
          <div class="flex items-center gap-3 mt-2 text-xs text-gray-400 dark:text-plumage-500">
            <span>${{d.model}} ${{d.model_version}}</span>
            ${{d.source_name ? '<span class="before:content-[\\\"\\u00b7\\\"] before:mr-3">' + d.source_name + '</span>' : ''}}
            <span class="before:content-[\\\"\\u00b7\\\"] before:mr-3">${{timeAgo(d.detected_at)}}</span>
          </div>
        </div>
        <div class="flex flex-col items-end gap-2 flex-shrink-0">
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
      ${{d.has_audio || d.snippet_path ? `
        <div class="mt-3">
          <img src="/api/v1/detections/${{d.id}}/spectrogram" loading="lazy"
               class="w-full h-16 rounded-lg object-cover bg-gray-100 dark:bg-plumage-800"
               alt="spectrogram" onerror="this.style.display='none'"/>
        </div>` : ''}}
      <div class="mt-3 pt-3 border-t border-gray-100 dark:border-plumage-800 flex items-center justify-between">
        <div class="flex items-center gap-2">
          ${{d.has_audio || d.snippet_path ? `<button onclick="playClip('${{d.id}}', this)" class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium bg-plumage-50 text-plumage-700 hover:bg-plumage-100 dark:bg-plumage-800 dark:text-plumage-300 dark:hover:bg-plumage-700 transition-colors">${{playSvg}} Play</button>` : ''}}
          <button onclick="reviewDetection('${{d.id}}', 'correct', this.closest('[data-id]'))" class="inline-flex items-center gap-1 px-2 py-1 rounded-lg text-xs font-medium text-emerald-700 hover:bg-emerald-50 dark:text-emerald-400 dark:hover:bg-emerald-900/30 transition-colors" title="Mark correct (c)">
            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M4.5 12.75l6 6 9-13.5"/></svg>
          </button>
          <button onclick="reviewDetection('${{d.id}}', 'false_positive', this.closest('[data-id]'))" class="inline-flex items-center gap-1 px-2 py-1 rounded-lg text-xs font-medium text-red-600 hover:bg-red-50 dark:text-red-400 dark:hover:bg-red-900/30 transition-colors" title="False positive (f)">
            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/></svg>
          </button>
        </div>
        <div class="review-strip"></div>
      </div>
      ${{d.alternatives && d.alternatives.length > 0 ? `
        <div class="mt-3 pt-3 border-t border-gray-100 dark:border-plumage-800">
          <p class="text-xs text-gray-400 dark:text-plumage-500 mb-1.5">Alternatives</p>
          <div class="flex flex-wrap gap-2">
            ${{d.alternatives.slice(0, 3).map(a => `<span class="text-xs bg-gray-100 dark:bg-plumage-800 px-2 py-0.5 rounded">${{a.common_name}} <span class="text-gray-400 dark:text-plumage-500">${{Math.round(a.confidence * 100)}}%</span></span>`).join('')}}
          </div>
        </div>` : ''}}`;
    return card;
  }}

  // Keyboard shortcuts for review: hover a card, press c/f.
  document.addEventListener('keydown', function(e) {{
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return;
    if (e.key !== 'c' && e.key !== 'f') return;
    const hovered = document.querySelector('[data-id]:hover');
    if (!hovered) return;
    const id = hovered.dataset.id;
    reviewDetection(id, e.key === 'c' ? 'correct' : 'false_positive', hovered);
  }});

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
  window.addEventListener('beforeunload', () => sse.close());
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
    .replace("ACTIVITY_PANEL_PLACEHOLDER", &visualization::activity_panel())
}

pub fn species_content() -> String {
    r##"<div class="mb-6">
  <h1 class="text-2xl font-bold tracking-tight">Species</h1>
  <p class="text-sm text-gray-500 dark:text-plumage-400 mt-0.5">Detected in the last 24 hours</p>
</div>

<div id="species-table" class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 overflow-hidden">
  <div class="p-8 text-center text-gray-400 dark:text-plumage-500 text-sm">Loading...</div>
</div>

<script>
(function() {
  fetch('/api/v1/species')
    .then(r => r.json())
    .then(data => {
      const el = document.getElementById('species-table');
      if (data.length === 0) {
        el.innerHTML = '<div class="p-8 text-center text-gray-400 dark:text-plumage-500 text-sm">No species detected in the last 24 hours</div>';
        return;
      }
      let html = `<table class="w-full"><thead>
        <tr class="border-b border-gray-200 dark:border-plumage-800 text-left text-xs font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider">
          <th class="px-4 py-3">Species</th>
          <th class="px-4 py-3 text-right">Detections</th>
          <th class="px-4 py-3 text-right hidden sm:table-cell">Avg Confidence</th>
          <th class="px-4 py-3 text-right hidden md:table-cell">Last Seen</th>
        </tr></thead><tbody>`;
      data.forEach((s, i) => {
        const bg = i % 2 === 0 ? '' : 'bg-gray-50 dark:bg-plumage-800/50';
        const pct = Math.round(s.avg_confidence * 100);
        const confClass = pct >= 80 ? 'text-emerald-600 dark:text-emerald-400' : pct >= 50 ? 'text-amber-600 dark:text-amber-400' : 'text-red-600 dark:text-red-400';
        const _tz = document.body.dataset.tz || 'UTC';
        const time = new Date(s.last_detected_at).toLocaleTimeString('en-GB', {hour:'2-digit', minute:'2-digit', hour12: false, timeZone: _tz});
        html += `<tr class="${bg} border-b border-gray-100 dark:border-plumage-800/50 last:border-0 cursor-pointer hover:bg-nuthatch-50/50 dark:hover:bg-nuthatch-900/10 transition-colors" onclick="location.href='/species/'+encodeURIComponent('${s.scientific_name}')">
          <td class="px-4 py-3">
            <p class="font-medium text-sm">${s.common_name}</p>
            <p class="text-xs text-gray-400 dark:text-plumage-500 italic">${s.scientific_name}</p>
          </td>
          <td class="px-4 py-3 text-right">
            <span class="inline-flex items-center justify-center min-w-[2rem] rounded-full bg-nuthatch-50 dark:bg-nuthatch-900/20 text-nuthatch-700 dark:text-nuthatch-400 text-sm font-semibold px-2 py-0.5">${s.detection_count}</span>
          </td>
          <td class="px-4 py-3 text-right hidden sm:table-cell">
            <span class="text-sm font-medium ${confClass}">${pct}%</span>
          </td>
          <td class="px-4 py-3 text-right text-sm text-gray-500 dark:text-plumage-400 hidden md:table-cell">${time}</td>
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

pub fn species_detail_content(scientific_name: &str) -> String {
    format!(
        r##"<div class="mb-6">
  <div class="flex items-center gap-2 mb-1">
    <a href="/species" class="text-nuthatch-600 dark:text-nuthatch-400 hover:underline text-sm">&larr; All species</a>
  </div>
  <h1 id="species-title" class="text-2xl font-bold tracking-tight">{scientific_name}</h1>
  <p class="text-sm text-gray-500 dark:text-plumage-400 italic mt-0.5">{scientific_name}</p>
</div>

<div id="species-insights" class="mb-6"></div>

<div id="species-detections" class="space-y-3">
  <div class="text-center py-12 text-gray-400 dark:text-plumage-500 text-sm">Loading detections...</div>
</div>

<script>
(function() {{
  const sciName = {sci_json};
  const _tz = document.body.dataset.tz || 'UTC';
  const _tf = {{ hour: '2-digit', minute: '2-digit', hour12: false, timeZone: _tz }};

  const playSvg = '<svg class="w-4 h-4" fill="currentColor" viewBox="0 0 20 20"><path d="M6.3 2.84A1.5 1.5 0 004 4.11v11.78a1.5 1.5 0 002.3 1.27l9.344-5.891a1.5 1.5 0 000-2.538L6.3 2.841z"/></svg>';
  const stopSvg = '<svg class="w-4 h-4" fill="currentColor" viewBox="0 0 20 20"><rect x="4" y="4" width="12" height="12" rx="1.5"/></svg>';
  let clipAudio = null;

  window.playClip = function(id, btn) {{
    if (clipAudio) {{ clipAudio.pause(); clipAudio = null; document.querySelectorAll('.clip-playing').forEach(b => {{ b.classList.remove('clip-playing'); b.innerHTML = playSvg + ' Play'; }}); }}
    clipAudio = new Audio('/api/v1/detections/' + id + '/audio');
    clipAudio.play();
    btn.classList.add('clip-playing');
    btn.innerHTML = stopSvg + ' Stop';
    clipAudio.onended = () => {{ clipAudio = null; btn.classList.remove('clip-playing'); btn.innerHTML = playSvg + ' Play'; }};
  }};

  // ── Behavioral insights ────────────────────────────────────────
  fetch('/api/v1/species/' + encodeURIComponent(sciName) + '/insights')
    .then(r => r.json())
    .then(ins => {{
      const el = document.getElementById('species-insights');
      const lines = buildInsights(ins, _tz);
      if (lines.length === 0) return;
      el.innerHTML = `<div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
        <div class="flex items-start gap-3">
          <svg class="w-5 h-5 text-nuthatch-500 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 18v-5.25m0 0a6.01 6.01 0 001.5-.189m-1.5.189a6.01 6.01 0 01-1.5-.189m3.75 7.478a12.06 12.06 0 01-4.5 0m3.75 2.383a14.406 14.406 0 01-3 0M14.25 18v-.192c0-.983.658-1.823 1.508-2.316a7.5 7.5 0 10-7.517 0c.85.493 1.509 1.333 1.509 2.316V18"/>
          </svg>
          <div class="text-sm text-gray-600 dark:text-plumage-300 space-y-1">
            ${{lines.map(l => '<p>' + l + '</p>').join('')}}
          </div>
        </div>
      </div>`;
    }})
    .catch(() => {{}});

  function buildInsights(data, tz) {{
    if (data.total_detections < 5) {{
      return ['Not enough detections to suggest habits. Check back after more observations.'];
    }}

    const insights = [];
    const hours = data.hourly_distribution;
    const total = hours.reduce((a, b) => a + b, 0);

    // Shift UTC hours to local timezone
    const tzOffsetHrs = getTimezoneOffsetHours(tz);
    const localHours = new Array(24).fill(0);
    for (let h = 0; h < 24; h++) {{
      const localH = ((h + Math.round(tzOffsetHrs)) % 24 + 24) % 24;
      localHours[localH] += hours[h];
    }}

    // Find active window (contiguous hours containing 80% of detections)
    const threshold = total * 0.8;
    let bestStart = 0, bestLen = 24;
    for (let start = 0; start < 24; start++) {{
      let sum = 0, len = 0;
      for (let j = 0; j < 24; j++) {{
        sum += localHours[(start + j) % 24];
        len = j + 1;
        if (sum >= threshold) break;
      }}
      if (sum >= threshold && len < bestLen) {{ bestStart = start; bestLen = len; }}
    }}
    const peakStart = bestStart;
    const peakEnd = (bestStart + bestLen - 1) % 24;

    // Compute sunrise/sunset if we have coordinates
    let sunrise = null, sunset = null;
    if (data.station_latitude != null && data.station_longitude != null) {{
      const sun = computeSunTimes(data.station_latitude, data.station_longitude, new Date(), tzOffsetHrs);
      if (sun) {{ sunrise = sun.sunrise; sunset = sun.sunset; }}
    }}

    // Time description
    const fmt = h => h.toString().padStart(2, '0') + ':00';
    if (bestLen <= 6) {{
      let desc = 'Most active between ' + fmt(peakStart) + ' and ' + fmt((peakEnd + 1) % 24) + '.';
      if (sunrise !== null) {{
        const relStart = ((peakStart - sunrise) + 24) % 24;
        const relEnd = ((peakEnd - sunrise) + 24) % 24;
        if (relStart <= 1 && relEnd <= 4) {{
          desc = 'Most active in the first few hours after sunrise (' + fmt(peakStart) + '\u2013' + fmt((peakEnd + 1) % 24) + ').';
        }} else if (peakStart >= sunrise - 1 && peakStart <= sunrise + 1) {{
          desc = 'Most active around sunrise (' + fmt(peakStart) + '\u2013' + fmt((peakEnd + 1) % 24) + ').';
        }} else if (sunset !== null && peakStart >= sunset - 2 && peakEnd <= sunset + 1) {{
          desc = 'Most active around dusk (' + fmt(peakStart) + '\u2013' + fmt((peakEnd + 1) % 24) + ').';
        }}
      }}
      insights.push(desc);
    }} else if (bestLen <= 14) {{
      insights.push('Active across a broad window, roughly ' + fmt(peakStart) + ' to ' + fmt((peakEnd + 1) % 24) + '.');
    }} else {{
      insights.push('Detected throughout the day with no strong time-of-day preference.');
    }}

    // Dawn/dusk bimodal check
    if (sunrise !== null && sunset !== null) {{
      const dawnHrs = [sunrise, (sunrise + 1) % 24, (sunrise + 2) % 24];
      const duskHrs = [(sunset - 1 + 24) % 24, sunset, (sunset + 1) % 24];
      const dawnPct = dawnHrs.reduce((s, h) => s + localHours[h], 0) / total;
      const duskPct = duskHrs.reduce((s, h) => s + localHours[h], 0) / total;
      if (dawnPct > 0.2 && duskPct > 0.2 && dawnPct + duskPct > 0.5) {{
        insights.push('Shows a dawn-and-dusk pattern \u2014 more active near sunrise and sunset.');
      }}
    }}

    // Consistency
    const firstMs = new Date(data.first_detected_at).getTime();
    const lastMs = new Date(data.last_detected_at).getTime();
    const totalDaysInRange = Math.max(1, Math.ceil((lastMs - firstMs) / 86400000));
    const ratio = data.days_detected / totalDaysInRange;
    if (totalDaysInRange >= 5) {{
      if (ratio >= 0.85) {{
        insights.push('Reliably present \u2014 detected on ' + data.days_detected + ' of the last ' + totalDaysInRange + ' days.');
      }} else if (ratio >= 0.5) {{
        insights.push('Regularly seen \u2014 detected on about ' + Math.round(ratio * 100) + '% of days.');
      }} else if (ratio < 0.25) {{
        insights.push('Uncommon visitor \u2014 detected on only ' + data.days_detected + ' of ' + totalDaysInRange + ' days.');
      }}
    }}

    // Confidence
    const avgPct = Math.round(data.avg_confidence * 100);
    if (avgPct < 50) {{
      insights.push('Detections tend to be low confidence (avg ' + avgPct + '%) \u2014 worth reviewing for false positives.');
    }}

    return insights;
  }}

  function getTimezoneOffsetHours(tz) {{
    try {{
      const now = new Date();
      const utcStr = now.toLocaleString('en-US', {{ timeZone: 'UTC', hour12: false }});
      const tzStr = now.toLocaleString('en-US', {{ timeZone: tz, hour12: false }});
      return (new Date(tzStr).getTime() - new Date(utcStr).getTime()) / 3600000;
    }} catch (e) {{ return 0; }}
  }}

  function computeSunTimes(lat, lon, date, tzOffsetHrs) {{
    const dayOfYear = Math.floor((date - new Date(date.getFullYear(), 0, 0)) / 86400000);
    const decl = -23.45 * Math.cos(2 * Math.PI / 365 * (dayOfYear + 10));
    const decRad = decl * Math.PI / 180;
    const latRad = lat * Math.PI / 180;
    const cosH = (Math.cos(90.833 * Math.PI / 180) - Math.sin(latRad) * Math.sin(decRad))
                 / (Math.cos(latRad) * Math.cos(decRad));
    if (cosH > 1 || cosH < -1) return null;
    const H = Math.acos(cosH) * 180 / Math.PI;
    const solarNoon = 12 - lon / 15;
    const riseUtc = solarNoon - H / 15;
    const setUtc = solarNoon + H / 15;
    return {{
      sunrise: Math.round(((riseUtc + tzOffsetHrs) % 24 + 24) % 24),
      sunset: Math.round(((setUtc + tzOffsetHrs) % 24 + 24) % 24),
    }};
  }}

  // ── Detection list ────────────────────────────────────────────
  fetch('/api/v1/detections?species=' + encodeURIComponent(sciName) + '&limit=100')
    .then(r => r.json())
    .then(data => {{
      const el = document.getElementById('species-detections');
      if (data.length === 0) {{
        el.innerHTML = '<div class="text-center py-12 text-gray-400 dark:text-plumage-500 text-sm">No detections found for this species</div>';
        return;
      }}
      // Update title with common name from first detection
      if (data[0].species.common_name) {{
        document.getElementById('species-title').textContent = data[0].species.common_name;
      }}
      el.innerHTML = data.map(d => {{
        const pct = Math.round(d.confidence * 100);
        const confClass = pct >= 80 ? 'text-emerald-700 bg-emerald-50 ring-emerald-600/20 dark:text-emerald-400 dark:bg-emerald-900/30 dark:ring-emerald-400/20'
          : pct >= 50 ? 'text-amber-700 bg-amber-50 ring-amber-600/20 dark:text-amber-400 dark:bg-amber-900/30 dark:ring-amber-400/20'
          : 'text-red-700 bg-red-50 ring-red-600/20 dark:text-red-400 dark:bg-red-900/30 dark:ring-red-400/20';
        const time = new Date(d.detected_at).toLocaleString('en-GB', {{
          month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false, timeZone: _tz
        }});
        const hasAudio = d.has_audio || d.snippet_path;
        return `<div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-2">
              <span class="inline-flex items-center rounded-md px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${{confClass}}">${{pct}}%</span>
              <span class="text-sm text-gray-500 dark:text-plumage-400">${{time}}</span>
            </div>
            <span class="text-xs text-gray-400 dark:text-plumage-600 font-mono">${{d.id.slice(0, 8)}}</span>
          </div>
          ${{hasAudio ? `<div class="mt-3">
            <img src="/api/v1/detections/${{d.id}}/spectrogram" loading="lazy"
                 class="w-full h-20 rounded-lg object-cover bg-gray-100 dark:bg-plumage-800"
                 alt="spectrogram" onerror="this.style.display='none'"/>
          </div>` : ''}}
          <div class="flex items-center justify-between mt-3 pt-3 border-t border-gray-100 dark:border-plumage-800">
            <div class="flex items-center gap-3">
              ${{hasAudio ? `<button onclick="playClip('${{d.id}}', this)" class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium bg-plumage-50 text-plumage-700 hover:bg-plumage-100 dark:bg-plumage-800 dark:text-plumage-300 dark:hover:bg-plumage-700 transition-colors">${{playSvg}} Play</button>` : ''}}
              <span class="text-xs text-gray-400 dark:text-plumage-500">${{d.model}} ${{d.model_version}}</span>
              ${{d.source_name ? '<span class="text-xs text-gray-400 dark:text-plumage-500 before:content-[\\u00b7] before:mr-2">' + d.source_name + '</span>' : ''}}
            </div>
            ${{d.has_embedding ? '<span class="text-xs text-plumage-500 dark:text-plumage-400">embedding</span>' : ''}}
          </div>
        </div>`;
      }}).join('');
    }})
    .catch(() => {{
      document.getElementById('species-detections').innerHTML =
        '<div class="text-center py-8 text-red-400 text-sm">Failed to load detections</div>';
    }});
}})();
</script>"##,
        scientific_name = scientific_name,
        sci_json = serde_json::to_string(scientific_name).unwrap_or_else(|_| "\"\"".to_string()),
    )
}

pub fn status_content(station_name: &str) -> String {
    format!(
        r##"<div class="mb-6">
  <h1 class="text-2xl font-bold tracking-tight">System Status</h1>
  <p class="text-sm text-gray-500 dark:text-plumage-400 mt-0.5">{station_name}</p>
</div>

<div id="status-cards" class="grid gap-4 sm:grid-cols-2">
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <h3 class="text-sm font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider mb-3">Station</h3>
    <dl class="space-y-2 text-sm">
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Name</dt><dd class="font-medium">{station_name}</dd></div>
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Status</dt><dd id="s-status" class="font-medium">--</dd></div>
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Total Detections</dt><dd id="s-count" class="font-medium">--</dd></div>
    </dl>
  </div>
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <h3 class="text-sm font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider mb-3">Pipeline</h3>
    <dl class="space-y-2 text-sm">
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">BirdNET processed</dt><dd id="s-bn-proc" class="font-medium">--</dd></div>
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">BirdNET dropped</dt><dd id="s-bn-drop" class="font-medium">--</dd></div>
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Perch processed</dt><dd id="s-perch-proc" class="font-medium">--</dd></div>
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Perch dropped</dt><dd id="s-perch-drop" class="font-medium">--</dd></div>
    </dl>
  </div>
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <h3 class="text-sm font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider mb-3">API</h3>
    <dl class="space-y-2 text-sm">
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Detections</dt><dd><code class="text-xs bg-gray-100 dark:bg-plumage-800 px-1.5 py-0.5 rounded">/api/v1/detections</code></dd></div>
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Species</dt><dd><code class="text-xs bg-gray-100 dark:bg-plumage-800 px-1.5 py-0.5 rounded">/api/v1/species</code></dd></div>
      <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Live Stream</dt><dd><code class="text-xs bg-gray-100 dark:bg-plumage-800 px-1.5 py-0.5 rounded">/api/v1/stream/events</code></dd></div>
    </dl>
  </div>
</div>

<script>
fetch('/api/v1/status')
  .then(r => r.json())
  .then(d => {{
    document.getElementById('s-status').innerHTML = '<span class="text-emerald-600 dark:text-emerald-400">' + d.status + '</span>';
    document.getElementById('s-count').textContent = d.detection_count.toLocaleString();
    if (d.pipeline) {{
      document.getElementById('s-bn-proc').textContent = d.pipeline.birdnet_chunks_processed.toLocaleString();
      const bnDrop = d.pipeline.birdnet_chunks_dropped;
      document.getElementById('s-bn-drop').innerHTML = bnDrop > 0
        ? '<span class="text-amber-500">' + bnDrop.toLocaleString() + '</span>'
        : '0';
      document.getElementById('s-perch-proc').textContent = d.pipeline.perch_chunks_processed.toLocaleString();
      const pDrop = d.pipeline.perch_chunks_dropped;
      document.getElementById('s-perch-drop').innerHTML = pDrop > 0
        ? '<span class="text-amber-500">' + pDrop.toLocaleString() + '</span>'
        : '0';
    }}
  }})
  .catch(() => {{
    document.getElementById('s-status').innerHTML = '<span class="text-red-500">unreachable</span>';
  }});
</script>"##,
        station_name = station_name,
    )
}

pub fn individuals_content() -> String {
    r##"<div class="flex items-center justify-between mb-6">
  <div>
    <h1 class="text-2xl font-bold tracking-tight">Individuals</h1>
    <p class="text-sm text-gray-500 dark:text-plumage-400 mt-0.5">Recurring visitors identified from Perch embeddings</p>
  </div>
</div>

<!-- Suggested clusters -->
<div id="suggestions-section" class="hidden mb-8">
  <h2 class="text-lg font-semibold mb-3 flex items-center gap-2">
    <span class="w-2 h-2 rounded-full bg-nuthatch-500"></span>
    Suggested Individuals
  </h2>
  <p class="text-sm text-gray-500 dark:text-plumage-400 mb-4">These clusters of similar detections may represent distinct individuals. Review and enroll them to start tracking.</p>
  <div id="suggestions-list" class="space-y-3"></div>
</div>

<!-- Enrolled individuals -->
<div id="individuals-list">
  <div class="text-center py-12 text-gray-400 dark:text-plumage-500 text-sm">Loading...</div>
</div>

<!-- Danger zone -->
<div id="danger-zone" class="hidden mt-8 pt-6 border-t border-gray-200 dark:border-plumage-800">
  <p class="text-xs text-gray-400 dark:text-plumage-500 mb-2">Danger zone</p>
  <button onclick="clearAllIndividuals()"
    class="px-3 py-1.5 rounded-lg text-xs font-medium text-red-600 dark:text-red-400 border border-red-200 dark:border-red-900/50 hover:bg-red-50 dark:hover:bg-red-900/20 transition-colors">
    Clear all enrolled individuals
  </button>
</div>

<!-- Enrollment modal for cluster -->
<div id="enroll-modal" class="hidden fixed inset-0 z-50 flex items-center justify-center bg-black/50">
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 shadow-xl w-full max-w-md mx-4 p-6">
    <h2 class="text-lg font-semibold mb-1">Enroll Individual</h2>
    <p id="enroll-species" class="text-sm text-gray-500 dark:text-plumage-400 mb-4 italic"></p>

    <input type="hidden" id="enroll-cluster-id">
    <div class="space-y-3">
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Label</label>
        <input id="enroll-label" type="text" placeholder="e.g. Barn Owl #1"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 outline-none">
      </div>
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Notes (optional)</label>
        <input id="enroll-notes" type="text" placeholder="e.g. Visits the north feeder regularly"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 outline-none">
      </div>
    </div>

    <div class="flex justify-end gap-2 mt-4">
      <button onclick="document.getElementById('enroll-modal').classList.add('hidden')"
        class="px-3 py-1.5 rounded-lg text-sm text-stone-500 dark:text-plumage-400 hover:bg-gray-100 dark:hover:bg-plumage-800 transition-colors">Cancel</button>
      <button id="enroll-btn"
        class="px-3 py-1.5 rounded-lg bg-nuthatch-600 text-white text-sm font-medium hover:bg-nuthatch-700 disabled:opacity-50 transition-colors"
        onclick="submitClusterEnroll()">Enroll</button>
    </div>
    <div id="enroll-status" class="mt-2 text-sm"></div>
  </div>
</div>

<script>
(function() {
  const _tz = document.body.dataset.tz || 'UTC';
  const _df = { month: 'short', day: 'numeric', timeZone: _tz };

  // ── Load suggested clusters ────────────────────────────────
  fetch('/api/v1/candidates')
    .then(r => r.json())
    .then(data => {
      if (data.length === 0) return;
      document.getElementById('suggestions-section').classList.remove('hidden');
      const el = document.getElementById('suggestions-list');
      el.innerHTML = data.map(c => {
        const first = new Date(c.first_seen_at).toLocaleDateString('en-GB', _df);
        const last = new Date(c.last_seen_at).toLocaleDateString('en-GB', _df);
        const range = first === last ? first : first + ' — ' + last;
        return `<div class="bg-white dark:bg-plumage-900 rounded-xl border border-nuthatch-200 dark:border-nuthatch-800/50 border-l-4 border-l-nuthatch-500 p-4 flex items-center justify-between gap-4">
          <div class="min-w-0 flex-1">
            <p class="font-semibold text-sm">${c.scientific_name}</p>
            <p class="text-xs text-gray-500 dark:text-plumage-400 mt-0.5">
              ${c.member_count} detections over ${c.distinct_days} day${c.distinct_days !== 1 ? 's' : ''}
              <span class="text-gray-400 dark:text-plumage-500 ml-1">${range}</span>
            </p>
          </div>
          <div class="flex items-center gap-2 flex-shrink-0">
            <button onclick="openClusterEnroll(${c.id}, '${c.scientific_name.replace(/'/g, "\\'")}')"
              class="px-3 py-1.5 rounded-lg bg-nuthatch-600 text-white text-xs font-medium hover:bg-nuthatch-700 transition-colors">Enroll</button>
            <button onclick="dismissCluster(${c.id}, this)"
              class="px-3 py-1.5 rounded-lg text-xs text-stone-500 dark:text-plumage-400 hover:bg-gray-100 dark:hover:bg-plumage-800 transition-colors">Dismiss</button>
          </div>
        </div>`;
      }).join('');
    })
    .catch(() => {});

  // ── Load enrolled individuals ──────────────────────────────
  fetch('/api/v1/individuals')
    .then(r => r.json())
    .then(data => {
      const el = document.getElementById('individuals-list');
      if (data.length === 0) {
        el.innerHTML = `<div class="text-center py-16 text-gray-400 dark:text-plumage-500">
          <svg class="w-12 h-12 mx-auto mb-3 opacity-50" fill="none" stroke="currentColor" stroke-width="1" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M15 19.128a9.38 9.38 0 002.625.372 9.337 9.337 0 004.121-.952 4.125 4.125 0 00-7.533-2.493M15 19.128v-.003c0-1.113-.285-2.16-.786-3.07M15 19.128v.106A12.318 12.318 0 018.624 21c-2.331 0-4.512-.645-6.374-1.766l-.001-.109a6.375 6.375 0 0111.964-3.07M12 6.375a3.375 3.375 0 11-6.75 0 3.375 3.375 0 016.75 0zm8.25 2.25a2.625 2.625 0 11-5.25 0 2.625 2.625 0 015.25 0z"/></svg>
          <p class="text-sm">No individuals enrolled yet</p>
          <p class="text-xs mt-1">Suggested individuals will appear above once Perch detects recurring visitors</p>
        </div>`;
        return;
      }

      const groups = {};
      data.forEach(ind => {
        const key = ind.scientific_name;
        if (!groups[key]) groups[key] = { scientific_name: key, individuals: [] };
        groups[key].individuals.push(ind);
      });

      document.getElementById('danger-zone').classList.remove('hidden');
      let html = '<h2 class="text-lg font-semibold mb-3">Enrolled Individuals</h2><div class="space-y-6">';
      Object.values(groups).forEach(g => {
        html += `<div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 overflow-hidden">
          <div class="px-5 py-3 border-b border-gray-100 dark:border-plumage-800 flex items-center justify-between">
            <div>
              <h3 class="font-semibold text-sm">${g.individuals[0].scientific_name}</h3>
              <p class="text-xs text-gray-400 dark:text-plumage-500 italic">${g.scientific_name}</p>
            </div>
            <span class="inline-flex items-center justify-center min-w-[1.5rem] rounded-full bg-nuthatch-50 dark:bg-nuthatch-900/20 text-nuthatch-700 dark:text-nuthatch-400 text-xs font-semibold px-2 py-0.5">${g.individuals.length}</span>
          </div>
          <div class="divide-y divide-gray-100 dark:divide-plumage-800">`;

        g.individuals.forEach(ind => {
          const enrolled = new Date(ind.enrolled_at).toLocaleDateString('en-GB', { month: 'short', day: 'numeric', year: 'numeric', timeZone: _tz });
          html += `<div class="px-5 py-3 flex items-center justify-between">
            <div>
              <p class="font-medium text-sm">${ind.label}</p>
              <p class="text-xs text-gray-400 dark:text-plumage-500 mt-0.5">Enrolled ${enrolled}${ind.notes ? ' — ' + ind.notes : ''}</p>
            </div>
            <span class="text-xs text-gray-400 dark:text-plumage-500 font-mono">${ind.id.slice(0, 8)}</span>
          </div>`;
        });

        html += '</div></div>';
      });
      html += '</div>';
      el.innerHTML = html;
    })
    .catch(() => {
      document.getElementById('individuals-list').innerHTML =
        '<div class="text-center py-8 text-red-400 text-sm">Failed to load individuals</div>';
    });
})();

function openClusterEnroll(clusterId, species) {
  document.getElementById('enroll-cluster-id').value = clusterId;
  document.getElementById('enroll-species').textContent = species;
  document.getElementById('enroll-label').value = '';
  document.getElementById('enroll-label').placeholder = species + ' #1';
  document.getElementById('enroll-notes').value = '';
  document.getElementById('enroll-status').textContent = '';
  document.getElementById('enroll-btn').disabled = false;
  document.getElementById('enroll-modal').classList.remove('hidden');
}

function dismissCluster(clusterId, btn) {
  btn.disabled = true;
  fetch('/api/v1/candidates/' + clusterId + '/dismiss', { method: 'POST' })
    .then(r => {
      if (r.ok) {
        btn.closest('[class*="border-l-nuthatch"]').remove();
        const remaining = document.querySelectorAll('#suggestions-list > div');
        if (remaining.length === 0) document.getElementById('suggestions-section').classList.add('hidden');
      }
    });
}

function submitClusterEnroll() {
  const clusterId = document.getElementById('enroll-cluster-id').value;
  const label = document.getElementById('enroll-label').value;
  const notes = document.getElementById('enroll-notes').value;
  const status = document.getElementById('enroll-status');

  if (!label.trim()) {
    status.innerHTML = '<span class="text-amber-500">Please enter a label</span>';
    return;
  }

  document.getElementById('enroll-btn').disabled = true;
  status.textContent = 'Enrolling...';

  fetch('/api/v1/candidates/' + clusterId + '/enroll', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ label: label.trim(), notes: notes.trim() || undefined }),
  })
  .then(r => {
    if (!r.ok) return r.text().then(t => Promise.reject(t));
    return r.json();
  })
  .then(() => {
    document.getElementById('enroll-modal').classList.add('hidden');
    location.reload();
  })
  .catch(e => {
    status.innerHTML = '<span class="text-red-500">Error: ' + e + '</span>';
    document.getElementById('enroll-btn').disabled = false;
  });
}

function clearAllIndividuals() {
  if (!confirm('Delete all enrolled individuals and their match history? This cannot be undone.')) return;
  fetch('/api/v1/individuals', { method: 'DELETE' })
    .then(r => r.json())
    .then(d => { alert('Deleted ' + d.deleted + ' individual(s).'); location.reload(); })
    .catch(e => alert('Failed: ' + e));
}
</script>"##
    .to_string()
}

pub fn settings_content(settings: &RuntimeSettings, initial: &InitialConfig) -> String {
    let display_min_confidence = settings.display_min_confidence;
    let birdnet_conf = settings.birdnet_min_confidence.unwrap_or(0.25);
    let birdnet_topk = settings.birdnet_top_k.unwrap_or(10);
    let birdnet_meta = settings.birdnet_meta_threshold.unwrap_or(0.01);
    let birdnet_allow = settings.birdnet_force_allow.as_deref().unwrap_or(&[]).join(", ");
    let perch_conf = settings.perch_min_confidence.unwrap_or(0.25);
    let perch_topk = settings.perch_top_k.unwrap_or(10);
    let lat = settings.station_latitude.map(|v| v.to_string()).unwrap_or_default();
    let lon = settings.station_longitude.map(|v| v.to_string()).unwrap_or_default();

    let has_birdnet = initial.birdnet_model_path.is_some();
    let has_perch = initial.perch_model_path.is_some();

    format!(
        r##"<div class="mb-6">
  <h1 class="text-2xl font-bold tracking-tight">Settings</h1>
  <p class="text-sm text-gray-500 dark:text-plumage-400 mt-0.5">Runtime-changeable configuration</p>
</div>

<div id="toast" class="fixed top-4 right-4 z-50 hidden"></div>

<form id="settings-form" class="space-y-6" onsubmit="return false;">

  <!-- Station -->
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <h3 class="text-sm font-semibold text-gray-900 dark:text-plumage-100 uppercase tracking-wider mb-4">Station</h3>
    <div class="grid gap-4 sm:grid-cols-2">
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Name</label>
        <input name="station_name" type="text" value="{station_name}"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
      </div>
      <div class="hidden sm:block"></div>
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Latitude</label>
        <input name="station_latitude" type="number" step="any" value="{lat}" placeholder="e.g. 44.5868"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
      </div>
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Longitude</label>
        <input name="station_longitude" type="number" step="any" value="{lon}" placeholder="e.g. -76.0283"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
      </div>
    </div>
    <div class="sm:col-span-2">
        <label class="block text-sm font-medium text-stone-700 dark:text-plumage-300 mb-1">Timezone</label>
        <input name="timezone" id="tz-input" type="text" list="tz-list" value="{timezone}"
          class="w-full rounded-lg border border-stone-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none"
          placeholder="Start typing to search...">
        <datalist id="tz-list"></datalist>
        <p class="mt-1 text-xs text-stone-400 dark:text-plumage-500">IANA timezone. Derived from coordinates if empty.</p>
        <script>
        (function(){{
          var dl = document.getElementById('tz-list');
          try {{
            Intl.supportedValuesOf('timeZone').forEach(function(tz) {{
              var o = document.createElement('option');
              o.value = tz;
              dl.appendChild(o);
            }});
          }} catch(e) {{
            // Fallback for older browsers: common timezones
            ['UTC','America/New_York','America/Chicago','America/Denver','America/Los_Angeles',
             'America/Toronto','America/Vancouver','America/Sao_Paulo','America/Argentina/Buenos_Aires',
             'Europe/London','Europe/Berlin','Europe/Paris','Europe/Moscow',
             'Asia/Tokyo','Asia/Shanghai','Asia/Kolkata','Asia/Dubai',
             'Australia/Sydney','Australia/Melbourne','Australia/Perth',
             'Pacific/Auckland','Africa/Johannesburg','Africa/Cairo'
            ].forEach(function(tz) {{
              var o = document.createElement('option');
              o.value = tz;
              dl.appendChild(o);
            }});
          }}
        }})();
        </script>
      </div>
    </div>
    <p class="mt-2 text-xs text-stone-400 dark:text-plumage-500">Station ID <code class="bg-stone-100 dark:bg-plumage-800 px-1 rounded">{station_id}</code> requires restart to change.</p>
  </div>

  <!-- Display -->
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <h3 class="text-sm font-semibold text-gray-900 dark:text-plumage-100 uppercase tracking-wider mb-4">Display</h3>
    <div class="grid gap-4 sm:grid-cols-2">
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Min Confidence (UI)</label>
        <input name="display_min_confidence" type="number" step="0.01" min="0" max="1" value="{display_min_confidence}"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
      </div>
    </div>
    <p class="mt-2 text-xs text-gray-400 dark:text-plumage-500">Detections below this confidence are still captured in the database but hidden from the dashboard, SSE feed, and species summary. Lower this to see more detections; raise it to reduce noise.</p>
  </div>

  <!-- Audio Sources -->
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-stone-200 dark:border-plumage-800 p-5">
    <div class="flex items-center justify-between mb-4">
      <h3 class="text-sm font-semibold text-gray-900 dark:text-plumage-100 uppercase tracking-wider">Audio Sources</h3>
      <button onclick="document.getElementById('add-source-form').classList.toggle('hidden')"
        class="text-xs text-nuthatch-600 dark:text-nuthatch-400 hover:underline">+ Add source</button>
    </div>
    <div id="sources-list" class="space-y-2 text-sm">Loading...</div>
    <div id="add-source-form" class="hidden mt-4 pt-4 border-t border-stone-200 dark:border-plumage-800 space-y-3">
      <div class="grid gap-3 sm:grid-cols-2">
        <div>
          <label class="block text-xs font-medium text-stone-600 dark:text-plumage-300 mb-1">Type</label>
          <select id="new-source-type" class="w-full rounded-lg border border-stone-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm">
            <option value="rtsp">RTSP</option>
            <option value="remote">Remote (Sitta)</option>
          </select>
        </div>
        <div>
          <label class="block text-xs font-medium text-stone-600 dark:text-plumage-300 mb-1">Name</label>
          <input id="new-source-name" type="text" placeholder="e.g. south_dam"
            class="w-full rounded-lg border border-stone-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm">
        </div>
        <div class="sm:col-span-2">
          <label class="block text-xs font-medium text-stone-600 dark:text-plumage-300 mb-1">URL</label>
          <input id="new-source-url" type="text" placeholder="rtsp://... or http://..."
            class="w-full rounded-lg border border-stone-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm">
        </div>
      </div>
      <button onclick="addSource()" class="px-3 py-1.5 rounded-lg bg-nuthatch-600 text-white text-sm font-medium hover:bg-nuthatch-700 transition-colors">Add</button>
      <span id="add-source-status" class="text-xs ml-2"></span>
    </div>
  </div>

  {birdnet_section}

  {perch_section}

  <!-- MQTT -->
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <div class="flex items-center justify-between mb-4">
      <h3 class="text-sm font-semibold text-gray-900 dark:text-plumage-100 uppercase tracking-wider">MQTT</h3>
      <span id="mqtt-status" class="text-xs text-stone-400 dark:text-plumage-500"></span>
    </div>
    <div id="mqtt-form-area"></div>
  </div>

  <!-- Actions -->
  <div class="flex items-center gap-3">
    <button type="submit" id="save-btn"
      class="inline-flex items-center px-4 py-2 rounded-lg bg-nuthatch-600 text-white text-sm font-medium hover:bg-nuthatch-700 focus:ring-2 focus:ring-nuthatch-500 focus:ring-offset-2 dark:focus:ring-offset-plumage-950 transition-colors">
      Save Changes
    </button>
    <span id="save-status" class="text-sm text-gray-400 dark:text-plumage-500"></span>
  </div>
</form>

<script>
(function() {{
  const form = document.getElementById('settings-form');
  const btn = document.getElementById('save-btn');
  const status = document.getElementById('save-status');
  const toast = document.getElementById('toast');

  function showToast(msg, ok) {{
    toast.className = 'fixed top-4 right-4 z-50 px-4 py-2.5 rounded-lg text-sm font-medium shadow-lg transition-opacity ' +
      (ok ? 'bg-emerald-600 text-white' : 'bg-red-600 text-white');
    toast.textContent = msg;
    toast.classList.remove('hidden');
    setTimeout(() => toast.classList.add('hidden'), 3000);
  }}

  form.addEventListener('submit', async () => {{
    btn.disabled = true;
    status.textContent = 'Saving...';

    const body = {{}};
    const fd = new FormData(form);
    for (const [k, v] of fd.entries()) {{
      if (v === '') continue;
      if (k === 'station_latitude' || k === 'station_longitude' ||
          k === 'display_min_confidence' ||
          k === 'birdnet_min_confidence' || k === 'birdnet_meta_threshold' ||
          k === 'perch_min_confidence') {{
        body[k] = parseFloat(v);
      }} else if (k === 'birdnet_top_k' || k === 'perch_top_k') {{
        body[k] = parseInt(v, 10);
      }} else if (k === 'birdnet_force_allow') {{
        body[k] = v.split(',').map(s => s.trim()).filter(s => s);
      }} else {{
        body[k] = v;
      }}
    }}

    try {{
      const res = await fetch('/api/v1/settings', {{
        method: 'PUT',
        headers: {{ 'Content-Type': 'application/json' }},
        body: JSON.stringify(body),
      }});
      const data = await res.json();
      if (res.ok) {{
        const n = data.updated.length;
        if (n === 0) {{
          showToast('No changes detected', true);
        }} else {{
          showToast(n + ' setting' + (n > 1 ? 's' : '') + ' updated' + (data.rebuild_triggered ? ' (rebuilding models...)' : ''), true);
        }}
        if (data.persist_error) {{
          status.textContent = 'Warning: ' + data.persist_error;
          status.className = 'text-sm text-amber-500';
        }} else {{
          status.textContent = '';
        }}
      }} else {{
        showToast('Failed to save: ' + (data.persist_error || res.statusText), false);
      }}
    }} catch (e) {{
      showToast('Network error: ' + e.message, false);
    }} finally {{
      btn.disabled = false;
    }}
  }});
}})();

// Source management
(function() {{
  function loadSources() {{
    fetch('/api/v1/sources')
      .then(r => r.json())
      .then(data => {{
        const el = document.getElementById('sources-list');
        if (data.length === 0) {{
          el.innerHTML = '<p class="text-stone-400 dark:text-plumage-500">No audio sources configured</p>';
          return;
        }}
        el.innerHTML = data.map(s => `<div class="flex items-center justify-between py-2 px-3 rounded-lg bg-stone-50 dark:bg-plumage-800">
          <div>
            <span class="font-medium">${{s.name}}</span>
            <span class="ml-2 text-xs px-1.5 py-0.5 rounded bg-stone-200 dark:bg-plumage-700 text-stone-600 dark:text-plumage-300">${{s.source_type}}</span>
            ${{s.url ? '<span class="ml-2 text-xs text-stone-400 dark:text-plumage-500 truncate max-w-xs inline-block align-bottom">' + s.url + '</span>' : ''}}
          </div>
          <button onclick="removeSource('${{s.name}}')" class="text-xs text-red-500 hover:text-red-700 dark:text-red-400 dark:hover:text-red-300">Remove</button>
        </div>`).join('');
      }})
      .catch(() => {{
        document.getElementById('sources-list').innerHTML = '<p class="text-red-400 text-xs">Failed to load sources</p>';
      }});
  }}
  loadSources();

  window.addSource = function() {{
    const t = document.getElementById('new-source-type').value;
    const n = document.getElementById('new-source-name').value.trim();
    const u = document.getElementById('new-source-url').value.trim();
    const st = document.getElementById('add-source-status');
    if (!n || !u) {{ st.innerHTML = '<span class="text-amber-500">Name and URL required</span>'; return; }}

    const body = t === 'rtsp'
      ? {{ type: 'rtsp', name: n, url: u }}
      : {{ type: 'remote', name: n, url: u }};

    st.textContent = 'Adding...';
    fetch('/api/v1/sources', {{
      method: 'POST',
      headers: {{ 'Content-Type': 'application/json' }},
      body: JSON.stringify(body),
    }})
    .then(async r => {{
      const text = await r.text();
      if (!r.ok) throw new Error(text || r.statusText);
      return text;
    }})
    .then(() => {{
      st.innerHTML = '<span class="text-emerald-500">Added</span>';
      document.getElementById('new-source-name').value = '';
      document.getElementById('new-source-url').value = '';
      setTimeout(() => document.getElementById('add-source-form').classList.add('hidden'), 500);
      loadSources();
    }})
    .catch(e => {{ st.innerHTML = '<span class="text-red-500">' + e.message + '</span>'; }});
  }};

  window.removeSource = function(name) {{
    if (!confirm('Remove source "' + name + '"?')) return;
    fetch('/api/v1/sources/' + encodeURIComponent(name), {{ method: 'DELETE' }})
      .then(r => {{ if (!r.ok) return r.text().then(t => Promise.reject(t)); loadSources(); }})
      .catch(e => alert('Failed: ' + e));
  }};
}})();

// MQTT config form
(function() {{
  const area = document.getElementById('mqtt-form-area');
  const status = document.getElementById('mqtt-status');
  if (!area) return;

  function inp(cls) {{ return 'w-full rounded-lg border border-stone-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none'; }}

  fetch('/api/v1/mqtt').then(r => r.json()).then(m => {{
    area.innerHTML = `<div class="space-y-3">
      <div class="flex items-center gap-3">
        <label class="flex items-center gap-2 text-sm cursor-pointer">
          <input id="mqtt-enabled" type="checkbox" ${{m.enabled ? 'checked' : ''}} class="rounded border-stone-300 dark:border-plumage-700 text-nuthatch-600 focus:ring-nuthatch-500">
          Enable MQTT
        </label>
      </div>
      <div id="mqtt-fields" class="${{m.enabled ? '' : 'opacity-50 pointer-events-none'}}">
        <div class="grid gap-3 sm:grid-cols-2">
          <div>
            <label class="block text-xs font-medium text-stone-600 dark:text-plumage-300 mb-1">Host</label>
            <input id="mqtt-host" type="text" value="${{m.host || ''}}" placeholder="localhost" class="${{inp()}}">
          </div>
          <div>
            <label class="block text-xs font-medium text-stone-600 dark:text-plumage-300 mb-1">Port</label>
            <input id="mqtt-port" type="number" value="${{m.port || 1883}}" class="${{inp()}}">
          </div>
          <div>
            <label class="block text-xs font-medium text-stone-600 dark:text-plumage-300 mb-1">Username (optional)</label>
            <input id="mqtt-user" type="text" value="${{m.username || ''}}" placeholder="" class="${{inp()}}">
          </div>
          <div>
            <label class="block text-xs font-medium text-stone-600 dark:text-plumage-300 mb-1">Password (optional)</label>
            <input id="mqtt-pass" type="password" value="${{m.password || ''}}" class="${{inp()}}">
          </div>
          <div>
            <label class="block text-xs font-medium text-stone-600 dark:text-plumage-300 mb-1">First-of-Day Min Confidence</label>
            <input id="mqtt-fod" type="number" step="0.01" min="0" max="1" value="${{m.first_of_day_min_confidence}}" class="${{inp()}}">
          </div>
          <div class="flex items-end">
            <label class="flex items-center gap-2 text-sm cursor-pointer pb-2">
              <input id="mqtt-ha" type="checkbox" ${{m.homeassistant_discovery ? 'checked' : ''}} class="rounded border-stone-300 dark:border-plumage-700 text-nuthatch-600 focus:ring-nuthatch-500">
              HA Discovery
            </label>
          </div>
        </div>
      </div>
      <div class="flex items-center gap-2 mt-2">
        <button onclick="saveMqtt()" class="px-3 py-1.5 rounded-lg bg-nuthatch-600 text-white text-sm font-medium hover:bg-nuthatch-700 transition-colors">Save MQTT</button>
        <span id="mqtt-save-status" class="text-xs"></span>
      </div>
      <p id="mqtt-running-status" class="text-xs text-stone-400 dark:text-plumage-500"></p>
    </div>`;

    document.getElementById('mqtt-enabled').addEventListener('change', function() {{
      document.getElementById('mqtt-fields').classList.toggle('opacity-50', !this.checked);
      document.getElementById('mqtt-fields').classList.toggle('pointer-events-none', !this.checked);
    }});

    if (m.running) {{
      status.innerHTML = '<span class="text-emerald-500">&#x2022;</span> ' + m.host + ':' + m.port;
      const rs = document.getElementById('mqtt-running-status');
      if (rs) rs.innerHTML = '<span class="text-emerald-500">&#x2022; Connected</span>';
    }} else if (m.enabled && m.host) {{
      status.innerHTML = '<span class="text-amber-500">&#x2022;</span> ' + m.host + ':' + m.port;
    }}
  }}).catch(() => {{
    area.innerHTML = '<p class="text-sm text-red-400">Failed to load MQTT config</p>';
  }});

  window.saveMqtt = function() {{
    const st = document.getElementById('mqtt-save-status');
    const body = {{
      enabled: document.getElementById('mqtt-enabled').checked,
      host: document.getElementById('mqtt-host').value.trim(),
      port: parseInt(document.getElementById('mqtt-port').value, 10) || 1883,
      username: document.getElementById('mqtt-user').value.trim() || null,
      password: document.getElementById('mqtt-pass').value || null,
      first_of_day_min_confidence: parseFloat(document.getElementById('mqtt-fod').value) || 0.75,
      homeassistant_discovery: document.getElementById('mqtt-ha').checked,
      homeassistant_prefix: 'homeassistant',
    }};
    st.textContent = 'Saving...';
    fetch('/api/v1/mqtt', {{
      method: 'PUT',
      headers: {{ 'Content-Type': 'application/json' }},
      body: JSON.stringify(body),
    }}).then(async r => {{
      const text = await r.text();
      if (!r.ok) throw new Error(text);
      st.innerHTML = '<span class="text-emerald-500">Saved &amp; applied</span>';
      // Refresh running status after a brief delay for connection to establish
      setTimeout(() => {{
        fetch('/api/v1/mqtt').then(r => r.json()).then(d => {{
          const rs = document.getElementById('mqtt-running-status');
          if (rs) rs.innerHTML = d.running
            ? '<span class="text-emerald-500">&#x2022; Connected</span>'
            : d.enabled ? '<span class="text-amber-500">&#x2022; Connecting...</span>' : '';
          const hs = document.getElementById('mqtt-status');
          if (hs) hs.innerHTML = d.running ? '<span class="text-emerald-500">&#x2022;</span> ' + d.host + ':' + d.port : '';
        }});
      }}, 2000);
    }}).catch(e => {{
      st.innerHTML = '<span class="text-red-500">' + e.message + '</span>';
    }});
  }};
}})();
</script>"##,
        station_name = settings.station_name,
        lat = lat,
        lon = lon,
        station_id = initial.station_id,
        timezone = settings.timezone,
        display_min_confidence = display_min_confidence,
        birdnet_section = if has_birdnet {{ format!(
            r#"<div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <h3 class="text-sm font-semibold text-gray-900 dark:text-plumage-100 uppercase tracking-wider mb-4">BirdNET</h3>
    <div class="grid gap-4 sm:grid-cols-2">
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Min Confidence</label>
        <input name="birdnet_min_confidence" type="number" step="0.01" min="0" max="1" value="{birdnet_conf}"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
      </div>
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Top K</label>
        <input name="birdnet_top_k" type="number" min="1" max="100" value="{birdnet_topk}"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
      </div>
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Meta Threshold</label>
        <input name="birdnet_meta_threshold" type="number" step="0.001" min="0" max="1" value="{birdnet_meta}"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
      </div>
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Force Allow (eBird codes)</label>
        <input name="birdnet_force_allow" type="text" value="{birdnet_allow}" placeholder="e.g. helgui1, redjun1"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
      </div>
    </div>
    <p class="mt-2 text-xs text-gray-400 dark:text-plumage-500">Model and labels paths require restart to change.</p>
  </div>"#,
            birdnet_conf = birdnet_conf,
            birdnet_topk = birdnet_topk,
            birdnet_meta = birdnet_meta,
            birdnet_allow = birdnet_allow,
        )}} else { String::new() },
        perch_section = if has_perch {{ format!(
            r#"<div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <h3 class="text-sm font-semibold text-gray-900 dark:text-plumage-100 uppercase tracking-wider mb-4">Perch</h3>
    <div class="grid gap-4 sm:grid-cols-2">
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Min Confidence</label>
        <input name="perch_min_confidence" type="number" step="0.01" min="0" max="1" value="{perch_conf}"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
      </div>
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Top K</label>
        <input name="perch_top_k" type="number" min="1" max="100" value="{perch_topk}"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
      </div>
    </div>
    <p class="mt-2 text-xs text-gray-400 dark:text-plumage-500">Model and labels paths require restart to change.</p>
  </div>"#,
            perch_conf = perch_conf,
            perch_topk = perch_topk,
        )}} else { String::new() },
    )
}
