// Linggo 网络工具：Windows 原生 HTTP GET（WinINet）。
// 仅用于「检查更新」；复用系统 HTTPS 证书、代理与拨号配置，不引入任何第三方网络依赖。
// 其余功能保持零联网，符合产品「完全离线」定位。

use windows::core::PCWSTR;
use windows::Win32::Networking::WinInet::{
    HttpQueryInfoW, INTERNET_FLAG_NO_CACHE_WRITE, INTERNET_FLAG_RELOAD, INTERNET_FLAG_SECURE,
    INTERNET_OPEN_TYPE_PRECONFIG, InternetCloseHandle, InternetOpenUrlW, InternetOpenW,
    InternetReadFile, InternetSetOptionW, HTTP_QUERY_FLAG_NUMBER, HTTP_QUERY_STATUS_CODE,
    INTERNET_OPTION_CONNECT_TIMEOUT, INTERNET_OPTION_RECEIVE_TIMEOUT,
};

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_err() -> u32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(0) as u32
}

fn err_text(code: u32) -> String {
    match code {
        12007 => "DNS 解析失败（无法找到服务器，请检查网络）".to_string(),
        12002 | 12011 | 12012 | 12038 => "请求超时（网络不通或响应过慢）".to_string(),
        12029 | 12015 | 12031 => "无法连接到 GitHub（网络受限或离线）".to_string(),
        12044 => "HTTPS 证书校验失败".to_string(),
        12005 | 12004 => "URL 地址不合法".to_string(),
        _ => format!("网络错误（WinINet 0x{code:X}）"),
    }
}

/// HTTPS GET 文本；timeout_secs 为连接/接收超时。
/// 失败返回中文错误描述。仅允许 http/https 协议。
pub fn http_get_text(url: &str, timeout_secs: u32, user_agent: &str) -> Result<String, String> {
    let low = url.to_ascii_lowercase();
    if !low.starts_with("https://") && !low.starts_with("http://") {
        return Err("仅支持 http/https 地址".to_string());
    }

    let agent = to_wide(user_agent);
    let h = unsafe {
        InternetOpenW(
            PCWSTR(agent.as_ptr()),
            INTERNET_OPEN_TYPE_PRECONFIG.0,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        )
    };
    if h.is_null() {
        return Err(err_text(last_err()));
    }
    // 连接/接收超时（毫秒）
    let c = timeout_secs * 1000;
    unsafe {
        let _ = InternetSetOptionW(Some(h as *const core::ffi::c_void), INTERNET_OPTION_CONNECT_TIMEOUT, Some(&c as *const u32 as *const _), 4);
        let _ = InternetSetOptionW(Some(h as *const core::ffi::c_void), INTERNET_OPTION_RECEIVE_TIMEOUT, Some(&c as *const u32 as *const _), 4);
    }

    let curl = to_wide(url);
    let flags = INTERNET_FLAG_RELOAD | INTERNET_FLAG_SECURE | INTERNET_FLAG_NO_CACHE_WRITE;
    let req = unsafe { InternetOpenUrlW(h, PCWSTR(curl.as_ptr()), None, flags, 0) };
    if req.is_null() {
        let e = err_text(last_err());
        unsafe { let _ = InternetCloseHandle(h); }
        return Err(e);
    }

    // 读取 HTTP 状态码（17 = HTTP_QUERY_FLAG_NUMBER | HTTP_QUERY_STATUS_CODE）
    let mut status: u32 = 0;
    let mut status_len: u32 = std::mem::size_of::<u32>() as u32;
    let status_ok = unsafe {
        HttpQueryInfoW(
            req,
            HTTP_QUERY_STATUS_CODE | HTTP_QUERY_FLAG_NUMBER,
            Some(&mut status as *mut u32 as *mut core::ffi::c_void),
            &mut status_len,
            None,
        )
        .is_ok()
    };

    let mut body: Vec<u8> = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let mut read: u32 = 0;
        unsafe {
            if InternetReadFile(req, buf.as_mut_ptr() as *mut core::ffi::c_void, buf.len() as u32, &mut read).is_err()
            {
                break;
            }
        }
        if read == 0 {
            break;
        }
        body.extend_from_slice(&buf[..read as usize]);
    }
    unsafe {
        let _ = InternetCloseHandle(req);
        let _ = InternetCloseHandle(h);
    }

    if status_ok && status >= 400 {
        return Err(format!("GitHub 返回 HTTP {status}"));
    }
    let text = String::from_utf8_lossy(&body).to_string();
    if text.is_empty() {
        return Err("响应为空".to_string());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_encoder_terminates_zero() {
        let v = to_wide("abc");
        assert_eq!(v, vec![97, 98, 99, 0]);
    }

    #[test]
    fn err_text_maps_http_to_friendly() {
        assert!(err_text(12007).contains("DNS"));
        assert!(err_text(12002).contains("超时"));
        assert!(err_text(99999).starts_with("网络错误"));
    }
}