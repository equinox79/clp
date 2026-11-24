#[cfg(windows)]
#[path = "platform_win.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "platform_mac.rs"]
mod platform;
mod snippets;
mod store;

use serde::{Deserialize, Serialize};
use snippets::{expand, Snippets};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use store::{fnv, Row, Store};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

struct AppState {
    dir: PathBuf,
    hotkey: Mutex<String>,
    theme: Mutex<String>,
    position: Mutex<String>,
    max_items: AtomicUsize,
    max_kb: AtomicUsize,
    check_updates: AtomicBool,
    close_on_blur: AtomicBool,
    font_size: AtomicUsize,
    shadow: AtomicBool,
    glass: AtomicBool,
    keys: Mutex<Keys>,
    update_available: Mutex<Option<String>>,
    store: Mutex<Store>,
    snips: Mutex<Snippets>,
    prev: Mutex<isize>,
    self_hash: AtomicU64,
}

/// 窓内のキーバインド。各要素は "Ctrl+J" "ArrowDown" "Escape" のような保存形式
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Keys {
    next: Vec<String>,
    prev: Vec<String>,
    choose: Vec<String>,
    switch_tab: Vec<String>,
    pin: Vec<String>,
    delete: Vec<String>,
    edit_snippets: Vec<String>,
    close: Vec<String>,
    /// 「上から1〜10番目を即貼り付け」の修飾キー（Ctrl / Alt / Shift / Super）
    quick_mod: String,
    /// 定型文タブで検索欄が空のとき、数字キー単体で即貼り付けする
    bare_digits: bool,
}

