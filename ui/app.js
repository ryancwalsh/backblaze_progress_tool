'use strict';
// Backblaze Progress UI. All data comes from the local Rust backend; nothing is sent anywhere.

const T = window.__TAURI__;
const invoke = T && T.core ? T.core.invoke : null;

const STATUS = {
  waiting:   { label: 'Waiting',         order: 5 },
  uploading: { label: 'Uploading',       order: 0 },
  partial:   { label: 'Partly uploaded', order: 1 },
  changed:   { label: 'Changed',         order: 6 },
  busy:      { label: 'In use',          order: 3 },
  locked:    { label: 'Locked',          order: 3 },
  noperm:    { label: 'No permission',   order: 2 },
  skipped:   { label: 'Skipped',         order: 3 },
  cloud:     { label: 'Cloud-only',      order: 4 },
  longpath:  { label: 'Path too long',   order: 2 },
  error:     { label: 'Error',           order: 2 },
  missing:   { label: 'Gone',            order: 7 },
  uploaded:  { label: 'Uploaded',        order: 8 },
};

const state = {
  snap: null, rows: [], view: [],
  sortKey: 'size', sortDir: -1,
  folder: '', group: 'status', selected: null,
  loading: false, timer: null,
};

const $ = (s) => document.querySelector(s);
const $$ = (s) => Array.from(document.querySelectorAll(s));
const ROW_H = 26, HEAD_H = 30;

