//! /diagnostics page content (Audio Health).

pub fn diagnostics_content() -> String {
    r##"<div class="mb-6">
  <h1 class="text-2xl font-bold tracking-tight">Audio Health</h1>
  <p class="text-sm text-gray-500 dark:text-plumage-400 mt-0.5">Why some detections lack a playable spectrogram</p>
</div>

<div id="ah-loading" class="text-center py-16 text-gray-400 dark:text-plumage-500 text-sm">Loading...</div>
<div id="ah-content" class="hidden space-y-4">

  <div id="ah-disabled" class="hidden bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800/50 rounded-xl p-4 text-sm text-amber-800 dark:text-amber-200">
    Snippet saving is <strong>disabled</strong> in config. No audio clips or spectrograms are being saved for new detections.
  </div>

  <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
    <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
      <h3 class="text-sm font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider mb-3">All-time</h3>
      <dl class="space-y-2 text-sm">
        <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Detections</dt><dd id="ah-total" class="font-medium">--</dd></div>
        <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">With clip</dt><dd id="ah-with" class="font-medium">--</dd></div>
        <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Missing audio</dt><dd id="ah-without" class="font-medium">--</dd></div>
        <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Coverage</dt><dd id="ah-coverage" class="font-medium">--</dd></div>
      </dl>
    </div>

    <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
      <h3 class="text-sm font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider mb-3">Snippet writer</h3>
      <dl class="space-y-2 text-sm">
        <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Clips saved</dt><dd id="ah-saved" class="font-medium">--</dd></div>
        <div class="flex justify-between">
          <dt class="text-gray-500 dark:text-plumage-400" title="Clips dropped because the writer's bounded channel was full — disk I/O can't keep up. The detection row is still saved without audio.">Backpressure drops</dt>
          <dd id="ah-dropped" class="font-medium">--</dd>
        </div>
        <div class="flex justify-between">
          <dt class="text-gray-500 dark:text-plumage-400" title="Process-time errors after the job was accepted: write_wav failed, fs metadata failed, or update_snippet_path failed. The detection row exists but snippet_path stayed NULL.">Write failures</dt>
          <dd id="ah-failed" class="font-medium">--</dd>
        </div>
        <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Bytes written</dt><dd id="ah-bytes" class="font-medium">--</dd></div>
      </dl>
      <p class="text-xs text-gray-400 dark:text-plumage-500 mt-3">Lifetime counters — persisted across restarts via the <code class="bg-gray-100 dark:bg-plumage-800 px-1 rounded">lifetime_metrics</code> table.</p>
    </div>

    <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
      <h3 class="text-sm font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider mb-3">Retention</h3>
      <dl class="space-y-2 text-sm">
        <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Retention days</dt><dd id="ah-retention" class="font-medium">--</dd></div>
        <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Disk cap</dt><dd id="ah-disk" class="font-medium">--</dd></div>
        <div class="flex justify-between"><dt class="text-gray-500 dark:text-plumage-400">Clip dir</dt><dd id="ah-dir" class="font-medium font-mono text-xs truncate max-w-[10rem]" title="">--</dd></div>
      </dl>
      <p class="text-xs text-gray-400 dark:text-plumage-500 mt-3">Clips reviewed as <em>correct</em> are kept past retention.</p>
    </div>
  </div>

  <div class="grid gap-4 md:grid-cols-2">
    <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
      <div class="flex items-baseline justify-between mb-3">
        <h3 class="text-sm font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider">What's protected</h3>
        <span id="ah-tiers-total" class="text-xs text-gray-400 dark:text-plumage-500"></span>
      </div>
      <div id="ah-tiers" class="space-y-1.5 text-sm"></div>
      <p class="text-xs text-gray-400 dark:text-plumage-500 mt-3">Each clip falls in one tier. Lower tiers are evicted first under disk pressure.</p>
    </div>

    <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
      <div class="flex items-baseline justify-between mb-3">
        <h3 class="text-sm font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider">Top species by clip count</h3>
        <span id="ah-cap" class="text-xs text-gray-400 dark:text-plumage-500"></span>
      </div>
      <div id="ah-top-species" class="space-y-1.5 text-sm"></div>
    </div>
  </div>

  <div id="ah-tip" class="hidden rounded-xl p-4 text-sm"></div>

  <!-- Missing-audio range: tells the user at a glance whether the gap is
       historical or still happening right now. -->
  <div id="ah-clipless-card" class="hidden bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <h3 class="text-sm font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider mb-3">Missing-audio detections</h3>
    <dl class="grid grid-cols-2 gap-x-4 gap-y-2 text-sm">
      <dt class="text-gray-500 dark:text-plumage-400">Count</dt>
      <dd id="ah-clipless-count" class="font-medium">--</dd>
      <dt class="text-gray-500 dark:text-plumage-400">Earliest</dt>
      <dd id="ah-clipless-first" class="font-medium font-mono text-xs">--</dd>
      <dt class="text-gray-500 dark:text-plumage-400">Most recent</dt>
      <dd id="ah-clipless-last" class="font-medium font-mono text-xs">--</dd>
    </dl>
    <p id="ah-clipless-hint" class="text-xs text-gray-400 dark:text-plumage-500 mt-3"></p>
  </div>

  <div class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-5">
    <div class="flex items-start justify-between gap-3 mb-4 flex-wrap">
      <div class="min-w-0">
        <h3 class="text-sm font-medium text-gray-500 dark:text-plumage-400 uppercase tracking-wider">Missing audio by day</h3>
        <p id="ah-chart-window-label" class="text-xs text-gray-400 dark:text-plumage-500 mt-0.5">Bar height = detections; orange portion = no clip on disk.</p>
      </div>
      <div class="flex items-center gap-3 flex-wrap text-xs text-gray-400 dark:text-plumage-500">
        <div id="ah-window-buttons" class="inline-flex rounded-lg border border-gray-200 dark:border-plumage-700 overflow-hidden"></div>
        <div class="flex items-center gap-3">
          <span class="inline-flex items-center gap-1.5"><span class="w-2 h-2 rounded-sm bg-emerald-500 inline-block"></span>with clip</span>
          <span class="inline-flex items-center gap-1.5"><span class="w-2 h-2 rounded-sm bg-amber-500 inline-block"></span>missing</span>
        </div>
      </div>
    </div>
    <div id="ah-chart" class="flex items-end gap-1 h-32"></div>
    <div id="ah-empty" class="hidden text-center py-8 text-gray-400 dark:text-plumage-500 text-sm">No detections in this window.</div>
  </div>
</div>

<div id="ah-error" class="hidden text-center py-8 text-red-400 text-sm">Failed to load audio health data.</div>

<script>
(function() {
  const _tz = document.body.dataset.tz || 'UTC';

  function fmtBytes(n) {
    if (!n) return '0 B';
    const u = ['B','KB','MB','GB','TB'];
    let i = 0;
    while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
    return n.toFixed(i === 0 ? 0 : 1) + ' ' + u[i];
  }
  function fmtDay(d) {
    const dt = new Date(d + 'T00:00:00Z');
    return dt.toLocaleDateString('en-GB', { month: 'short', day: 'numeric', timeZone: _tz });
  }

  // Chart window in days. The button row toggles between fixed values;
  // the chart re-fetches with the selected value while the rest of the
  // page stays put (totals + metrics + clipless are all-time).
  const WINDOW_OPTIONS = [30, 90, 365];
  const DEFAULT_WINDOW = 90;
  let currentWindow = DEFAULT_WINDOW;

  function renderWindowButtons() {
    const el = document.getElementById('ah-window-buttons');
    el.innerHTML = WINDOW_OPTIONS.map(d => {
      const active = d === currentWindow;
      const cls = active
        ? 'bg-nuthatch-600 text-white'
        : 'text-gray-500 dark:text-plumage-400 hover:bg-gray-50 dark:hover:bg-plumage-800/50';
      const label = d === 365 ? '1y' : d + 'd';
      return '<button type="button" data-days="' + d + '" class="px-2.5 py-1 text-xs font-medium ' + cls + '">' + label + '</button>';
    }).join('');
    el.querySelectorAll('button[data-days]').forEach(b => {
      b.addEventListener('click', () => {
        currentWindow = parseInt(b.dataset.days, 10);
        renderWindowButtons();
        loadAudioHealth(currentWindow);
      });
    });
  }

  function fmtDateTime(iso) {
    return new Date(iso).toLocaleString('en-GB', {
      year: 'numeric', month: 'short', day: 'numeric',
      hour: '2-digit', minute: '2-digit', hour12: false, timeZone: _tz
    });
  }

  function loadAudioHealth(days) {
    fetch('/api/v1/audio-health?days=' + days)
    .then(r => { if (!r.ok) throw new Error('http ' + r.status); return r.json(); })
    .then(data => {
      document.getElementById('ah-loading').classList.add('hidden');
      document.getElementById('ah-content').classList.remove('hidden');

      if (!data.enabled) {
        document.getElementById('ah-disabled').classList.remove('hidden');
      }

      const t = data.totals;
      document.getElementById('ah-total').textContent = t.total.toLocaleString();
      document.getElementById('ah-with').textContent = t.with_clip.toLocaleString();
      const missEl = document.getElementById('ah-without');
      missEl.textContent = t.without_clip.toLocaleString();
      if (t.without_clip > 0) {
        missEl.classList.add('text-amber-600','dark:text-amber-400');
      }
      const cov = t.total > 0 ? Math.round(100 * t.with_clip / t.total) : 0;
      const covEl = document.getElementById('ah-coverage');
      covEl.textContent = t.total > 0 ? cov + '%' : '--';
      if (t.total > 0) {
        covEl.classList.add(cov >= 90 ? 'text-emerald-600' : cov >= 60 ? 'text-amber-600' : 'text-red-600');
        covEl.classList.add(cov >= 90 ? 'dark:text-emerald-400' : cov >= 60 ? 'dark:text-amber-400' : 'dark:text-red-400');
      }

      const m = data.metrics;
      document.getElementById('ah-saved').textContent = m.clips_saved.toLocaleString();
      const drEl = document.getElementById('ah-dropped');
      drEl.textContent = m.clips_dropped.toLocaleString();
      if (m.clips_dropped > 0) {
        drEl.classList.add('text-amber-600','dark:text-amber-400');
      }
      const failEl = document.getElementById('ah-failed');
      failEl.textContent = (m.clips_failed || 0).toLocaleString();
      if (m.clips_failed > 0) {
        failEl.classList.add('text-red-600','dark:text-red-400');
      }
      document.getElementById('ah-bytes').textContent = fmtBytes(m.bytes_written);

      // ── Clipless range card ──
      const cl = data.clipless;
      const clCard = document.getElementById('ah-clipless-card');
      if (cl) {
        document.getElementById('ah-clipless-count').textContent = cl.count.toLocaleString();
        document.getElementById('ah-clipless-first').textContent = fmtDateTime(cl.first_detected_at);
        document.getElementById('ah-clipless-last').textContent = fmtDateTime(cl.last_detected_at);
        const hint = document.getElementById('ah-clipless-hint');
        const recent = cl.recent_count || 0;
        // recent_count is "new clipless rows in the last 15 min" — the
        // only signal that distinguishes an active gap from a historical
        // one. Absolute age of `last_detected_at` doesn't, because a
        // gap that ended 30 min ago still has a recent timestamp.
        if (recent > 0) {
          hint.textContent = recent.toLocaleString() + ' new clipless detection' + (recent === 1 ? '' : 's') +
            ' in the last 15 minutes — the snippet writer is missing clips RIGHT NOW. ' +
            'Check Write failures + Backpressure drops above; if both are 0 the writer task may have died silently.';
          hint.className = 'text-xs text-red-600 dark:text-red-400 mt-3';
        } else {
          hint.textContent = 'No new clipless detections in the last 15 minutes — the writer is currently keeping up. ' +
            'The count above is historical. Older clipless rows accumulate during periods when the writer was unhealthy ' +
            '(crashed task, full disk, etc.) and stay until retention sweeps them.';
          hint.className = 'text-xs text-gray-400 dark:text-plumage-500 mt-3';
        }
        clCard.classList.remove('hidden');
      } else {
        clCard.classList.add('hidden');
      }

      if (data.retention) {
        document.getElementById('ah-retention').textContent =
          data.retention.retention_days > 0 ? data.retention.retention_days + ' days' : 'unlimited';
        document.getElementById('ah-disk').textContent =
          data.retention.max_disk_mb > 0 ? data.retention.max_disk_mb + ' MB' : 'unlimited';
      } else {
        document.getElementById('ah-retention').textContent = 'n/a';
        document.getElementById('ah-disk').textContent = 'n/a';
      }
      const dirEl = document.getElementById('ah-dir');
      dirEl.textContent = data.clip_dir || 'n/a';
      if (data.clip_dir) dirEl.title = data.clip_dir;

      // ── Tier breakdown (what's protected) ──
      function spanDays(mul) {
        if (!data.retention || data.retention.retention_days === 0) return 'unlimited';
        const days = data.retention.retention_days * mul;
        if (days >= 365) return Math.round(days / 365 * 10) / 10 + ' yr';
        if (days >= 30)  return Math.round(days / 30) + ' mo';
        return days + ' d';
      }
      const tiersEl = document.getElementById('ah-tiers');
      const tierTotal = (data.tiers ? (
        data.tiers.reviewed_correct + data.tiers.first_ever + data.tiers.first_season +
        data.tiers.first_week + data.tiers.first_day + data.tiers.high_score + data.tiers.common
      ) : 0);
      document.getElementById('ah-tiers-total').textContent = tierTotal.toLocaleString() + ' clip' + (tierTotal === 1 ? '' : 's');
      const tiers = [
        { label: 'Reviewed correct',  count: data.tiers ? data.tiers.reviewed_correct : 0, span: 'forever',                              cls: 'bg-emerald-500' },
        { label: 'First ever',        count: data.tiers ? data.tiers.first_ever : 0,       span: spanDays(data.retention ? data.retention.first_ever_multiplier   : 0), cls: 'bg-purple-500' },
        { label: 'First of season',   count: data.tiers ? data.tiers.first_season : 0,     span: spanDays(data.retention ? data.retention.first_season_multiplier : 0), cls: 'bg-blue-500'   },
        { label: 'First this week',   count: data.tiers ? data.tiers.first_week : 0,       span: spanDays(data.retention ? data.retention.first_week_multiplier   : 0), cls: 'bg-teal-500'   },
        { label: 'First today',       count: data.tiers ? data.tiers.first_day : 0,        span: spanDays(data.retention ? data.retention.first_day_multiplier    : 0), cls: 'bg-sky-500'    },
        { label: 'High score (>= 0.6)', count: data.tiers ? data.tiers.high_score : 0,     span: spanDays(data.retention ? data.retention.high_score_multiplier   : 0), cls: 'bg-amber-500'  },
        { label: 'Common',            count: data.tiers ? data.tiers.common : 0,           span: spanDays(1),                            cls: 'bg-stone-400'  },
      ];
      const tierMax = Math.max(1, ...tiers.map(t => t.count));
      tiersEl.innerHTML = tiers.map(t => {
        const pct = (t.count / tierMax) * 100;
        return '<div class="flex items-center gap-3">' +
               '  <span class="w-32 text-xs text-gray-600 dark:text-plumage-300 flex-shrink-0">' + t.label + '</span>' +
               '  <div class="flex-1 h-3 bg-stone-100 dark:bg-plumage-800/60 rounded-sm overflow-hidden">' +
               '    <div class="h-full ' + t.cls + '" style="width:' + pct + '%"></div>' +
               '  </div>' +
               '  <span class="w-10 text-right text-xs font-mono text-gray-600 dark:text-plumage-300">' + t.count.toLocaleString() + '</span>' +
               '  <span class="w-14 text-right text-[10px] uppercase tracking-wider text-gray-400 dark:text-plumage-500">' + t.span + '</span>' +
               '</div>';
      }).join('');

      // ── Top species by clip count ──
      const topEl = document.getElementById('ah-top-species');
      const cap = (data.retention && data.retention.per_species_cap) ? data.retention.per_species_cap : 0;
      document.getElementById('ah-cap').textContent =
        cap > 0 ? 'cap: ' + cap + ' / species' : 'no per-species cap';
      const topSpecies = data.top_species || [];
      if (topSpecies.length === 0) {
        topEl.innerHTML = '<p class="text-xs text-gray-400 dark:text-plumage-500">No clips on disk yet.</p>';
      } else {
        const topMax = Math.max(1, ...topSpecies.map(s => s.clip_count));
        topEl.innerHTML = topSpecies.map(s => {
          const pct = (s.clip_count / topMax) * 100;
          const overCap = cap > 0 && s.clip_count > cap;
          const barCls = overCap ? 'bg-amber-500' : 'bg-nuthatch-500';
          return '<a href="/species/' + encodeURIComponent(s.scientific_name) + '" class="flex items-center gap-3 -mx-1 px-1 py-0.5 rounded hover:bg-gray-50 dark:hover:bg-plumage-800/40 transition-colors">' +
                 '  <span class="w-32 text-xs truncate text-stone-700 dark:text-plumage-200" title="' + s.scientific_name + '">' + s.common_name + '</span>' +
                 '  <div class="flex-1 h-3 bg-stone-100 dark:bg-plumage-800/60 rounded-sm overflow-hidden">' +
                 '    <div class="h-full ' + barCls + '" style="width:' + pct + '%"></div>' +
                 '  </div>' +
                 '  <span class="w-10 text-right text-xs font-mono ' + (overCap ? 'text-amber-600 dark:text-amber-400' : 'text-gray-600 dark:text-plumage-300') + '">' + s.clip_count.toLocaleString() + '</span>' +
                 '</a>';
        }).join('');
      }

      // Diagnostic tip
      const tipEl = document.getElementById('ah-tip');
      let tip = '';
      if (!data.enabled) {
        // Already covered by the disabled banner.
      } else if (m.clips_dropped > 0 && m.clips_saved > 0 && m.clips_dropped / (m.clips_saved + m.clips_dropped) > 0.05) {
        tip = '<strong>Backpressure detected.</strong> The snippet writer is dropping more than 5% of clips since the last restart. Disk I/O (SD card or USB) likely can\'t keep up with detection rate. New detections in this window have no audio.';
        tipEl.className = 'rounded-xl p-4 text-sm bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800/50 text-amber-800 dark:text-amber-200';
      } else if (data.retention && data.retention.retention_days > 0 && t.without_clip > t.with_clip) {
        tip = '<strong>Retention is the likely cause.</strong> Most missing-audio detections are probably older than ' + data.retention.retention_days + ' days and were swept by the retention worker. Detections marked <em>correct</em> via review are spared.';
        tipEl.className = 'rounded-xl p-4 text-sm bg-plumage-50 dark:bg-plumage-900/40 border border-plumage-200 dark:border-plumage-800/50 text-plumage-800 dark:text-plumage-200';
      }
      if (tip) {
        tipEl.innerHTML = tip;
        tipEl.classList.remove('hidden');
      }

      // ── Chart ──
      document.getElementById('ah-chart-window-label').textContent =
        (days === 365 ? 'Last 12 months. ' : 'Last ' + days + ' days. ') +
        'Bar height = detections; orange portion = no clip on disk.';
      const chart = document.getElementById('ah-chart');
      const empty = document.getElementById('ah-empty');
      const series = (data.daily || []).slice().reverse(); // oldest -> newest, left -> right
      if (series.length === 0) {
        chart.classList.add('hidden');
        empty.classList.remove('hidden');
        return;
      }
      chart.classList.remove('hidden');
      empty.classList.add('hidden');
      const max = series.reduce((m, d) => Math.max(m, d.total), 0) || 1;
      chart.innerHTML = series.map(d => {
        const totalH = (d.total / max) * 100;
        const withH = d.total > 0 ? (d.with_clip / d.total) * totalH : 0;
        const missH = totalH - withH;
        const pct = d.total > 0 ? Math.round(100 * d.with_clip / d.total) : 0;
        const title = fmtDay(d.day) + ': ' + d.total + ' detections, ' + d.with_clip + ' with clip (' + pct + '%), ' + (d.total - d.with_clip) + ' missing';
        // For 365-day windows the per-bar label is too dense; drop it.
        const label = days <= 90
          ? '<div class="text-[9px] text-gray-400 dark:text-plumage-600 text-center mt-0.5 truncate">' + fmtDay(d.day).split(' ')[1] + '</div>'
          : '';
        return '<div class="flex-1 min-w-0 flex flex-col items-stretch justify-end gap-px" title="' + title.replace(/"/g, '&quot;') + '">' +
                 '<div class="bg-amber-500/80 rounded-t-sm" style="height:' + missH + '%"></div>' +
                 '<div class="bg-emerald-500/80" style="height:' + withH + '%"></div>' +
                 label +
               '</div>';
      }).join('');
    })
    .catch(() => {
      document.getElementById('ah-loading').classList.add('hidden');
      document.getElementById('ah-error').classList.remove('hidden');
    });
  }

  renderWindowButtons();
  loadAudioHealth(currentWindow);
})();
</script>"##
        .to_string()
}

