//! Detection-detail page content.

pub fn detection_detail_content(detection_id: &str) -> String {
    format!(
        r##"<div id="detail-loading" class="text-center py-16 text-gray-400 dark:text-plumage-500">Loading...</div>
<div id="detail-content" class="hidden">

  <!-- Breadcrumb -->
  <nav id="det-breadcrumb" class="flex items-center gap-2 text-xs text-gray-400 dark:text-plumage-500 mb-2 flex-wrap" aria-label="Breadcrumb">
    <a href="/" class="hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">Dashboard</a>
    <span aria-hidden="true">/</span>
    <a href="/species" class="hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">Species</a>
    <span aria-hidden="true">/</span>
    <a id="det-bc-species" class="hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors" href="#"></a>
    <span aria-hidden="true">/</span>
    <span class="text-gray-500 dark:text-plumage-400">Detection</span>
  </nav>
  <div class="flex items-start justify-between gap-4 mb-3">
    <div class="min-w-0">
      <h1 class="text-2xl font-bold tracking-tight"><a id="det-common" class="hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors" href="#"></a></h1>
      <p class="text-sm text-gray-500 dark:text-plumage-400 italic mt-0.5"><a id="det-scientific" class="hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors" href="#"></a></p>
    </div>
    <div class="flex items-center gap-3 flex-shrink-0">
      <span id="det-badge" class="inline-flex items-center rounded-md px-2.5 py-1 text-sm font-medium ring-1 ring-inset"></span>
    </div>
  </div>
  <div id="det-meta" class="flex items-center gap-2 text-sm text-gray-500 dark:text-plumage-400 mb-3 flex-wrap"></div>
  <div class="mb-6 flex flex-wrap gap-2">
    <a id="det-cta-species" href="#" class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-nuthatch-50 text-nuthatch-700 hover:bg-nuthatch-100 dark:bg-nuthatch-900/20 dark:text-nuthatch-400 dark:hover:bg-nuthatch-900/40 transition-colors">
      <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M3.75 6A2.25 2.25 0 016 3.75h2.25A2.25 2.25 0 0110.5 6v2.25a2.25 2.25 0 01-2.25 2.25H6a2.25 2.25 0 01-2.25-2.25V6zM3.75 15.75A2.25 2.25 0 016 13.5h2.25a2.25 2.25 0 012.25 2.25V18a2.25 2.25 0 01-2.25 2.25H6A2.25 2.25 0 013.75 18v-2.25zM13.5 6a2.25 2.25 0 012.25-2.25H18A2.25 2.25 0 0120.25 6v2.25A2.25 2.25 0 0118 10.5h-2.25a2.25 2.25 0 01-2.25-2.25V6zM13.5 15.75a2.25 2.25 0 012.25-2.25H18a2.25 2.25 0 012.25 2.25V18A2.25 2.25 0 0118 20.25h-2.25A2.25 2.25 0 0113.5 18v-2.25z"/></svg>
      All detections of this species
    </a>
    <a id="det-cta-rare" href="/rare" class="hidden inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-amber-50 text-amber-700 hover:bg-amber-100 dark:bg-amber-900/20 dark:text-amber-400 dark:hover:bg-amber-900/40 transition-colors">
      <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M11.48 3.499a.562.562 0 011.04 0l2.125 5.111a.563.563 0 00.475.345l5.518.442c.499.04.701.663.321.988l-4.204 3.602a.563.563 0 00-.182.557l1.285 5.385a.562.562 0 01-.84.61l-4.725-2.885a.563.563 0 00-.586 0L6.982 20.54a.562.562 0 01-.84-.61l1.285-5.386a.562.562 0 00-.182-.557l-4.204-3.602a.563.563 0 01.321-.988l5.518-.442a.563.563 0 00.475-.345L11.48 3.5z"/></svg>
      Other rare moments
    </a>
  </div>

  <!-- Review strip -->
  <div id="det-review" class="mb-6 flex items-center gap-3"></div>

  <!-- Spectrogram + Audio -->
  <div id="det-audio-section" class="mb-6 hidden">
    <img id="det-spectrogram" loading="lazy" class="w-full h-48 rounded-xl object-cover bg-gray-100 dark:bg-plumage-800" alt="spectrogram"/>
    <div class="mt-3 flex items-center gap-3">
      <button id="det-play-btn" class="inline-flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium bg-plumage-50 text-plumage-700 hover:bg-plumage-100 dark:bg-plumage-800 dark:text-plumage-300 dark:hover:bg-plumage-700 transition-colors">
        <svg class="w-4 h-4" fill="currentColor" viewBox="0 0 20 20"><path d="M6.3 2.84A1.5 1.5 0 004 4.11v11.78a1.5 1.5 0 002.3 1.27l9.344-5.891a1.5 1.5 0 000-2.538L6.3 2.841z"/></svg>
        Play clip
      </button>
      <a id="det-download-btn" class="inline-flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium bg-plumage-50 text-plumage-700 hover:bg-plumage-100 dark:bg-plumage-800 dark:text-plumage-300 dark:hover:bg-plumage-700 transition-colors" title="Download audio clip">
        <svg class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5M16.5 12L12 16.5m0 0L7.5 12m4.5 4.5V3"/></svg>
        Download
      </a>
      <button onclick="reviewThis('correct')" class="inline-flex items-center gap-1.5 px-3 py-2 rounded-lg text-sm font-medium text-emerald-700 hover:bg-emerald-50 dark:text-emerald-400 dark:hover:bg-emerald-900/30 transition-colors">
        <svg class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M4.5 12.75l6 6 9-13.5"/></svg> Correct
      </button>
      <button onclick="reviewThis('false_positive')" class="inline-flex items-center gap-1.5 px-3 py-2 rounded-lg text-sm font-medium text-red-600 hover:bg-red-50 dark:text-red-400 dark:hover:bg-red-900/30 transition-colors">
        <svg class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/></svg> False positive
      </button>
      <button onclick="deleteThis()" class="inline-flex items-center gap-1.5 px-3 py-2 rounded-lg text-sm font-medium text-gray-400 hover:text-red-600 hover:bg-red-50 dark:text-plumage-500 dark:hover:text-red-400 dark:hover:bg-red-900/30 transition-colors ml-auto" title="Delete detection">
        <svg class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0"/></svg> Delete
      </button>
    </div>
  </div>

  <!-- Predictions (full top_k) -->
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5 mb-6">
    <h2 class="text-base font-semibold mb-4">Model Predictions</h2>
    <div id="det-predictions" class="space-y-2"></div>
  </div>

  <!-- Correlated detections -->
  <div id="det-correlated-section" class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5 mb-6 hidden">
    <h2 class="text-base font-semibold mb-4">Other Models (same audio moment)</h2>
    <div id="det-correlated" class="space-y-3"></div>
  </div>

</div>

<script>
(function() {{
  const ID = '{detection_id}';
  const _tz = document.body.dataset.tz || 'UTC';

  fetch('/api/v1/detections/' + ID)
    .then(r => {{ if (!r.ok) throw new Error(r.status); return r.json(); }})
    .then(d => {{
      document.getElementById('detail-loading').classList.add('hidden');
      document.getElementById('detail-content').classList.remove('hidden');

      // Header (links to species page)
      const sciUrl = '/species/' + encodeURIComponent(d.species.scientific_name);
      const dcEl = document.getElementById('det-common');
      dcEl.textContent = d.species.common_name;
      dcEl.href = sciUrl;
      const dsEl = document.getElementById('det-scientific');
      dsEl.textContent = d.species.scientific_name;
      dsEl.href = sciUrl;
      const bcEl = document.getElementById('det-bc-species');
      bcEl.textContent = d.species.common_name;
      bcEl.href = sciUrl;
      document.getElementById('det-cta-species').href = sciUrl;

      const pct = Math.round(d.confidence * 100);
      const badge = document.getElementById('det-badge');
      badge.textContent = pct + '%';
      if (d.confidence >= 0.8) badge.className = 'inline-flex items-center rounded-md px-2.5 py-1 text-sm font-medium ring-1 ring-inset text-emerald-700 bg-emerald-50 ring-emerald-600/20 dark:text-emerald-400 dark:bg-emerald-900/30 dark:ring-emerald-400/20';
      else if (d.confidence >= 0.5) badge.className = 'inline-flex items-center rounded-md px-2.5 py-1 text-sm font-medium ring-1 ring-inset text-amber-700 bg-amber-50 ring-amber-600/20 dark:text-amber-400 dark:bg-amber-900/30 dark:ring-amber-400/20';
      else badge.className = 'inline-flex items-center rounded-md px-2.5 py-1 text-sm font-medium ring-1 ring-inset text-red-700 bg-red-50 ring-red-600/20 dark:text-red-400 dark:bg-red-900/30 dark:ring-red-400/20';

      // Meta (rich row with rarity + source)
      const timeStr = window.sitta.fmtDateTime(d.detected_at, _tz);
      const sep = '<span class="text-stone-300 dark:text-plumage-700" aria-hidden="true">·</span>';
      const metaParts = [];
      metaParts.push('<span>' + window.sitta.esc(timeStr) + '</span>');
      metaParts.push('<span>' + window.sitta.esc(d.model + ' ' + d.model_version) + '</span>');
      if (d.source_name) metaParts.push('<span>' + window.sitta.esc(d.source_name) + '</span>');
      const rb = window.sitta.rarityBadges(d);
      if (rb) metaParts.push(rb);
      if (d.range_unverified) metaParts.push('<span class="px-1.5 py-0.5 text-[10px] font-medium rounded bg-amber-50 text-amber-700 ring-1 ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-300 dark:ring-amber-400/20" title="Species not in BirdNET range model — not verified by geographic filter">Range unverified</span>');
      document.getElementById('det-meta').innerHTML = metaParts.join(' ' + sep + ' ');

      // Surface "Other rare moments" CTA when this detection is itself rare.
      if (d.rarity && (d.rarity.first_ever || d.rarity.first_season || d.rarity.first_week || d.rarity.first_day || d.rarity.score >= 0.6)) {{
        document.getElementById('det-cta-rare').classList.remove('hidden');
      }}

      // Review
      if (d.review) {{
        const rv = document.getElementById('det-review');
        if (d.review.status === 'correct') rv.innerHTML = '<span class="text-sm text-emerald-600 dark:text-emerald-400 font-medium flex items-center gap-1"><svg class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M4.5 12.75l6 6 9-13.5"/></svg>Reviewed: Correct</span>';
        else rv.innerHTML = '<span class="text-sm text-red-500 dark:text-red-400 font-medium">Reviewed: False positive</span>';
      }}

      // Audio + spectrogram
      if (d.has_audio) {{
        const sec = document.getElementById('det-audio-section');
        sec.classList.remove('hidden');
        document.getElementById('det-spectrogram').src = '/api/v1/detections/' + ID + '/spectrogram';
        const dlBtn = document.getElementById('det-download-btn');
        const safeName = (d.species?.common_name || 'clip').replace(/[^a-zA-Z0-9 ]/g, '').replace(/ +/g, '_');
        const safeTime = (d.detected_at || '').replace(/[:.]/g, '-').slice(0, 19);
        dlBtn.href = '/api/v1/detections/' + ID + '/audio';
        dlBtn.download = safeName + '_' + safeTime + '.wav';
        const playBtn = document.getElementById('det-play-btn');
        let audio = null;
        playBtn.onclick = function() {{
          if (audio) {{ audio.pause(); audio = null; playBtn.querySelector('svg').innerHTML = '<path d="M6.3 2.84A1.5 1.5 0 004 4.11v11.78a1.5 1.5 0 002.3 1.27l9.344-5.891a1.5 1.5 0 000-2.538L6.3 2.841z"/>'; return; }}
          audio = new Audio('/api/v1/detections/' + ID + '/audio');
          audio.play();
          playBtn.querySelector('svg').innerHTML = '<rect x="4" y="4" width="12" height="12" rx="1.5"/>';
          audio.onended = function() {{ audio = null; playBtn.querySelector('svg').innerHTML = '<path d="M6.3 2.84A1.5 1.5 0 004 4.11v11.78a1.5 1.5 0 002.3 1.27l9.344-5.891a1.5 1.5 0 000-2.538L6.3 2.841z"/>'; }};
        }};
      }}

      // Predictions — top-1 + all alternatives as horizontal bars
      const predsDiv = document.getElementById('det-predictions');
      const allPreds = [{{ common_name: d.species.common_name, scientific_name: d.species.scientific_name, confidence: d.confidence, rank: 0 }}]
        .concat((d.alternatives || []).map(a => ({{ common_name: a.common_name, scientific_name: a.scientific_name, confidence: a.confidence, rank: a.rank }})));
      const maxConf = Math.max(...allPreds.map(p => p.confidence));
      allPreds.forEach((p, i) => {{
        const barPct = Math.round((p.confidence / maxConf) * 100);
        const confPct = Math.round(p.confidence * 100);
        const isTop = i === 0;
        predsDiv.innerHTML += `
          <div class="flex items-center gap-3">
            <div class="w-48 flex-shrink-0 text-right">
              <span class="text-sm ${{isTop ? 'font-semibold' : 'text-gray-600 dark:text-plumage-300'}}">${{p.common_name}}</span>
            </div>
            <div class="flex-1 h-6 bg-gray-100 dark:bg-plumage-800 rounded overflow-hidden">
              <div class="h-full rounded ${{isTop ? 'bg-nuthatch-500' : 'bg-plumage-400 dark:bg-plumage-600'}}" style="width:${{barPct}}%"></div>
            </div>
            <span class="w-12 text-right text-sm font-mono ${{isTop ? 'font-bold' : 'text-gray-500 dark:text-plumage-400'}}">${{confPct}}%</span>
          </div>`;
      }});

      // Correlated detections
      if (d.correlated && d.correlated.length > 0) {{
        const sec = document.getElementById('det-correlated-section');
        sec.classList.remove('hidden');
        const div = document.getElementById('det-correlated');
        d.correlated.forEach(c => {{
          const cpct = Math.round(c.confidence * 100);
          const sameSpecies = c.species.scientific_name === d.species.scientific_name;
          const csUrl = '/species/' + encodeURIComponent(c.species.scientific_name);
          div.innerHTML += `
            <div class="block p-3 rounded-lg border border-gray-100 dark:border-plumage-800 hover:bg-gray-50 dark:hover:bg-plumage-800/50 transition-colors">
              <div class="flex items-center justify-between gap-3">
                <div class="min-w-0">
                  <a href="${{csUrl}}" class="text-sm font-medium hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">${{window.sitta.esc(c.species.common_name)}}</a>
                  ${{!sameSpecies ? '<span class="ml-2 text-xs px-1.5 py-0.5 rounded bg-amber-50 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400">different species</span>' : '<span class="ml-2 text-xs px-1.5 py-0.5 rounded bg-emerald-50 text-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-400">agrees</span>'}}
                </div>
                <div class="flex items-center gap-2 text-sm text-gray-500 dark:text-plumage-400 flex-shrink-0">
                  <span>${{c.model}} ${{c.model_version}}</span>
                  <a href="/detections/${{c.id}}" class="font-mono hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">${{cpct}}%</a>
                </div>
              </div>
            </div>`;
        }});
      }}
    }})
    .catch(e => {{
      document.getElementById('detail-loading').innerHTML = '<p class="text-red-500">Detection not found</p>';
    }});

  window.reviewThis = function(status) {{
    fetch('/api/v1/detections/' + ID + '/review', {{
      method: 'PUT',
      headers: {{ 'Content-Type': 'application/json' }},
      body: JSON.stringify({{ status }})
    }}).then(r => {{
      if (!r.ok) return;
      const rv = document.getElementById('det-review');
      if (status === 'correct') rv.innerHTML = '<span class="text-sm text-emerald-600 dark:text-emerald-400 font-medium flex items-center gap-1"><svg class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M4.5 12.75l6 6 9-13.5"/></svg>Reviewed: Correct</span>';
      else rv.innerHTML = '<span class="text-sm text-red-500 dark:text-red-400 font-medium">Reviewed: False positive</span>';
    }});
  }};

  window.deleteThis = function() {{
    if (!confirm('Delete this detection? This cannot be undone.')) return;
    fetch('/api/v1/detections/' + ID, {{ method: 'DELETE' }}).then(r => {{
      if (r.ok) window.location.href = '/';
    }});
  }};
}})();
</script>"##,
        detection_id = detection_id,
    )
}