// ---------- formatting ----------
function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
function fmtBytes(n) {
  if (n == null) return '—';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0, v = Number(n);
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return (i === 0 ? v.toFixed(0) : v.toFixed(v < 10 ? 2 : v < 100 ? 1 : 0)) + ' ' + u[i];
}
function fmtNum(n) { return n == null ? '—' : Number(n).toLocaleString(); }
function fmtTime(ms) { return ms ? new Date(ms).toLocaleString() : '—'; }
function fmtAgo(ms) {
  if (!ms) return '';
  const s = Math.round((Date.now() - ms) / 1000);
  if (s < 90) return `${s}s ago`;
  if (s < 5400) return `${Math.round(s / 60)} min ago`;
  if (s < 172800) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} days ago`;
}
function parentOf(p) { const i = Math.max(p.lastIndexOf('\\'), p.lastIndexOf('/')); return i > 0 ? p.slice(0, i) : ''; }
function badge(st) { return `<span class="badge s-${esc(st)}">${esc((STATUS[st] || { label: st }).label)}</span>`; }
function setStatus(msg, isErr) { const el = $('#status'); el.textContent = msg; el.classList.toggle('error', !!isErr); }

// ---------- loading ----------
async function refresh() {
  if (state.loading) return;
  state.loading = true;
  setStatus('Reading Backblaze files…');
  const t0 = performance.now();
  try {
    let snap;
    if (invoke) {
      snap = await invoke('load_snapshot', { checkFiles: $('#chkFiles').checked });
    } else {
      // Opened outside the app (for UI development): use a sample file if present.
      snap = await (await fetch('sample-snapshot.json')).json();
    }
    state.snap = snap;
    state.rows = snap.rows.map((r) => Object.assign(r, { parent: parentOf(r.path), lc: r.path.toLowerCase() }));
    renderAll();
    const secs = ((performance.now() - t0) / 1000).toFixed(1);
    setStatus(`Loaded ${fmtNum(snap.rows.length)} queued files in ${secs}s · ${new Date().toLocaleTimeString()}`);
  } catch (e) {
    setStatus('Error: ' + (e && e.message ? e.message : e), true);
  } finally {
    state.loading = false;
  }
}

function renderAll() {
  populateFilters();
  renderCards();
  applyFilters();
  renderHealth();
  renderHistory();
  renderIssues();
  renderExclusions();
  renderEvents();
  if (state.selected) {
    const again = state.rows.find((r) => r.path === state.selected.path);
    state.selected = again || null;
    renderDetail();
  }
}

// ---------- summary cards ----------
function renderCards() {
  const s = state.snap;
  const pending = state.rows.filter((r) => r.status !== 'uploaded');
  const pendingBytes = pending.reduce((a, r) => a + r.size, 0);
  const todoBytes = state.rows.reduce((a, r) => a + r.size, 0);
  const lf = s.largeFile;
  const st = s.settings || {};
  $('#ver').textContent = st.clientVersion ? `· Backblaze ${st.clientVersion}` : '';

  const cards = [];
  cards.push(`<div class="card"><h3>Backblaze says remaining</h3>
    <div class="big">${fmtBytes(s.remainingBytes)}</div>
    <div class="sub">${fmtNum(s.remainingFiles)} files · updated ${esc(fmtAgo(s.remainingWrittenMs))}</div></div>`);
  cards.push(`<div class="card"><h3>Not yet uploaded (this app)</h3>
    <div class="big">${fmtBytes(pendingBytes)}</div>
    <div class="sub">${fmtNum(pending.length)} of ${fmtNum(state.rows.length)} queued files</div></div>`);
  cards.push(`<div class="card"><h3>To-do list</h3>
    <div class="big">${fmtBytes(todoBytes)}</div>
    <div class="sub" title="${esc(s.todoFile)}">written ${esc(fmtTime(s.todoWrittenMs))}</div></div>`);
  let up = '<div class="big">Idle</div>';
  if (s.current && s.current.state) {
    up = `<div class="big">${esc(s.current.state)}</div><div class="sub" title="${esc(s.current.fullPath)}">${esc(s.current.file || '')}</div>`;
  }
  if (lf) {
    const pct = lf.bytes ? Math.min(100, (lf.chunkBytesSent / lf.bytes) * 100) : 0;
    up += `<div class="sub" title="${esc(lf.path)}">Large file ${fmtBytes(lf.bytes)} · ${lf.chunksSent} new chunk(s) logged</div>
      <div class="bar" title="Chunks written to the upload log since the to-do list (unchanged chunks are skipped and not counted)"><span style="width:${pct.toFixed(1)}%"></span></div>`;
  }
  cards.push(`<div class="card"><h3>Uploading now</h3>${up}</div>`);
  cards.push(`<div class="card"><h3>Speed</h3>
    <div class="big">${st.lastKbps != null ? (st.lastKbps / 1000).toFixed(1) + ' Mbit/s' : '—'}</div>
    <div class="sub">measured: large ${st.measuredKbpsLarge != null ? (st.measuredKbpsLarge / 1000).toFixed(1) : '—'} · small ${st.measuredKbpsSmall != null ? (st.measuredKbpsSmall / 1000).toFixed(1) : '—'} Mbit/s</div></div>`);
  cards.push(`<div class="card"><h3>Settings</h3>
    <div class="big">${esc(st.threads || '?')} thread${st.threads === '1' ? '' : 's'}</div>
    <div class="sub">throttle ${esc(st.throttle || '?')}${st.autoThrottle === 'true' ? ' (auto)' : ''} · ${esc(st.schedule || '?')}</div></div>`);
  cards.push(`<div class="card"><h3>Last backup pass completed</h3>
    <div class="big">${esc(fmtAgo(s.lastBackupCompletedMs) || '—')}</div>
    <div class="sub">${esc(fmtTime(s.lastBackupCompletedMs))}</div></div>`);
  if (s.volumes && s.volumes.length) {
    const lines = s.volumes.map((v) => `${esc(v.drive || v.guid)} ${fmtBytes(v.remainingBytes)} / ${fmtNum(v.remainingFiles)} files`).join('<br>');
    cards.push(`<div class="card"><h3>Remaining per drive</h3><div class="sub" style="white-space:normal">${lines}</div></div>`);
  }
  $('#cards').innerHTML = cards.join('');
}

// ---------- filters ----------
function populateFilters() {
  const count = (k) => {
    const m = new Map();
    for (const r of state.rows) m.set(r[k], (m.get(r[k]) || 0) + 1);
    return m;
  };
  const fill = (sel, m, labelFn, sortFn) => {
    const cur = sel.value;
    const first = sel.options[0].outerHTML;
    const entries = Array.from(m.entries()).sort(sortFn || ((a, b) => b[1] - a[1]));
    sel.innerHTML = first + entries.map(([k, n]) => `<option value="${esc(k)}">${esc(labelFn ? labelFn(k) : k || '(none)')} (${fmtNum(n)})</option>`).join('');
    if (Array.from(sel.options).some((o) => o.value === cur)) sel.value = cur;
  };
  fill($('#fStatus'), count('status'), (k) => (STATUS[k] || { label: k }).label);
  fill($('#fDrive'), count('drive'), null, (a, b) => String(a[0]).localeCompare(String(b[0])));
  fill($('#fExt'), count('ext'));
}

function applyFilters() {
  const words = $('#fSearch').value.toLowerCase().split(/\s+/).filter(Boolean);
  const st = $('#fStatus').value, dr = $('#fDrive').value, ex = $('#fExt').value;
  const minB = (parseFloat($('#fMin').value) || 0) * 1024 * 1024;
  const hideUp = $('#fHideUploaded').checked && st !== 'uploaded';
  const folder = state.folder.toLowerCase();
  state.view = state.rows.filter((r) =>
    (!hideUp || r.status !== 'uploaded') &&
    (!st || r.status === st) &&
    (!dr || r.drive === dr) &&
    (!ex || r.ext === ex) &&
    (r.size >= minB) &&
    (!folder || r.lc.startsWith(folder)) &&
    words.every((w) => r.lc.includes(w)));
  const tag = $('#fFolderTag');
  tag.classList.toggle('hidden', !state.folder);
  tag.textContent = state.folder ? `Folder: ${state.folder}  ✕` : '';
  sortView();
  const bytes = state.view.reduce((a, r) => a + r.size, 0);
  $('#viewInfo').textContent = `Showing ${fmtNum(state.view.length)} files · ${fmtBytes(bytes)}`;
  renderTable(true);
  renderGroups();
}

function sortView() {
  const k = state.sortKey, d = state.sortDir;
  const val = (r) => k === 'status' ? (STATUS[r.status] || { order: 9 }).order : r[k];
  state.view.sort((a, b) => {
    const x = val(a), y = val(b);
    if (x === y) return b.size - a.size;
    if (typeof x === 'number' && typeof y === 'number') return (x - y) * d;
    return String(x ?? '').localeCompare(String(y ?? ''), undefined, { sensitivity: 'base' }) * d;
  });
  $$('#thead > div').forEach((el) => {
    el.classList.toggle('sorted', el.dataset.k === k);
    el.classList.toggle('asc', el.dataset.k === k && d === 1);
  });
}

// ---------- virtual table ----------
function renderTable(resetScroll) {
  const body = $('#tbody');
  if (resetScroll) body.scrollTop = 0;
  $('#spacer').style.height = HEAD_H + state.view.length * ROW_H + 'px';
  const first = Math.max(0, Math.floor((body.scrollTop) / ROW_H) - 10);
  const count = Math.ceil(body.clientHeight / ROW_H) + 20;
  const slice = state.view.slice(first, first + count);
  const layer = $('#rowsLayer');
  layer.style.top = HEAD_H + first * ROW_H + 'px';
  const selPath = state.selected && state.selected.path;
  layer.innerHTML = slice.map((r, i) => `<div class="trow${r.path === selPath ? ' sel' : ''}" data-i="${first + i}">
      <div>${badge(r.status)}</div>
      <div class="num">${fmtBytes(r.size)}</div>
      <div title="${esc(r.name)}">${esc(r.name)}</div>
      <div title="${esc(r.parent)}">${esc(r.parent)}</div>
      <div>${esc(r.ext)}</div>
      <div>${esc(fmtTime(r.queuedMtime))}</div>
      <div title="${esc(r.reason)}">${esc(r.reason)}</div>
    </div>`).join('');
}

// ---------- side tabs ----------
function renderHealth() {
  const h = state.snap.health || [];
  const warn = state.snap.warnings || [];
  let html = warn.map((w) => `<div class="hitem warn"><h4>${esc(w)}</h4></div>`).join('');
  if (!h.length && !warn.length) html = '<p class="muted">No problems found in Backblaze\'s reports and logs.</p>';
  html += h.map((x) => `<div class="hitem ${esc(x.level)}">
      <h4>${esc(x.title)} <span class="muted">(${fmtNum(x.count)})</span></h4>
      <div>${esc(x.detail)}</div>
      ${x.first || x.last ? `<div class="meta">first ${esc(x.first || '?')} · last ${esc(x.last || '?')}</div>` : ''}
      ${x.examples && x.examples.length ? `<ul>${x.examples.map((e) => `<li>${esc(e)}</li>`).join('')}</ul>` : ''}
    </div>`).join('');
  $('#tab-health').innerHTML = html;
}

function renderGroups() {
  const g = state.group;
  const m = new Map();
  for (const r of state.view) {
    const k = r[g] || '(none)';
    const e = m.get(k) || { n: 0, b: 0 };
    e.n++; e.b += r.size; m.set(k, e);
  }
  const rows = Array.from(m.entries()).sort((a, b) => b[1].b - a[1].b).slice(0, 200);
  $('#groupTable').innerHTML = `<table class="mini"><tr><th>${esc(g)}</th><th class="num">Files</th><th class="num">Size</th></tr>${rows.map(([k, e]) =>
    `<tr class="click" data-k="${esc(k)}"><td class="path">${g === 'status' ? badge(k) : esc(k)}</td><td class="num">${fmtNum(e.n)}</td><td class="num">${fmtBytes(e.b)}</td></tr>`).join('')}</table>`;
}

function renderHistory() {
  const h = state.snap.history || [];
  const el = $('#tab-history');
  if (h.length < 2) {
    el.innerHTML = `<p class="muted">The app records Backblaze's "remaining" figure each time it refreshes (stored only on this computer). A chart appears once there are two or more readings.</p>${historyTable(h)}`;
    return;
  }
  const W = 400, H = 200, P = 34;
  const t0 = h[0].t, t1 = h[h.length - 1].t;
  const maxB = Math.max(...h.map((p) => p.bytes)) || 1;
  const x = (t) => P + ((t - t0) / Math.max(1, t1 - t0)) * (W - P - 8);
  const y = (b) => H - P + 8 - (b / maxB) * (H - P - 8);
  const pts = h.map((p) => `${x(p.t).toFixed(1)},${y(p.bytes).toFixed(1)}`).join(' ');
  el.innerHTML = `<h5>Remaining bytes over time</h5>
    <svg viewBox="0 0 ${W} ${H}" width="100%" role="img" aria-label="Remaining backup size over time">
      <line x1="${P}" y1="${H - P + 8}" x2="${W - 8}" y2="${H - P + 8}" stroke="currentColor" opacity=".25"/>
      <line x1="${P}" y1="8" x2="${P}" y2="${H - P + 8}" stroke="currentColor" opacity=".25"/>
      <text x="${P - 4}" y="14" text-anchor="end">${esc(fmtBytes(maxB))}</text>
      <text x="${P - 4}" y="${H - P + 8}" text-anchor="end">0</text>
      <text x="${P}" y="${H - 8}">${esc(new Date(t0).toLocaleString())}</text>
      <text x="${W - 8}" y="${H - 8}" text-anchor="end">${esc(new Date(t1).toLocaleString())}</text>
      <polyline points="${pts}" fill="none" style="stroke:var(--accent);stroke-width:2"/>
      ${h.map((p) => `<circle cx="${x(p.t).toFixed(1)}" cy="${y(p.bytes).toFixed(1)}" r="2.5" style="fill:var(--accent)"><title>${esc(fmtTime(p.t))}: ${esc(fmtBytes(p.bytes))}, ${esc(fmtNum(p.files))} files</title></circle>`).join('')}
    </svg>${historyTable(h)}`;
}
function historyTable(h) {
  return `<table class="mini"><tr><th>When</th><th class="num">Files</th><th class="num">Size</th></tr>${h.slice().reverse().slice(0, 100).map((p) =>
    `<tr><td>${esc(fmtTime(p.t))}</td><td class="num">${fmtNum(p.files)}</td><td class="num">${fmtBytes(p.bytes)}</td></tr>`).join('')}</table>`;
}

