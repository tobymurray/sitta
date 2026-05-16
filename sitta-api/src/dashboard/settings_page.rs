//! /settings page content.

use crate::settings::{InitialConfig, RuntimeSettings};

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

    let presence_min = settings.presence_min_detections;
    let presence_window = settings.presence_window_minutes;
    let presence_immediate = settings
        .presence_immediate_threshold
        .map(|v| v.to_string())
        .unwrap_or_default();

    let has_birdnet = initial.birdnet_model_path.is_some();
    let has_perch = initial.perch_model_path.is_some();

    // Build the "System info" rows: every read-only restart-required value
    // we know about, with the file path or value rendered as a code chip.
    let system_rows = render_system_rows(initial);
    let model_rows = render_model_rows(initial);

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
    <div class="mt-4">
      <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Species Image URL (optional)</label>
      <input name="species_image_url" type="text" value="{species_image_url}"
        class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none"
        placeholder="e.g. http://192.168.1.50/birds or https://my-cdn.com/species">
      <p class="mt-1 text-xs text-gray-400 dark:text-plumage-500">Base URL for custom species images. The UI will try <code>{{url}}/{{Scientific_name}}.jpg</code> before falling back to Wikipedia. Leave empty to use Wikipedia only.</p>
    </div>
    <div class="mt-4 flex items-center justify-between">
      <div>
        <label class="text-sm font-medium text-gray-700 dark:text-plumage-300">Show range-unverified detections</label>
        <p class="text-xs text-gray-400 dark:text-plumage-500 mt-0.5">Perch species not in BirdNET's geographic model. These bypass the range filter because no occurrence data exists.</p>
      </div>
      <input name="show_range_unverified" type="checkbox" {show_range_unverified_checked}
        class="h-4 w-4 rounded border-gray-300 text-nuthatch-600 focus:ring-nuthatch-500 ml-4 flex-shrink-0">
    </div>
  </div>

  <!-- Detection Persistence -->
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <h3 class="text-sm font-semibold text-gray-900 dark:text-plumage-100 uppercase tracking-wider mb-1">Detection Persistence</h3>
    <p class="text-xs text-gray-400 dark:text-plumage-500 mb-4">Perch labels non-bird sounds — <code class="bg-gray-100 dark:bg-plumage-800 px-1 rounded">Animal</code>, <code class="bg-gray-100 dark:bg-plumage-800 px-1 rounded">Vehicle</code>, <code class="bg-gray-100 dark:bg-plumage-800 px-1 rounded">Bark</code>, <code class="bg-gray-100 dark:bg-plumage-800 px-1 rounded">voice</code>, <code class="bg-gray-100 dark:bg-plumage-800 px-1 rounded">Music</code>, …. These detections fire from pets, road traffic, household activity, etc., and can dominate clip storage on a noisy station.</p>
    <div class="space-y-3">
      <div class="flex items-center justify-between">
        <div>
          <label class="text-sm font-medium text-gray-700 dark:text-plumage-300">Skip clips for non-species labels</label>
          <p class="text-xs text-gray-400 dark:text-plumage-500 mt-0.5">Don't save WAV files for environment detections. Detection rows still land in the DB so you can audit Perch's classifier behavior.</p>
        </div>
        <input name="skip_environment_clips" type="checkbox" {skip_environment_clips_checked}
          class="h-4 w-4 rounded border-gray-300 text-nuthatch-600 focus:ring-nuthatch-500 ml-4 flex-shrink-0">
      </div>
      <div class="flex items-center justify-between">
        <div>
          <label class="text-sm font-medium text-gray-700 dark:text-plumage-300">Skip detections for non-species labels</label>
          <p class="text-xs text-gray-400 dark:text-plumage-500 mt-0.5">Stronger: don't even record the detection. Saves DB writes during noisy hours and keeps the rare/alert feeds clean. Disable this if you want to debug Perch's environment classifier.</p>
        </div>
        <input name="skip_environment_detections" type="checkbox" {skip_environment_detections_checked}
          class="h-4 w-4 rounded border-gray-300 text-nuthatch-600 focus:ring-nuthatch-500 ml-4 flex-shrink-0">
      </div>
    </div>
  </div>

  <!-- Detection Confirmation -->
  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <h3 class="text-sm font-semibold text-gray-900 dark:text-plumage-100 uppercase tracking-wider mb-1">Detection Confirmation</h3>
    <p class="text-xs text-gray-400 dark:text-plumage-500 mb-4">A species must be heard repeatedly within a sliding window before alerts fire. Detections still land in the database individually; only the SSE / MQTT broadcast is gated.</p>
    <div class="grid gap-4 sm:grid-cols-3">
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Min Detections</label>
        <input name="presence_min_detections" type="number" min="1" max="20" value="{presence_min}"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
        <p class="mt-1 text-xs text-gray-400 dark:text-plumage-500">Set to 1 to disable confirmation (every detection broadcasts immediately).</p>
      </div>
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Window (minutes)</label>
        <input name="presence_window_minutes" type="number" min="1" max="240" value="{presence_window}"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
        <p class="mt-1 text-xs text-gray-400 dark:text-plumage-500">Sliding window for repeat-detection confirmation.</p>
      </div>
      <div>
        <label class="block text-sm font-medium text-gray-700 dark:text-plumage-300 mb-1">Immediate Threshold</label>
        <input name="presence_immediate_threshold" type="number" step="0.01" min="0" max="1" value="{presence_immediate}" placeholder="e.g. 0.90"
          class="w-full rounded-lg border border-gray-300 dark:border-plumage-700 bg-white dark:bg-plumage-800 px-3 py-2 text-sm focus:ring-2 focus:ring-nuthatch-500 focus:border-nuthatch-500 outline-none">
        <p class="mt-1 text-xs text-gray-400 dark:text-plumage-500">Single detection at or above this confidence bypasses the repeat requirement. Empty = disabled.</p>
      </div>
    </div>
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

  <!-- System info: read-only, restart-required values -->
  <details class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 group">
    <summary class="cursor-pointer p-5 select-none flex items-center justify-between">
      <div>
        <h3 class="text-sm font-semibold text-gray-900 dark:text-plumage-100 uppercase tracking-wider">System info</h3>
        <p class="text-xs text-gray-400 dark:text-plumage-500 mt-0.5">Read-only — change in <code class="bg-gray-100 dark:bg-plumage-800 px-1 rounded">config.toml</code> and restart.</p>
      </div>
      <svg class="w-4 h-4 text-gray-400 transition-transform group-open:rotate-180" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M19.5 8.25l-7.5 7.5-7.5-7.5"/></svg>
    </summary>
    <div class="border-t border-gray-200 dark:border-plumage-800 px-5 py-4 space-y-5">
{model_rows}
      <dl class="space-y-2 text-sm">
{system_rows}
      </dl>
    </div>
  </details>
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
          k === 'perch_min_confidence' || k === 'presence_immediate_threshold') {{
        body[k] = parseFloat(v);
      }} else if (k === 'birdnet_top_k' || k === 'perch_top_k' ||
                 k === 'presence_min_detections' || k === 'presence_window_minutes') {{
        body[k] = parseInt(v, 10);
      }} else if (k === 'birdnet_force_allow') {{
        body[k] = v.split(',').map(s => s.trim()).filter(s => s);
      }} else {{
        body[k] = v;
      }}
    }}
    // Checkboxes: unchecked inputs are absent from FormData; set explicitly.
    body.show_range_unverified = form.querySelector('[name=show_range_unverified]').checked;
    body.skip_environment_clips = form.querySelector('[name=skip_environment_clips]').checked;
    body.skip_environment_detections = form.querySelector('[name=skip_environment_detections]').checked;

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
          // Inference-rebuild fields land in config.toml but consumer
          // hot-reload isn't wired yet — be honest about that.
          showToast(n + ' setting' + (n > 1 ? 's' : '') + ' saved' + (data.rebuild_triggered ? ' (inference changes apply on restart)' : ''), true);
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
            <input id="mqtt-host" type="text" value="${{m.host || ''}}" placeholder="e.g. 192.168.1.50 or localhost" class="${{inp()}}">
            <p class="mt-0.5 text-[10px] text-stone-400 dark:text-plumage-600">Hostname or IP only — no protocol prefix (not tcp:// or http://)</p>
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
          <div>
            <label class="block text-xs font-medium text-stone-600 dark:text-plumage-300 mb-1">HA Discovery Prefix</label>
            <input id="mqtt-ha-prefix" type="text" value="${{m.homeassistant_prefix || 'homeassistant'}}" placeholder="homeassistant" class="${{inp()}}">
          </div>
          <div class="flex items-end sm:col-span-2">
            <label class="flex items-center gap-2 text-sm cursor-pointer pb-2">
              <input id="mqtt-ha" type="checkbox" ${{m.homeassistant_discovery ? 'checked' : ''}} class="rounded border-stone-300 dark:border-plumage-700 text-nuthatch-600 focus:ring-nuthatch-500">
              Publish HA auto-discovery messages
            </label>
          </div>
        </div>
      </div>
      <div class="flex items-center gap-2 mt-2">
        <button onclick="testMqtt()" class="px-3 py-1.5 rounded-lg border border-stone-300 dark:border-plumage-700 text-sm font-medium hover:bg-stone-100 dark:hover:bg-plumage-800 transition-colors">Test Connection</button>
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

  function getMqttFormBody() {{
    const prefix = document.getElementById('mqtt-ha-prefix').value.trim() || 'homeassistant';
    return {{
      enabled: document.getElementById('mqtt-enabled').checked,
      host: document.getElementById('mqtt-host').value.trim(),
      port: parseInt(document.getElementById('mqtt-port').value, 10) || 1883,
      username: document.getElementById('mqtt-user').value.trim() || null,
      password: document.getElementById('mqtt-pass').value || null,
      first_of_day_min_confidence: parseFloat(document.getElementById('mqtt-fod').value) || 0.75,
      homeassistant_discovery: document.getElementById('mqtt-ha').checked,
      homeassistant_prefix: prefix,
    }};
  }}

  window.testMqtt = function() {{
    const st = document.getElementById('mqtt-save-status');
    const body = getMqttFormBody();
    if (!body.host) {{ st.innerHTML = '<span class="text-amber-500">Enter a host first</span>'; return; }}
    st.innerHTML = '<span class="text-stone-400 dark:text-plumage-500">Testing...</span>';
    fetch('/api/v1/mqtt/test', {{
      method: 'POST',
      headers: {{ 'Content-Type': 'application/json' }},
      body: JSON.stringify(body),
    }}).then(r => r.json()).then(d => {{
      st.innerHTML = d.success
        ? '<span class="text-emerald-500">' + d.message + '</span>'
        : '<span class="text-red-500">' + d.message + '</span>';
    }}).catch(e => {{
      st.innerHTML = '<span class="text-red-500">Test failed: ' + e.message + '</span>';
    }});
  }};

  window.saveMqtt = function() {{
    const st = document.getElementById('mqtt-save-status');
    const body = getMqttFormBody();
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
        species_image_url = settings.species_image_url.as_deref().unwrap_or(""),
        show_range_unverified_checked = if settings.show_range_unverified { "checked" } else { "" },
        skip_environment_clips_checked = if settings.skip_environment_clips { "checked" } else { "" },
        skip_environment_detections_checked = if settings.skip_environment_detections { "checked" } else { "" },
        presence_min = presence_min,
        presence_window = presence_window,
        presence_immediate = presence_immediate,
        system_rows = system_rows,
        model_rows = model_rows,
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

/// Render the read-only "System info" rows. One <dt>/<dd> per known
/// restart-required config value, with the value shown as a code chip
/// (or "–" when unset).
fn render_system_rows(initial: &InitialConfig) -> String {
    fn row(label: &str, value: Option<&str>) -> String {
        let v = value.filter(|s| !s.is_empty()).unwrap_or("–");
        format!(
            r#"      <div class="flex items-baseline justify-between gap-3">
        <dt class="text-gray-500 dark:text-plumage-400 flex-shrink-0">{label}</dt>
        <dd class="text-right"><code class="text-xs bg-gray-100 dark:bg-plumage-800 px-1.5 py-0.5 rounded break-all">{value}</code></dd>
      </div>"#,
            label = label,
            value = html_escape(v),
        )
    }

    let rows = [
        row("Station ID", Some(&initial.station_id)),
        row("API bind", Some(&initial.api_bind)),
        row("Database", Some(&initial.store_path)),
        row("BirdNET model", initial.birdnet_model_path.as_deref()),
        row("BirdNET labels", initial.birdnet_labels_path.as_deref()),
        row("BirdNET meta-model", initial.birdnet_meta_model_path.as_deref()),
        row("Perch model", initial.perch_model_path.as_deref()),
        row("Perch labels", initial.perch_labels_path.as_deref()),
    ];
    rows.join("\n")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render the "Loaded models" subsection for the System info collapsible.
/// One card per model with name, file path, file size, mtime, and the
/// audio characteristics (sample rate, window, embedding flag) when
/// available.
fn render_model_rows(initial: &InitialConfig) -> String {
    if initial.loaded_models.is_empty() {
        return String::new();
    }

    let cards: Vec<String> = initial
        .loaded_models
        .iter()
        .map(|m| {
            let kind_label = match m.kind {
                "meta_model" => "Range filter",
                _ => "Classifier",
            };
            let size_str = m
                .file_size_bytes
                .map(format_bytes)
                .unwrap_or_else(|| "–".into());
            let mtime_str = m
                .file_modified_ms
                .and_then(|ms| {
                    chrono::DateTime::from_timestamp_millis(ms)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                })
                .unwrap_or_else(|| "–".into());

            let mut audio_chip = String::new();
            if let (Some(sr), Some(ws)) = (m.sample_rate, m.window_samples) {
                let secs = ws as f64 / f64::from(sr);
                audio_chip = format!(
                    r#"<span class="text-[11px] text-gray-500 dark:text-plumage-500">{} Hz · {:.0}s window</span>"#,
                    sr, secs
                );
            }
            let mut emb_chip = String::new();
            if let Some(true) = m.has_embeddings {
                emb_chip = r#"<span class="px-1.5 py-0.5 text-[10px] font-medium rounded bg-emerald-50 text-emerald-700 ring-1 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-300 dark:ring-emerald-400/20">embeddings</span>"#.to_string();
            }
            let path_value = if m.model_path.is_empty() { "–" } else { &m.model_path };

            format!(
                r#"<div class="rounded-lg border border-gray-200 dark:border-plumage-800 p-3">
        <div class="flex items-center justify-between gap-2 mb-1.5">
          <div class="min-w-0">
            <span class="font-medium text-sm">{name}</span>
            <span class="ml-1.5 text-[10px] uppercase tracking-wider text-gray-400 dark:text-plumage-500">{kind}</span>
          </div>
          <div class="flex items-center gap-2 flex-shrink-0">{emb_chip}{audio_chip}</div>
        </div>
        <div class="text-xs"><code class="bg-gray-100 dark:bg-plumage-800 px-1.5 py-0.5 rounded break-all">{path}</code></div>
        <div class="mt-1.5 text-[11px] text-gray-500 dark:text-plumage-500">{size} · modified {mtime}</div>
      </div>"#,
                name = html_escape(&m.name),
                kind = kind_label,
                emb_chip = emb_chip,
                audio_chip = audio_chip,
                path = html_escape(path_value),
                size = size_str,
                mtime = mtime_str,
            )
        })
        .collect();

    format!(
        r#"      <div>
        <h4 class="text-xs font-semibold uppercase tracking-wider text-gray-500 dark:text-plumage-400 mb-2">Loaded models</h4>
        <div class="space-y-2">
{cards}
        </div>
      </div>"#,
        cards = cards.join("\n"),
    )
}

/// Format bytes as a short, human-readable string (e.g. "29.1 MB").
fn format_bytes(b: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes = b as f64;
    if bytes >= GB {
        format!("{:.2} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes / KB)
    } else {
        format!("{} B", b)
    }
}