fn v(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

impl Default for Keys {
    fn default() -> Self {
        Keys {
            next: v(&["ArrowDown", "Ctrl+J"]),
            prev: v(&["ArrowUp", "Ctrl+K"]),
            choose: v(&["Enter"]),
            switch_tab: v(&["Tab"]),
            pin: v(&["Ctrl+P"]),
            delete: v(&["Ctrl+D", "Delete"]),
            edit_snippets: v(&["Ctrl+E"]),
            close: v(&["Escape"]),
            quick_mod: "Alt".into(),
            bare_digits: true,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Config {
    hotkey: String,
    max_items: usize,
    theme: String,
    position: String,
    max_kb: usize,
    check_updates: bool,
    close_on_blur: bool,
    font_size: usize,
    /// 窓の影
    shadow: bool,
    /// 窓の背景をすりガラス風にする（macOS: vibrancy / Windows: Acrylic）
    glass: bool,
    keys: Keys,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: "Ctrl+Alt+V".into(),
            max_items: 3000,
            theme: "auto".into(),
            position: "center".into(),
            max_kb: 1024,
            check_updates: true,
            close_on_blur: true,
            font_size: 14,
            shadow: default_shadow(),
            glass: false,
            keys: Keys::default(),
        }
    }
}

#[derive(Serialize, Clone)]
struct Shown {
    history: Vec<Row>,
    snippets: Vec<Row>,
}

fn get_clipboard() -> Option<String> {
    for _ in 0..4 {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            match cb.get_text() {
                Ok(t) => return Some(t),
                Err(arboard::Error::ContentNotAvailable) => return None,
                Err(_) => {}
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    None
}

fn set_clipboard(text: &str) -> bool {
    for _ in 0..6 {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if cb.set_text(text.to_string()).is_ok() {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

/// 暗い配色のテーマか（src/themes.js と合わせる）。"auto" は OS に従うので None
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn theme_is_dark(theme: &str) -> Option<bool> {
    match theme {
        "auto" => None,
        "dark" | "nord" | "dracula" => Some(true),
        _ => Some(false),
    }
}

/// 窓の外観（ライト/ダーク）をテーマに合わせる。すりガラスの素材の明暗がこれに従う（macOS）
fn apply_window_theme(w: &tauri::WebviewWindow, theme: &str) {
    #[cfg(target_os = "macos")]
    {
        let t = theme_is_dark(theme).map(|d| if d { tauri::Theme::Dark } else { tauri::Theme::Light });
        let _ = w.set_theme(t);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (w, theme);
}

/// 窓の影・すりガラス効果を反映する
fn apply_effects(w: &tauri::WebviewWindow, shadow: bool, glass: bool) {
    use tauri::window::{Effect, EffectsBuilder};
    let _ = w.set_shadow(shadow);
    if glass {
        // macOS: メニューと同じ素材。角丸は CSS の #app (12px) に合わせる
        #[cfg(target_os = "macos")]
        let fx = EffectsBuilder::new().effect(Effect::Menu).state(tauri::window::EffectState::Active).radius(12.0);
        #[cfg(not(target_os = "macos"))]
        let fx = EffectsBuilder::new().effect(Effect::Acrylic);
        let _ = w.set_effects(fx.build());
    } else {
        let _ = w.set_effects(None);
    }
    platform::refresh_shadow(w);
}

fn hide_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

fn place_window(app: &AppHandle, w: &tauri::WebviewWindow) {
    let (Ok(cur), Ok(size)) = (app.cursor_position(), w.outer_size()) else { return };
    let mode = app.state::<AppState>().position.lock().unwrap().clone();
    // 入力カーソル位置（取れないアプリではマウス位置にフォールバック）
    let anchor: Option<(i32, i32)> = match mode.as_str() {
        "caret" => platform::caret_pos().or(Some((cur.x as i32, cur.y as i32))),
        "mouse" => Some((cur.x as i32, cur.y as i32)),
        _ => None,
    };
    let (ax, ay) = anchor.unwrap_or((cur.x as i32, cur.y as i32));
    let Ok(Some(m)) = app.monitor_from_point(ax as f64, ay as f64) else { return };
    let (mx, my) = (m.position().x, m.position().y);
    let (mw, mh) = (m.size().width as i32, m.size().height as i32);
    let (ww, wh) = (size.width as i32, size.height as i32);
    let (x, y) = if anchor.is_some() {
        let off = if mode == "caret" { 6 } else { 12 };
        (ax, ay + off)
    } else {
        (mx + (mw - ww) / 2, my + (mh - wh) / 4)
    };
    let x = x.clamp(mx, (mx + mw - ww).max(mx));
    let y = y.clamp(my, (my + mh - wh).max(my));
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

fn show_main(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        return;
    }
    let st = app.state::<AppState>();
    *st.prev.lock().unwrap() = platform::foreground();
    let payload = {
        let mut sn = st.snips.lock().unwrap();
        sn.reload_if_changed();
        Shown {
            history: st.store.lock().unwrap().search(""),
            snippets: sn.search(""),
        }
    };
    let _ = app.emit("shown", payload);
    place_window(app, &w);
    let _ = w.show();
    platform::refresh_shadow(&w);
    platform::activate_self();
    let _ = w.set_focus();
}

// ---------- commands ----------

#[tauri::command]
fn search(app: AppHandle, query: String, mode: String) -> Vec<Row> {
    let st = app.state::<AppState>();
    if mode == "snippets" {
        st.snips.lock().unwrap().search(&query)
    } else {
        st.store.lock().unwrap().search(&query)
    }
}

#[tauri::command]
fn full_text(app: AppHandle, id: i64, mode: String) -> String {
    let st = app.state::<AppState>();
    let t = if mode == "snippets" {
        st.snips.lock().unwrap().get(id).map(|s| s.body.clone())
    } else {
        st.store.lock().unwrap().get_text(id)
    };
    t.unwrap_or_default().chars().take(3000).collect()
}

#[tauri::command]
fn choose(app: AppHandle, id: i64, mode: String) -> Result<(), String> {
    let st = app.state::<AppState>();
    let text = if mode == "snippets" {
        let clip = get_clipboard().unwrap_or_default();
        st.snips.lock().unwrap().get(id).map(|s| expand(&s.body, &clip))
    } else {
        let mut s = st.store.lock().unwrap();
        s.touch(id);
        s.get_text(id)
    }
    .ok_or("not found")?;

    st.self_hash.store(fnv(&text), Ordering::SeqCst);
    if !set_clipboard(&text) {
        return Err("clipboard busy".into());
    }
    hide_main(&app);
    let prev = *st.prev.lock().unwrap();
    std::thread::spawn(move || {
        platform::restore_foreground(prev, Duration::from_millis(400));
        std::thread::sleep(Duration::from_millis(15));
        platform::send_paste();
    });
    Ok(())
}

#[tauri::command]
fn toggle_pin(app: AppHandle, id: i64) {
    app.state::<AppState>().store.lock().unwrap().toggle_pin(id);
}

#[tauri::command]
fn delete_item(app: AppHandle, id: i64) {
    app.state::<AppState>().store.lock().unwrap().delete(id);
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    hide_main(&app);
}

#[derive(Serialize)]
struct Settings {
    hotkey: String,
    theme: String,
    position: String,
    autostart: bool,
    max_items: usize,
    max_kb: usize,
    count: usize,
    data_dir: String,
    version: String,
    check_updates: bool,
    close_on_blur: bool,
    font_size: usize,
    shadow: bool,
    glass: bool,
    update_available: Option<String>,
}

/// config.toml を書き出す（設定画面から変更するたびに全体を書き直す。コメントは残らない）
fn write_config(dir: &PathBuf, c: &Config) {
    let body = toml::to_string_pretty(c).unwrap_or_default();
    let _ = std::fs::write(
        dir.join("config.toml"),
        format!("# clp の設定。設定画面から変更できます（直接編集した場合は再起動してください）\n{body}"),
    );
}

fn save_config(st: &AppState) {
    write_config(
        &st.dir,
        &Config {
            hotkey: st.hotkey.lock().unwrap().clone(),
            max_items: st.max_items.load(Ordering::SeqCst),
            theme: st.theme.lock().unwrap().clone(),
            position: st.position.lock().unwrap().clone(),
            max_kb: st.max_kb.load(Ordering::SeqCst),
            check_updates: st.check_updates.load(Ordering::SeqCst),
            close_on_blur: st.close_on_blur.load(Ordering::SeqCst),
            font_size: st.font_size.load(Ordering::SeqCst),
            shadow: st.shadow.load(Ordering::SeqCst),
            glass: st.glass.load(Ordering::SeqCst),
            keys: st.keys.lock().unwrap().clone(),
        },
    );
}

#[tauri::command]
fn get_keys(app: AppHandle) -> Keys {
    app.state::<AppState>().keys.lock().unwrap().clone()
}

#[tauri::command]
fn set_keys(app: AppHandle, keys: Keys) -> Result<(), String> {
    if keys.choose.is_empty() {
        return Err("「貼り付けて閉じる」には、1つ以上のキーが必要です".into());
    }
    if keys.close.is_empty() {
        return Err("「閉じる」には、1つ以上のキーが必要です".into());
    }
    if !matches!(keys.quick_mod.as_str(), "Ctrl" | "Alt" | "Shift" | "Super") {
        return Err("即貼り付けの修飾キーが不正です".into());
    }
    let st = app.state::<AppState>();
    *st.keys.lock().unwrap() = keys.clone();
    save_config(&st);
    let _ = app.emit("keys", keys);
    Ok(())
}

#[tauri::command]
fn set_ui(app: AppHandle, close_on_blur: bool, font_size: usize) {
    let st = app.state::<AppState>();
    let fs = font_size.clamp(11, 24);
    st.close_on_blur.store(close_on_blur, Ordering::SeqCst);
    st.font_size.store(fs, Ordering::SeqCst);
    save_config(&st);
    let _ = app.emit("ui", fs);
}

#[derive(Serialize, Clone)]
struct Effects {
    shadow: bool,
    glass: bool,
}

#[tauri::command]
fn set_effects(app: AppHandle, shadow: bool, glass: bool) {
    let st = app.state::<AppState>();
    st.shadow.store(shadow, Ordering::SeqCst);
    st.glass.store(glass, Ordering::SeqCst);
    save_config(&st);
    if let Some(w) = app.get_webview_window("main") {
        apply_effects(&w, shadow, glass);
    }
    let _ = app.emit("effects", Effects { shadow, glass });
}

#[tauri::command]
fn set_check_updates(app: AppHandle, enabled: bool) {
    let st = app.state::<AppState>();
    st.check_updates.store(enabled, Ordering::SeqCst);
    save_config(&st);
}

#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    notes: String,
}

fn set_tray_tip(app: &AppHandle, ver: Option<&str>) {
    if let Some(t) = app.tray_by_id("main") {
        let tip = match ver {
            Some(v) => format!("clp — 新しい版 v{v} があります"),
            None => "clp".to_string(),
        };
        let _ = t.set_tooltip(Some(tip));
    }
}

/// GitHub Releases の latest.json を見て、新しい版があれば情報を返す
#[tauri::command]
async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let found = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("更新の確認に失敗しました: {e}"))?;
    let st = app.state::<AppState>();
    match found {
        Some(u) => {
            *st.update_available.lock().unwrap() = Some(u.version.clone());
            set_tray_tip(&app, Some(&u.version));
            Ok(Some(UpdateInfo { version: u.version.clone(), notes: u.body.clone().unwrap_or_default() }))
        }
        None => {
            *st.update_available.lock().unwrap() = None;
            set_tray_tip(&app, None);
            Ok(None)
        }
    }
}

/// ダウンロードして署名を検証し、インストールして再起動する
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let upd = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("更新の確認に失敗しました: {e}"))?
        .ok_or("更新はありません")?;
    upd.download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| format!("更新のインストールに失敗しました: {e}"))?;
    app.restart()
}

#[tauri::command]
fn set_limits(app: AppHandle, max_items: usize, max_kb: usize) {
    let st = app.state::<AppState>();
    let max_items = max_items.clamp(50, 100_000);
    let max_kb = max_kb.clamp(1, 20_000);
    st.max_items.store(max_items, Ordering::SeqCst);
    st.max_kb.store(max_kb, Ordering::SeqCst);
    st.store.lock().unwrap().set_max(max_items);
    save_config(&st);
}

#[tauri::command]
fn clear_history(app: AppHandle) -> usize {
    app.state::<AppState>().store.lock().unwrap().clear_unpinned()
}

#[derive(Serialize, Deserialize)]
struct SnipDto {
    title: String,
    body: String,
}

#[tauri::command]
fn get_snippets(app: AppHandle) -> Result<Vec<SnipDto>, String> {
    let sn = app.state::<AppState>().snips.lock().unwrap().dto();
    sn
}

#[tauri::command]
fn save_snippets(app: AppHandle, list: Vec<SnipDto>) -> Result<(), String> {
    let pairs = list.into_iter().map(|s| (s.title, s.body)).collect();
    app.state::<AppState>().snips.lock().unwrap().save(pairs)
}

#[tauri::command]
fn set_position(app: AppHandle, position: String) {
    let st = app.state::<AppState>();
    *st.position.lock().unwrap() = position;
    save_config(&st);
}

#[tauri::command]
fn set_theme(app: AppHandle, theme: String) {
    let st = app.state::<AppState>();
    *st.theme.lock().unwrap() = theme.clone();
    save_config(&st);
    if let Some(w) = app.get_webview_window("main") {
        apply_window_theme(&w, &theme);
    }
    let _ = app.emit("theme", theme);
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Settings {
    Settings {
        hotkey: app.state::<AppState>().hotkey.lock().unwrap().clone(),
        theme: app.state::<AppState>().theme.lock().unwrap().clone(),
        position: app.state::<AppState>().position.lock().unwrap().clone(),
        autostart: platform::autostart_enabled(),
        max_items: app.state::<AppState>().max_items.load(Ordering::SeqCst),
        max_kb: app.state::<AppState>().max_kb.load(Ordering::SeqCst),
        count: app.state::<AppState>().store.lock().unwrap().count(),
        data_dir: app.state::<AppState>().dir.display().to_string(),
        version: app.package_info().version.to_string(),
        check_updates: app.state::<AppState>().check_updates.load(Ordering::SeqCst),
        close_on_blur: app.state::<AppState>().close_on_blur.load(Ordering::SeqCst),
        font_size: app.state::<AppState>().font_size.load(Ordering::SeqCst),
        shadow: app.state::<AppState>().shadow.load(Ordering::SeqCst),
        glass: app.state::<AppState>().glass.load(Ordering::SeqCst),
        update_available: app.state::<AppState>().update_available.lock().unwrap().clone(),
    }
}

#[tauri::command]
fn set_hotkey(app: AppHandle, hotkey: String) -> Result<(), String> {
    let st = app.state::<AppState>();
    let gs = app.global_shortcut();
    let old = st.hotkey.lock().unwrap().clone();
    if old == hotkey {
        return Ok(());
    }
    let _ = gs.unregister(old.as_str());
    if let Err(e) = gs.register(hotkey.as_str()) {
        let _ = gs.register(old.as_str());
        return Err(format!("このキーは使えません（他のアプリが使用中の可能性）: {e}"));
    }
    *st.hotkey.lock().unwrap() = hotkey.clone();
    save_config(&st);
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct BundleCfg {
    hotkey: String,
    theme: String,
    position: String,
    max_items: usize,
    max_kb: usize,
    // 以下は後から追加した項目（古いファイルには無いので、無ければ既定値）
    #[serde(default)]
    keys: Keys,
    #[serde(default = "default_true")]
    close_on_blur: bool,
    #[serde(default = "default_font")]
    font_size: usize,
    #[serde(default = "default_shadow")]
    shadow: bool,
    #[serde(default)]
    glass: bool,
}

fn default_true() -> bool {
    true
}
fn default_font() -> usize {
    14
}
/// 影は macOS では既定でオン。Windows は枠なし窓に影を付けると DWM の 1px 枠が付くため既定はオフ
fn default_shadow() -> bool {
    cfg!(target_os = "macos")
}

#[derive(Serialize, Deserialize)]
struct Bundle {
    version: u32,
    config: BundleCfg,
    snippets: Vec<SnipDto>,
}

/// 設定・テーマ・定型文を1つの JSON に書き出す（履歴は機密を含みうるため対象外）。
/// 戻り値は保存先パス。キャンセル時は空文字。
#[tauri::command]
async fn export_settings(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    let st = app.state::<AppState>();
    let snippets = st
        .snips
        .lock()
        .unwrap()
        .dto()
        .map_err(|e| format!("snippets.toml に書式エラーがあるため書き出せません: {e}"))?;
    let bundle = Bundle {
        version: 1,
        config: BundleCfg {
            hotkey: st.hotkey.lock().unwrap().clone(),
            theme: st.theme.lock().unwrap().clone(),
            position: st.position.lock().unwrap().clone(),
            max_items: st.max_items.load(Ordering::SeqCst),
            max_kb: st.max_kb.load(Ordering::SeqCst),
            keys: st.keys.lock().unwrap().clone(),
            close_on_blur: st.close_on_blur.load(Ordering::SeqCst),
            font_size: st.font_size.load(Ordering::SeqCst),
            shadow: st.shadow.load(Ordering::SeqCst),
            glass: st.glass.load(Ordering::SeqCst),
        },
        snippets,
    };
    let Some(p) = app
        .dialog()
        .file()
        .set_file_name("clp-settings.json")
        .add_filter("clp 設定 (JSON)", &["json"])
        .blocking_save_file()
    else {
        return Ok(String::new());
    };
    let path = p.into_path().map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// JSON を読み込んで反映する。戻り値は結果メッセージ。キャンセル時は空文字。
#[tauri::command]
async fn import_settings(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(p) = app
        .dialog()
        .file()
        .add_filter("clp 設定 (JSON)", &["json"])
        .blocking_pick_file()
    else {
        return Ok(String::new());
    };
    let path = p.into_path().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let b: Bundle = serde_json::from_str(&text)
        .map_err(|e| format!("clp の設定ファイルとして読み込めません: {e}"))?;
    if b.version > 1 {
        return Err("このファイルは新しいバージョンの clp で作成されています".into());
    }
    if b.snippets.len() > snippets::MAX_SNIPPETS {
        return Err(format!("定型文が上限（{}個）を超えています", snippets::MAX_SNIPPETS));
    }
    let c = b.config;
    let n = b.snippets.len();
    save_snippets(app.clone(), b.snippets)?;
    set_theme(app.clone(), c.theme);
    if matches!(c.position.as_str(), "center" | "mouse" | "caret") {
        set_position(app.clone(), c.position);
    }
    set_limits(app.clone(), c.max_items, c.max_kb);
    set_ui(app.clone(), c.close_on_blur, c.font_size);
    set_effects(app.clone(), c.shadow, c.glass);
    if let Err(e) = set_keys(app.clone(), c.keys) {
        return Err(format!("キー設定が不正です: {e}"));
    }
    let mut msg = format!("読み込みました（定型文 {n} 件）");
    if let Err(e) = set_hotkey(app.clone(), c.hotkey.clone()) {
        msg += &format!("。ただしホットキー {} は設定できませんでした（{e}）", c.hotkey);
    }
    Ok(msg)
}

#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    platform::set_autostart(enabled)
}

fn open_settings(app: &AppHandle) {
    hide_main(app);
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = tauri::WebviewWindowBuilder::new(app, "settings", tauri::WebviewUrl::App("settings.html".into()))
        .title("clp 設定")
        .inner_size(820.0, 600.0)
        .min_inner_size(700.0, 520.0)
        .center()
        .build();
}

#[tauri::command]
fn open_dir(app: AppHandle) {
    let d = app.state::<AppState>().dir.clone();
    platform::open_folder(&d);
}

#[tauri::command]
fn open_snippets(app: AppHandle) {
    let path = app.state::<AppState>().snips.lock().unwrap().path.clone();
    hide_main(&app);
    platform::open_text(&path);
}

// ---------- clipboard monitor ----------

fn on_clipboard_changed(app: &AppHandle) {
    if platform::clip_excluded() {
        return;
    }
    let Some(text) = get_clipboard() else { return };
    let st = app.state::<AppState>();
    if text.trim().is_empty() || text.len() > st.max_kb.load(Ordering::SeqCst) * 1024 {
        return;
    }
    let h = fnv(&text);
    if st.self_hash.compare_exchange(h, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
        return; // 自分が書き戻したもの
    }
    st.store.lock().unwrap().insert(&text);
}

fn start_monitor(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last = 0u32;
        loop {
            let s = platform::clip_seq();
            if s != last {
                last = s;
                on_clipboard_changed(&app);
            }
            std::thread::sleep(Duration::from_millis(120));
        }
    });
}

// ---------- entry ----------

fn data_dir(app: &tauri::App) -> PathBuf {
    let d = app
        .path()
        .data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("clp");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _sc, ev| {
                    if ev.state() == ShortcutState::Pressed {
                        show_main(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let dir = data_dir(app);
            let cfg_path = dir.join("config.toml");
            if !cfg_path.exists() {
                write_config(&dir, &Config::default());
            }
            let cfg: Config = std::fs::read_to_string(&cfg_path)
                .ok()
                .and_then(|t| toml::from_str(&t).ok())
                .unwrap_or_default();

            let store = Store::open(&dir.join("history.db"), cfg.max_items)?;
            app.manage(AppState {
                dir: dir.clone(),
                hotkey: Mutex::new(cfg.hotkey.clone()),
                theme: Mutex::new(cfg.theme.clone()),
                position: Mutex::new(cfg.position.clone()),
                max_items: AtomicUsize::new(cfg.max_items),
                max_kb: AtomicUsize::new(cfg.max_kb),
                check_updates: AtomicBool::new(cfg.check_updates),
                close_on_blur: AtomicBool::new(cfg.close_on_blur),
                font_size: AtomicUsize::new(cfg.font_size),
                shadow: AtomicBool::new(cfg.shadow),
                glass: AtomicBool::new(cfg.glass),
                keys: Mutex::new(cfg.keys.clone()),
                update_available: Mutex::new(None),
                store: Mutex::new(store),
                snips: Mutex::new(Snippets::new(dir.join("snippets.toml"))),
                prev: Mutex::new(0),
                self_hash: AtomicU64::new(0),
            });

            if let Err(e) = app.global_shortcut().register(cfg.hotkey.as_str()) {
                eprintln!("hotkey register failed ({}): {e}", cfg.hotkey);
            }

            let m_show = MenuItem::with_id(app, "show", "表示", true, None::<&str>)?;
            let m_snip = MenuItem::with_id(app, "snip", "snippets.toml を開く", true, None::<&str>)?;
            let m_set = MenuItem::with_id(app, "settings", "設定…", true, None::<&str>)?;
            let m_quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
            let m_upd = MenuItem::with_id(app, "update", "更新を確認…", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&m_show, &m_snip, &m_set, &m_upd, &m_quit])?;
            let mut tray = TrayIconBuilder::with_id("main")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("clp");
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, ev| match ev.id.as_ref() {
                "show" => show_main(app),
                "snip" => open_snippets(app.clone()),
                "settings" | "update" => open_settings(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, ev| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = ev
                {
                    show_main(tray.app_handle());
                }
            })
            .build(app)?;

            // macOS: Dock に出さずメニューバー常駐にし、全画面アプリ上にも出せるようにする
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            if let Some(w) = app.get_webview_window("main") {
                platform::setup_window(&w);
                apply_window_theme(&w, &cfg.theme);
                apply_effects(&w, cfg.shadow, cfg.glass);
            }
            platform::ensure_permissions();

            // 起動の少し後に、新しい版がないか裏で確認する（設定でオフにできる）
            if cfg.check_updates {
                let h = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(8));
                    let _ = tauri::async_runtime::block_on(check_update(h));
                });
            }

            start_monitor(app.handle().clone());
            Ok(())
        })
        .on_window_event(|win, ev| match ev {
            _ if win.label() != "main" => {}
            WindowEvent::Focused(false) => {
                let st = win.app_handle().state::<AppState>();
                if st.close_on_blur.load(Ordering::SeqCst) {
                    let _ = win.hide();
                }
            }
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = win.hide();
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            search,
            full_text,
            choose,
            toggle_pin,
            delete_item,
            hide_window,
            open_snippets,
            get_settings,
            set_hotkey,
            set_theme,
            set_position,
            export_settings,
            import_settings,
            set_check_updates,
            get_keys,
            set_keys,
            set_ui,
            set_effects,
            check_update,
            install_update,
            set_limits,
            clear_history,
            open_dir,
            get_snippets,
            save_snippets,
            set_autostart
        ])
        .run(tauri::generate_context!())
        .expect("error while running clp");
}
