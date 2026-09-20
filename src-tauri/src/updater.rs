// Linggo 鏇存柊妫€娴嬶細鍚姩鍚庨潤榛樼敤 git ls-remote 鎺㈡祴 GitHub 鏈€鏂?tag 妫€鏌ユ柊鐗堟湰銆?// 鐞嗙敱锛?03 鎺掓煡缁撹锛夛細api.github.com 鍖垮悕鏈?60 娆?鏃?IP 闄愭祦涓斿叡浜嚭鍙?IP 甯歌秴闄愨啋403锛?// 鏀圭敤 git 鍗忚 ls-remote --tags锛堝厤璁よ瘉鍏嶉檺娴侊級鎵撳寘鏈€楂?tag 鎺㈡祴锛屾案涓嶈Е鍙?api 闄愭祦銆?// 浠呮鏌ヤ笉鑷姩涓嬭浇锛涙湁鏂扮増鏈?鈫?鎵樼洏绯荤粺姘旂悆 + F5 涓荤獥鍙充笅瑙掑渾褰㈡寜閽紱
// 銆屽叧闂洿鏂版娴嬨€嶅彧鍏冲惎鍔ㄨ嚜鍔ㄦ鏌ワ紝璁剧疆椤点€屾鏌ユ洿鏂般€嶆寜閽粛鍙墜鍔ㄥ己鍒舵鏌ャ€?// 鎺㈡祴鐢?git 鍛戒护琛岋紝鍏朵綑鍔熻兘闆惰仈缃戙€?
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_INFO, NIM_MODIFY, NIIF_INFO, NOTIFYICONDATAW};

/// GitHub 浠撳簱锛堝彂甯冨畨瑁呭寘涓?tag 鐗堟湰鍙凤級
const GITHUB_OWNER: &str = "WaneDay";
const GITHUB_REPO: &str = "Linggo";
/// 鍙戝竷椤碉紙鏃犵増鏈俊鎭椂鐨勫厹搴曡烦杞級
const RELEASE_PAGE_URL: &str = "https://github.com/WaneDay/Linggo/releases";

/// 鎵樼洏娑堟伅绐楀彛鍙ユ焺锛堢敱 lib.rs 鍦ㄥ垱寤烘墭鐩樺悗鍐欏叆锛涚敤浜庢寕姘旂悆閫氱煡锛?static TRAY_HWND: std::sync::OnceLock<isize> = std::sync::OnceLock::new();

pub fn set_tray_handle(hwnd: *mut core::ffi::c_void) {
    let _ = TRAY_HWND.set(hwnd as isize);
}

fn tray_hwnd() -> Option<HWND> {
    TRAY_HWND.get().map(|&raw| HWND(raw as *mut core::ffi::c_void))
}

/// 妫€鏌ョ粨鏋滐紙杩斿洖缁欏墠绔睍绀?/ 浜嬩欢璐熻浇锛?#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub has_update: bool,
    pub current: String,
    pub latest: String,
    pub url: String,
    pub note: Option<String>,
    pub error: Option<String>,
}

/// 瑙ｆ瀽 "v0.1.1" / "0.1.0" 涔嬬被鐨勫墠涓夋鏁板瓧鍙凤紱鏃犳硶璇嗗埆杩斿洖 None銆?fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
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


