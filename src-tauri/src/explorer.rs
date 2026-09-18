// Linggo Explorer 辅助：F4「新建文件夹」。
// 解析「鼠标所在文件夹」（光标下的资源管理器窗口 → IShellWindows(COM) 取当前目录；
// 桌面 → FOLDERID_Desktop 用户桌面），并负责文件名清洗 + 重名自增 + 创建目录。
// 创建目录不触发翻译：弹窗已像 F2 一样实时翻译预览，回车时直接把当前译文/原文作为名字。
// 注意：改动本文件内中文务必用 edit/write 工具（PowerShell Set-Content 会写坏 CJK，见日志 17）。

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use windows::core::{Interface, VARIANT};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};
use windows::Win32::UI::Shell::{
    IWebBrowserApp, IShellWindows, KF_FLAG_DEFAULT, SHChangeNotify, SHGetKnownFolderPath,
    ShellWindows, FOLDERID_Desktop, SHCNE_MKDIR, SHCNF_FLAGS, SHCNF_FLUSHNOWAIT, SHCNF_PATHW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetCursorPos, GetWindowLongW, GetWindowRect,
    GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GWL_STYLE,
};

/// F4 目标解析：返回光标下应创建文件夹的目录绝对路径。
/// 命中优先级：资源管理器窗口（取当前浏览目录）→ 桌面（Progman/WorkerW → 用户桌面目录）
/// → 前台窗口若是资源管理器也兜底采用。都不命中返回 None（前端弹警告）。
pub fn folder_under_cursor() -> Option<String> {
    let (x, y) = cursor()?;
    if let Some((hwnd, cls, proc)) = window_at_cursor(x, y) {
        if proc.eq_ignore_ascii_case("explorer.exe") {
            if let Some(p) = explorer_path_of(hwnd) {
                return Some(p);
            }
            // IShellWindows 偶发取不到（非文件系统视图如「此电脑」）→ 落到下一个判定
        }
        if cls == "Progman" || cls == "WorkerW" || cls == "SysListView32" {
            if let Some(p) = desktop_path() {
                return Some(p);
            }
        }
    }
    // 鼠标悬在非资源管理器窗口上时，若前台是资源管理器仍可用（常见：打字窗口盖住部分目录）
    let fg = crate::win32::foreground_hwnd();
    if fg != 0 && crate::win32::process_name_of(fg).eq_ignore_ascii_case("explorer.exe") {
        if let Some(p) = explorer_path_of(fg) {
            return Some(p);
        }
    }
    None
}

fn cursor() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    if unsafe { GetCursorPos(&mut p) }.is_ok() {
        Some((p.x, p.y))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 光标下窗口查找（顶层、可见、非 toolwindow、非本进程；不跳过桌面层）
// ---------------------------------------------------------------------------

struct Finder {
    target: POINT,
    hit: Option<(isize, String, String)>,
}

unsafe extern "system" fn find_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut Finder);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex & 0x0000_0080 != 0 {
        return BOOL(1); // WS_EX_TOOLWINDOW
    }
    let st = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    if st & 0x4000_0000 != 0 {
        return BOOL(1); // WS_CHILD（桌面列表是 Progman 子窗口，但命中的顶层是 Progman 本身）
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32));
    if pid == std::process::id() {
        return BOOL(1);
    }
    let mut r = RECT::default();
    if GetWindowRect(hwnd, &mut r).is_err() {
        return BOOL(1);
    }
    if (r.right - r.left) < 40 || (r.bottom - r.top) < 40 {
        return BOOL(1);
    }
    let t = ctx.target;
    if t.x >= r.left && t.x <= r.right && t.y >= r.top && t.y <= r.bottom {
        let mut cls = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut cls);
        let cls = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
        let proc = crate::win32::process_name_of(hwnd.0 as isize);
        ctx.hit = Some((hwnd.0 as isize, cls, proc));
        return BOOL(0);
    }
    BOOL(1)
}

