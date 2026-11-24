//! Windows 依存部分。macOS は後日 platform_mac.rs として同じ関数群を実装する。
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::DataExchange::{
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, RegisterClipboardFormatW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, SetForegroundWindow, GUITHREADINFO,
};

use std::path::Path;
use std::process::Command;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

fn no_window(c: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000)
}

pub fn autostart_enabled() -> bool {
    no_window(Command::new("reg").args(["query", RUN_KEY, "/v", "clp"]))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let ok = if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        no_window(Command::new("reg").args([
            "add", RUN_KEY, "/v", "clp", "/t", "REG_SZ", "/d",
            &format!("\"{}\"", exe.display()), "/f",
        ]))
        .status()
    } else {
        no_window(Command::new("reg").args(["delete", RUN_KEY, "/v", "clp", "/f"])).status()
    }
    .map(|s| s.success())
    .unwrap_or(false);
    if ok || !enabled { Ok(()) } else { Err("レジストリの更新に失敗しました".into()) }
}

pub fn open_folder(p: &Path) {
    let _ = Command::new("explorer.exe").arg(p).spawn();
}

pub fn open_text(p: &Path) {
    let _ = Command::new("notepad.exe").arg(p).spawn();
}

/// macOS 用のウィンドウ設定フック（Windows では何もしない）
pub fn setup_window(_w: &tauri::WebviewWindow) {}
pub fn refresh_shadow(_w: &tauri::WebviewWindow) {}
pub fn activate_self() {}
pub fn ensure_permissions() {}

pub fn clip_seq() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// パスワードマネージャー等が付ける「履歴に残さない」フラグ
pub fn clip_excluded() -> bool {
    unsafe {
        let f = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        f != 0 && IsClipboardFormatAvailable(f).is_ok()
    }
}

pub fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// 前面アプリの識別名（小文字の実行ファイル名。取得できなければ空）
pub fn foreground_app() -> Vec<String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return vec![];
        }
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return vec![] };
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return vec![];
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let name = path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
        if name.is_empty() { vec![] } else { vec![name] }
    }
}

/// 前面ウィンドウが `target` に戻るのを待つ（固定スリープではなくポーリング）
pub fn restore_foreground(target: isize, timeout: Duration) {
    if target == 0 {
        return;
    }
    let start = Instant::now();
    let mut tried = false;
    while foreground() != target && start.elapsed() < timeout {
        if !tried && start.elapsed() > Duration::from_millis(40) {
            unsafe {
                let _ = SetForegroundWindow(HWND(target as *mut _));
            }
            tried = true;
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}

/// 前面アプリの入力カーソル（キャレット）の画面座標（x, 下端y）。
/// Chrome/VS Code など独自描画のアプリでは取得できず None。
pub fn caret_pos() -> Option<(i32, i32)> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }
        let tid = GetWindowThreadProcessId(fg, None);
        let mut gi = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(tid, &mut gi).ok()?;
        if gi.hwndCaret.0.is_null() {
            return None;
        }
        let r = gi.rcCaret;
        if r.right == 0 && r.bottom == 0 && r.left == 0 && r.top == 0 {
            return None;
        }
        let mut p = POINT { x: r.left, y: r.bottom };
        if !ClientToScreen(gi.hwndCaret, &mut p).as_bool() {
            return None;
        }
        Some((p.x, p.y))
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// ホットキーの修飾キーが押しっぱなしでも Ctrl+V として届くよう、先に解放してから送る
pub fn send_paste() {
    let mut inputs: Vec<INPUT> = Vec::new();
    for vk in [VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN, VK_CONTROL] {
        let down = unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0;
        if down {
            inputs.push(key(vk, true));
        }
    }
    inputs.push(key(VK_CONTROL, false));
    inputs.push(key(VK_V, false));
    inputs.push(key(VK_V, true));
    inputs.push(key(VK_CONTROL, true));
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}
