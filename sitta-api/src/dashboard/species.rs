//! Species list page content.

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

