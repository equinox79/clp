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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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
    store: Mutex<Store>,
    snips: Mutex<Snippets>,
    prev: Mutex<isize>,
    self_hash: AtomicU64,
}

#[derive(Deserialize)]
#[serde(default)]
struct Config {
    hotkey: String,
    max_items: usize,
    theme: String,
    position: String,
    max_kb: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: "Ctrl+Alt+V".into(),
            max_items: 3000,
            theme: "auto".into(),
            position: "center".into(),
            max_kb: 1024,
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
}

fn write_config(dir: &PathBuf, c: &Config) {
    let body = format!(
        "# 設定画面から変更できます\nhotkey = \"{}\"\ntheme = \"{}\"\n# 表示位置: center / mouse / caret\nposition = \"{}\"\n# 履歴の最大保持数（ピン留めは対象外）\nmax_items = {}\n# 1件あたりの最大サイズ(KB)。超えるテキストは保存しません\nmax_kb = {}\n",
        c.hotkey, c.theme, c.position, c.max_items, c.max_kb
    );
    let _ = std::fs::write(dir.join("config.toml"), body);
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
        },
    );
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
            let menu = Menu::with_items(app, &[&m_show, &m_snip, &m_set, &m_quit])?;
            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("clp");
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, ev| match ev.id.as_ref() {
                "show" => show_main(app),
                "snip" => open_snippets(app.clone()),
                "settings" => open_settings(app),
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
            }
            platform::ensure_permissions();

            start_monitor(app.handle().clone());
            Ok(())
        })
        .on_window_event(|win, ev| match ev {
            _ if win.label() != "main" => {}
            WindowEvent::Focused(false) => {
                let _ = win.hide();
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
