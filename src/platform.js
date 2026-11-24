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
};

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
function modsHelp() {
  return IS_MAC ? '⌃ / ⌥ / ⇧ / ⌘' : 'Ctrl / Alt / Shift';
}
