#[cfg(windows)]
#[path = "platform_win.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "platform_mac.rs"]
mod platform;
mod i18n;
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
    /// 履歴に残さないアプリ（小文字の実行ファイル名 / macOS はアプリ名・Bundle ID）
    exclude: Mutex<Vec<String>>,
    /// 表示言語の設定値（"auto" または言語コード）
    lang: Mutex<String>,
    /// 記録の一時停止（再起動で解除。保存しない）
    paused: AtomicBool,
    recent: Mutex<Vec<String>>,
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
    /// 整形して貼り付け（前後の空白を除去 / 改行を除いて1行に / 大文字 / 小文字）
    paste_trim: Vec<String>,
    paste_oneline: Vec<String>,
    paste_upper: Vec<String>,
    paste_lower: Vec<String>,
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
            paste_trim: v(&["Shift+Enter"]),
            paste_oneline: v(&["Ctrl+Enter"]),
            paste_upper: v(&["Ctrl+Shift+U"]),
            paste_lower: v(&["Ctrl+Shift+L"]),
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
    /// 履歴に残さないアプリ
    exclude_apps: Vec<String>,
    /// 表示言語。"auto"（OS に合わせる）または言語コード（例: "ja"）
    language: String,
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
            exclude_apps: Vec::new(),
            language: "auto".into(),
            keys: Keys::default(),
        }
    }
}

#[derive(Serialize, Clone)]
struct Shown {
    history: Vec<Row>,
    snippets: Vec<Row>,
    paused: bool,
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
            paused: st.paused.load(Ordering::SeqCst),
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

/// 貼り付け前の整形
fn transform(text: &str, kind: Option<&str>) -> String {
    match kind {
        Some("trim") => text.trim().to_string(),
        Some("oneline") => text.split(['\r', '\n']).map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" "),
        Some("upper") => text.to_uppercase(),
        Some("lower") => text.to_lowercase(),
        _ => text.to_string(),
    }
}

#[tauri::command]
fn choose(app: AppHandle, id: i64, mode: String, xform: Option<String>) -> Result<(), String> {
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
    let text = transform(&text, xform.as_deref());
    if text.is_empty() {
        return Err("empty".into());
    }

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
    language: String,
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
            exclude_apps: st.exclude.lock().unwrap().clone(),
            language: st.lang.lock().unwrap().clone(),
            keys: st.keys.lock().unwrap().clone(),
        },
    );
}

/// 現在の表示言語で文言を取り出す（{name} を vars で置換）
fn tr(app: &AppHandle, key: &str, vars: &[(&str, &str)]) -> String {
    let st = app.state::<AppState>();
    let setting = st.lang.lock().unwrap().clone();
    i18n::tr(&st.dir, &setting, key, vars)
}

/// 言語ファイル（辞書）。コードは実際に使う言語
#[tauri::command]
fn get_dict(app: AppHandle) -> serde_json::Value {
    let st = app.state::<AppState>();
    let setting = st.lang.lock().unwrap().clone();
    let code = i18n::resolve(&setting, &st.dir);
    serde_json::json!({ "code": code, "dict": i18n::dict(&st.dir, &code) })
}

#[tauri::command]
fn list_langs(app: AppHandle) -> Vec<(String, String)> {
    i18n::list(&app.state::<AppState>().dir)
}

#[tauri::command]
fn set_language(app: AppHandle, language: String) {
    let st = app.state::<AppState>();
    *st.lang.lock().unwrap() = language;
    save_config(&st);
    refresh_tray(&app);
    let _ = app.emit("lang", ());
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let paused = app.state::<AppState>().paused.load(Ordering::SeqCst);
    let m_show = MenuItem::with_id(app, "show", tr(app, "tray.show", &[]), true, None::<&str>)?;
    let m_pause = tauri::menu::CheckMenuItem::with_id(app, "pause", tr(app, "tray.pause", &[]), true, paused, None::<&str>)?;
    let m_snip = MenuItem::with_id(app, "snip", tr(app, "tray.snippets", &[]), true, None::<&str>)?;
    let m_set = MenuItem::with_id(app, "settings", tr(app, "tray.settings", &[]), true, None::<&str>)?;
    let m_upd = MenuItem::with_id(app, "update", tr(app, "tray.update", &[]), true, None::<&str>)?;
    let m_quit = MenuItem::with_id(app, "quit", tr(app, "tray.quit", &[]), true, None::<&str>)?;
    Menu::with_items(app, &[&m_show, &m_pause, &m_snip, &m_set, &m_upd, &m_quit])
}

/// トレイのメニューとツールチップを、現在の言語・状態で作り直す
fn refresh_tray(app: &AppHandle) {
    let Some(t) = app.tray_by_id("main") else { return };
    if let Ok(m) = build_menu(app) {
        let _ = t.set_menu(Some(m));
    }
    let st = app.state::<AppState>();
    let ver = st.update_available.lock().unwrap().clone();
    let tip = if st.paused.load(Ordering::SeqCst) {
        tr(app, "tray.tip_paused", &[])
    } else if let Some(v) = ver {
        tr(app, "tray.tip_update", &[("v", &v)])
    } else {
        "clp".to_string()
    };
    let _ = t.set_tooltip(Some(tip));
}

