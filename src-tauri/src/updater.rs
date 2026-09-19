// Linggo 更新检测：启动后静默用 git ls-remote 探测 GitHub 最新 tag 检查新版本。
// 理由（403 排查结论）：api.github.com 匿名有 60 次/时/IP 限流且共享出口 IP 常超限→403；
// 改用 git 协议 ls-remote --tags（免认证免限流）打包最高 tag 探测，永不触发 api 限流。
// 仅检查不自动下载；有新版本 → 托盘系统气球 + F5 主窗右下角圆形按钮；
// 「关闭更新检测」只关启动自动检查，设置页「检查更新」按钮仍可手动强制检查。
// 探测用 git 命令行，其余功能零联网。

use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_INFO, NIM_MODIFY, NIIF_INFO, NOTIFYICONDATAW};

/// GitHub 仓库（发布安装包与 tag 版本号）
const GITHUB_OWNER: &str = "WaneDay";
const GITHUB_REPO: &str = "Linggo";
/// 发布页（无版本信息时的兜底跳转）
const RELEASE_PAGE_URL: &str = "https://github.com/WaneDay/Linggo/releases";

/// 托盘消息窗口句柄（由 lib.rs 在创建托盘后写入；用于挂气球通知）
static TRAY_HWND: std::sync::OnceLock<isize> = std::sync::OnceLock::new();

pub fn set_tray_handle(hwnd: *mut core::ffi::c_void) {
    let _ = TRAY_HWND.set(hwnd as isize);
}

fn tray_hwnd() -> Option<HWND> {
    TRAY_HWND.get().map(|&raw| HWND(raw as *mut core::ffi::c_void))
}

/// 检查结果（返回给前端展示 / 事件负载）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub has_update: bool,
    pub current: String,
    pub latest: String,
    pub url: String,
    pub note: Option<String>,
    pub error: Option<String>,
}

/// 解析 "v0.1.1" / "0.1.0" 之类的前三段数字号；无法识别返回 None。
fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let t = s.trim().trim_start_matches(['v', 'V']);
    let mut nums = [0u32; 3];
    let mut i = 0usize;
    for seg in t.split(|c: char| !c.is_ascii_digit()) {
        if seg.is_empty() {
            continue;
        }
        if i >= 3 {
            break;
        }
        nums[i] = seg.parse::<u32>().ok()?;
        i += 1;
    }
    if i == 0 {
        None
    } else {
        Some((nums[0], nums[1], nums[2]))
    }
}

/// 拉取 GitHub Releases 最新一条（含 tag / 发布页 / 正文）
/// 拉取 GitHub 最新 tag（轻量；免认证免限流：直接 ls-remote --tags 走 git 协议，
/// 不碰 api.github.com —— 该端点匿名配额 60 次/时/IP，共享出口 IP 极易被限流弹 403）。
fn latest_release_json() -> Result<serde_json::Value, String> {
    let tags_url = format!("https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}.git");
    let out = std::process::Command::new("git")
        .args(["ls-remote", "--tags", &tags_url])
        .output()
        .map_err(|e| format!("git 不可用：{e}"))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(if msg.trim().is_empty() {
            "git ls-remote 探测标签失败".to_string()
        } else {
            msg.trim().to_string()
        });
    }
    let txt = String::from_utf8_lossy(&out.stdout);
    let mut best_ver: (u32, u32, u32) = (0, 0, 0);
    let mut best_tag: Option<String> = None;
    for line in txt.lines() {
        let name = match line.split_whitespace().nth(1) {
            Some(n) => n.trim(),
            None => continue,
        };
        let Some(stripped) = name.strip_prefix("refs/tags/") else {
            continue;
        };
        if stripped.ends_with("^{}") {
            continue;
        }
        if let Some(v) = parse_version(stripped) {
            if v > best_ver {
                best_ver = v;
                best_tag = Some(stripped.to_string());
            }
        }
    }
    match best_tag {
        Some(tag) => Ok(serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases/tag/{tag}")
        })),
        None => Err("仓库暂无发布版本".to_string()),
    }
}

