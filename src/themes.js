// 'auto' は CSS 既定（OSのライト/ダークに追従）。それ以外は変数を上書きする。
const THEMES = [
  { id: 'auto', name: '自動（OS追従）' },
  { id: 'light', name: 'ライト', v: { bg: '#fbfbfc', fg: '#1d1d1f', sub: '#6e6e73', line: '#e2e2e6', sel: '#e6efff', mark: '#ffe58a', accent: '#2f6df6', 'accent-fg': '#ffffff' } },
  { id: 'dark', name: 'ダーク', v: { bg: '#232326', fg: '#ececee', sub: '#b4b4bd', line: '#38383d', sel: '#2f3d5c', mark: '#6b5a12', accent: '#6c9bff', 'accent-fg': '#10131a' } },
  { id: 'nord', name: 'Nord', v: { bg: '#2e3440', fg: '#eceff4', sub: '#aab5cc', line: '#3b4252', sel: '#434c5e', mark: '#5e5a3a', accent: '#88c0d0', 'accent-fg': '#2e3440' } },
  { id: 'solarized-light', name: 'Solarized Light', v: { bg: '#fdf6e3', fg: '#586e75', sub: '#93a1a1', line: '#eee8d5', sel: '#eee8d5', mark: '#f2dc8a', accent: '#268bd2', 'accent-fg': '#ffffff' } },
  { id: 'dracula', name: 'Dracula', v: { bg: '#282a36', fg: '#f8f8f2', sub: '#aeb2d0', line: '#44475a', sel: '#44475a', mark: '#6b5a2a', accent: '#bd93f9', 'accent-fg': '#282a36' } },
  { id: 'mint', name: 'ミント', v: { bg: '#f4fbf7', fg: '#1f3a2e', sub: '#6f9382', line: '#d5ebe0', sel: '#d9f3e6', mark: '#c8efa8', accent: '#2fa36b', 'accent-fg': '#ffffff' } },
  { id: 'sky', name: 'スカイ', v: { bg: '#f3f9fe', fg: '#1c3447', sub: '#6f90a8', line: '#d6e7f4', sel: '#dcecfa', mark: '#ffe9a8', accent: '#1e88e5', 'accent-fg': '#ffffff' } },
  { id: 'lavender', name: 'ラベンダー', v: { bg: '#f8f6fe', fg: '#33294d', sub: '#8b80ab', line: '#e4def4', sel: '#e9e2fb', mark: '#f6dcff', accent: '#7c5cd6', 'accent-fg': '#ffffff' } },
  { id: 'paper', name: 'ペーパー', v: { bg: '#faf5ea', fg: '#3b3226', sub: '#9a8c78', line: '#e9dfc9', sel: '#f0e4c8', mark: '#ffdf8a', accent: '#b5651d', 'accent-fg': '#ffffff' } },
  { id: 'sakura', name: 'さくら', v: { bg: '#fff7f9', fg: '#4a2f3a', sub: '#a07a8a', line: '#f3dbe3', sel: '#ffe3ec', mark: '#ffd0dc', accent: '#e5588a', 'accent-fg': '#ffffff' } },
];

function applyTheme(id) {
  const s = document.documentElement.style;
  for (const k of ['bg', 'fg', 'sub', 'line', 'sel', 'mark', 'accent', 'accent-fg']) s.removeProperty('--' + k);
  const t = THEMES.find((x) => x.id === id);
  if (t && t.v) for (const k in t.v) s.setProperty('--' + k, t.v[k]);
}
