//! macOS 依存部分。platform_win.rs と同じ関数群を実装する。
//!
//! 注意: このファイルは Mac 実機でビルド・検証されていません（開発は Windows 上で行ったため）。
//! 初回ビルドでコンパイルエラーが出た場合は、objc2 / core-graphics のバージョン差による API 差異を疑ってください。
use objc2::rc::autoreleasepool;
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, msg_send};
use std::ffi::{c_char, CStr};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn CGEventSourceFlagsState(state: i32) -> u64;
}

const KEY_V: u16 = 9;
const MASK_MODS: u64 = 0x20000 | 0x40000 | 0x80000 | 0x100000; // shift|control|option|command

// ---------- 自動起動（LaunchAgent） ----------

fn agent_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    Path::new(&home).join("Library/LaunchAgents/local.clp.app.plist")
}

pub fn autostart_enabled() -> bool {
    agent_path().exists()
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let p = agent_path();
    if !enabled {
        let _ = std::fs::remove_file(&p);
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>local.clp.app</string>\n<key>ProgramArguments</key><array><string>{}</string></array>\n<key>RunAtLoad</key><true/>\n</dict></plist>\n",
        exe.display()
    );
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&p, plist).map_err(|e| e.to_string())
}

pub fn open_folder(p: &Path) {
    let _ = Command::new("open").arg(p).spawn();
}

pub fn open_text(p: &Path) {
    let _ = Command::new("open").arg("-t").arg(p).spawn();
}

// ---------- 権限 ----------

/// アクセシビリティ権限（自動ペーストに必須）が無ければ、設定画面を開いて案内する
pub fn ensure_permissions() {
    if unsafe { AXIsProcessTrusted() } == 0 {
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}

// ---------- ウィンドウ ----------

/// 全画面アプリ・全 Space の上にも出せるようにする（NSPanel 相当の挙動）
pub fn setup_window(w: &tauri::WebviewWindow) {
    if let Ok(ptr) = w.ns_window() {
        let win = ptr as *mut AnyObject;
        unsafe {
            // canJoinAllSpaces(1<<0) | fullScreenAuxiliary(1<<8)
            let _: () = msg_send![win, setCollectionBehavior: (1usize | (1usize << 8))];
            // NSStatusWindowLevel
            let _: () = msg_send![win, setLevel: 25isize];
        }
    }
}

/// 透明な窓の影は内容の形から作られるため、表示や効果の切り替え後に作り直す
pub fn refresh_shadow(w: &tauri::WebviewWindow) {
    if let Ok(ptr) = w.ns_window() {
        let win = ptr as *mut AnyObject;
        unsafe {
            let _: () = msg_send![win, invalidateShadow];
        }
    }
}

/// 自アプリを前面化（Accessory ポリシーのアプリは明示しないとフォーカスを取れない）
pub fn activate_self() {
    autoreleasepool(|_| unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: Bool::YES];
    });
}

// ---------- クリップボード ----------

/// NSPasteboard.changeCount（変更のたびに増える）
pub fn clip_seq() -> u32 {
    autoreleasepool(|_| unsafe {
        let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        let n: isize = msg_send![pb, changeCount];
        n as u32
    })
}

/// パスワードマネージャー等の「機密/一時」マーカー（nspasteboard.org の規約）
pub fn clip_excluded() -> bool {
    autoreleasepool(|_| unsafe {
        let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        let types: *mut AnyObject = msg_send![pb, types];
        if types.is_null() {
            return false;
        }
        let count: usize = msg_send![types, count];
        for i in 0..count {
            let t: *mut AnyObject = msg_send![types, objectAtIndex: i];
            let c: *const c_char = msg_send![t, UTF8String];
            if c.is_null() {
                continue;
            }
            let s = CStr::from_ptr(c).to_string_lossy();
            if s == "org.nspasteboard.ConcealedType" || s == "org.nspasteboard.TransientType" {
                return true;
            }
        }
        false
    })
}

// ---------- 前面アプリの記憶・復帰 ----------

/// 最前面アプリのプロセスID
pub fn foreground() -> isize {
    autoreleasepool(|_| unsafe {
        let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: *mut AnyObject = msg_send![ws, frontmostApplication];
        if app.is_null() {
            return 0;
        }
        let pid: i32 = msg_send![app, processIdentifier];
        pid as isize
    })
}

/// 前面アプリの識別名（小文字のアプリ名と Bundle ID）
pub fn foreground_app() -> Vec<String> {
    autoreleasepool(|_| unsafe {
        let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: *mut AnyObject = msg_send![ws, frontmostApplication];
        if app.is_null() {
            return vec![];
        }
        let mut out = Vec::new();
        for sel in 0..2 {
            let s: *mut AnyObject = if sel == 0 { msg_send![app, localizedName] } else { msg_send![app, bundleIdentifier] };
            if s.is_null() {
                continue;
            }
            let c: *const c_char = msg_send![s, UTF8String];
            if !c.is_null() {
                out.push(CStr::from_ptr(c).to_string_lossy().to_lowercase());
            }
        }
        out
    })
}

pub fn restore_foreground(target: isize, timeout: Duration) {
    if target == 0 {
        return;
    }
    let start = Instant::now();
    let mut tried = false;
    while foreground() != target && start.elapsed() < timeout {
        if !tried && start.elapsed() > Duration::from_millis(40) {
            autoreleasepool(|_| unsafe {
                let app: *mut AnyObject = msg_send![
                    class!(NSRunningApplication),
                    runningApplicationWithProcessIdentifier: target as i32
                ];
                if !app.is_null() {
                    // NSApplicationActivateIgnoringOtherApps
                    let _: Bool = msg_send![app, activateWithOptions: 2usize];
                }
            });
            tried = true;
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}

// ---------- 貼り付け（Cmd+V） ----------

/// 入力カーソル位置の取得は未対応（Accessibility API が必要）。マウス位置にフォールバックする
pub fn caret_pos() -> Option<(i32, i32)> {
    None
}

pub fn send_paste() {
    // ホットキーの修飾キーが離れるのを待つ（最大300ms）
    let start = Instant::now();
    while unsafe { CGEventSourceFlagsState(0) } & MASK_MODS != 0
        && start.elapsed() < Duration::from_millis(300)
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let Ok(src) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) else { return };
    for down in [true, false] {
        if let Ok(ev) = CGEvent::new_keyboard_event(src.clone(), KEY_V, down) {
            ev.set_flags(CGEventFlags::CGEventFlagCommand);
            ev.post(CGEventTapLocation::HID);
        }
    }
}