fn window_at_cursor(x: i32, y: i32) -> Option<(isize, String, String)> {
    let mut ctx = Finder {
        target: POINT { x, y },
        hit: None,
    };
    let _ = unsafe { EnumWindows(Some(find_cb), LPARAM(&mut ctx as *mut Finder as isize)) };
    ctx.hit
}

// ---------------------------------------------------------------------------
// COM：IShellWindows 枚举资源管理器窗口 / 桌面目录
// ---------------------------------------------------------------------------

/// 在 COM 初始化公寓内执行 f；S_OK/S_FALSE 后配对 CoUninitialize。
/// RPC_E_CHANGED_MODE（其它公寓模式已就绪）时直接用既有公寓，不卸载。
fn with_com<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        let owns = hr.0 >= 0;
        let r = f();
        if owns {
            CoUninitialize();
        }
        r
    }
}

/// 用 IShellWindows（Shell.Application 的同款 COM 接口）按窗口句柄反查当前浏览目录。
fn explorer_path_of(hwnd: isize) -> Option<String> {
    with_com(|| unsafe {
        let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
        let n = shell.Count().ok()?;
        for i in 0..n {
            let d = match shell.Item(&VARIANT::from(i)) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let wb: IWebBrowserApp = match d.cast() {
                Ok(w) => w,
                Err(_) => continue,
            };
            let h = wb.HWND().map(|h| h.0).unwrap_or(0);
            if h != hwnd {
                continue;
            }
            let url = wb.LocationURL().ok()?;
            let url = url.to_string();
            return file_url_to_path(url.trim());
        }
        None
    })
}

/// 用户桌面目录（SHGetKnownFolderPath，须 CoTaskMemFree 释放）。
fn desktop_path() -> Option<String> {
    with_com(|| unsafe {
        let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None) else {
            return None;
        };
        let s = p.to_string().ok()?;
        CoTaskMemFree(Some(p.as_ptr() as *const _));
        Some(s)
    })
}

/// 把 Explorer 的 file:// 地址还原成本地目录路径；非本地文件系统视图返回 None。
fn file_url_to_path(url: &str) -> Option<String> {
    let (rest, is_host) = if let Some(r) = url.strip_prefix("file:///") {
        (r, false)
    } else if let Some(r) = url.strip_prefix("file://") {
        (r, true)
    } else {
        return None;
    };
    let mut p = percent_decode(rest).replace('/', "\\");
    if is_host && !p.starts_with('\\') {
        p.insert_str(0, "\\\\"); // file://server/share → \\server\share
    }
    let path = PathBuf::from(p);
    if path.is_dir() {
        Some(path.to_string_lossy().to_string())
    } else {
        None
    }
}

/// 简单百分号解码（%XX，UTF-8；非法序列原样保留）。
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = hex_val(bytes[i + 1]);
            let l = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (h, l) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 创建目录命令（文件名清洗 + 重名自增）
// ---------------------------------------------------------------------------

/// Windows 建立文件的保留设备名（不分扩展名处判定，CON/NUL/COM1-9/LPT1-9）。
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 清洗为合法文件夹名：剔除 <>:"/\|?* 与 C0 控制符，去尾部点/空格，
/// 命中保留设备名追加下划线，空串/纯点回退「新建文件夹」。
pub fn sanitize_dir_name(input: &str) -> String {
    let mut s: String = input
        .trim()
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\u{0}'..='\u{1f}' | '\u{7f}'))
        .collect();
    s = s
        .trim_end_matches(|c| c == ' ' || c == '.')
        .to_string();
    let base = s.split('.').next().unwrap_or("").to_uppercase();
    if RESERVED.contains(&base.as_str()) {
        match s.find('.') {
            Some(pos) => s.insert(pos, '_'),
            None => s.push('_'),
        }
    }
    if s.is_empty() || s == "." || s == ".." {
        s = "新建文件夹".to_string();
    }
    s
}