function renderIssues() {
  const s = state.snap.skipped || [];
  $('#tab-issues').innerHTML = `<p class="muted small">From Backblaze's skipped-files report (bzlist_skipped_files.txt). TEMPORARY_FILE_BUSY is retried automatically; PERMANENT_BAD_PERMISSIONS is not.</p>
    ${s.length ? `<table class="mini"><tr><th>When</th><th>Code</th><th>Path</th></tr>${s.slice().reverse().map((x) =>
      `<tr class="click" data-path="${esc(x.path)}"><td>${esc(x.time)}</td><td>${esc(x.code)}</td><td class="path">${esc(x.path)}${x.inTodo ? '' : ' <span class="muted">(not in to-do)</span>'}</td></tr>`).join('')}</table>` : '<p>No skipped files reported.</p>'}`;
}

function renderExclusions() {
  const e = state.snap.exclusions || {};
  const rules = e.editableRules || [];
  const cols = ['plat', 'skipFirstCharThenStartsWith', 'contains_1', 'contains_2', 'doesNotContain', 'endsWith', 'hasFileExtension'];
  $('#tab-excl').innerHTML = `
    <h5>Excluded file types</h5><div class="chips">${(e.fileTypes || []).map((t) => `<span class="chip">.${esc(t)}</span>`).join('') || '<span class="muted">none</span>'}</div>
    <h5>Excluded folders</h5>${(e.dirs || []).length ? `<table class="mini">${e.dirs.map((d) => `<tr><td class="path">${esc(d)}</td></tr>`).join('')}</table>` : '<p class="muted">none</p>'}
    <h5>Advanced rules (bzexcluderules_editable.xml): ${rules.length}</h5>
    ${rules.length ? `<table class="mini"><tr>${cols.map((c) => `<th>${esc(c.replace('skipFirstCharThenStartsWith', 'startsWith'))}</th>`).join('')}</tr>${rules.map((r) =>
      `<tr>${cols.map((c) => `<td class="path">${esc(r[c] === '*' ? '' : r[c] || '')}</td>`).join('')}</tr>`).join('')}</table>` : ''}
    <p class="muted small">Change exclusions in Backblaze's own Settings > Exclusions. This app never edits them.</p>`;
}

