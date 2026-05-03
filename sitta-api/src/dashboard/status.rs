//! /status page content.

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

