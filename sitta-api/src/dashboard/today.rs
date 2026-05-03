//! /today recap page — what happened in the last 24 h.
//!
//! Glanceable summary the user can scroll on a phone first thing in the
//! morning: species count, recording coverage, rare moments, top species,
//! and an hour heatmap. Pulls from existing endpoints (/species, /effort,
//! /detections?rarity=true, /activity/hourly) — no new backend.

pub fn today_content(station_name: &str) -> String {
    format!(
        r##"<div class="mb-5">
  <h1 class="text-2xl font-bold tracking-tight">Today</h1>
  <p class="text-sm text-gray-500 dark:text-plumage-400 mt-0.5" id="today-subtitle">{station_name}</p>
</div>

<!-- Hero stats: 3-up on mobile, scannable -->
<div class="grid grid-cols-3 gap-2 mb-5">
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 px-3 py-3 border-t-2 border-t-nuthatch-500">
    <p class="text-[10px] font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider">Species</p>
    <p id="hero-species" class="text-2xl font-bold mt-1 text-nuthatch-700 dark:text-nuthatch-400">--</p>
  </div>
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 px-3 py-3 border-t-2 border-t-plumage-500">
    <p class="text-[10px] font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider">Detections</p>
    <p id="hero-detections" class="text-2xl font-bold mt-1 text-plumage-700 dark:text-plumage-300">--</p>
  </div>
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 px-3 py-3 border-t-2 border-t-emerald-500">
    <p class="text-[10px] font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider">Coverage</p>
    <p id="hero-coverage" class="text-2xl font-bold mt-1 text-emerald-700 dark:text-emerald-400">--</p>
  </div>
</div>

<!-- Rare moments (only renders if any) -->
<div id="rare-section" class="hidden mb-5 bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-sm font-semibold flex items-center gap-2">
      <svg class="w-4 h-4 text-amber-500" fill="currentColor" viewBox="0 0 20 20"><path d="M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z"/></svg>
      Rare moments
    </h2>
    <a href="/rare" class="text-xs text-nuthatch-600 dark:text-nuthatch-400 hover:underline">All &rarr;</a>
  </div>
  <div id="rare-list" class="space-y-2"></div>
</div>

<!-- Top 5 species -->
<div class="mb-5 bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-sm font-semibold">Most heard</h2>
    <a href="/species" class="text-xs text-nuthatch-600 dark:text-nuthatch-400 hover:underline">All &rarr;</a>
  </div>
  <div id="top-species" class="space-y-2">
    <div class="text-xs text-gray-400 dark:text-plumage-500 italic">Loading...</div>
  </div>
</div>

<!-- Hour heatmap -->
<div class="mb-5 bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
  <h2 class="text-sm font-semibold mb-1">When today was loud</h2>
  <p class="text-[11px] text-gray-400 dark:text-plumage-500 mb-3">Detections per hour, station time</p>
  <div id="hour-heatmap" class="flex items-end gap-0.5 h-20"></div>
  <div class="flex justify-between text-[10px] text-gray-400 dark:text-plumage-500 mt-1 px-0.5">
    <span>00</span><span>06</span><span>12</span><span>18</span><span>23</span>
  </div>
</div>

<!-- Coverage detail -->
<div class="mb-5 bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-sm font-semibold">Listening time</h2>
    <a href="/diagnostics" class="text-xs text-nuthatch-600 dark:text-nuthatch-400 hover:underline">Diagnostics &rarr;</a>
  </div>
  <div id="coverage-detail" class="space-y-2">
    <div class="text-xs text-gray-400 dark:text-plumage-500 italic">Loading...</div>
  </div>
</div>

<script>
(function() {{
  const _tz = document.body.dataset.tz || 'UTC';

  // Window: midnight in the station's TZ to now.
  function startOfTodayMs() {{
    const dateStr = new Date().toLocaleDateString('en-CA', {{ timeZone: _tz }});
    const refDate = new Date(dateStr + 'T00:00:00Z');
    const utcStr = refDate.toLocaleString('en-US', {{ timeZone: 'UTC' }});
    const tzStr = refDate.toLocaleString('en-US', {{ timeZone: _tz }});
    const offsetMs = new Date(tzStr).getTime() - new Date(utcStr).getTime();
    return refDate.getTime() - offsetMs;
  }}

  const SINCE = startOfTodayMs();
  const NOW = Date.now();

  // Subtitle: human date in station TZ.
  const dateLabel = new Date(NOW).toLocaleDateString('en-GB', {{
    weekday: 'long', month: 'long', day: 'numeric', timeZone: _tz
  }});
  document.getElementById('today-subtitle').textContent = dateLabel;

  // ── Top species + hero stats ────────────────────────────────
  fetch('/api/v1/species?since=' + SINCE + '&until=' + NOW)
    .then(r => r.json())
    .then(species => {{
      species.sort((a, b) => b.detection_count - a.detection_count);
      const total = species.reduce((s, d) => s + d.detection_count, 0);
      document.getElementById('hero-species').textContent = species.length;
      document.getElementById('hero-detections').textContent = total.toLocaleString();

      const top = species.slice(0, 5);
      const peak = top[0] ? top[0].detection_count : 1;
      const list = document.getElementById('top-species');
      if (top.length === 0) {{
        list.innerHTML = '<div class="text-xs text-gray-400 dark:text-plumage-500 italic">No detections yet today</div>';
        return;
      }}
      list.innerHTML = top.map((s, i) => {{
        const pct = Math.round((s.detection_count / peak) * 100);
        const rank = i + 1;
        const sciEnc = encodeURIComponent(s.scientific_name);
        return `<a href="/species/${{sciEnc}}" class="block hover:bg-gray-50 dark:hover:bg-plumage-800/40 rounded-lg px-2 py-1.5 -mx-2 -my-1.5 transition-colors">
          <div class="flex items-baseline justify-between gap-2 mb-1">
            <div class="flex items-baseline gap-2 min-w-0">
              <span class="text-[10px] text-gray-400 dark:text-plumage-600 font-mono w-3">${{rank}}</span>
              <span class="text-sm font-medium truncate">${{window.sitta.esc(s.common_name)}}</span>
            </div>
            <span class="text-xs text-gray-500 dark:text-plumage-400 font-mono flex-shrink-0">${{s.detection_count}}</span>
          </div>
          <div class="h-1 bg-gray-100 dark:bg-plumage-800 rounded-full overflow-hidden ml-5">
            <div class="h-full bg-nuthatch-500 dark:bg-nuthatch-400 rounded-full" style="width:${{pct}}%"></div>
          </div>
        </a>`;
      }}).join('');
    }})
    .catch(() => {{
      document.getElementById('top-species').innerHTML =
        '<div class="text-xs text-red-400 italic">Failed to load species</div>';
    }});

  // ── Coverage / effort ───────────────────────────────────────
  fetch('/api/v1/effort?since=' + SINCE + '&until=' + NOW)
    .then(r => r.json())
    .then(eff => {{
      const pct = Math.round((eff.overall_coverage || 0) * 100);
      document.getElementById('hero-coverage').textContent = pct + '%';

      const hours = (eff.total_recording_seconds || 0) / 3600;
      const detail = document.getElementById('coverage-detail');
      if (!eff.sources || eff.sources.length === 0) {{
        detail.innerHTML = '<div class="text-xs text-gray-400 dark:text-plumage-500 italic">No sources configured</div>';
        return;
      }}
      const totalLine = `<div class="flex items-center justify-between text-xs mb-2 pb-2 border-b border-gray-100 dark:border-plumage-800">
        <span class="text-gray-500 dark:text-plumage-400">Total recorded</span>
        <span class="font-medium font-mono">${{hours.toFixed(1)}}h</span>
      </div>`;
      const sourceLines = eff.sources.map(s => {{
        const sPct = Math.round((s.coverage || 0) * 100);
        const sHours = (s.total_seconds || 0) / 3600;
        const barCls = sPct >= 80 ? 'bg-emerald-500' : sPct >= 40 ? 'bg-amber-500' : 'bg-red-500';
        return `<div>
          <div class="flex items-center justify-between text-xs mb-1">
            <span class="truncate">${{window.sitta.esc(s.source_name)}}</span>
            <span class="text-gray-500 dark:text-plumage-400 font-mono">${{sHours.toFixed(1)}}h · ${{sPct}}%</span>
          </div>
          <div class="h-1 bg-gray-100 dark:bg-plumage-800 rounded-full overflow-hidden">
            <div class="h-full ${{barCls}} rounded-full" style="width:${{sPct}}%"></div>
          </div>
        </div>`;
      }}).join('');
      detail.innerHTML = totalLine + sourceLines;
    }})
    .catch(() => {{
      document.getElementById('hero-coverage').textContent = '--';
      document.getElementById('coverage-detail').innerHTML =
        '<div class="text-xs text-red-400 italic">Failed to load effort</div>';
    }});

  // ── Hour heatmap ────────────────────────────────────────────
  fetch('/api/v1/activity/hourly?since=' + SINCE + '&until=' + NOW)
    .then(r => r.json())
    .then(act => {{
      // Aggregate across species into a single 24-element array.
      const hours = new Array(24).fill(0);
      for (const sp of (act.species || [])) {{
        for (let h = 0; h < 24; h++) {{
          hours[h] += sp.hours[h] || 0;
        }}
      }}
      // Server returns UTC hour buckets. Shift into station-local hours.
      const offsetH = stationTzOffsetHours(NOW);
      const local = new Array(24).fill(0);
      for (let h = 0; h < 24; h++) {{
        local[(h + offsetH + 24) % 24] = hours[h];
      }}

      // Mark hours past "now" as not-yet-observable so they don't look like silence.
      const nowLocalHour = parseInt(new Date(NOW).toLocaleString('en-GB', {{
        hour: '2-digit', hour12: false, timeZone: _tz
      }}), 10);
      const peak = Math.max(1, ...local);
      const heatmap = document.getElementById('hour-heatmap');
      heatmap.innerHTML = local.map((c, h) => {{
        const future = h > nowLocalHour;
        const heightPct = future ? 6 : Math.max(c > 0 ? 6 : 2, Math.round((c / peak) * 100));
        const cls = future
          ? 'bg-gray-100 dark:bg-plumage-800/40'
          : c === 0
            ? 'bg-gray-200 dark:bg-plumage-700/60'
            : c < peak * 0.25
              ? 'bg-nuthatch-200 dark:bg-nuthatch-700/40'
              : c < peak * 0.6
                ? 'bg-nuthatch-400 dark:bg-nuthatch-500'
                : 'bg-nuthatch-600 dark:bg-nuthatch-400';
        return `<div class="flex-1 ${{cls}} rounded-sm transition-all" style="height:${{heightPct}}%" title="${{String(h).padStart(2, '0')}}:00 — ${{c}}"></div>`;
      }}).join('');
    }})
    .catch(() => {{
      document.getElementById('hour-heatmap').innerHTML =
        '<div class="text-xs text-red-400 italic">Failed to load activity</div>';
    }});

  // Compute station-TZ hour offset (signed) at the given timestamp. Used to
  // shift the UTC hour buckets the API returns into station-local hours.
  function stationTzOffsetHours(ms) {{
    const d = new Date(ms);
    const utcStr = d.toLocaleString('en-US', {{ timeZone: 'UTC' }});
    const tzStr = d.toLocaleString('en-US', {{ timeZone: _tz }});
    const diff = new Date(tzStr).getTime() - new Date(utcStr).getTime();
    return Math.round(diff / 3600000);
  }}

  // ── Rare moments ────────────────────────────────────────────
  fetch('/api/v1/detections?rarity=true&since=' + SINCE + '&until=' + NOW + '&limit=20')
    .then(r => r.json())
    .then(resp => {{
      const items = resp.items || resp;
      if (!Array.isArray(items) || items.length === 0) return;
      // Sort by rarity tier then confidence (matches /rare).
      items.sort((a, b) => {{
        const rank = (r) => {{
          if (!r) return 99;
          if (r.first_ever) return 0;
          if (r.first_season) return 1;
          if (r.first_week) return 2;
          if (r.first_day) return 3;
          return 4 - (r.score || 0);
        }};
        const ra = rank(a.rarity), rb = rank(b.rarity);
        return ra !== rb ? ra - rb : b.confidence - a.confidence;
      }});
      const top = items.slice(0, 3);
      const list = document.getElementById('rare-list');
      list.innerHTML = top.map(d => {{
        const sciEnc = encodeURIComponent(d.species.scientific_name);
        const time = new Date(d.detected_at).toLocaleTimeString('en-GB', {{
          hour: '2-digit', minute: '2-digit', hour12: false, timeZone: _tz
        }});
        return `<a href="/detections/${{d.id}}" class="block hover:bg-gray-50 dark:hover:bg-plumage-800/40 rounded-lg px-2 py-2 -mx-2 transition-colors">
          <div class="flex items-center gap-2 mb-1 flex-wrap">
            <span class="text-sm font-semibold truncate">${{window.sitta.esc(d.species.common_name)}}</span>
            ${{window.sitta.rarityBadges(d)}}
          </div>
          <div class="text-[11px] text-gray-400 dark:text-plumage-500">${{time}} &middot; ${{Math.round(d.confidence * 100)}}%</div>
        </a>`;
      }}).join('');
      document.getElementById('rare-section').classList.remove('hidden');
    }})
    .catch(() => {{}});
}})();
</script>"##,
        station_name = station_name,
    )
}