function renderEvents() {
  const ev = state.snap.events || [];
  $('#tab-events').innerHTML = ev.length ? `<table class="mini"><tr><th>When</th><th>Event</th></tr>${ev.map((x) =>
    `<tr><td style="white-space:nowrap">${esc(x.time)}</td><td>${esc(x.text)}</td></tr>`).join('')}</table>` : '<p class="muted">No backup-pass events found.</p>';
}

function renderDetail() {
  const el = $('#detail');
  const r = state.selected;
  if (!r) { el.classList.add('hidden'); return; }
  el.classList.remove('hidden');
  const rows = [
    ['Status', badge(r.status)],
    ['Why', esc(r.reason)],
    ['Size (queued)', `${fmtBytes(r.size)} <span class="muted">(${fmtNum(r.size)} bytes)</span>`],
    ['Size now', r.curSize != null ? fmtBytes(r.curSize) : '<span class="muted">not checked</span>'],
    ['Modified (queued)', esc(fmtTime(r.queuedMtime))],
    ['Modified now', r.curMtime ? esc(fmtTime(r.curMtime)) : '<span class="muted">—</span>'],
    ['Exists', r.exists == null ? '<span class="muted">not checked</span>' : r.exists ? 'yes' : 'no'],
    ['Open check', esc(r.openError || (r.exists ? 'opened OK' : '—'))],
    ['Skipped report', r.skipCode ? `${esc(r.skipCode)} at ${esc(r.skipTime)}` : '—'],
    ['Last log line', r.log ? `${esc(r.logTime)}<br>${esc(r.log)}` : '—'],
    ['Uploaded at', r.uploadedAt ? esc(fmtTime(r.uploadedAt)) : '—'],
    ['Chunks sent', r.chunksSent ? `${r.chunksSent} (${fmtBytes(r.chunkBytesSent)})` : '—'],
  ];
  el.innerHTML = `<strong>${esc(r.name)}</strong>
    <div class="path">${esc(r.path)}</div>
    <div class="btns"><button id="btnReveal">Show in folder</button><button id="btnCopy">Copy path</button><button id="btnFolder">Filter to this folder</button><button id="btnClose">Close</button></div>
    <dl>${rows.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${v}</dd>`).join('')}</dl>`;
  $('#btnReveal').onclick = () => invoke && invoke('reveal', { path: r.path }).catch((e) => setStatus('Error: ' + e, true));
  $('#btnCopy').onclick = () => navigator.clipboard.writeText(r.path).then(() => setStatus('Path copied'));
  $('#btnFolder').onclick = () => { state.folder = r.parent + (r.path.includes('\\') ? '\\' : '/'); applyFilters(); };
  $('#btnClose').onclick = () => { state.selected = null; renderDetail(); renderTable(false); };
}

// ---------- CSV export ----------
async function exportCsv() {
  const cols = [
    ['status', (r) => (STATUS[r.status] || { label: r.status }).label], ['reason', (r) => r.reason],
    ['size_bytes', (r) => r.size], ['path', (r) => r.path], ['ext', (r) => r.ext], ['drive', (r) => r.drive],
    ['modified_queued', (r) => r.queuedMtime ? new Date(r.queuedMtime).toISOString() : ''],
    ['modified_now', (r) => r.curMtime ? new Date(r.curMtime).toISOString() : ''],
    ['exists', (r) => r.exists == null ? '' : r.exists], ['open_check', (r) => r.openError || ''],
    ['skip_code', (r) => r.skipCode || ''], ['skip_time', (r) => r.skipTime || ''],
    ['log_time', (r) => r.logTime || ''], ['log', (r) => r.log || ''],
    ['uploaded_at', (r) => r.uploadedAt ? new Date(r.uploadedAt).toISOString() : ''], ['chunks_sent', (r) => r.chunksSent],
  ];
  const q = (v) => { const s = String(v ?? ''); return /[",\r\n]/.test(s) ? '"' + s.replace(/"/g, '""') + '"' : s; };
  const csv = [cols.map((c) => c[0]).join(',')].concat(state.view.map((r) => cols.map((c) => q(c[1](r))).join(','))).join('\r\n');
  const name = `backblaze_pending_${new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')}.csv`;
  try {
    if (!invoke) throw new Error('export works only inside the app');
    const p = await invoke('save_text', { name, contents: csv });
    setStatus(`Exported ${fmtNum(state.view.length)} rows to ${p}`);
  } catch (e) { setStatus('Export failed: ' + e, true); }
}

