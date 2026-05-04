//! Embedded HTML dashboard pages.
//!
//! Each page is rendered by wrapping page-specific content in a shared shell
//! (sidebar, header, Tailwind/htmx CDN scripts). No template engine — just
//! `format!()` with a layout string. Per-page content lives in submodules
//! and is re-exported here so `dashboard::xxx_content(..)` keeps working.

mod detection;
mod diagnostics;
mod individuals;
mod live;
mod rare;
mod settings_page;
mod species;
mod species_detail;
mod status;
mod today;

pub use detection::detection_detail_content;
pub use diagnostics::diagnostics_content;
pub use individuals::individuals_content;
pub use live::dashboard_content;
pub use rare::rare_content;
pub use settings_page::settings_content;
pub use species::species_content;
pub use species_detail::species_detail_content;
pub use status::status_content;
pub use today::today_content;

use axum::response::Html;

/// Render a full HTML page with the shared shell.
pub fn page(title: &str, active: &str, content: &str, timezone: &str) -> Html<String> {
    Html(format!(
        r##"<!DOCTYPE html>
<html lang="en" class="h-full">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — Sitta</title>
<script src="https://cdn.tailwindcss.com"></script>
<script>
tailwind.config = {{
  darkMode: 'class',
  theme: {{
    extend: {{
      fontFamily: {{ sans: ['system-ui', '-apple-system', 'sans-serif'] }},
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
  .no-scrollbar {{ -ms-overflow-style: none; scrollbar-width: none; }}
  .no-scrollbar::-webkit-scrollbar {{ display: none; }}
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
      {nav_today}
      {nav_species}
      {nav_rare}
      {nav_status}
      {nav_diagnostics}
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
      <p class="text-xs text-stone-400 dark:text-plumage-600 mt-2">Sitta v{sitta_version}</p>
    </div>
  </nav>

  <!-- Main content -->
  <div class="flex-1 flex flex-col min-w-0">

    <!-- Mobile header -->
    <header class="lg:hidden bg-white dark:bg-plumage-900 border-b border-stone-200 dark:border-plumage-800">
      <div class="flex items-center gap-2 px-3 py-2">
        <div class="w-6 h-6 rounded-md bg-gradient-to-br from-plumage-500 to-nuthatch-500 flex items-center justify-center flex-shrink-0">
          <svg class="w-3.5 h-3.5 text-white" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M12 3c-1.5 0-3 .5-4 2-1.5 2-1 5 1 7l3 3 3-3c2-2 2.5-5 1-7-1-1.5-2.5-2-4-2z"/></svg>
        </div>
        <span class="text-sm font-bold bg-gradient-to-r from-plumage-300 to-nuthatch-400 bg-clip-text text-transparent flex-shrink-0">Sitta</span>
        <nav class="ml-auto flex gap-0.5 overflow-x-auto no-scrollbar">
          <a href="/" class="px-2 py-1 text-xs rounded-md whitespace-nowrap {mob_dashboard}">Live</a>
          <a href="/today" class="px-2 py-1 text-xs rounded-md whitespace-nowrap {mob_today}">Today</a>
          <a href="/species" class="px-2 py-1 text-xs rounded-md whitespace-nowrap {mob_species}">Species</a>
          <a href="/rare" class="px-2 py-1 text-xs rounded-md whitespace-nowrap {mob_rare}">Rare</a>
          <a href="/status" class="px-2 py-1 text-xs rounded-md whitespace-nowrap {mob_status}">Status</a>
          <a href="/diagnostics" class="px-2 py-1 text-xs rounded-md whitespace-nowrap {mob_diagnostics}">Audio</a>
          <a href="/individuals" class="px-2 py-1 text-xs rounded-md whitespace-nowrap {mob_individuals}">Ind.</a>
          <a href="/settings" class="px-2 py-1 text-xs rounded-md whitespace-nowrap {mob_settings}">
            <svg class="w-3.5 h-3.5 inline-block" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.324.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 011.37.49l1.296 2.247a1.125 1.125 0 01-.26 1.431l-1.003.827c-.293.24-.438.613-.431.992a6.759 6.759 0 010 .255c-.007.378.138.75.43.99l1.005.828c.424.35.534.954.26 1.43l-1.298 2.247a1.125 1.125 0 01-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.57 6.57 0 01-.22.128c-.331.183-.581.495-.644.869l-.213 1.28c-.09.543-.56.941-1.11.941h-2.594c-.55 0-1.02-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 01-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 01-1.369-.49l-1.297-2.247a1.125 1.125 0 01.26-1.431l1.004-.827c.292-.24.437-.613.43-.992a6.932 6.932 0 010-.255c.007-.378-.138-.75-.43-.99l-1.004-.828a1.125 1.125 0 01-.26-1.43l1.297-2.247a1.125 1.125 0 011.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.087.22-.128.332-.183.582-.495.644-.869l.214-1.281z"/><path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/></svg>
          </a>
        </nav>
      </div>
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

// ── Shared detection rendering helpers (global) ────────────────
// `window.sitta` is the page-level helper namespace. Card renderers on every
// page (dashboard, species detail, detection detail, future rare/timeline)
// share these primitives so links are consistent app-wide.
window.sitta = (function() {{
  function esc(s) {{ return String(s == null ? '' : s).replace(/[&<>"']/g, c => ({{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}}[c])); }}

  function confidenceBadge(d) {{
    const pct = Math.round(d.confidence * 100);
    const cls = pct >= 80
      ? 'text-emerald-700 bg-emerald-50 ring-emerald-600/20 dark:text-emerald-400 dark:bg-emerald-900/30 dark:ring-emerald-400/20'
      : pct >= 50
        ? 'text-amber-700 bg-amber-50 ring-amber-600/20 dark:text-amber-400 dark:bg-amber-900/30 dark:ring-amber-400/20'
        : 'text-red-700 bg-red-50 ring-red-600/20 dark:text-red-400 dark:bg-red-900/30 dark:ring-red-400/20';
    return '<span class="inline-flex items-center rounded-md px-2 py-0.5 text-xs font-medium ring-1 ring-inset ' + cls + '">' + pct + '%</span>';
  }}

  function individualBadge(d) {{
    if (!d.individual) return '';
    const sim = Math.round((d.individual.similarity || 0) * 100);
    return '<a href="/individuals" title="Matched ' + esc(d.individual.label) + ' at ' + sim + '% similarity" class="inline-flex items-center gap-1 px-1.5 py-0.5 text-[10px] font-semibold rounded ring-1 bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-300 dark:ring-emerald-400/20 hover:opacity-90 transition-opacity">' +
      '<svg class="w-2.5 h-2.5" fill="currentColor" viewBox="0 0 20 20"><path d="M10 9a3 3 0 100-6 3 3 0 000 6zm-7 9a7 7 0 1114 0H3z"/></svg>' +
      esc(d.individual.label) +
    '</a>';
  }}

  function rarityBadges(d) {{
    if (!d.rarity) return '';
    const r = d.rarity;
    let html = '';
    const chip = (filter, cls, label) => '<a href="/rare?filter=' + filter + '" title="See other ' + label.toLowerCase() + ' detections" class="px-1.5 py-0.5 text-[10px] font-semibold rounded ring-1 hover:opacity-90 transition-opacity ' + cls + '">' + label + '</a>';
    if (r.first_ever) {{
      html += chip('first_ever', 'bg-purple-100 text-purple-700 ring-purple-600/20 dark:bg-purple-900/40 dark:text-purple-300 dark:ring-purple-400/20', 'First ever');
    }} else if (r.first_season) {{
      html += chip('first_season', 'bg-blue-100 text-blue-700 ring-blue-600/20 dark:bg-blue-900/40 dark:text-blue-300 dark:ring-blue-400/20', 'First of season');
    }} else if (r.first_week) {{
      html += chip('first_week', 'bg-teal-100 text-teal-700 ring-teal-600/20 dark:bg-teal-900/40 dark:text-teal-300 dark:ring-teal-400/20', 'First this week');
    }} else if (r.first_day) {{
      html += chip('first_day', 'bg-sky-100 text-sky-700 ring-sky-600/20 dark:bg-sky-900/40 dark:text-sky-300 dark:ring-sky-400/20', 'First today');
    }}
    if (r.score >= 0.6 && !r.first_ever) {{
      html += chip('high_score', 'bg-amber-100 text-amber-700 ring-amber-600/20 dark:bg-amber-900/40 dark:text-amber-300 dark:ring-amber-400/20', 'Rare');
    }}
    return html;
  }}

  function fmtDateTime(iso, tz) {{
    return new Date(iso).toLocaleString('en-GB', {{ month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false, timeZone: tz || 'UTC' }});
  }}

  // ── Detection card sub-components ─────────────────────────────
  // Markup duplicated across the dashboard live feed, species detail,
  // /rare, and the detection-detail correlated panel. Page-level layouts
  // around the card frame differ (donut/no-donut, alternatives, footers),
  // so we expose the genuinely-shared pieces rather than one mega-card.
  const _PLAY_SVG = '<svg class="w-4 h-4" fill="currentColor" viewBox="0 0 20 20"><path d="M6.3 2.84A1.5 1.5 0 004 4.11v11.78a1.5 1.5 0 002.3 1.27l9.344-5.891a1.5 1.5 0 000-2.538L6.3 2.841z"/></svg>';

  function spectrogramBlock(d, opts) {{
    opts = opts || {{}};
    const heightCls = opts.height || 'h-20';
    const showPlaceholder = opts.showPlaceholder !== false;
    if (d.has_audio || d.snippet_path) {{
      return '<div class="mt-3 relative cursor-pointer group" id="spect-' + d.id + '" onclick="seekSpectrogram(event, \'' + d.id + '\')">' +
        '<img src="/api/v1/detections/' + d.id + '/spectrogram" loading="lazy" class="w-full ' + heightCls + ' rounded-lg object-cover bg-gray-100 dark:bg-plumage-800" alt="spectrogram" onerror="this.parentElement.style.display=\'none\'"/>' +
        '<div class="playhead absolute top-0 bottom-0 w-0.5 bg-white/80 dark:bg-nuthatch-400/80 pointer-events-none transition-none" style="left:0%;display:none"></div>' +
        '<div class="absolute inset-0 rounded-lg bg-black/0 group-hover:bg-black/5 dark:group-hover:bg-white/5 transition-colors pointer-events-none"></div>' +
      '</div>';
    }}
    if (!showPlaceholder) return '';
    return '<div class="mt-3 px-3 py-2 rounded-lg bg-gray-50 dark:bg-plumage-800/50 text-xs text-gray-400 dark:text-plumage-500 italic">No audio clip on disk &middot; <a href="/diagnostics" class="hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors not-italic">why?</a></div>';
  }}

  function playButton(d) {{
    if (!(d.has_audio || d.snippet_path)) return '';
    return '<button onclick="playClip(\'' + d.id + '\', this)" class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium bg-plumage-50 text-plumage-700 hover:bg-plumage-100 dark:bg-plumage-800 dark:text-plumage-300 dark:hover:bg-plumage-700 transition-colors">' + _PLAY_SVG + ' Play</button>';
  }}

  // ── Audio player + spectrogram seek (shared across pages) ─────
  // Buttons are rendered server-side (each page chooses its own SVG size /
  // label), so the player saves the button's original innerHTML the first
  // time it activates and restores it on stop. One global player at a time:
  // playing a new clip stops the previous one. The spectrogram playhead
  // syncs to the current play position via requestAnimationFrame.
  let _audio = null, _id = null, _frame = null;
  const _STOP_HTML = '<svg class="w-4 h-4" fill="currentColor" viewBox="0 0 20 20"><rect x="4" y="4" width="12" height="12" rx="1.5"/></svg> Stop';

  function _stop() {{
    if (_frame) {{ cancelAnimationFrame(_frame); _frame = null; }}
    if (_audio) {{ _audio.pause(); _audio = null; }}
    document.querySelectorAll('.clip-playing').forEach(b => {{
      b.classList.remove('clip-playing');
      if (b._sittaDefaultHtml != null) b.innerHTML = b._sittaDefaultHtml;
    }});
    document.querySelectorAll('.playhead').forEach(ph => ph.style.display = 'none');
    _id = null;
  }}

  function _animatePlayhead() {{
    if (!_audio || _audio.paused) return;
    const ph = document.querySelector('#spect-' + CSS.escape(_id) + ' .playhead');
    if (ph && _audio.duration) {{
      ph.style.left = (_audio.currentTime / _audio.duration * 100) + '%';
      ph.style.display = '';
    }}
    _frame = requestAnimationFrame(_animatePlayhead);
  }}

  function _markPlaying(btn) {{
    if (!btn) return;
    if (btn._sittaDefaultHtml == null) btn._sittaDefaultHtml = btn.innerHTML;
    btn.classList.add('clip-playing');
    btn.innerHTML = _STOP_HTML;
  }}

  function playClip(id, btn) {{
    if (_id === id && _audio && !_audio.paused) {{ _stop(); return; }}
    _stop();
    _id = id;
    _audio = new Audio('/api/v1/detections/' + id + '/audio');
    _audio.play();
    _markPlaying(btn);
    _animatePlayhead();
    _audio.onended = () => _stop();
  }}

  function seekSpectrogram(event, id) {{
    const rect = event.currentTarget.getBoundingClientRect();
    const pct = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
    if (_id === id && _audio) {{
      _audio.currentTime = pct * _audio.duration;
      if (_audio.paused) _audio.play();
    }} else {{
      _stop();
      _id = id;
      _audio = new Audio('/api/v1/detections/' + id + '/audio');
      _audio.addEventListener('loadedmetadata', () => {{
        _audio.currentTime = pct * _audio.duration;
        _audio.play();
        _animatePlayhead();
      }});
      const btn = event.currentTarget.parentElement.querySelector('[onclick*="playClip"]');
      _markPlaying(btn);
      _audio.onended = () => _stop();
    }}
  }}

  return {{ esc, confidenceBadge, individualBadge, rarityBadges, fmtDateTime, spectrogramBlock, playButton, playClip, seekSpectrogram }};
}})();
window.playClip = window.sitta.playClip;
window.seekSpectrogram = window.sitta.seekSpectrogram;

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
        sitta_version = env!("CARGO_PKG_VERSION"),
        nav_dashboard = nav_item("Dashboard", "/", "dashboard", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M3.75 6A2.25 2.25 0 016 3.75h2.25A2.25 2.25 0 0110.5 6v2.25a2.25 2.25 0 01-2.25 2.25H6a2.25 2.25 0 01-2.25-2.25V6zM3.75 15.75A2.25 2.25 0 016 13.5h2.25a2.25 2.25 0 012.25 2.25V18a2.25 2.25 0 01-2.25 2.25H6A2.25 2.25 0 013.75 18v-2.25zM13.5 6a2.25 2.25 0 012.25-2.25H18A2.25 2.25 0 0120.25 6v2.25A2.25 2.25 0 0118 10.5h-2.25a2.25 2.25 0 01-2.25-2.25V6zM13.5 15.75a2.25 2.25 0 012.25-2.25H18a2.25 2.25 0 012.25 2.25V18A2.25 2.25 0 0118 20.25h-2.25A2.25 2.25 0 0113.5 18v-2.25z"/>"#),
        nav_today = nav_item("Today", "/today", "today", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M6.75 3v2.25M17.25 3v2.25M3 18.75V7.5a2.25 2.25 0 012.25-2.25h13.5A2.25 2.25 0 0121 7.5v11.25m-18 0A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75m-18 0v-7.5A2.25 2.25 0 015.25 9h13.5A2.25 2.25 0 0121 11.25v7.5"/>"#),
        nav_species = nav_item("Species", "/species", "species", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M3.75 12h16.5m-16.5 3.75h16.5M3.75 19.5h16.5M5.625 4.5h12.75a1.875 1.875 0 010 3.75H5.625a1.875 1.875 0 010-3.75z"/>"#),
        nav_rare = nav_item("Rare moments", "/rare", "rare", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M11.48 3.499a.562.562 0 011.04 0l2.125 5.111a.563.563 0 00.475.345l5.518.442c.499.04.701.663.321.988l-4.204 3.602a.563.563 0 00-.182.557l1.285 5.385a.562.562 0 01-.84.61l-4.725-2.885a.563.563 0 00-.586 0L6.982 20.54a.562.562 0 01-.84-.61l1.285-5.386a.562.562 0 00-.182-.557l-4.204-3.602a.563.563 0 01.321-.988l5.518-.442a.563.563 0 00.475-.345L11.48 3.5z"/>"#),
        nav_status = nav_item("Status", "/status", "status", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M9.348 14.651a3.75 3.75 0 010-5.303m5.304 0a3.75 3.75 0 010 5.303m-7.425 2.122a6.75 6.75 0 010-9.546m9.546 0a6.75 6.75 0 010 9.546M5.106 18.894c-3.808-3.808-3.808-9.98 0-13.788m13.788 0c3.808 3.808 3.808 9.98 0 13.788M12 12h.008v.008H12V12zm.375 0a.375.375 0 11-.75 0 .375.375 0 01.75 0z"/>"#),
        nav_diagnostics = nav_item("Audio Health", "/diagnostics", "diagnostics", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M3 13.125C3 12.504 3.504 12 4.125 12h.375c.621 0 1.125.504 1.125 1.125v6.75C5.625 20.496 5.121 21 4.5 21h-.375A1.125 1.125 0 013 19.875v-6.75zm6 0c0-.621.504-1.125 1.125-1.125h.375c.621 0 1.125.504 1.125 1.125v6.75c0 .621-.504 1.125-1.125 1.125h-.375A1.125 1.125 0 019 19.875v-6.75zm6-7.5c0-.621.504-1.125 1.125-1.125h.375c.621 0 1.125.504 1.125 1.125v14.25c0 .621-.504 1.125-1.125 1.125h-.375A1.125 1.125 0 0115 19.875V5.625z"/>"#),
        nav_individuals = nav_item("Individuals", "/individuals", "individuals", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M15 19.128a9.38 9.38 0 002.625.372 9.337 9.337 0 004.121-.952 4.125 4.125 0 00-7.533-2.493M15 19.128v-.003c0-1.113-.285-2.16-.786-3.07M15 19.128v.106A12.318 12.318 0 018.624 21c-2.331 0-4.512-.645-6.374-1.766l-.001-.109a6.375 6.375 0 0111.964-3.07M12 6.375a3.375 3.375 0 11-6.75 0 3.375 3.375 0 016.75 0zm8.25 2.25a2.625 2.625 0 11-5.25 0 2.625 2.625 0 015.25 0z"/>"#),
        nav_settings = nav_item("Settings", "/settings", "settings", active,
            r#"<path stroke-linecap="round" stroke-linejoin="round" d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.324.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 011.37.49l1.296 2.247a1.125 1.125 0 01-.26 1.431l-1.003.827c-.293.24-.438.613-.431.992a6.759 6.759 0 010 .255c-.007.378.138.75.43.99l1.005.828c.424.35.534.954.26 1.43l-1.298 2.247a1.125 1.125 0 01-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.57 6.57 0 01-.22.128c-.331.183-.581.495-.644.869l-.213 1.28c-.09.543-.56.941-1.11.941h-2.594c-.55 0-1.02-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 01-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 01-1.369-.49l-1.297-2.247a1.125 1.125 0 01.26-1.431l1.004-.827c.292-.24.437-.613.43-.992a6.932 6.932 0 010-.255c.007-.378-.138-.75-.43-.99l-1.004-.828a1.125 1.125 0 01-.26-1.43l1.297-2.247a1.125 1.125 0 011.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.087.22-.128.332-.183.582-.495.644-.869l.214-1.281z"/><path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>"#),
        mob_dashboard = if active == "dashboard" { "bg-nuthatch-50 text-nuthatch-800 dark:bg-nuthatch-900/30 dark:text-nuthatch-400 font-medium" } else { "text-stone-500 dark:text-plumage-300" },
        mob_today = if active == "today" { "bg-nuthatch-50 text-nuthatch-800 dark:bg-nuthatch-900/30 dark:text-nuthatch-400 font-medium" } else { "text-stone-500 dark:text-plumage-300" },
        mob_species = if active == "species" { "bg-nuthatch-50 text-nuthatch-800 dark:bg-nuthatch-900/30 dark:text-nuthatch-400 font-medium" } else { "text-stone-500 dark:text-plumage-300" },
        mob_rare = if active == "rare" { "bg-nuthatch-50 text-nuthatch-800 dark:bg-nuthatch-900/30 dark:text-nuthatch-400 font-medium" } else { "text-stone-500 dark:text-plumage-300" },
        mob_status = if active == "status" { "bg-nuthatch-50 text-nuthatch-800 dark:bg-nuthatch-900/30 dark:text-nuthatch-400 font-medium" } else { "text-stone-500 dark:text-plumage-300" },
        mob_diagnostics = if active == "diagnostics" { "bg-nuthatch-50 text-nuthatch-800 dark:bg-nuthatch-900/30 dark:text-nuthatch-400 font-medium" } else { "text-stone-500 dark:text-plumage-300" },
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


