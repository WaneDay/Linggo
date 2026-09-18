// Linggo 系统工具：自启注册表、历史记录持久化、统一错误（步骤 11 细化）。

use std::io;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 历史记录条目（时间、语向、原文、译文）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub ts: u64,
    pub src: String,
    pub tgt: String,
    pub text: String,
    pub translated: String,
}

fn data_dir() -> PathBuf {
    let mut d = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    d.push("Linggo");
    d
}

fn history_path() -> PathBuf {
    data_dir().join("history.json")
}

/// 读取历史（损坏/缺失时回退为空，不致命）
pub fn load_history() -> Vec<HistoryItem> {
    let p = history_path();
    let s = std::fs::read_to_string(&p).unwrap_or_default();
    serde_json::from_str(&s).unwrap_or_default()
}

/// 冗余落盘（IO 很小，直接 fs::write 覆盖写新数组）
pub fn save_history(items: &[HistoryItem]) -> Result<(), String> {
    let _ = data_dir();
    let p = history_path();
    let json = serde_json::to_string(items).map_err(|e| e.to_string())?;
    std::fs::write(&p, json).map_err(|e| e.to_string())
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Linggo";

const DWM_KEY: &str = r"Software\Microsoft\Windows\DWM";

/// 读取 Windows「个性化」强调色（DWM AccentColor，DWORD，字节序 AABBGGRR）。
/// 返回 "#RRGGBB"；读取失败返回 None（前端回退主题强调色）。
#[tauri::command]
pub fn system_accent_color() -> Option<String> {
    let v: u32 = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(DWM_KEY)
        .ok()?
        .get_value::<u32, _>("AccentColor")
        .ok()?;
    let r = (v & 0x0000_00ff) as u8;
    let g = ((v >> 8) & 0x0000_00ff) as u8;
    let b = ((v >> 16) & 0x0000_00ff) as u8;
    if r == 0 && g == 0 && b == 0 {
        return None;
    }
    Some(format!("#{:02x}{:02x}{:02x}", r, g, b))
}

/// 是否已注册开机自启
pub fn is_autostart() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .ok()
        .and_then(|k| k.get_value::<String, _>(VALUE_NAME).ok())
        .is_some()
}

/// 用系统默认方式打开链接（仅允许 http/https，防注入）。
/// 更新入口「打开发布页」使用；复用系统默认浏览器与代理设置。
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    let trimmed = url.trim();
    let low = trimmed.to_ascii_lowercase();
    if !low.starts_with("https://") && !low.starts_with("http://") {
        return Err("仅支持打开 http/https 链接".to_string());
    }
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::Foundation::HWND;
    use windows::core::{w, PCWSTR};
    let wide: Vec<u16> = trimmed.encode_utf16().chain(std::iter::once(0)).collect();
    let r = unsafe {
        ShellExecuteW(
            HWND::default(),
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    let code = r.0 as isize;
    if code <= 32 {
        return Err(format!("无法打开链接（ShellExecute 错误 {code}）"));
    }
    Ok(())
}

/// 设置开机自启：值 `"<exe>" --silent`（静默常驻托盘，不弹主窗口）
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(RUN_KEY).map_err(|e| e.to_string())?;
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let cmd = format!("\"{}\" --silent", exe.display());
        key.set_value(VALUE_NAME, &cmd).map_err(|e| e.to_string())
    } else {
        match key.delete_value(VALUE_NAME) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}