fn norm_apps(list: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in list {
        let s = s.trim().to_lowercase();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

#[tauri::command]
fn get_excludes(app: AppHandle) -> Vec<String> {
    app.state::<AppState>().exclude.lock().unwrap().clone()
}

#[tauri::command]
fn set_excludes(app: AppHandle, list: Vec<String>) {
    let st = app.state::<AppState>();
    *st.exclude.lock().unwrap() = norm_apps(list);
    save_config(&st);
}

/// 最近コピーしたアプリ（設定画面の「除外に追加」候補）
#[tauri::command]
fn recent_apps(app: AppHandle) -> Vec<String> {
    app.state::<AppState>().recent.lock().unwrap().clone()
}

#[tauri::command]
fn get_keys(app: AppHandle) -> Keys {
    app.state::<AppState>().keys.lock().unwrap().clone()
}

#[tauri::command]
fn set_keys(app: AppHandle, keys: Keys) -> Result<(), String> {
    if keys.choose.is_empty() {
        return Err(tr(&app, "err.keys_choose", &[]));
    }
    if keys.close.is_empty() {
        return Err(tr(&app, "err.keys_close", &[]));
    }
    if !matches!(keys.quick_mod.as_str(), "Ctrl" | "Alt" | "Shift" | "Super") {
        return Err(tr(&app, "err.keys_mod", &[]));
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

/// GitHub Releases の latest.json を見て、新しい版があれば情報を返す
#[tauri::command]
async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let found = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| tr(&app, "err.update_check", &[("e", &e.to_string())]))?;
    let st = app.state::<AppState>();
    match found {
        Some(u) => {
            *st.update_available.lock().unwrap() = Some(u.version.clone());
            refresh_tray(&app);
            Ok(Some(UpdateInfo { version: u.version.clone(), notes: u.body.clone().unwrap_or_default() }))
        }
        None => {
            *st.update_available.lock().unwrap() = None;
            refresh_tray(&app);
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
        .map_err(|e| tr(&app, "err.update_check", &[("e", &e.to_string())]))?
        .ok_or_else(|| tr(&app, "err.no_update", &[]))?;
    upd.download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| tr(&app, "err.update_install", &[("e", &e.to_string())]))?;
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
        language: app.state::<AppState>().lang.lock().unwrap().clone(),
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
        return Err(tr(&app, "err.hotkey", &[("e", &e.to_string())]));
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
    #[serde(default)]
    exclude_apps: Vec<String>,
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
        .map_err(|e| tr(&app, "err.export_snip", &[("e", &e.to_string())]))?;
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
            exclude_apps: st.exclude.lock().unwrap().clone(),
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
        .map_err(|e| tr(&app, "err.import_bad", &[("e", &e.to_string())]))?;
    if b.version > 1 {
        return Err(tr(&app, "err.import_newer", &[]));
    }
    if b.snippets.len() > snippets::MAX_SNIPPETS {
        return Err(tr(&app, "err.import_max", &[("n", &snippets::MAX_SNIPPETS.to_string())]));
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
    set_excludes(app.clone(), c.exclude_apps);
    if let Err(e) = set_keys(app.clone(), c.keys) {
        return Err(tr(&app, "err.import_keys", &[("e", &e)]));
    }
    let mut msg = tr(&app, "import.ok", &[("n", &n.to_string())]);
    if let Err(e) = set_hotkey(app.clone(), c.hotkey.clone()) {
        msg += &tr(&app, "import.hk_fail", &[("k", &c.hotkey), ("e", &e)]);
    }
    Ok(msg)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    platform::set_autostart(enabled).map_err(|e| tr(&app, &e, &[]))
}

fn open_settings(app: &AppHandle) {
    hide_main(app);
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = tauri::WebviewWindowBuilder::new(app, "settings", tauri::WebviewUrl::App("settings.html".into()))
        .title(tr(app, "set.title", &[]))
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
    let st = app.state::<AppState>();
    if st.paused.load(Ordering::SeqCst) || platform::clip_excluded() {
        return;
    }
    let src = platform::foreground_app();
    if !src.is_empty() {
        if st.exclude.lock().unwrap().iter().any(|e| src.contains(e)) {
            return;
        }
        let mut r = st.recent.lock().unwrap();
        let name = src[0].clone();
        if name != "clp.exe" && name != "clp" {
            r.retain(|x| *x != name);
            r.insert(0, name);
            r.truncate(8);
        }
    }
    let Some(text) = get_clipboard() else { return };
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
                exclude: Mutex::new(norm_apps(cfg.exclude_apps.clone())),
                lang: Mutex::new(cfg.language.clone()),
                paused: AtomicBool::new(false),
                recent: Mutex::new(Vec::new()),
                update_available: Mutex::new(None),
                store: Mutex::new(store),
                snips: Mutex::new(Snippets::new(dir.join("snippets.toml"))),
                prev: Mutex::new(0),
                self_hash: AtomicU64::new(0),
            });

            if let Err(e) = app.global_shortcut().register(cfg.hotkey.as_str()) {
                eprintln!("hotkey register failed ({}): {e}", cfg.hotkey);
            }

            let menu = build_menu(app.handle())?;
            let mut tray = TrayIconBuilder::with_id("main")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("clp");
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, ev| match ev.id.as_ref() {
                "show" => show_main(app),
                "pause" => {
                    let st = app.state::<AppState>();
                    let on = !st.paused.load(Ordering::SeqCst);
                    st.paused.store(on, Ordering::SeqCst);
                    refresh_tray(app);
                }                "snip" => open_snippets(app.clone()),
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
            get_dict,
            list_langs,
            set_language,
            get_excludes,
            set_excludes,
            recent_apps,
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