// ---------- wiring ----------
function setAuto() {
  clearInterval(state.timer);
  const mins = parseInt($('#selAuto').value, 10);
  if (mins > 0) state.timer = setInterval(refresh, mins * 60000);
}

function init() {
  $('#btnRefresh').addEventListener('click', refresh);
  $('#btnExport').addEventListener('click', exportCsv);
  $('#selAuto').addEventListener('change', setAuto);
  let deb;
  $('#fSearch').addEventListener('input', () => { clearTimeout(deb); deb = setTimeout(applyFilters, 150); });
  ['#fStatus', '#fDrive', '#fExt', '#fMin', '#fHideUploaded'].forEach((s) => $(s).addEventListener('change', applyFilters));
  $('#fFolderTag').addEventListener('click', () => { state.folder = ''; applyFilters(); });
  $('#btnClear').addEventListener('click', () => {
    $('#fSearch').value = ''; $('#fStatus').value = ''; $('#fDrive').value = ''; $('#fExt').value = ''; $('#fMin').value = '';
    $('#fHideUploaded').checked = true; state.folder = ''; applyFilters();
  });
  $('#thead').addEventListener('click', (e) => {
    const k = e.target.closest('[data-k]') && e.target.closest('[data-k]').dataset.k;
    if (!k) return;
    if (state.sortKey === k) state.sortDir = -state.sortDir;
    else { state.sortKey = k; state.sortDir = (k === 'size' || k === 'queuedMtime') ? -1 : 1; }
    sortView(); renderTable(false);
  });
  let raf = 0;
  $('#tbody').addEventListener('scroll', () => { if (!raf) raf = requestAnimationFrame(() => { raf = 0; renderTable(false); }); });
  window.addEventListener('resize', () => renderTable(false));
  $('#rowsLayer').addEventListener('click', (e) => {
    const row = e.target.closest('.trow'); if (!row) return;
    state.selected = state.view[+row.dataset.i]; renderDetail(); renderTable(false);
  });
  $('#rowsLayer').addEventListener('dblclick', (e) => {
    const row = e.target.closest('.trow'); if (!row || !invoke) return;
    invoke('reveal', { path: state.view[+row.dataset.i].path }).catch((err) => setStatus('Error: ' + err, true));
  });
  $('#tbody').addEventListener('keydown', (e) => {
    if (!state.selected || (e.key !== 'ArrowDown' && e.key !== 'ArrowUp')) return;
    e.preventDefault();
    const i = state.view.indexOf(state.selected) + (e.key === 'ArrowDown' ? 1 : -1);
    if (i < 0 || i >= state.view.length) return;
    state.selected = state.view[i];
    const body = $('#tbody');
    const top = HEAD_H + i * ROW_H;
    if (top < body.scrollTop + HEAD_H) body.scrollTop = top - HEAD_H;
    else if (top + ROW_H > body.scrollTop + body.clientHeight) body.scrollTop = top + ROW_H - body.clientHeight;
    renderDetail(); renderTable(false);
  });
  $('#tabs').addEventListener('click', (e) => {
    const b = e.target.closest('button[data-tab]'); if (!b) return;
    $$('#tabs button').forEach((x) => x.classList.toggle('active', x === b));
    $$('.tab').forEach((t) => t.classList.toggle('hidden', t.id !== 'tab-' + b.dataset.tab));
  });
  $('#tab-groups .seg').addEventListener('click', (e) => {
    const b = e.target.closest('button[data-g]'); if (!b) return;
    state.group = b.dataset.g;
    $$('#tab-groups .seg button').forEach((x) => x.classList.toggle('active', x === b));
    renderGroups();
  });
  $('#groupTable').addEventListener('click', (e) => {
    const tr = e.target.closest('tr[data-k]'); if (!tr) return;
    const k = tr.dataset.k === '(none)' ? '' : tr.dataset.k;
    if (state.group === 'status') $('#fStatus').value = k;
    else if (state.group === 'drive') $('#fDrive').value = k;
    else if (state.group === 'ext') $('#fExt').value = k;
    else if (state.group === 'folder') state.folder = k;
    applyFilters();
  });
  $('#tab-issues').addEventListener('click', (e) => {
    const tr = e.target.closest('tr[data-path]'); if (!tr) return;
    $('#fSearch').value = tr.dataset.path; $('#fHideUploaded').checked = false; applyFilters();
  });
  setAuto();
  refresh();
}

document.addEventListener('DOMContentLoaded', init);
