const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $q = document.getElementById('q');
const $list = document.getElementById('list');
const $prev = document.getElementById('preview');
const $th = document.getElementById('t-history');
const $ts = document.getElementById('t-snippets');

let mode = 'history';
let cache = { history: [], snippets: [] };
let rows = [];
let sel = 0;
let seq = 0;

const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const escRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

function hl(text, re) {
  if (!re) return esc(text);
  let out = '', last = 0;
  text.replace(re, (m, i) => { out += esc(text.slice(last, i)) + '<mark>' + esc(m) + '</mark>'; last = i + m.length; return m; });
  return out + esc(text.slice(last));
}

function fmtTime(ms) {
  if (!ms) return '';
  const d = new Date(ms), n = new Date();
  const p = (x) => String(x).padStart(2, '0');
  return d.toDateString() === n.toDateString() ? `${p(d.getHours())}:${p(d.getMinutes())}` : `${d.getMonth() + 1}/${d.getDate()}`;
}

function render() {
  const terms = $q.value.trim().split(/\s+/).filter(Boolean);
  const re = terms.length ? new RegExp(terms.map(escRe).join('|'), 'gi') : null;
  if (!rows.length) {
    $list.innerHTML = `<div class="empty">${mode === 'snippets' ? '定型文がありません（Ctrl+E で編集）' : '履歴がありません'}</div>`;
    $prev.textContent = '';
    return;
  }
  $list.innerHTML = rows.map((r, i) => {
    const no = mode === 'snippets' && i < 10 ? (i + 1) % 10 : '';
    return `<div class="row${i === sel ? ' sel' : ''}" data-i="${i}">` +
      `<span class="no">${no}</span><span class="pin">${r.pinned ? '📌' : ''}</span>` +
      (r.label ? `<span class="label">${hl(r.label, re)}</span>` : '') +
      `<span class="txt">${hl(r.preview, re)}</span><span class="ts">${fmtTime(r.ts)}</span></div>`;
  }).join('');
  updatePreview();
}

function select(i) {
  if (!rows.length) return;
  sel = Math.max(0, Math.min(rows.length - 1, i));
  const prev = $list.querySelector('.sel');
  if (prev) prev.classList.remove('sel');
  const el = $list.children[sel];
  el.classList.add('sel');
  el.scrollIntoView({ block: 'nearest' });
  updatePreview();
}

async function updatePreview() {
  const r = rows[sel];
  if (!r) return;
  const my = ++seq;
  const t = await invoke('full_text', { id: r.id, mode });
  if (my === seq) $prev.textContent = t;
}

async function refresh() {
  const query = $q.value.trim();
  sel = 0;
  if (!query) { rows = cache[mode]; render(); return; }
  const my = ++seq;
  const res = await invoke('search', { query, mode });
  if (my !== seq) return;
  rows = res;
  render();
}

function setMode(m) {
  mode = m;
  $th.classList.toggle('on', m === 'history');
  $ts.classList.toggle('on', m === 'snippets');
  refresh();
}

function choose(i) {
  const r = rows[i];
  if (r) invoke('choose', { id: r.id, mode });
}

invoke('get_settings').then((s) => applyTheme(s.theme));
listen('theme', (e) => applyTheme(e.payload));

listen('shown', (e) => {
  cache = e.payload;
  $q.value = '';
  setMode('history');
  $q.focus();
});

$q.addEventListener('input', refresh);

$list.addEventListener('click', (e) => {
  const row = e.target.closest('.row');
  if (row) choose(Number(row.dataset.i));
});

document.addEventListener('keydown', (e) => {
  // IME変換中のキーはすべて無視（Enterでの誤貼付防止）
  if (e.isComposing || e.keyCode === 229) return;
  const k = e.key;

  // macOS の Option+数字は別の文字になるため、e.key ではなく物理キー(e.code)で判定する
  const dm = /^Digit([0-9])$/.exec(e.code);
  if (e.altKey && dm) { e.preventDefault(); choose((Number(dm[1]) + 9) % 10); return; }
  if (k === 'Enter') { e.preventDefault(); choose(sel); return; }
  if (k === 'ArrowDown' || (e.ctrlKey && k === 'j')) { e.preventDefault(); select(sel + 1); return; }
  if (k === 'ArrowUp' || (e.ctrlKey && k === 'k')) { e.preventDefault(); select(sel - 1); return; }
  if (k === 'Tab') { e.preventDefault(); setMode(mode === 'history' ? 'snippets' : 'history'); return; }
  if (k === 'Escape') {
    e.preventDefault();
    if ($q.value) { $q.value = ''; refresh(); } else invoke('hide_window');
    return;
  }
  if (e.ctrlKey && k === 'p' && mode === 'history' && rows[sel]) {
    e.preventDefault();
    invoke('toggle_pin', { id: rows[sel].id }).then(async () => {
      cache.history = await invoke('search', { query: '', mode: 'history' });
      const keep = rows[sel] && rows[sel].id;
      await refresh();
      const ni = rows.findIndex((r) => r.id === keep);
      if (ni >= 0) select(ni);
    });
    return;
  }
  if (((e.ctrlKey && k === 'd') || (k === 'Delete' && !$q.value)) && mode === 'history' && rows[sel]) {
    e.preventDefault();
    const at = sel;
    invoke('delete_item', { id: rows[sel].id }).then(async () => {
      cache.history = await invoke('search', { query: '', mode: 'history' });
      await refresh();
      select(at);
    });
    return;
  }
  if (e.ctrlKey && k === 'e' && mode === 'snippets') { e.preventDefault(); invoke('open_snippets'); return; }
  // 定型文タブ: 検索欄が空なら数字キー単体で即貼り
  if (mode === 'snippets' && !$q.value && /^[0-9]$/.test(k) && !e.ctrlKey && !e.altKey) {
    e.preventDefault();
    choose((Number(k) + 9) % 10);
  }
});
