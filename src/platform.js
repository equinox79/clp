// OS ごとのショートカット表記。内部の保存形式は常に "Ctrl+Alt+V"（Ctrl / Alt / Shift / Super）のまま、表示だけ変える。
const IS_MAC = /Mac/i.test(navigator.platform) || /Mac OS X/.test(navigator.userAgent);
const MOD_LABEL = IS_MAC
  ? { Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Super: '⌘' }
  : { Ctrl: 'Ctrl', Alt: 'Alt', Shift: 'Shift', Super: 'Win' };
const KEY_LABEL = {
  ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→', Up: '↑', Down: '↓', Left: '←', Right: '→',
  Space: IS_MAC ? 'Space' : 'Space', Backquote: '`', Minus: '-', Equal: '=', Comma: ',', Period: '.', Slash: '/',
  Semicolon: ';', Quote: "'", BracketLeft: '[', BracketRight: ']', Backslash: '\\',
  PageUp: 'PgUp', PageDown: 'PgDn', Delete: IS_MAC ? '⌦' : 'Delete', Enter: IS_MAC ? '↩' : 'Enter',
  Escape: 'Esc', Backspace: IS_MAC ? '⌫' : 'Backspace', Tab: IS_MAC ? '⇥' : 'Tab',
};

/** 窓内キーバインドの既定値（Rust 側の Keys::default と同じ内容） */
const DEFAULT_KEYS = {
  next: ['ArrowDown', 'Ctrl+J'],
  prev: ['ArrowUp', 'Ctrl+K'],
  choose: ['Enter'],
  switch_tab: ['Tab'],
  pin: ['Ctrl+P'],
  delete: ['Ctrl+D', 'Delete'],
  edit_snippets: ['Ctrl+E'],
  close: ['Escape'],
  paste_trim: ['Shift+Enter'],
  paste_oneline: ['Ctrl+Enter'],
  paste_upper: ['Ctrl+Shift+U'],
  paste_lower: ['Ctrl+Shift+L'],
  quick_mod: 'Alt',
  bare_digits: true,
};

/** 割り当て可能な操作の一覧（キー名, 表示名） */
const KEY_ACTIONS = [
  ['next', '次の項目へ'],
  ['prev', '前の項目へ'],
  ['choose', '貼り付けて閉じる'],
  ['paste_trim', '前後の空白を除いて貼り付け'],
  ['paste_oneline', '改行を除いて1行で貼り付け'],
  ['paste_upper', '大文字にして貼り付け'],
  ['paste_lower', '小文字にして貼り付け'],
  ['switch_tab', '履歴 ⇔ 定型文 の切り替え'],
  ['pin', 'ピン留め切替（履歴）'],
  ['delete', '履歴を削除'],
  ['edit_snippets', 'snippets.toml を開く（定型文）'],
  ['close', '入力クリア / 閉じる'],
];

/** キーイベント → 物理キー名（"KeyA"→"A", "Digit1"→"1"。対応外のキーは null） */
function eventKeyName(e) {
  const c = e.code;
  if (/^Key[A-Z]$/.test(c)) return c.slice(3);
  if (/^Digit[0-9]$/.test(c)) return c.slice(5);
  if (/^(F([1-9]|1[0-2])|Space|Enter|Tab|Escape|Backspace|Delete|Backquote|Minus|Equal|Comma|Period|Slash|Semicolon|Quote|BracketLeft|BracketRight|Backslash|Insert|Home|End|PageUp|PageDown|ArrowUp|ArrowDown|ArrowLeft|ArrowRight)$/.test(c)) return c;
  return null;
}

/** キーイベント → 保存形式 "Ctrl+J"（修飾キーだけ・対応外のキーは null） */
function eventCombo(e) {
  const k = eventKeyName(e);
  if (!k) return null;
  const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Super'].filter(Boolean);
  return [...mods, k].join('+');
}

/** 修飾キーなしで割り当てられるキー（文字入力と衝突しないもの） */
const PLAIN_OK = /^(Enter|Tab|Escape|Delete|PageUp|PageDown|ArrowUp|ArrowDown|F([1-9]|1[0-2]))$/;

/** 窓内キーバインドとして使える組み合わせか。使えなければ理由を返す */
function bindingProblem(c) {
  const p = c.split('+');
  const key = p[p.length - 1];
  const mods = p.slice(0, -1);
  if (!mods.some((m) => m !== 'Shift') && !PLAIN_OK.test(key)) {
    return `文字入力と衝突するため、${modsHelp(true)} のいずれかと組み合わせてください`;
  }
  return '';
}

/** 修飾キー配列 + キー名 → 表示文字列（Win: "Ctrl+Alt+V" / Mac: "⌃⌥V"） */
function combo(mods, key) {
  const k = KEY_LABEL[key] || key;
  const m = mods.map((x) => MOD_LABEL[x] || x);
  return IS_MAC ? m.join('') + k : [...m, k].join('+');
}

/** 保存形式 "Ctrl+Alt+V" → 表示用 */
function fmtHotkey(s) {
  const p = (s || '').split('+');
  return combo(p.slice(0, -1), p[p.length - 1]);
}

/** 修飾キーの総称（メッセージ用） */
function modsHelp(noShift) {
  if (noShift) return IS_MAC ? '⌃ / ⌥ / ⌘' : 'Ctrl / Alt';
  return IS_MAC ? '⌃ / ⌥ / ⇧ / ⌘' : 'Ctrl / Alt / Shift';
}
