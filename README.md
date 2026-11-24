# clp

テキスト専用の高速クリップボード履歴ツール（Windows / macOS）。
ホットキーで即呼び出し、日本語の部分一致検索、選んだ項目をそのまま前のアプリへ貼り付けます。定型文（最大30個）にも対応。

- Tauri v2 + Rust + 素の HTML/JS（常駐メモリ・起動とも軽量）
- 履歴は SQLite に保存し、検索は全件をメモリ上で行う（数千件でも1ms未満）
- **Windows**: 動作確認済み
- **macOS**: 実装済みだが **Mac 実機では未検証**（下記「macOS について」参照）

## 使い方

1. `clp.exe` を起動すると、タスクトレイに常駐します。
2. **`Ctrl+Alt+V`**（変更可）で窓を開きます。
3. 項目を選んで **`Enter`** を押すと、直前に使っていたアプリへ貼り付けて閉じます。

トレイアイコンは左クリックで窓を開き、右クリックで「表示 / snippets.toml を開く / 設定… / 終了」のメニューを出します。

### キー操作

| キー | 動作 |
|---|---|
| `↑` `↓` / `Ctrl+J` `Ctrl+K` | 選択移動 |
| `Enter` | 貼り付けて閉じる（IME変換中は無効） |
| `Tab` | 履歴 ⇔ 定型文 の切り替え |
| `Alt+数字` | 上から1〜10番目を即貼り付け（`0`が10番目） |
| 数字キー単体 | 定型文タブで検索欄が空のとき、即貼り付け |
| `Ctrl+P` | ピン留めの切り替え（履歴） |
| `Ctrl+D` / `Delete` | 履歴を削除（`Delete`は検索欄が空のとき） |
| `Ctrl+E` | `snippets.toml` を開く（定型文タブ） |
| `Esc` | 検索欄をクリア → もう一度で閉じる |

検索はスペース区切りで AND 検索です。全角/半角、大文字/小文字、ひらがな/カタカナの違いは区別しません。

## 定型文

設定画面の「定型文」ページで、追加・編集・並べ替え・複製・削除ができます（自動保存、最大30個）。
`%APPDATA%\clp\snippets.toml` を直接編集することもできます。

```toml
[[snippet]]
title = "挨拶"
body = "お世話になっております。"

[[snippet]]
title = "署名"
body = """
--
氏名
"""
```

本文で使える変数:

| 変数 | 展開結果 |
|---|---|
| `{date}` | `2026-11-21` |
| `{time}` | `14:05` |
| `{datetime}` | `2026-11-21 14:05` |
| `{clipboard}` | 貼り付け時点のクリップボードの内容 |

## 設定

トレイの「設定…」から変更できます。

- **一般**: ホットキー、表示位置（画面中央 / マウス位置 / 入力カーソル付近）、Windows 起動時の自動常駐
- **外観**: テーマ（自動・ライト7種・ダーク3種）
- **定型文**: エディタ
- **履歴**: 最大件数（既定3000）、1件の最大サイズ（既定1024KB）、履歴の全削除（ピン留めは残る）

入力カーソル付近は、カーソル位置を取得できないアプリ（Chrome、VS Code など）ではマウス位置に表示されます。

## データの場所

`%APPDATA%\clp\`

| ファイル | 内容 |
|---|---|
| `history.db` | 履歴（SQLite） |
| `snippets.toml` | 定型文 |
| `config.toml` | 設定 |

## プライバシー

- パスワードマネージャーなどが付ける「履歴に残さない」フラグ付きのコピーは保存しません。
- 履歴は **平文** で `history.db` に保存されます。機密情報をコピーした場合は、設定画面から削除してください。
- 通信は一切行いません。

## ビルド

必要なもの: Rust（MSVC ツールチェーン）、Node.js、Visual Studio Build Tools（C++）、WebView2。

```powershell
npm install
npx tauri build --no-bundle
```

出力: `src-tauri\target\release\clp.exe`

開発時は `npx tauri dev`。

### 構成

```
src/            フロントエンド（index.html / settings.html / main.js / themes.js / style.css）
src-tauri/src/
  lib.rs        コマンド、トレイ、ホットキー、クリップボード監視
  store.rs      履歴ストア（SQLite + メモリ検索）
  snippets.rs   定型文の読み書き・変数展開
  platform_win.rs   Windows 依存部分（前面ウィンドウ復帰、キー送信、キャレット位置、自動起動）
  platform_mac.rs   macOS 依存部分（NSPasteboard、NSWorkspace、CGEvent、LaunchAgent）
```

OS 依存の処理は `platform_*.rs` に集約しており、同名の関数群を OS ごとに実装しています。

## macOS について

Mac 版のコード（`src-tauri/src/platform_mac.rs`）は書いてありますが、開発を Windows で行ったため **Mac でのビルド・動作は未検証** です。初回ビルドでエラーが出たら、`objc2` / `core-graphics` クレートのバージョン差による API 違いが原因の可能性が高いです。

Mac でビルドする手順:

```sh
xcode-select --install          # Xcode コマンドラインツール
npm install
npx tauri build                 # src-tauri/target/release/bundle/ に .app と .dmg
```

- **アクセシビリティ権限**: 自動貼り付け（Cmd+V の送信）に必要です。初回起動時にシステム設定が開くので、「プライバシーとセキュリティ → アクセシビリティ」で clp を許可してください。再ビルドすると許可が外れることがあります。
- **Dock には出ません**。メニューバー（トレイ）に常駐します。
- **ホットキー**: 設定画面では `Cmd` キーが `Super` と表示されます。既定は `Ctrl+Alt+V`（Control+Option+V）です。
- **即貼り**: `Alt+数字` は `Option+数字` です。
- **入力カーソル付近への表示は未対応**です（選んだ場合はマウス位置に表示）。
- **自動起動**は `~/Library/LaunchAgents/local.clp.app.plist` で行います。
- データの場所は `~/Library/Application Support/clp/` です。
- 配布する場合は Apple Developer への登録（年 $99）、署名、公証が必要です。自分用なら不要で、初回は右クリック →「開く」で起動します。
- 透過ウィンドウのために macOS のプライベート API（`macOSPrivateApi`）を使っているため、Mac App Store には出せません。

## ライセンス

[MIT License](LICENSE)

## 注意事項

- コード署名をしていないため、他の PC へブラウザ等で配布すると SmartScreen の警告が出ます。「詳細情報」→「実行」で起動するか、ファイルのプロパティで「ブロックの解除」をしてください。
- 管理者権限で動いているアプリへは、通常権限の clp からは貼り付けできません。
- ホットキー・キー送信・クリップボード監視を使うため、ウイルス対策ソフトが誤検知する場合があります。
