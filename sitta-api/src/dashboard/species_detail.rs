//! Species-detail page content.

pub fn species_detail_content(scientific_name: &str) -> String {
    format!(
        r##"<div class="mb-6">
  <div class="flex items-center gap-2 mb-1">
    <a href="/species" class="text-nuthatch-600 dark:text-nuthatch-400 hover:underline text-sm">&larr; All species</a>
  </div>
  <div class="flex items-start gap-5">
    <div class="flex-1">
      <h1 id="species-title" class="text-2xl font-bold tracking-tight">{scientific_name}</h1>
      <p class="text-sm text-gray-500 dark:text-plumage-400 italic mt-0.5">{scientific_name}</p>
      <p id="species-wiki-extract" class="text-sm text-stone-600 dark:text-plumage-300 mt-2 line-clamp-3 hidden"></p>
      <a id="species-wiki-link" class="text-xs text-nuthatch-600 dark:text-nuthatch-400 hover:underline mt-1 hidden" target="_blank" rel="noopener">Wikipedia &rarr;</a>
    </div>
    <div id="species-image-container" class="hidden flex-shrink-0">
      <img id="species-image" class="w-32 h-32 sm:w-40 sm:h-40 rounded-xl object-cover shadow-sm" alt="">
    </div>
  </div>
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

  // ── Species image: try custom URL, then Wikipedia ──────────────
  (function() {{
    const wikiName = sciName.replace(/ /g, '_');
    // Check if a custom image base URL is configured via settings.
    fetch('/api/v1/settings')
      .then(r => r.json())
      .then(s => {{
        if (s.species_image_url) {{
          // Custom source: {{url}}/{{Scientific_name}}.jpg
          const customUrl = s.species_image_url.replace(/\/$/, '') + '/' + wikiName + '.jpg';
          const img = new Image();
          img.onload = function() {{ showImage(customUrl); }};
          img.onerror = function() {{ loadFromWikipedia(); }};
          img.src = customUrl;
          return;
        }}
        loadFromWikipedia();
      }})
      .catch(() => loadFromWikipedia());

    function loadFromWikipedia() {{
      fetch('https://en.wikipedia.org/api/rest_v1/page/summary/' + encodeURIComponent(wikiName))
        .then(r => r.json())
        .then(data => {{
          if (data.thumbnail && data.thumbnail.source) {{
            showImage(data.thumbnail.source);
          }}
          if (data.extract) {{
            const el = document.getElementById('species-wiki-extract');
            el.textContent = data.extract;
            el.classList.remove('hidden');
          }}
          if (data.content_urls && data.content_urls.desktop) {{
            const link = document.getElementById('species-wiki-link');
            link.href = data.content_urls.desktop.page;
            link.classList.remove('hidden');
          }}
        }})
        .catch(() => {{}});
    }}

    function showImage(url) {{
      const img = document.getElementById('species-image');
      const container = document.getElementById('species-image-container');
      img.src = url;
      img.alt = sciName;
      container.classList.remove('hidden');
    }}
  }})();

  // ── Behavioral insights ────────────────────────────────────────
  fetch('/api/v1/species/' + encodeURIComponent(sciName) + '/insights')
    .then(r => r.json())
    .then(ins => {{
      const el = document.getElementById('species-insights');
      el.innerHTML = buildInsightsPanel(ins, _tz);
    }})
    .catch(() => {{}});

  function buildInsightsPanel(data, tz) {{
    const parts = [];
    const tzOff = getTimezoneOffsetHours(tz);

    // ── Today likelihood badge ──────────────────────────────────
    const pct = Math.round(data.today_likelihood * 100);
    const likelyColor = pct >= 70 ? 'emerald' : pct >= 40 ? 'amber' : 'stone';
    const likelyLabel = pct >= 70 ? 'Likely today' : pct >= 40 ? 'Possible today' : pct > 0 ? 'Unlikely today' : 'Insufficient data';
    const rangeNote = data.range_score != null
      ? `<span class="text-xs text-gray-400 dark:text-plumage-500 ml-2">Range model: ${{Math.round(data.range_score * 100)}}%</span>` : '';

    parts.push(`<div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
      <div class="flex items-center justify-between mb-3">
        <div class="flex items-center gap-3">
          <div class="flex items-center justify-center w-12 h-12 rounded-full bg-${{likelyColor}}-50 dark:bg-${{likelyColor}}-900/30">
            <span class="text-lg font-bold text-${{likelyColor}}-700 dark:text-${{likelyColor}}-400">${{pct}}%</span>
          </div>
          <div>
            <div class="text-sm font-semibold text-gray-900 dark:text-plumage-100">${{likelyLabel}}</div>
            <div class="text-xs text-gray-500 dark:text-plumage-400">Chance of detection today${{rangeNote}}</div>
          </div>
        </div>
        <div class="text-right text-xs text-gray-400 dark:text-plumage-500">
          <div>${{data.total_detections.toLocaleString()}} total detections</div>
          <div>${{data.days_detected}} days observed</div>
        </div>
      </div>
    </div>`);

    // ── Monthly activity chart ──────────────────────────────────
    if (data.monthly_distribution) {{
      const months = data.monthly_distribution;
      const maxM = Math.max(...months, 1);
      const mNames = ['Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct','Nov','Dec'];
      const now = new Date();
      const thisMonth = now.getMonth();

      let bars = '';
      for (let i = 0; i < 12; i++) {{
        const h = Math.max(2, Math.round((months[i] / maxM) * 48));
        const isCurrent = i === thisMonth;
        const barColor = isCurrent
          ? 'bg-nuthatch-500 dark:bg-nuthatch-400'
          : months[i] > 0
            ? 'bg-plumage-300 dark:bg-plumage-600'
            : 'bg-gray-100 dark:bg-plumage-800';
        const label = months[i] > 0 ? months[i] : '';
        bars += `<div class="flex flex-col items-center gap-1 flex-1">
          <span class="text-[10px] text-gray-400 dark:text-plumage-500 h-4">${{label}}</span>
          <div class="w-full rounded-sm ${{barColor}}" style="height:${{h}}px"></div>
          <span class="text-[10px] ${{isCurrent ? 'font-bold text-nuthatch-600 dark:text-nuthatch-400' : 'text-gray-400 dark:text-plumage-500'}}">${{mNames[i]}}</span>
        </div>`;
      }}

      // Seasonal insight text
      const activeMonths = months.filter(m => m > 0).length;
      const peakMonth = months.indexOf(maxM);
      let seasonText = '';
      if (activeMonths === 0) {{
        seasonText = '';
      }} else if (activeMonths <= 4) {{
        seasonText = 'Seasonal visitor \u2014 detected mainly in ' + mNames[peakMonth] + ' and nearby months.';
      }} else if (activeMonths <= 8) {{
        seasonText = 'Present for much of the year, peaking in ' + mNames[peakMonth] + '.';
      }} else {{
        seasonText = 'Year-round resident \u2014 detected in ' + activeMonths + ' of 12 months.';
      }}

      parts.push(`<div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
        <h3 class="text-xs font-semibold text-gray-500 dark:text-plumage-400 uppercase tracking-wider mb-3">Seasonal Pattern</h3>
        <div class="flex items-end gap-0.5 h-16">${{bars}}</div>
        ${{seasonText ? '<p class="text-sm text-gray-600 dark:text-plumage-300 mt-3">' + seasonText + '</p>' : ''}}
      </div>`);
    }}

    // ── Behavioral insights (time of day) ───────────────────────
    const lines = buildTimeInsights(data, tz, tzOff);
    if (lines.length > 0) {{
      parts.push(`<div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
        <h3 class="text-xs font-semibold text-gray-500 dark:text-plumage-400 uppercase tracking-wider mb-2">Time of Day</h3>
        <div class="text-sm text-gray-600 dark:text-plumage-300 space-y-1">
          ${{lines.map(l => '<p>' + l + '</p>').join('')}}
        </div>
      </div>`);
    }}

    // ── Data sufficiency callouts ────────────────────────────────
    if (data.data_sufficiency && data.data_sufficiency.gaps && data.data_sufficiency.gaps.length > 0) {{
      const gapItems = data.data_sufficiency.gaps.map(g =>
        `<li class="flex items-start gap-2">
          <svg class="w-4 h-4 text-amber-500 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v3.75m9-.75a9 9 0 11-18 0 9 9 0 0118 0zm-9 3.75h.008v.008H12v-.008z"/>
          </svg>
          <span>${{g}}</span>
        </li>`
      ).join('');
      parts.push(`<div class="bg-amber-50 dark:bg-amber-900/20 rounded-xl border border-amber-200 dark:border-amber-800/50 p-4">
        <h3 class="text-xs font-semibold text-amber-700 dark:text-amber-400 uppercase tracking-wider mb-2">More Data Needed</h3>
        <ul class="text-sm text-amber-800 dark:text-amber-300 space-y-2">${{gapItems}}</ul>
      </div>`);
    }}

    // ── Notable detections ──────────────────────────────────────
    if (data.notable_detections && data.notable_detections.length > 0) {{
      const rows = data.notable_detections.map(n => {{
        const time = new Date(n.detected_at).toLocaleString('en-GB', {{
          month: 'short', day: 'numeric', year: 'numeric',
          hour: '2-digit', minute: '2-digit', hour12: false, timeZone: tz
        }});
        const badges = [];
        if (n.first_ever) badges.push('<span class="px-1.5 py-0.5 text-[10px] font-semibold rounded bg-purple-100 text-purple-700 dark:bg-purple-900/40 dark:text-purple-300">First ever</span>');
        else if (n.first_season) badges.push('<span class="px-1.5 py-0.5 text-[10px] font-semibold rounded bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300">First of season</span>');
        const scorePct = Math.round(n.rarity_score * 100);
        return `<a href="/detections/${{n.detection_id}}" class="flex items-center justify-between py-1.5 hover:bg-gray-50 dark:hover:bg-plumage-800/50 -mx-1 px-1 rounded transition-colors">
          <div class="flex items-center gap-2">
            ${{badges.join('')}}
            <span class="text-sm text-gray-600 dark:text-plumage-300">${{time}}</span>
          </div>
          <span class="text-xs font-medium text-nuthatch-600 dark:text-nuthatch-400">${{scorePct}}% rare</span>
        </a>`;
      }}).join('');
      parts.push(`<div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
        <h3 class="text-xs font-semibold text-gray-500 dark:text-plumage-400 uppercase tracking-wider mb-2">Notable Detections</h3>
        <div class="divide-y divide-gray-100 dark:divide-plumage-800">${{rows}}</div>
      </div>`);
    }}

    return parts.join('');
  }}

  function buildTimeInsights(data, tz, tzOffsetHrs) {{
    if (data.total_detections < 5) {{
      return ['Not enough detections to suggest habits. Check back after more observations.'];
    }}

    const insights = [];
    const hours = data.hourly_distribution;
    const total = hours.reduce((a, b) => a + b, 0);

    // Shift UTC hours to local timezone
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
    .then(resp => {{
      const data = resp.items || resp;
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
        const time = window.sitta.fmtDateTime(d.detected_at, _tz);
        const hasAudio = d.has_audio || d.snippet_path;

        return `<div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4">
          <div class="flex items-center justify-between gap-2">
            <div class="flex items-center gap-2 flex-wrap min-w-0">
              ${{window.sitta.confidenceBadge(d)}}
              ${{window.sitta.rarityBadges(d)}}
              ${{window.sitta.individualBadge(d)}}
              <a href="/detections/${{d.id}}" class="text-sm text-gray-600 dark:text-plumage-300 hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors">${{time}}</a>
            </div>
            <a href="/detections/${{d.id}}" class="text-xs text-gray-400 dark:text-plumage-600 font-mono hover:text-nuthatch-600 dark:hover:text-nuthatch-400 transition-colors flex-shrink-0">${{d.id.slice(0, 8)}}</a>
          </div>
          ${{window.sitta.spectrogramBlock(d)}}
          <div class="flex items-center justify-between mt-3 pt-3 border-t border-gray-100 dark:border-plumage-800">
            <div class="flex items-center gap-3">
              ${{window.sitta.playButton(d)}}
              <span class="text-xs text-gray-400 dark:text-plumage-500">${{d.model}} ${{d.model_version}}</span>
              ${{d.source_name ? '<span class="text-xs text-gray-400 dark:text-plumage-500 before:content-[\\u00b7] before:mr-2">' + window.sitta.esc(d.source_name) + '</span>' : ''}}
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

