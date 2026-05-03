//! /individuals page content.

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
            <p class="font-semibold text-sm">${c.common_name || c.scientific_name}</p>
            <p class="text-xs text-gray-400 dark:text-plumage-500 italic">${c.scientific_name}</p>
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
              <h3 class="font-semibold text-sm">${g.individuals[0].common_name || g.scientific_name}</h3>
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

