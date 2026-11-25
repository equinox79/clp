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
    $list.innerHTML = `<div class="empty">${mode === 'snippets' ? t('empty.snippets', { key: first('edit_snippets') }) : t('empty.history')}</div>`;
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

/** プレビュー用: 見えない文字（改行・タブ・行末の空白・全角空白・制御文字など）を記号で可視化する */
function visible(text) {
  const c = (s) => `<span class="ctl">${s}</span>`;
  return text.split(/(?<=\n)/).map((line) => {
    const m = /^([\s\S]*?)(\r?\n?)$/.exec(line);
    let body = m[1], eol = m[2];
    const tw = /[ ]+$/.exec(body);
    const trail = tw ? tw[0].length : 0;
    if (trail) body = body.slice(0, -trail);
    let out = '';
    for (const ch of body) {
      const n = ch.codePointAt(0);
      if (ch === '\t') out += c('→') + '\t';
      else if (n === 0x3000) out += c('□');
      else if (n === 0xa0) out += c('⍽');
      else if (n >= 0x200b && n <= 0x200d || n === 0xfeff || n === 0x2060) out += c('·');
      else if (n < 0x20 || n === 0x7f) out += c(String.fromCodePoint(n === 0x7f ? 0x2421 : 0x2400 + n));
      else out += esc(ch);
    }
    if (trail) out += c('·'.repeat(Math.min(trail, 20)));
    if (eol.includes('\r')) out += c('␍');
    if (eol.includes('\n')) out += c('↵') + '\n';
    return out;
  }).join('');
}

async function updatePreview() {
  const r = rows[sel];
  if (!r) return;
  const my = ++seq;
  const t = await invoke('full_text', { id: r.id, mode });
  if (my === seq) $prev.innerHTML = visible(t);
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

function choose(i, xform) {
  const r = rows[i];
  if (r) invoke('choose', { id: r.id, mode, xform: xform || null });
}

/* ---------- キーバインド（設定画面で変更可。保存形式は "Ctrl+J" など） ---------- */
let KEYS = DEFAULT_KEYS;
let bindMap = {};
const first = (a) => fmtHotkey((KEYS[a] || [])[0] || '');

function applyKeys(k) {
  KEYS = { ...DEFAULT_KEYS, ...k };
  bindMap = {};
  for (const [act] of KEY_ACTIONS) for (const c of KEYS[act] || []) bindMap[c] = act;
  document.getElementById('hint').textContent = t('hint', {
    choose: first('choose'), tab: first('switch_tab'), quick: combo([KEYS.quick_mod], t('hint.digit')),
    pin: first('pin'), delete: first('delete'), trim: first('paste_trim'), close: first('close'),
  });
  if (!rows.length) render();
}

function modsOf(e) {
  return [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Super'].filter(Boolean);
}

const ACT = {
  next: () => select(sel + 1),
  prev: () => select(sel - 1),
  choose: () => choose(sel),
  paste_trim: () => choose(sel, 'trim'),
  paste_oneline: () => choose(sel, 'oneline'),
  paste_upper: () => choose(sel, 'upper'),
  paste_lower: () => choose(sel, 'lower'),
  switch_tab: () => setMode(mode === 'history' ? 'snippets' : 'history'),
  close: () => { if ($q.value) { $q.value = ''; refresh(); } else invoke('hide_window'); },
  pin: () => {
    if (mode !== 'history' || !rows[sel]) return false;
    invoke('toggle_pin', { id: rows[sel].id }).then(async () => {
      cache.history = await invoke('search', { query: '', mode: 'history' });
      const keep = rows[sel] && rows[sel].id;
      await refresh();
      const ni = rows.findIndex((r) => r.id === keep);
      if (ni >= 0) select(ni);
    });
  },
  delete: () => {
    if (mode !== 'history' || !rows[sel]) return false;
    const at = sel;
    invoke('delete_item', { id: rows[sel].id }).then(async () => {
      cache.history = await invoke('search', { query: '', mode: 'history' });
      await refresh();
      select(at);
    });
  },
  edit_snippets: () => { if (mode !== 'snippets') return false; invoke('open_snippets'); },
};

I18N_READY.then(() => invoke('get_keys').then(applyKeys));
listen('keys', (e) => applyKeys(e.payload));
listen('lang', () => location.reload());
const setFont = (px) => document.documentElement.style.setProperty('--fs', px + 'px');
invoke('get_settings').then((s) => { applyTheme(s.theme); setFont(s.font_size); setEffects(s); });
listen('theme', (e) => applyTheme(e.payload));
listen('ui', (e) => setFont(e.payload));
const setEffects = (fx) => {
  const c = document.documentElement.classList;
  c.toggle('win', !IS_MAC);
  c.toggle('shadow', !!fx.shadow);
  c.toggle('glass', !!fx.glass);
  const tr = Number.isFinite(fx.glass_transparency) ? fx.glass_transparency : 25;
  document.documentElement.style.setProperty('--glass-op', (100 - tr) + '%');
};
listen('effects', (e) => setEffects(e.payload));

listen('shown', (e) => {
  cache = e.payload;
  document.getElementById('paused').hidden = !e.payload.paused;
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
  // macOS の Option+数字は別の文字になるため、e.key ではなく物理キー(e.code)で判定する
  const c = eventCombo(e);
  if (!c) return;
  const mods = modsOf(e);
  const dm = /^Digit([0-9])$/.exec(e.code);

  // 上から1〜10番目を即貼り付け（修飾キー + 数字）
  if (dm && mods.length === 1 && mods[0] === KEYS.quick_mod) {
    e.preventDefault(); choose((Number(dm[1]) + 9) % 10); return;
  }
  // 定型文タブ: 検索欄が空なら数字キー単体で即貼り
  if (dm && !mods.length && KEYS.bare_digits && mode === 'snippets' && !$q.value) {
    e.preventDefault(); choose((Number(dm[1]) + 9) % 10); return;
  }
  const act = bindMap[c];
  if (!act) return;
  // Delete / Backspace 単体は、検索欄に文字があるときは文字の編集に使う
  if ((c === 'Delete' || c === 'Backspace') && $q.value) return;
  if (ACT[act]() === false) return;
  e.preventDefault();
});
