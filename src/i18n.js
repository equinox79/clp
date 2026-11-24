// 多言語対応。辞書は Rust 側（src-tauri/src/i18n.rs）が「en → 選択言語 → ユーザーの locales/*.json」の順に重ねて返す。
let DICT = {};
let LANG = 'en';

/** 文言を取り出す。{name} は vars で置換する。無いキーはキー名をそのまま返す */
function t(key, vars) {
  let s = DICT[key];
  if (s == null) s = key;
  if (vars) s = s.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? vars[k] : m));
  return s;
}

/** data-i18n / data-i18n-html / data-i18n-ph / data-i18n-title を持つ要素に文言を入れる */
function applyI18n(root = document) {
  const base = { mods: typeof modsHelp === 'function' ? modsHelp() : '' };
  root.querySelectorAll('[data-i18n]').forEach((el) => { el.textContent = t(el.dataset.i18n, base); });
  root.querySelectorAll('[data-i18n-html]').forEach((el) => { el.innerHTML = t(el.dataset.i18nHtml, base); });
  root.querySelectorAll('[data-i18n-ph]').forEach((el) => { el.placeholder = t(el.dataset.i18nPh); });
  root.querySelectorAll('[data-i18n-title]').forEach((el) => { el.title = t(el.dataset.i18nTitle); });
}

const I18N_READY = window.__TAURI__.core.invoke('get_dict').then((d) => {
  DICT = d.dict; LANG = d.code;
  document.documentElement.lang = LANG;
  applyI18n();
});
