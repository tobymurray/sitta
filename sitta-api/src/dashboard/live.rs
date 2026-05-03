//! Live-feed dashboard content (the home page).

use crate::visualization;

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

  // Start-of-today in the station's timezone, as Unix ms.
  function startOfTodayMs() {{
    // Get today's date string in station TZ: "YYYY-MM-DD"
    const dateStr = new Date().toLocaleDateString('en-CA', {{ timeZone: _tz }});
    // Compute TZ offset at that date's midnight
    const refDate = new Date(dateStr + 'T00:00:00Z');
    const utcStr = refDate.toLocaleString('en-US', {{ timeZone: 'UTC' }});
    const tzStr = refDate.toLocaleString('en-US', {{ timeZone: _tz }});
    const offsetMs = new Date(tzStr).getTime() - new Date(utcStr).getTime();
    // Midnight in station TZ = midnight UTC minus the offset
    return refDate.getTime() - offsetMs;
  }}

  function timeAgo(iso) {{
    const s = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
    if (s < 5) return 'just now';
    if (s < 60) return s + 's ago';
    if (s < 3600) return Math.floor(s/60) + 'm ago';
    return new Date(iso).toLocaleTimeString('en-GB', _tf);
  }}

  // Audio playback (window.playClip) and spectrogram seek (window.seekSpectrogram)
  // are installed by the global window.sitta IIFE — see dashboard::page().

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

  window.deleteDetection = function(id, card) {{
    if (!confirm('Delete this detection? This cannot be undone.')) return;
    fetch('/api/v1/detections/' + id, {{ method: 'DELETE' }}).then(r => {{
      if (!r.ok) return;
      card.style.transition = 'opacity 0.3s, max-height 0.3s';
      card.style.opacity = '0';
      card.style.maxHeight = card.scrollHeight + 'px';
      requestAnimationFrame(() => {{ card.style.maxHeight = '0'; card.style.overflow = 'hidden'; card.style.padding = '0'; card.style.margin = '0'; card.style.border = 'none'; }});
      setTimeout(() => card.remove(), 300);
    }});
  }}

  // Bucketing window: matches the server-side default for /api/v1/dashboard/feed.
  // Two non-rare detections of the same species fold into one card if they
  // arrive within this many seconds of each other.
  const BUCKET_SECONDS = 1800;

  function isRare(rar) {{
    return !!rar && (rar.first_ever || rar.first_season || rar.first_week || rar.first_day || rar.score >= 0.6);
  }}

  // Build (or rebuild) a bucket card. `item` shape:
  //   {{ best: <detection>, first_detected_at: ISO, last_detected_at: ISO, count: N }}
  // `existing` is an optional DOM element to replace in-place (preserves
  // animation classes etc.); otherwise a new element is created.
  function createCard(item, existing) {{
    const d = item.best;
    const [badge, bar] = confColor(d.confidence);
    const pct = Math.round(d.confidence * 100);
    const card = existing || document.createElement('div');
    if (!existing) {{
      card.className = 'slide-in bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4 transition-all';
    }}
    card.dataset.id = d.id;
    card.dataset.species = d.species.scientific_name;
    card.dataset.firstMs = String(new Date(item.first_detected_at).getTime());
    card.dataset.lastMs = String(new Date(item.last_detected_at).getTime());
    card.dataset.count = String(item.count);
    card.dataset.bestConfidence = String(d.confidence);
    card.dataset.rare = isRare(d.rarity) ? '1' : '0';

    const sciEnc = encodeURIComponent(d.species.scientific_name);
    const lastTimeAgo = timeAgo(item.last_detected_at);
    const multi = item.count > 1;
    card.innerHTML = `
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2 flex-wrap">
            <a href="/species/${{sciEnc}}" class="font-semibold text-base truncate hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">${{window.sitta.esc(d.species.common_name)}}</a>
            ${{window.sitta.confidenceBadge(d)}}
            ${{window.sitta.rarityBadges(d)}}
          </div>
          <p class="text-sm text-gray-500 dark:text-plumage-400 italic mt-0.5">
            <a href="/species/${{sciEnc}}" class="hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">${{window.sitta.esc(d.species.scientific_name)}}</a>
          </p>
          <div class="flex items-center gap-3 mt-2 text-xs text-gray-400 dark:text-plumage-500 flex-wrap">
            <span>${{d.model}} ${{d.model_version}}</span>
            ${{d.source_name ? '<span class="before:content-[\\\"\\u00b7\\\"] before:mr-3">' + window.sitta.esc(d.source_name) + '</span>' : ''}}
            <a href="/detections/${{d.id}}" class="before:content-[\\\"\\u00b7\\\"] before:mr-3 hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">${{multi ? 'last heard ' + lastTimeAgo : lastTimeAgo}}</a>
            ${{d.range_unverified ? '<span class="px-1.5 py-0.5 text-[10px] font-medium rounded bg-amber-50 text-amber-700 ring-1 ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-300 dark:ring-amber-400/20" title="Species not in BirdNET range model — not verified by geographic filter">Range unverified</span>' : ''}}
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
      ${{window.sitta.spectrogramBlock(d, {{ height: 'h-16', showPlaceholder: false }})}}
      <div class="mt-3 pt-3 border-t border-gray-100 dark:border-plumage-800 flex items-center justify-between">
        <div class="flex items-center gap-2">
          ${{window.sitta.playButton(d)}}${{d.has_audio || d.snippet_path ? `<a href="/api/v1/detections/${{d.id}}/audio" download="${{(d.species?.common_name || 'clip').replace(/[^a-zA-Z0-9 ]/g, '').replace(/ +/g, '_')}}_${{(d.detected_at || '').replace(/[:.]/g, '-').slice(0, 19)}}.wav" class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium bg-plumage-50 text-plumage-700 hover:bg-plumage-100 dark:bg-plumage-800 dark:text-plumage-300 dark:hover:bg-plumage-700 transition-colors" title="Download audio clip"><svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5M16.5 12L12 16.5m0 0L7.5 12m4.5 4.5V3"/></svg></a>` : ''}}
          <button onclick="reviewDetection('${{d.id}}', 'correct', this.closest('[data-id]'))" class="inline-flex items-center gap-1 px-2 py-1 rounded-lg text-xs font-medium text-emerald-700 hover:bg-emerald-50 dark:text-emerald-400 dark:hover:bg-emerald-900/30 transition-colors" title="Mark correct (c)">
            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M4.5 12.75l6 6 9-13.5"/></svg>
          </button>
          <button onclick="reviewDetection('${{d.id}}', 'false_positive', this.closest('[data-id]'))" class="inline-flex items-center gap-1 px-2 py-1 rounded-lg text-xs font-medium text-red-600 hover:bg-red-50 dark:text-red-400 dark:hover:bg-red-900/30 transition-colors" title="False positive (f)">
            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/></svg>
          </button>
          <button onclick="deleteDetection('${{d.id}}', this.closest('[data-id]'))" class="inline-flex items-center gap-1 px-2 py-1 rounded-lg text-xs font-medium text-gray-400 hover:text-red-600 hover:bg-red-50 dark:text-plumage-500 dark:hover:text-red-400 dark:hover:bg-red-900/30 transition-colors" title="Delete detection">
            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0"/></svg>
          </button>
        </div>
        <div class="review-strip"></div>
      </div>
      ${{multi ? `
        <div class="mt-3 pt-3 border-t border-gray-100 dark:border-plumage-800 flex items-center justify-between text-xs text-gray-500 dark:text-plumage-400" data-bucket-footer="1">
          <span><span class="font-semibold text-stone-700 dark:text-plumage-200">${{item.count}}</span> detections &middot; first heard ${{timeAgo(item.first_detected_at)}}</span>
          <a href="/species/${{sciEnc}}" class="text-nuthatch-600 dark:text-nuthatch-400 hover:underline font-medium">All detections of this species &rarr;</a>
        </div>` : ''}}
      ${{d.alternatives && d.alternatives.length > 0 ? `
        <div class="mt-3 pt-3 border-t border-gray-100 dark:border-plumage-800">
          <p class="text-xs text-gray-400 dark:text-plumage-500 mb-1.5">Alternatives</p>
          <div class="flex flex-wrap gap-2">
            ${{d.alternatives.slice(0, 3).map(a => `<a href="/species/${{encodeURIComponent(a.scientific_name)}}" class="text-xs bg-gray-100 dark:bg-plumage-800 px-2 py-0.5 rounded hover:bg-gray-200 dark:hover:bg-plumage-700 transition-colors">${{window.sitta.esc(a.common_name)}} <span class="text-gray-400 dark:text-plumage-500">${{Math.round(a.confidence * 100)}}%</span></a>`).join('')}}
          </div>
        </div>` : ''}}`;
    return card;
  }}

  // Build a synthetic single-detection bucket from a raw SSE detection event.
  function bucketFromDetection(d) {{
    return {{
      best: d,
      first_detected_at: d.detected_at,
      last_detected_at: d.detected_at,
      count: 1,
    }};
  }}

  // Reconstruct a bucket-shaped object from a card's data-* attributes so we
  // can fold a new detection into it.
  function bucketFromCard(card) {{
    return {{
      best: {{
        id: card.dataset.id,
        confidence: parseFloat(card.dataset.bestConfidence),
        species: {{ scientific_name: card.dataset.species }},
      }},
      first_ms: parseInt(card.dataset.firstMs, 10),
      last_ms: parseInt(card.dataset.lastMs, 10),
      count: parseInt(card.dataset.count, 10),
    }};
  }}

  // Try to fold detection `d` into one of the cards already in the feed.
  // Returns the card it folded into (and re-renders + moves to top), or null
  // if no fold target exists.
  function foldIntoExisting(d) {{
    if (isRare(d.rarity)) return null;
    const sci = d.species.scientific_name;
    const dMs = new Date(d.detected_at).getTime();
    // Iterate from newest to oldest; first match wins.
    const cards = feed.querySelectorAll(':scope > [data-species]');
    for (const card of cards) {{
      if (card.dataset.species !== sci) continue;
      if (card.dataset.rare === '1') continue;
      const firstMs = parseInt(card.dataset.firstMs, 10);
      const lastMs = parseInt(card.dataset.lastMs, 10);
      // Session-style merge: fold whenever the gap to the bucket's most
      // recent detection is within the window. Matches the backend rule
      // (a bucket stays open as long as consecutive detections keep
      // arriving inside `bucket_seconds`), so a chatty species ends up
      // on one card no matter how long the page is open.
      if (dMs - lastMs > BUCKET_SECONDS * 1000) continue;
      // Build the merged bucket and rebuild the card in place.
      const existingBest = bucketFromCard(card);
      const newBestIsBetter = d.confidence > existingBest.best.confidence;
      const merged = {{
        best: newBestIsBetter ? d : null, // we'll fill below if not new-best
        first_detected_at: new Date(Math.min(firstMs, dMs)).toISOString(),
        last_detected_at: new Date(Math.max(lastMs, dMs)).toISOString(),
        count: existingBest.count + 1,
      }};
      if (newBestIsBetter) {{
        createCard(merged, card);
      }} else {{
        // Best detection is unchanged. Keep its rendered content, just bump
        // count/last/timeAgo. We rebuild from the existing best by parsing
        // dataset values back — but we don't have full best data on the card.
        // Simpler: shallow-update the data attributes and the visible bits.
        card.dataset.lastMs = String(new Date(merged.last_detected_at).getTime());
        card.dataset.firstMs = String(new Date(merged.first_detected_at).getTime());
        card.dataset.count = String(merged.count);
        // Re-render the meta + footer text bits via lightweight DOM edits.
        const meta = card.querySelector('.flex.items-center.gap-3.mt-2');
        if (meta) {{
          const timeLink = meta.querySelector('a[href^="/detections/"]');
          if (timeLink) timeLink.textContent = 'last heard ' + timeAgo(merged.last_detected_at);
        }}
        // Replace (or insert) the bucket footer.
        let bucketFooter = card.querySelector('[data-bucket-footer]');
        if (!bucketFooter) {{
          bucketFooter = document.createElement('div');
          bucketFooter.dataset.bucketFooter = '1';
          bucketFooter.className = 'mt-3 pt-3 border-t border-gray-100 dark:border-plumage-800 flex items-center justify-between text-xs text-gray-500 dark:text-plumage-400';
          card.appendChild(bucketFooter);
        }}
        const sciEnc2 = encodeURIComponent(sci);
        bucketFooter.innerHTML =
          '<span><span class="font-semibold text-stone-700 dark:text-plumage-200">' + merged.count + '</span> detections &middot; first heard ' + timeAgo(merged.first_detected_at) + '</span>' +
          '<a href="/species/' + sciEnc2 + '" class="text-nuthatch-600 dark:text-nuthatch-400 hover:underline font-medium">All detections of this species &rarr;</a>';
      }}
      // Move to top with a brief glow so the user notices it just sang again.
      if (feed.firstChild !== card) feed.prepend(card);
      card.classList.remove('slide-in');
      void card.offsetWidth; // restart animation
      card.classList.add('slide-in');
      return card;
    }}
    return null;
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

  // Initial load: fetch already-bucketed feed from server.
  fetch('/api/v1/dashboard/feed?bucket_seconds=' + BUCKET_SECONDS + '&limit=50')
    .then(r => r.json())
    .then(items => {{
      if (!Array.isArray(items)) return;
      if (items.length > 0 && emptyState) emptyState.remove();
      // Append in returned order (already DESC by last_detected_at).
      items.forEach(item => {{
        const card = createCard(item);
        card.classList.remove('slide-in');
        feed.appendChild(card);
        count += item.count;
      }});
      document.getElementById('detection-count').textContent = count + ' shown';
    }})
    .catch(() => {{}});

  // Load stats (scoped to calendar today in station timezone)
  function loadStats() {{
    fetch('/api/v1/species?since=' + startOfTodayMs())
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

  // SSE live feed.
  const sse = new EventSource('/api/v1/stream/events');
  window.addEventListener('beforeunload', () => sse.close());
  sse.addEventListener('detection', (e) => {{
    const d = JSON.parse(e.data);
    if (emptyState) emptyState.remove();
    // Try to fold into an existing same-species bucket within the window
    // (rare detections always get their own card via isRare() inside).
    const folded = foldIntoExisting(d);
    if (!folded) {{
      const card = createCard(bucketFromDetection(d));
      feed.prepend(card);
    }}
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

