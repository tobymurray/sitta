//! /rare page content.

pub fn rare_content() -> String {
    r##"<div class="mb-6">
  <h1 class="text-2xl font-bold tracking-tight">Rare moments</h1>
  <p class="text-sm text-gray-500 dark:text-plumage-400 mt-0.5">Notable detections from the last 14 days</p>
</div>

<div id="rare-filters" class="flex flex-wrap gap-2 mb-5"></div>

<div id="rare-stats" class="hidden grid grid-cols-2 sm:grid-cols-5 gap-2 mb-5"></div>

<div id="rare-list" class="space-y-3">
  <div class="text-center py-12 text-gray-400 dark:text-plumage-500 text-sm">Loading...</div>
</div>

<script>
(function() {
  const _tz = document.body.dataset.tz || 'UTC';
  const SINCE = Date.now() - 14 * 86400000;

  // Read ?filter=foo from the URL so badge clicks land on the matching tab.
  const params = new URLSearchParams(location.search);
  let activeFilter = params.get('filter') || 'all';
  const validFilters = ['all', 'first_ever', 'first_season', 'first_week', 'first_day', 'high_score'];
  if (!validFilters.includes(activeFilter)) activeFilter = 'all';

  function rarityRank(r) {
    if (!r) return 99;
    if (r.first_ever) return 0;
    if (r.first_season) return 1;
    if (r.first_week) return 2;
    if (r.first_day) return 3;
    return 4 - r.score; // 4 - score so higher score sorts earlier
  }

  function matchesFilter(r, f) {
    if (!r) return false;
    if (f === 'all') return r.first_ever || r.first_season || r.first_week || r.first_day || r.score >= 0.6;
    if (f === 'first_ever') return r.first_ever;
    if (f === 'first_season') return r.first_season;
    if (f === 'first_week') return r.first_week;
    if (f === 'first_day') return r.first_day;
    if (f === 'high_score') return r.score >= 0.6;
    return false;
  }

  function renderFilters(counts) {
    const opts = [
      { key: 'all', label: 'All' },
      { key: 'first_ever', label: 'First ever', cls: 'purple' },
      { key: 'first_season', label: 'First of season', cls: 'blue' },
      { key: 'first_week', label: 'First this week', cls: 'teal' },
      { key: 'first_day', label: 'First today', cls: 'sky' },
      { key: 'high_score', label: 'High score', cls: 'amber' },
    ];
    const el = document.getElementById('rare-filters');
    el.innerHTML = opts.map(o => {
      const c = counts[o.key] || 0;
      const isActive = o.key === activeFilter;
      const base = 'inline-flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-colors';
      const cls = isActive
        ? 'bg-nuthatch-600 text-white hover:bg-nuthatch-700'
        : 'bg-white dark:bg-plumage-900 text-stone-600 dark:text-plumage-300 ring-1 ring-stone-200 dark:ring-plumage-800 hover:bg-stone-50 dark:hover:bg-plumage-800/50';
      return `<button type="button" data-filter="${o.key}" class="${base} ${cls}">
        <span>${o.label}</span>
        <span class="${isActive ? 'text-nuthatch-50' : 'text-stone-400 dark:text-plumage-500'}">${c}</span>
      </button>`;
    }).join('');
    el.querySelectorAll('button[data-filter]').forEach(b => {
      b.addEventListener('click', () => {
        activeFilter = b.dataset.filter;
        const url = new URL(location.href);
        if (activeFilter === 'all') url.searchParams.delete('filter');
        else url.searchParams.set('filter', activeFilter);
        history.replaceState(null, '', url.toString());
        renderFilters(counts);
        renderList(allDetections);
      });
    });
  }

  function renderList(detections) {
    const el = document.getElementById('rare-list');
    const filtered = detections.filter(d => matchesFilter(d.rarity, activeFilter));

    // Sort: first_ever > first_season > first_week > first_day > by score; then by recency.
    filtered.sort((a, b) => {
      const ra = rarityRank(a.rarity), rb = rarityRank(b.rarity);
      if (ra !== rb) return ra - rb;
      return new Date(b.detected_at) - new Date(a.detected_at);
    });

    if (filtered.length === 0) {
      el.innerHTML = '<div class="text-center py-12 text-gray-400 dark:text-plumage-500 text-sm">No rare detections in this window.</div>';
      return;
    }

    el.innerHTML = filtered.map(d => {
      const sciEnc = encodeURIComponent(d.species.scientific_name);
      const time = window.sitta.fmtDateTime(d.detected_at, _tz);
      const hasAudio = d.has_audio || d.snippet_path;
      return `<div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2 flex-wrap">
              <a href="/species/${sciEnc}" class="font-semibold text-base hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">${window.sitta.esc(d.species.common_name)}</a>
              ${window.sitta.confidenceBadge(d)}
              ${window.sitta.rarityBadges(d)}
            </div>
            <p class="text-sm text-gray-500 dark:text-plumage-400 italic mt-0.5">
              <a href="/species/${sciEnc}" class="hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">${window.sitta.esc(d.species.scientific_name)}</a>
            </p>
            <div class="flex items-center gap-3 mt-2 text-xs text-gray-400 dark:text-plumage-500">
              ${d.source_name ? '<span>' + window.sitta.esc(d.source_name) + '</span>' : ''}
              <a href="/detections/${d.id}" class="${d.source_name ? 'before:content-[\"\\u00b7\"] before:mr-3' : ''} hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">${time}</a>
              ${d.rarity && d.rarity.local_count != null ? '<span class="before:content-[\"\\u00b7\"] before:mr-3">' + d.rarity.local_count + ' prior</span>' : ''}
              ${d.rarity && d.rarity.days_since_last != null ? '<span class="before:content-[\"\\u00b7\"] before:mr-3">' + d.rarity.days_since_last + 'd since last</span>' : ''}
            </div>
          </div>
        </div>
        ${window.sitta.spectrogramBlock(d)}
        <div class="flex items-center justify-between mt-3 pt-3 border-t border-gray-100 dark:border-plumage-800">
          <div class="flex items-center gap-3">
            ${window.sitta.playButton(d)}
            <a href="/detections/${d.id}" class="text-xs text-stone-500 dark:text-plumage-400 hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">View detection &rarr;</a>
          </div>
          <span class="text-xs text-gray-400 dark:text-plumage-600 font-mono">${Math.round((d.rarity ? d.rarity.score : 0) * 100)}% rare</span>
        </div>
      </div>`;
    }).join('');
  }

  let allDetections = [];

  fetch('/api/v1/detections?rarity=true&limit=500&since=' + SINCE)
    .then(r => { if (!r.ok) throw new Error('http ' + r.status); return r.json(); })
    .then(resp => {
      allDetections = resp.items || resp;

      // Compute counts per filter for the chip labels.
      const counts = { all: 0, first_ever: 0, first_season: 0, first_week: 0, first_day: 0, high_score: 0 };
      allDetections.forEach(d => {
        if (!d.rarity) return;
        if (d.rarity.first_ever || d.rarity.first_season || d.rarity.first_week || d.rarity.first_day || d.rarity.score >= 0.6) counts.all++;
        if (d.rarity.first_ever) counts.first_ever++;
        if (d.rarity.first_season) counts.first_season++;
        if (d.rarity.first_week) counts.first_week++;
        if (d.rarity.first_day) counts.first_day++;
        if (d.rarity.score >= 0.6) counts.high_score++;
      });

      renderFilters(counts);
      renderList(allDetections);
    })
    .catch(() => {
      document.getElementById('rare-list').innerHTML =
        '<div class="text-center py-8 text-red-400 text-sm">Failed to load rare detections</div>';
    });

  // Audio playback (window.playClip + spectrogram scrubbing) is provided
  // by the shared player on window.sitta — see dashboard::page().
})();
</script>"##
        .to_string()
}