/// 鎷夊彇 GitHub Releases 鏈€鏂颁竴鏉★紙鍚?tag / 鍙戝竷椤?/ 姝ｆ枃锛?/// 鎷夊彇 GitHub 鏈€鏂?tag锛堣交閲忥紱鍏嶈璇佸厤闄愭祦锛氱洿鎺?ls-remote --tags 璧?git 鍗忚锛?/// 涓嶇 api.github.com 鈥斺€?璇ョ鐐瑰尶鍚嶉厤棰?60 娆?鏃?IP锛屽叡浜嚭鍙?IP 鏋佹槗琚檺娴佸脊 403锛夈€?fn latest_release_json() -> Result<serde_json::Value, String> {
    // 鍙岄€氶亾锛氣憼 api.github.com/releases/latest锛堜富锛涙湰鏈哄疄娴嬮€氫笖鍏嶉檺娴侀厤棰濆皻浣欙紝鑳芥嬁 tag锛夛紱
    // 鈶?浠呭綋 api 鏄庣‘ 403/瓒呮椂 鎵嶅厹搴?git ls-remote锛堢璧?git 鍗忚浣嗗厤璁よ瘉锛汣REATE_NO_WINDOW 闈欓粯锛夈€?    let api_url = format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest");
    // 涓婚€氶亾鐢?Windows 鑷甫鐨?System32\curl.exe锛堢函 std 瀛愯繘绋嬨€丆REATE_NO_WINDOW 闈欓粯锛?    // 鏃犳柊渚濊禆 reqwest锛?.13.0 瀹炴祴 200 鎷垮埌 v0.1.1锛夆啋 鑳芥鏌ユ洿鏂帮紱鍙湁 curl 闈?200 鎵嶈惤 git 鍏滃簳銆?    let curl_exe = if cfg!(target_arch = "x86_64") {
        r"C:\Windows\System32\curl.exe"

        r"C:\Windows\SysWOW64\curl.exe"
    };
    let api_res = std::process::Command::new(curl_exe)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW锛歝url 瀛愯繘绋嬮潤榛樺悗鍙帮紝涓嶅脊鎺у埗鍙扮獥
        .args(["--silent", "--fail", "--max-time", "20", "-L", "-H", "Accept: application/vnd.github+json",
               "-H", "User-Agent: linggo-check", &api_url])
        .output();
    if let Ok(o) = api_res {
        if o.status.success() {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&o.stdout) {
                return Ok(v);
            }
        }
    }
    let tags_url = format!("https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}.git");
    let out = std::process::Command::new("git")
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW锛歡it 瀛愯繘绋嬮潤榛樺悗鍙帮紝涓嶅脊鎺у埗鍙扮獥
        .args(["ls-remote", "--tags", &tags_url])
        .output()
        .map_err(|e| format!("git 涓嶅彲鐢細{e}"))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(if msg.trim().is_empty() {
            "git ls-remote 鎺㈡祴鏍囩澶辫触".to_string()
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
        None => Err("浠撳簱鏆傛棤鍙戝竷鐗堟湰".to_string()),
    }
}

/// 缃戠粶妫€鏌?+ 鐗堟湰姣旇緝锛堜笉 panic锛夛紱缃戠粶澶辫触鏃舵妸閿欒鍐欏叆 info.error銆?pub fn check_remote(_app: &AppHandle) -> UpdateInfo {
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
                    n.push('鈥?);
                }
                n
            });
            match (parse_version(&info.latest), parse_version(&info.current)) {
                (Some(remote), Some(local)) => info.has_update = remote > local,
                _ => info.error = Some("杩滅▼涓庢湰鍦扮増鏈彿鏃犳硶姣旇緝".to_string()),
            }
        }
        Err(e) => {
            info.error = Some(if e.contains("HTTP 404") {
                "浠撳簱鏆傛棤鍙戝竷鐗堟湰".to_string()
            } else {
                e
            });
        }
    }
    info
}

/// 鏈夋柊鐗堟湰鏃讹細骞挎挱缁欐墍鏈夌獥鍙?鈫?涓荤嚎绋嬪脊鎵樼洏姘旂悆锛堜笌鎵樼洏鍚岀嚎绋嬶紝淇濊瘉 Shell_NotifyIcon 鏈夋晥锛夈€?fn notify_update(app: &AppHandle, info: &UpdateInfo) {
    let h = app.clone();
    let payload = info.clone();
    let latest = info.latest.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = h.emit("update-available", &payload);
        show_balloon(&latest, &payload.url);
    });
}

/// 鎵樼洏绯荤粺姘旂悆锛圢IF_INFO锛夈€傚浘鏍囷細榛樿搴旂敤鍥炬爣锛涙爣棰?姝ｆ枃 UTF-16 鍐欏叆瀹氶暱缂撳啿銆?fn show_balloon(latest: &str, url: &str) {
    let Some(hwnd) = tray_hwnd() else { return };
    let title = format!("Linggo 鏂扮増鏈?v{latest}");
    let msg = format!("鍙戠幇鏂扮増鏈?v{latest}銆傛墦寮€ Linggo 涓荤獥鍙ｏ紝鐐瑰嚮鍙充笅瑙掑渾褰㈡寜閽煡鐪嬪彂甯冮〉銆俓n{url}");
    let mut nid = NOTIFYICONDATAW::default();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 0; // 鏈簲鐢ㄥ敮涓€鎵樼洏锛宼ray-icon 鍐呴儴璁℃暟鍣ㄩ涓紝uID 蹇呬负 0
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

/// 鍚姩鑷姩妫€鏌ワ細寤惰繜 5s 寰呯晫闈㈠氨缁紱寮€鍏冲叧闂垯闈欓粯璺宠繃銆傚叏绋嬩笉鍛婅銆?pub fn spawn_auto_check(app: AppHandle) {
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

/// 璁剧疆椤点€屾鏌ユ洿鏂般€嶏紙鎵嬪姩锛夛細涓嶅彈寮€鍏抽檺鍒讹紱鏈夋洿鏂板悓鏃惰Е鍙戞皵鐞冧笌涓荤獥鎸夐挳銆?#[tauri::command]
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
        utf16_into("涓枃abc", &mut buf);
        assert_eq!(buf, [20013, 25991, 97, 0]);
    }
}