/// 在 base 下为 dir 找一个不冲突的路径：自身未占用直接返回，占用则 ` (n)` 递增。
fn next_candidate(base: &Path, dir: &str) -> Result<PathBuf, String> {
    let mut candidate = base.join(dir);
    if !candidate.exists() {
        return Ok(candidate);
    }
    let mut n = 2usize;
    loop {
        candidate = base.join(format!("{dir} ({n})"));
        if !candidate.exists() {
            return Ok(candidate);
        }
        n += 1;
        if n > 9999 {
            return Err("同名文件夹过多".to_string());
        }
    }
}

/// 通知 Shell 目录已新建，Explorer 立即刷新显示（否则可能滞后数百毫秒，体感有延迟）。
fn notify_dir_created(path: &Path) {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SHChangeNotify(
            SHCNE_MKDIR,
            SHCNF_FLAGS(SHCNF_PATHW.0 | SHCNF_FLUSHNOWAIT.0),
            Some(wide.as_ptr() as *const core::ffi::c_void),
            None,
        );
    }
}

/// F4 创建命令：base=目标目录，name=（已翻译或原始）名字。
/// 清洗非法字符 → 重名自动追加 ` (2) (3)…` → fs::create_dir → 通知 Shell 即时刷新。
/// 成功返回实际创建路径；`async` 使其在异步线程执行，不占用事件循环（回车到关闭无阻塞）。
#[tauri::command]
pub async fn create_folder(app: AppHandle, base: String, name: String) -> Result<String, String> {
    let _ = app;
    let raw = name.trim();
    if raw.is_empty() {
        return Err("文件夹名字为空".to_string());
    }
    let dir = sanitize_dir_name(raw);
    let base = PathBuf::from(base.trim());
    if !base.is_dir() {
        return Err(format!("目标目录不存在：{}", base.to_string_lossy()));
    }
    let candidate = next_candidate(&base, &dir)?;
    std::fs::create_dir(&candidate).map_err(|e| format!("创建失败：{e}"))?;
    notify_dir_created(&candidate);
    Ok(candidate.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_invalid_and_trim() {
        assert_eq!(sanitize_dir_name("a/b\\c:d*e?f\"g<h>i|j*"), "abcdefghij");
        assert_eq!(sanitize_dir_name("  hello.  "), "hello");
        assert_eq!(sanitize_dir_name("hello."), "hello");
        assert_eq!(sanitize_dir_name("照片 2024"), "照片 2024");
        assert_eq!(sanitize_dir_name("CON"), "CON_");
        assert_eq!(sanitize_dir_name("nul.txt"), "nul_.txt");
        assert_eq!(sanitize_dir_name(".."), "新建文件夹");
        assert_eq!(sanitize_dir_name("   "), "新建文件夹");
        assert_eq!(sanitize_dir_name("COM9"), "COM9_");
    }

    #[test]
    fn sanitize_rejects_nested_paths() {
        // 严禁把名字当路径（路径注入防护）：/ 和 \ 必须被剔除
        let s = sanitize_dir_name("C:/Windows/System32");
        assert!(!s.contains('/') && !s.contains('\\'));
        assert!(!Path::new(&s).is_absolute());
    }

    #[test]
    fn percent_decode_basic() {
        assert_eq!(percent_decode("abc%20def%2F%E6%B5%8B"), "abc def/测");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn file_url_to_local_path_matches_temp() {
        let td = std::env::temp_dir();
        let url = format!("file:///{}", td.to_string_lossy().replace('\\', "/"));
        let p = file_url_to_path(&url).expect("temp dir 应被还原");
        assert!(Path::new(&p).is_dir(), "还原路径应为存在的目录：{p}");
    }

    #[test]
    fn create_folder_collides_with_increment() {
        let td = std::env::temp_dir().join(format!("linggo-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).expect("创建临时目录");
        let base = td.as_path();
        let d1 = next_candidate(base, "测试").expect("同名先生成原始路径");
        std::fs::create_dir(&d1).expect("创建原始目录");
        let d2 = next_candidate(base, "测试").expect("同名应自增");
        assert_eq!(d2.file_name().unwrap().to_string_lossy(), "测试 (2)");
        assert!(!d2.exists(), "自增路径不应已被占用");
        let _ = std::fs::remove_dir_all(&td);
    }
}