/// 网络检查 + 版本比较（不 panic）；网络失败时把错误写入 info.error。
pub fn check_remote(_app: &AppHandle) -> UpdateInfo {
    let current = crate::constants::APP_VERSION.to_string();
    let mut info = UpdateInfo {
        has_update: false,
        current,
        latest: String::new(),
        url: RELEASE_PAGE_URL.to_string(),
        note: None,
        error: None,
    };
    match latest_release_json() {
        Ok(v) => {
            info.latest = v["tag_name"].as_str().unwrap_or("").to_string();
            info.url = v["html_url"].as_str().unwrap_or(RELEASE_PAGE_URL).to_string();
            info.note = v["body"].as_str().map(|s| {
                let mut n = s.trim().to_string();
                if n.chars().count() > 200 {
                    n = n.chars().take(200).collect();
                    n.push('…');
                }
                n
            });
            match (parse_version(&info.latest), parse_version(&info.current)) {
                (Some(remote), Some(local)) => info.has_update = remote > local,
                _ => info.error = Some("远程与本地版本号无法比较".to_string()),
            }
        }
        Err(e) => {
            info.error = Some(if e.contains("HTTP 404") {
                "仓库暂无发布版本".to_string()
            } else {
                e
            });
        }
    }
    info
}

/// 有新版本时：广播给所有窗口 → 主线程弹托盘气球（与托盘同线程，保证 Shell_NotifyIcon 有效）。
fn notify_update(app: &AppHandle, info: &UpdateInfo) {
    let h = app.clone();
    let payload = info.clone();
    let latest = info.latest.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = h.emit("update-available", &payload);
        show_balloon(&latest, &payload.url);
    });
}

/// 托盘系统气球（NIF_INFO）。图标：默认应用图标；标题/正文 UTF-16 写入定长缓冲。
fn show_balloon(latest: &str, url: &str) {
    let Some(hwnd) = tray_hwnd() else { return };
    let title = format!("Linggo 新版本 v{latest}");
    let msg = format!("发现新版本 v{latest}。打开 Linggo 主窗口，点击右下角圆形按钮查看发布页。\n{url}");
    let mut nid = NOTIFYICONDATAW::default();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 0; // 本应用唯一托盘，tray-icon 内部计数器首个，uID 必为 0
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = NIIF_INFO;
    utf16_into(&title, &mut nid.szInfoTitle);
    utf16_into(&msg, &mut nid.szInfo);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

fn utf16_into(s: &str, buf: &mut [u16]) {
    let n = buf.len().saturating_sub(1);
    for (i, u) in s.encode_utf16().take(n).enumerate() {
        buf[i] = u;
    }
}

/// 启动自动检查：延迟 5s 待界面就绪；开关关闭则静默跳过。全程不告警。
pub fn spawn_auto_check(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(5));
        if !crate::settings::safe_current(&app).check_updates_enabled {
            return;
        }
        let info = check_remote(&app);
        if info.has_update {
            notify_update(&app, &info);
        }
    });
}

/// 设置页「检查更新」（手动）：不受开关限制；有更新同时触发气球与主窗按钮。
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let info = check_remote(&app);
        if let Some(e) = &info.error {
            return Err(e.clone());
        }
        if info.has_update {
            notify_update(&app, &info);
        }
        Ok(info)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parse_handles_prefix() {
        assert_eq!(parse_version("v0.1.1"), Some((0, 1, 1)));
        assert_eq!(parse_version("0.10.3"), Some((0, 10, 3)));
        assert_eq!(parse_version("v1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("0.1.1-beta.2"), Some((0, 1, 1)));
        assert_eq!(parse_version("abc"), None);
    }

    #[test]
    fn newer_than() {
        assert!((0, 1, 2) > (0, 1, 1));
        assert!(((0, 9, 0) > (0, 10, 0)) == false);
        assert!((1, 0, 0) > (0, 9, 9));
    }

    #[test]
    fn utf16_into_truncates() {
        let mut buf = [0u16; 4];
        utf16_into("中文abc", &mut buf);
        assert_eq!(buf, [20013, 25991, 97, 0]);
    }
}