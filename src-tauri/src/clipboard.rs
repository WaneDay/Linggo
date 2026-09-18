// Linggo 剪贴板模块：arboard 文本/图片读写、300ms 防抖、
// F1 划词抓取（Ctrl+C + 防抖判定）、F2 回贴（写剪贴板→聚焦原窗口→Ctrl+V）。

use base64::Engine as _;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::borrow::Cow;
use std::time::{Duration, Instant};
use tauri::Manager;

/// 剪贴板防抖窗口：值保持稳定该时长后才视为最终结果
pub const DEBOUNCE_MS: u64 = 300;

pub fn read_text() -> String {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut c| c.get_text().ok())
        .unwrap_or_default()
}

pub fn write_text(text: &str) -> Result<(), String> {
    let mut c = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    c.set_text(text.to_string()).map_err(|e| e.to_string())
}

/// 把一张 PNG（base64，可含 data: 前缀）写入剪贴板（F3「复制截图」）
pub fn write_image_png(b64: &str) -> Result<(), String> {
    let raw = b64.split(',').last().unwrap_or("");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .map_err(|e| e.to_string())?;
    let img = image::load_from_memory(&bytes)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_image(arboard::ImageData {
        width: w as usize,
        height: h as usize,
        bytes: Cow::from(img.into_raw()),
    })
    .map_err(|e| e.to_string())
}

fn with_enigo(f: fn(&mut Enigo)) {
    if let Ok(mut e) = Enigo::new(&Settings::default()) {
        f(&mut e);
    }
}

/// 用系统按键模拟发送 Ctrl+C（选择性抓取原文）
fn send_copy() {
    with_enigo(|e| {
        let _ = e.key(Key::Control, Direction::Press);
        std::thread::sleep(Duration::from_millis(12));
        let _ = e.key(Key::Unicode('c'), Direction::Click);
        std::thread::sleep(Duration::from_millis(12));
        let _ = e.key(Key::Control, Direction::Release);
    });
}

/// 哨兵标记：先清空剪贴板，只有 Ctrl+C 后出现「非哨兵且非空」才算真正抓到选区。
/// 这样即使选区文本与历史剪贴板相同也能识别；抓取失败则恢复原剪贴板。
const MARKER: &str = "\u{E000}LINGGO_SELECTION\u{E001}";

/// F1 划词抓取：先把剪贴板置为哨兵 → 把焦点还给目标窗口（确保 Ctrl+C 打在选区所在窗口上）
/// → 发 Ctrl+C → 轮询直到出现非哨兵稳定值（防抖 300ms）。
/// 未捕获到新选区（超时 1500ms）→ 恢复原剪贴板并返回空串，绝不回退成旧的剪贴板内容。
pub fn grab_selection(target: isize) -> String {
    let before = read_text();
    let _ = write_text(MARKER);
    std::thread::sleep(Duration::from_millis(40));
    // 弹窗收得不干净时前台可能停在 Linggo 自己的窗口上；先还焦给源窗口再复制。
    if target != 0 {
        crate::win32::focus_restore(target);
        std::thread::sleep(Duration::from_millis(60));
    }
    send_copy();
    let t0 = Instant::now();
    let mut candidate: Option<(String, Instant)> = None;
    loop {
        std::thread::sleep(Duration::from_millis(30));
        let now = read_text();
        if now != MARKER && !now.is_empty() {
            match &candidate {
                Some((val, since)) if val == &now => {
                    if since.elapsed() >= Duration::from_millis(DEBOUNCE_MS) {
                        return now;
                    }
                }
                _ => candidate = Some((now, Instant::now())),
            }
        }
        if t0.elapsed() > Duration::from_millis(1_500) {
            break;
        }
    }
    // 未捕获：恢复原剪贴板，返回空串由上层提示
    let _ = write_text(&before);
    String::new()
}

/// 异步版（阻塞工作在后台线程，避免卡 UI/热键线程）
pub async fn grab_selection_async(target: isize) -> String {
    tauri::async_runtime::spawn_blocking(move || grab_selection(target))
        .await
        .unwrap_or_default()
}

/// F2 回贴：写剪贴板 → 聚焦原前台窗口 → Ctrl+V 粘贴
pub fn paste_text_to(text: &str, target_hwnd: Option<isize>) -> Result<(), String> {
    write_text(text)?;
    std::thread::sleep(Duration::from_millis(120));
    if let Some(hwnd) = target_hwnd {
        crate::win32::focus_restore(hwnd);
    }
    std::thread::sleep(Duration::from_millis(60));
    with_enigo(|e| {
        let _ = e.key(Key::Control, Direction::Press);
        std::thread::sleep(Duration::from_millis(12));
        let _ = e.key(Key::Unicode('v'), Direction::Click);
        std::thread::sleep(Duration::from_millis(12));
        let _ = e.key(Key::Control, Direction::Release);
    });
    Ok(())
}

/// 异步版回贴
pub async fn paste_text_to_async(text: String, target_hwnd: Option<isize>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || paste_text_to(&text, target_hwnd))
        .await
        .map_err(|e| e.to_string())?
}

/// 复制文本到剪贴板（主窗口「复制译文」/ 弹窗「复制原文」）
#[tauri::command]
pub fn copy_text(text: String) -> Result<(), String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("没有可复制的内容".to_string());
    }
    write_text(t)
}

/// F2 回车回贴：译文写入剪贴板 → 聚焦触发时记录的原窗口 → 模拟 Ctrl+V
#[tauri::command]
pub async fn commit_paste(app: tauri::AppHandle, text: String) -> Result<(), String> {
    let target = app.state::<crate::state::AppState>().target_hwnd.lock().unwrap().clone();
    paste_text_to_async(text, target).await
}

/// 复制一张 PNG（base64，可含 data: 前缀）到剪贴板（F3「复制截图」）
#[tauri::command]
pub fn copy_image(b64: String) -> Result<(), String> {
    write_image_png(&b64)
}