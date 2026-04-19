//! Configurable activity visualization panel for the dashboard.
//!
//! The panel fetches hourly detection data from `/api/v1/activity/hourly`
//! and renders it using a pluggable visualization type (stored in
//! localStorage). Currently implemented: **ridge plot**. The structure
//! makes it straightforward to add dot matrix, sparklines, etc.

/// Returns the HTML + inline JS for the daily activity panel.
///
/// The JS is self-contained: it fetches data, picks the renderer based
/// on `localStorage.getItem('sitta-viz-type')`, and draws into the
/// container. No Rust-side interpolation needed, so this is a plain
/// string (no `format!`).
pub fn activity_panel() -> String {
    r##"<!-- Daily activity visualization -->
<div class="mb-6">
  <div class="flex items-center justify-between mb-3">
    <div>
      <h2 class="text-lg font-semibold">Daily Activity</h2>
      <p id="activity-date" class="text-xs text-gray-400 dark:text-plumage-500"></p>
    </div>
    <div class="flex items-center gap-2">
      <button id="activity-prev" class="p-1 rounded hover:bg-gray-100 dark:hover:bg-plumage-800 text-gray-400 dark:text-plumage-500 transition-colors">
        <svg class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M15.75 19.5L8.25 12l7.5-7.5"/></svg>
      </button>
      <button id="activity-next" class="p-1 rounded hover:bg-gray-100 dark:hover:bg-plumage-800 text-gray-400 dark:text-plumage-500 transition-colors">
        <svg class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M8.25 4.5l7.5 7.5-7.5 7.5"/></svg>
      </button>
      <select id="viz-type" class="text-xs bg-white dark:bg-plumage-800 border border-gray-200 dark:border-plumage-700 rounded-md px-2 py-1 text-gray-600 dark:text-plumage-300 cursor-pointer">
        <option value="ridge">Ridge plot</option>
        <option value="dots">Dot matrix</option>
        <option value="spark">Sparklines</option>
      </select>
    </div>
  </div>
  <div id="activity-panel" class="bg-white dark:bg-plumage-900 rounded-xl border border-gray-200 dark:border-plumage-800 p-4 sm:p-4 p-2">
    <div class="text-center py-8 text-gray-400 dark:text-plumage-500 text-sm">Loading activity data...</div>
  </div>
  <div id="activity-tooltip" class="hidden fixed z-50 pointer-events-none bg-white dark:bg-plumage-800 border border-gray-200 dark:border-plumage-700 rounded-lg shadow-lg px-3 py-2 text-xs"></div>
</div>

<script>
(function() {
  const panel = document.getElementById('activity-panel');
  const tooltip = document.getElementById('activity-tooltip');
  const dateLabel = document.getElementById('activity-date');
  const vizSelect = document.getElementById('viz-type');
  const tz = document.body.dataset.tz || 'UTC';
  let currentData = null;

  // Compute local midnight in epoch ms for a given date offset from today.
  let dayOffset = 0;
  function localMidnight(offset) {
    const d = new Date();
    d.setDate(d.getDate() + offset);
    // Use Intl to get the local date parts in the station timezone,
    // then construct a UTC timestamp for that local midnight.
    const parts = new Intl.DateTimeFormat('en-CA', {
      timeZone: tz, year: 'numeric', month: '2-digit', day: '2-digit'
    }).formatToParts(d);
    const y = parts.find(p => p.type === 'year').value;
    const m = parts.find(p => p.type === 'month').value;
    const dd = parts.find(p => p.type === 'day').value;
    // Create a date string and parse it in the target timezone.
    // We'll use the offset between local time and UTC at that midnight.
    const midnightLocal = new Date(`${y}-${m}-${dd}T00:00:00`);
    // Approximate: get the timezone offset for that date
    const formatter = new Intl.DateTimeFormat('en-US', {
      timeZone: tz, timeZoneName: 'shortOffset'
    });
    const tzParts = formatter.formatToParts(midnightLocal);
    const tzStr = tzParts.find(p => p.type === 'timeZoneName')?.value || '+00:00';
    // Parse offset like "GMT-4" or "GMT+5:30"
    const match = tzStr.match(/GMT([+-]?\d+)?(?::(\d+))?/);
    let offsetMs = 0;
    if (match) {
      const hrs = parseInt(match[1] || '0', 10);
      const mins = parseInt(match[2] || '0', 10);
      offsetMs = (hrs * 60 + (hrs < 0 ? -mins : mins)) * 60000;
    }
    return new Date(`${y}-${m}-${dd}T00:00:00Z`).getTime() - offsetMs;
  }

  function formatDate(epochMs) {
    return new Intl.DateTimeFormat('en-US', {
      timeZone: tz, weekday: 'long', month: 'long', day: 'numeric'
    }).format(new Date(epochMs));
  }

  function loadData() {
    const since = localMidnight(dayOffset);
    const until = since + 86400000;
    dateLabel.textContent = formatDate(since);
    fetch(`/api/v1/activity/hourly?since=${since}&until=${until}`)
      .then(r => r.json())
      .then(data => { currentData = data; render(); })
      .catch(() => {
        panel.innerHTML = '<div class="text-center py-8 text-red-400 text-sm">Failed to load activity data</div>';
      });
  }

  function render() {
    if (!currentData || currentData.species.length === 0) {
      panel.innerHTML = '<div class="text-center py-8 text-gray-400 dark:text-plumage-500 text-sm">No detections for this day</div>';
      return;
    }
    const type = vizSelect.value;
    panel.innerHTML = '';
    if (vizRenderers[type]) vizRenderers[type](panel, currentData, tooltip);
    else vizRenderers.ridge(panel, currentData, tooltip);
  }

  // ── Visualization type persistence ────────────────────────────
  const savedType = localStorage.getItem('sitta-viz-type');
  if (savedType && vizSelect.querySelector(`option[value="${savedType}"]`)) {
    vizSelect.value = savedType;
  }
  vizSelect.addEventListener('change', () => {
    localStorage.setItem('sitta-viz-type', vizSelect.value);
    render();
  });

  // ── Day navigation ────────────────────────────────────────────
  document.getElementById('activity-prev').addEventListener('click', () => { dayOffset--; loadData(); });
  document.getElementById('activity-next').addEventListener('click', () => { dayOffset++; loadData(); });

  // ── Visualization renderers ───────────────────────────────────

  const vizRenderers = { ridge: renderRidgePlot, dots: renderDotMatrix, spark: renderSparklines };

  // ── Monotone cubic Hermite interpolation (Fritsch-Carlson) ────
  function monotonePath(pts) {
    const n = pts.length;
    if (n < 2) return '';
    if (n === 2) return `M${pts[0].x.toFixed(1)},${pts[0].y.toFixed(1)}L${pts[1].x.toFixed(1)},${pts[1].y.toFixed(1)}`;
    const dx = [], dy = [], m = [];
    for (let i = 0; i < n - 1; i++) {
      dx.push(pts[i+1].x - pts[i].x);
      dy.push(pts[i+1].y - pts[i].y);
      m.push(dy[i] / dx[i]);
    }
    const t = [m[0]];
    for (let i = 1; i < n - 1; i++) {
      if (m[i-1] * m[i] <= 0) t.push(0);
      else t.push(3 * (dx[i-1] + dx[i]) / ((2*dx[i] + dx[i-1]) / m[i-1] + (dx[i] + 2*dx[i-1]) / m[i]));
    }
    t.push(m[n-2]);
    for (let i = 0; i < n - 1; i++) {
      if (Math.abs(m[i]) < 1e-10) { t[i] = 0; t[i+1] = 0; continue; }
      const a = t[i] / m[i], b = t[i+1] / m[i];
      const s = a*a + b*b;
      if (s > 9) { const tau = 3/Math.sqrt(s); t[i] = tau*a*m[i]; t[i+1] = tau*b*m[i]; }
    }
    let path = `M${pts[0].x.toFixed(1)},${pts[0].y.toFixed(1)}`;
    for (let i = 0; i < n - 1; i++) {
      const d = dx[i] / 3;
      path += `C${(pts[i].x + d).toFixed(1)},${(pts[i].y + t[i]*d).toFixed(1)},${(pts[i+1].x - d).toFixed(1)},${(pts[i+1].y - t[i+1]*d).toFixed(1)},${pts[i+1].x.toFixed(1)},${pts[i+1].y.toFixed(1)}`;
    }
    return path;
  }

  // ── Ridge plot renderer ───────────────────────────────────────
  function renderRidgePlot(container, data, tip) {
    const species = data.species;
    const isDark = document.documentElement.classList.contains('dark');
    const isMobile = container.clientWidth < 500;
    const LABEL_W = isMobile ? 70 : 130;
    const ROW_H = isMobile ? 28 : 36;
    const OVERLAP = isMobile ? 10 : 14;
    const PAD = { top: 20, right: isMobile ? 8 : 16, bottom: 24, left: LABEL_W + 8 };
    const W = container.clientWidth - (isMobile ? 16 : 32);
    const H = PAD.top + PAD.bottom + Math.max(1, species.length) * (ROW_H - OVERLAP) + OVERLAP;
    const plotW = W - PAD.left - PAD.right;

    const maxCount = Math.max(1, ...species.flatMap(s => s.hours));

    const ns = 'http://www.w3.org/2000/svg';
    const svg = document.createElementNS(ns, 'svg');
    svg.setAttribute('viewBox', `0 0 ${W} ${H}`);
    svg.setAttribute('class', 'w-full select-none');

    // Hour grid lines + labels
    const hourLabels = isMobile ? [0, 6, 12, 18] : [0, 3, 6, 9, 12, 15, 18, 21];
    hourLabels.forEach(h => {
      const x = PAD.left + (h / 23) * plotW;
      const line = document.createElementNS(ns, 'line');
      line.setAttribute('x1', x); line.setAttribute('x2', x);
      line.setAttribute('y1', PAD.top - 4);
      line.setAttribute('y2', H - PAD.bottom + 4);
      line.setAttribute('stroke', isDark ? 'rgba(163,188,207,0.08)' : 'rgba(0,0,0,0.05)');
      line.setAttribute('stroke-width', '1');
      svg.appendChild(line);

      const txt = document.createElementNS(ns, 'text');
      txt.setAttribute('x', x); txt.setAttribute('y', H - PAD.bottom + 16);
      txt.setAttribute('text-anchor', 'middle');
      txt.setAttribute('class', 'fill-gray-400 dark:fill-plumage-500');
      txt.style.fontSize = isMobile ? '8px' : '10px';
      txt.textContent = isMobile ? h.toString().padStart(2, '0') : h.toString().padStart(2, '0') + ':00';
      svg.appendChild(txt);
    });

    // Defs for gradient
    const defs = document.createElementNS(ns, 'defs');
    svg.appendChild(defs);

    // Draw species ridges (bottom-up so top species renders last / on top)
    for (let i = species.length - 1; i >= 0; i--) {
      const sp = species[i];
      const baseY = PAD.top + i * (ROW_H - OVERLAP) + ROW_H;
      const xScale = plotW / 23;

      // Build points with sqrt scaling for visual compression
      const points = sp.hours.map((v, h) => ({
        x: PAD.left + h * xScale,
        y: baseY - Math.max(0, Math.sqrt(v / maxCount)) * (ROW_H * 0.85)
      }));

      const curvePath = monotonePath(points);
      const closedPath = curvePath +
        ` L${points[points.length - 1].x.toFixed(1)},${baseY.toFixed(1)}` +
        ` L${points[0].x.toFixed(1)},${baseY.toFixed(1)} Z`;

      // Gradient for this species
      const gradId = 'ridge-grad-' + i;
      const grad = document.createElementNS(ns, 'linearGradient');
      grad.id = gradId;
      grad.setAttribute('x1', '0'); grad.setAttribute('y1', '1');
      grad.setAttribute('x2', '0'); grad.setAttribute('y2', '0');
      const stop1 = document.createElementNS(ns, 'stop');
      stop1.setAttribute('offset', '0%');
      stop1.setAttribute('stop-color', isDark ? '#d97226' : '#c45c1c');
      stop1.setAttribute('stop-opacity', '0.02');
      const stop2 = document.createElementNS(ns, 'stop');
      stop2.setAttribute('offset', '100%');
      stop2.setAttribute('stop-color', isDark ? '#e38a47' : '#d97226');
      // Opacity decreases for lower-ranked species
      const fillOpacity = 0.15 + 0.55 * (1 - i / Math.max(1, species.length - 1));
      stop2.setAttribute('stop-opacity', fillOpacity.toFixed(2));
      grad.appendChild(stop1);
      grad.appendChild(stop2);
      defs.appendChild(grad);

      // Hover group
      const g = document.createElementNS(ns, 'g');
      g.style.cursor = 'pointer';
      g.dataset.species = sp.scientific_name;
      g.dataset.index = i;

      // Filled area
      const area = document.createElementNS(ns, 'path');
      area.setAttribute('d', closedPath);
      area.setAttribute('fill', `url(#${gradId})`);
      g.appendChild(area);

      // Stroke line
      const stroke = document.createElementNS(ns, 'path');
      stroke.setAttribute('d', curvePath);
      stroke.setAttribute('fill', 'none');
      stroke.setAttribute('stroke', isDark ? '#e38a47' : '#d97226');
      stroke.setAttribute('stroke-width', '1.5');
      stroke.setAttribute('stroke-opacity', (0.3 + 0.5 * (1 - i / Math.max(1, species.length - 1))).toFixed(2));
      g.appendChild(stroke);

      // Detection dots — visible anchors for sparse species
      const dots = [];
      sp.hours.forEach((v, h) => {
        if (v === 0) return;
        const dot = document.createElementNS(ns, 'circle');
        dot.setAttribute('cx', points[h].x.toFixed(1));
        dot.setAttribute('cy', points[h].y.toFixed(1));
        // Larger dots for sparse species, smaller for busy ones
        const r = sp.total <= 5 ? 3 : sp.total <= 20 ? 2.5 : 2;
        dot.setAttribute('r', r);
        dot.setAttribute('fill', isDark ? '#e38a47' : '#d97226');
        dot.setAttribute('fill-opacity', sp.total <= 5 ? '0.9' : '0.5');
        dots.push(dot);
        g.appendChild(dot);
      });

      // Hit area (invisible wide strip for hover)
      const hit = document.createElementNS(ns, 'rect');
      hit.setAttribute('x', PAD.left);
      hit.setAttribute('y', baseY - ROW_H);
      hit.setAttribute('width', plotW);
      hit.setAttribute('height', ROW_H);
      hit.setAttribute('fill', 'transparent');
      g.appendChild(hit);

      // Hover events
      g.addEventListener('mouseenter', () => {
        stroke.setAttribute('stroke-width', '2.5');
        stroke.setAttribute('stroke-opacity', '1');
        area.style.filter = 'brightness(1.2)';
        dots.forEach(d => { d.setAttribute('fill-opacity', '1'); d.setAttribute('r', parseFloat(d.getAttribute('r')) + 0.5); });
      });
      g.addEventListener('mouseleave', () => {
        stroke.setAttribute('stroke-width', '1.5');
        stroke.setAttribute('stroke-opacity', (0.3 + 0.5 * (1 - i / Math.max(1, species.length - 1))).toFixed(2));
        area.style.filter = '';
        dots.forEach(d => { d.setAttribute('fill-opacity', sp.total <= 5 ? '0.9' : '0.5'); d.setAttribute('r', sp.total <= 5 ? 3 : sp.total <= 20 ? 2.5 : 2); });
        tip.classList.add('hidden');
      });
      g.addEventListener('mousemove', (e) => {
        const rect = svg.getBoundingClientRect();
        const svgX = (e.clientX - rect.left) * (W / rect.width);
        const hourIdx = Math.round(((svgX - PAD.left) / plotW) * 23);
        const h = Math.max(0, Math.min(23, hourIdx));
        const count = sp.hours[h];
        tip.innerHTML = `<span class="font-semibold">${sp.common_name}</span><br>${h.toString().padStart(2,'0')}:00 &mdash; ${count} detection${count !== 1 ? 's' : ''}`;
        tip.style.left = (e.clientX + 12) + 'px';
        tip.style.top = (e.clientY - 10) + 'px';
        tip.classList.remove('hidden');
      });
      g.addEventListener('click', () => {
        location.href = '/species/' + encodeURIComponent(sp.scientific_name);
      });

      svg.appendChild(g);

      // Species label
      const label = document.createElementNS(ns, 'text');
      label.setAttribute('x', PAD.left - 8);
      label.setAttribute('y', baseY - (ROW_H * 0.3));
      label.setAttribute('text-anchor', 'end');
      label.setAttribute('class', 'fill-gray-600 dark:fill-plumage-300');
      label.style.fontSize = isMobile ? '9px' : '11px';
      label.style.cursor = 'pointer';
      // Truncate long names (shorter on mobile)
      const maxLen = isMobile ? 10 : 18;
      const name = sp.common_name.length > maxLen ? sp.common_name.slice(0, maxLen - 1) + '\u2026' : sp.common_name;
      label.textContent = name;
      label.addEventListener('click', () => {
        location.href = '/species/' + encodeURIComponent(sp.scientific_name);
      });

      // Count badge
      const badge = document.createElementNS(ns, 'text');
      badge.setAttribute('x', W - PAD.right + 4);
      badge.setAttribute('y', baseY - (ROW_H * 0.3));
      badge.setAttribute('text-anchor', 'start');
      badge.setAttribute('class', 'fill-gray-400 dark:fill-plumage-500');
      badge.style.fontSize = '9px';
      badge.textContent = sp.total.toString();

      svg.appendChild(label);
      svg.appendChild(badge);
    }

    container.appendChild(svg);
  }

  // ── Dot matrix renderer ───────────────────────────────────────
  function renderDotMatrix(container, data, tip) {
    const species = data.species;
    const isDark = document.documentElement.classList.contains('dark');
    const isMob = container.clientWidth < 500;
    const LABEL_W = isMob ? 70 : 130;
    const CELL = isMob ? 10 : 20;
    const GAP = isMob ? 1 : 2;
    const PAD = { top: 24, right: isMob ? 8 : 16, bottom: 8, left: LABEL_W + 8 };
    const W = PAD.left + 24 * (CELL + GAP) + PAD.right;
    const H = PAD.top + species.length * (CELL + GAP) + PAD.bottom;

    const maxCount = Math.max(1, ...species.flatMap(s => s.hours));

    const ns = 'http://www.w3.org/2000/svg';
    const svg = document.createElementNS(ns, 'svg');
    svg.setAttribute('viewBox', `0 0 ${W} ${H}`);
    svg.setAttribute('class', 'w-full select-none');

    // Hour labels
    const dotHourStep = isMob ? 6 : 3;
    for (let h = 0; h < 24; h += dotHourStep) {
      const x = PAD.left + h * (CELL + GAP) + CELL / 2;
      const txt = document.createElementNS(ns, 'text');
      txt.setAttribute('x', x); txt.setAttribute('y', PAD.top - 6);
      txt.setAttribute('text-anchor', 'middle');
      txt.setAttribute('class', 'fill-gray-400 dark:fill-plumage-500');
      txt.style.fontSize = isMob ? '8px' : '10px';
      txt.textContent = h.toString().padStart(2, '0');
      svg.appendChild(txt);
    }

    species.forEach((sp, i) => {
      const y = PAD.top + i * (CELL + GAP) + CELL / 2;

      // Label
      const label = document.createElementNS(ns, 'text');
      label.setAttribute('x', PAD.left - 8);
      label.setAttribute('y', y + 4);
      label.setAttribute('text-anchor', 'end');
      label.setAttribute('class', 'fill-gray-600 dark:fill-plumage-300');
      label.style.fontSize = isMob ? '9px' : '11px';
      label.style.cursor = 'pointer';
      const dotMaxLen = isMob ? 10 : 18;
      const name = sp.common_name.length > dotMaxLen ? sp.common_name.slice(0, dotMaxLen - 1) + '\u2026' : sp.common_name;
      label.textContent = name;
      label.addEventListener('click', () => {
        location.href = '/species/' + encodeURIComponent(sp.scientific_name);
      });
      svg.appendChild(label);

      // Dots
      sp.hours.forEach((count, h) => {
        if (count === 0) return;
        const cx = PAD.left + h * (CELL + GAP) + CELL / 2;
        const r = Math.max(2, (Math.sqrt(count / maxCount)) * (CELL / 2 - 1));
        const opacity = 0.25 + 0.75 * (count / maxCount);

        const circle = document.createElementNS(ns, 'circle');
        circle.setAttribute('cx', cx);
        circle.setAttribute('cy', y);
        circle.setAttribute('r', r.toFixed(1));
        circle.setAttribute('fill', isDark ? '#e38a47' : '#d97226');
        circle.setAttribute('fill-opacity', opacity.toFixed(2));
        circle.style.cursor = 'pointer';

        circle.addEventListener('mouseenter', (e) => {
          circle.setAttribute('stroke', isDark ? '#fae8d6' : '#a34619');
          circle.setAttribute('stroke-width', '1.5');
          tip.innerHTML = `<span class="font-semibold">${sp.common_name}</span><br>${h.toString().padStart(2,'0')}:00 &mdash; ${count} detection${count !== 1 ? 's' : ''}`;
          tip.style.left = (e.clientX + 12) + 'px';
          tip.style.top = (e.clientY - 10) + 'px';
          tip.classList.remove('hidden');
        });
        circle.addEventListener('mouseleave', () => {
          circle.removeAttribute('stroke');
          circle.removeAttribute('stroke-width');
          tip.classList.add('hidden');
        });
        circle.addEventListener('click', () => {
          location.href = '/species/' + encodeURIComponent(sp.scientific_name);
        });

        svg.appendChild(circle);
      });
    });

    container.appendChild(svg);
  }

  // ── Sparklines renderer ───────────────────────────────────────
  function renderSparklines(container, data, tip) {
    const species = data.species;
    const isDark = document.documentElement.classList.contains('dark');
    const maxCount = Math.max(1, ...species.flatMap(s => s.hours));

    const wrapper = document.createElement('div');
    wrapper.className = 'space-y-1';

    species.forEach(sp => {
      const row = document.createElement('div');
      row.className = 'flex items-center gap-3 py-1 px-2 rounded-lg hover:bg-gray-50 dark:hover:bg-plumage-800/50 cursor-pointer transition-colors';
      row.addEventListener('click', () => {
        location.href = '/species/' + encodeURIComponent(sp.scientific_name);
      });

      // Label
      const label = document.createElement('span');
      const isMob = container.clientWidth < 500;
      label.className = 'text-gray-600 dark:text-plumage-300 truncate';
      label.style.width = isMob ? '70px' : '130px';
      label.style.flexShrink = '0';
      label.style.fontSize = isMob ? '9px' : '12px';
      label.textContent = sp.common_name;

      // Sparkline SVG
      const sparkW = 200, sparkH = 24;
      const ns = 'http://www.w3.org/2000/svg';
      const svg = document.createElementNS(ns, 'svg');
      svg.setAttribute('viewBox', `0 0 ${sparkW} ${sparkH}`);
      svg.setAttribute('class', 'flex-1');
      svg.style.maxHeight = '24px';

      const pts = sp.hours.map((v, h) => ({
        x: (h / 23) * sparkW,
        y: sparkH - Math.max(1, (v / maxCount) * (sparkH - 2)) - 1
      }));

      const curvePath = monotonePath(pts);
      const areaPath = curvePath +
        ` L${sparkW},${sparkH} L0,${sparkH} Z`;

      const area = document.createElementNS(ns, 'path');
      area.setAttribute('d', areaPath);
      area.setAttribute('fill', isDark ? 'rgba(227,138,71,0.15)' : 'rgba(217,114,38,0.1)');
      svg.appendChild(area);

      const line = document.createElementNS(ns, 'path');
      line.setAttribute('d', curvePath);
      line.setAttribute('fill', 'none');
      line.setAttribute('stroke', isDark ? '#e38a47' : '#d97226');
      line.setAttribute('stroke-width', '1.5');
      svg.appendChild(line);

      // Count
      const count = document.createElement('span');
      count.className = 'text-xs text-gray-400 dark:text-plumage-500 tabular-nums text-right';
      count.style.width = '32px';
      count.style.flexShrink = '0';
      count.textContent = sp.total.toString();

      row.appendChild(label);
      row.appendChild(svg);
      row.appendChild(count);
      wrapper.appendChild(row);
    });

    container.appendChild(wrapper);
  }

  // ── Initialize ────────────────────────────────────────────────
  loadData();
})();
</script>"##
        .to_string()
}
