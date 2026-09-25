// Linggo Google Translate 引擎：走 Google 网页版翻译端点，无需用户 API Key、无需登录。
//
// 端点（与参考项目「陪读蛙翻译 read-frog」的 src/utils/host/translate/api/google.ts 同源）：
//   主通道  POST https://translate-pa.googleapis.com/v1/translateHtml
//           Header: X-Goog-API-Key: AIzaSy...（Google 网页版自带公开 Key）
//                   Content-Type: application/json+protobuf
//           Body  : [[[<转义后的文本>], <sl>, <tl>], "wt_lib"]
//           响应 : [["译文"]]        —— 译文仍是 HTML 编码
//   兜底通道 GET  https://translate.googleapis.com/translate_a/single?client=gtx&sl=&tl=&dt=t&q=
//           （主通道被限流/改版时仍有机会出译文；响应 [[["译文"],...]]）
//
// 两端点都把请求当 HTML 解析，所以纯文本必须先转义（否则 "a & b" 被当实体、"x <b" 被当标签吞掉），
// 响应也要反转义一次（escape_text / unescape_text）。
//
// 换行：translateHtml 把字面 "\n" 当可折叠 HTML 空白，多行会被压成一行（参考项目实测：&#10; 也会折叠）。
// 沿用其做法：用 <br data-linggo-lb="1"> 标记「对」把行粘成**一个**请求项（整段单发，sl=auto 才不会在
// 短行上误判语种），响应再按标记对切回多行并还原源行的缩进/项目符号。
//
// 网络策略（与旧引擎回落逻辑一致）：
//   - 单次失败（断网 / DNS / 超时 / 非 2xx / 载荷异常）→ 返回 Err 或 None，
//     由上层 llama_backend 按 settings.engine_order 依次尝试 opus → nllb → gguf；
//   - 失败后写「负缓存」FAIL_COOLDOWN，期间直接返回 None（不重复付 3~12s 超时成本），
//     避免断网时每次 F1/F3 都先干等一遍 Google；
//   - 超时压得比本地引擎大但有限，保证 Google 排首位时体感不拖垮。
//
// 传输层用 Windows 自带 curl.exe 子进程（与 updater.rs / mt_engine.rs 下载同款）：
// 不引入 reqwest 等新依赖；CREATE_NO_WINDOW 保证 GUI 进程不弹控制台窗口。
// 代理：先读 https_proxy/HTTP_PROXY 等环境变量，再读 WinINET 系统代理（Clash/V2Ray 等本机代理
// 场景下 curl 不会自动继承系统代理，必须显式 --proxy，否则表现为「连不上」）。

use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 主通道：Google 网页版 HTML 翻译端点
const HTML_URL: &str = "https://translate-pa.googleapis.com/v1/translateHtml";
/// 主通道公开 API Key（Google 网页翻译前端自带，非用户密钥）
const HTML_API_KEY: &str = "AIzaSyATBXajvzQLTDHEQbcpq0Ihe0vWDHmO520";
/// 客户端标识（网页版 wt_lib）
const HTML_CLIENT: &str = "wt_lib";
/// 兜底通道：老的 gtx 单条接口
const LEGACY_URL: &str = "https://translate.googleapis.com/translate_a/single";

/// 连接超时（秒）：被墙 / 断网时尽快失败，把时间留给后续本地引擎
const CONNECT_TIMEOUT_SECS: &str = "4";
/// 单请求总超时（秒）
const MAX_TIME_SECS: &str = "12";
/// 探活超时（秒）：与参考项目一致，够短不拖启动
const PROBE_TIMEOUT_SECS: &str = "4";
/// 失败后的负缓存时长：期间不再重试 Google，直接回落下一引擎
const FAIL_COOLDOWN: Duration = Duration::from_secs(180);
/// 批量（F3 覆盖原文）并发请求数：Google 单请求 ~0.3~0.6s，串行 30 行太慢，6 路并发兼顾速度与限流
const BATCH_CONCURRENCY: usize = 6;
/// 系统代理缓存时长（代理可被用户随时改，缓存 60s 后重读）
const PROXY_TTL: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// 可达性状态（负缓存 + 最近一次结果，供设置面板展示）
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleStatus {
    /// 最近一次是否成功（null = 尚未尝试过）
    pub reachable: Option<bool>,
    /// 面向用户的可读说明
    pub note: String,
    /// 当前是否处于失败冷却期（冷却中直接回落下一引擎）
    pub cooling: bool,
    /// 探测到的系统代理（空 = 直连）
    pub proxy: String,
}

#[derive(Clone, Copy)]
struct Reach {
    ok: bool,
    at: Instant,
}

fn reach_slot() -> &'static Mutex<Option<Reach>> {
    static R: std::sync::OnceLock<Mutex<Option<Reach>>> = std::sync::OnceLock::new();
    R.get_or_init(|| Mutex::new(None))
}

fn record(ok: bool) {
    if let Ok(mut g) = reach_slot().lock() {
        *g = Some(Reach { ok, at: Instant::now() });
    }
}

fn last_reach() -> Option<Reach> {
    *reach_slot().lock().unwrap_or_else(|e| e.into_inner())
}

/// 失败冷却中 → true（此时应立即返回 None 回落下一引擎）
fn cooling() -> bool {
    matches!(last_reach(), Some(r) if !r.ok && r.at.elapsed() < FAIL_COOLDOWN)
}

/// 最近状态（供设置面板展示；不触网）
pub fn status() -> GoogleStatus {
    let g = last_reach();
    let cooling = matches!(g, Some(r) if !r.ok && r.at.elapsed() < FAIL_COOLDOWN);
    let note = match g {
        None => "尚未检测：默认优先尝试，失败自动按排序回落下一引擎".to_string(),
        Some(r) if r.ok => "网络可用：Google Translate 已连通（引擎排序默认置顶）".to_string(),
        Some(r) if r.at.elapsed() < FAIL_COOLDOWN => format!(
            "最近一次不可用（{:.0}s 前），冷却期内直接回落下一引擎",
            r.at.elapsed().as_secs_f32()
        ),
        Some(_) => "最近一次不可用，已过冷却期：下次翻译会重新尝试".to_string(),
    };
    GoogleStatus { reachable: g.map(|r| r.ok), note, cooling, proxy: proxy_display() }
}

// ---------------------------------------------------------------------------
// HTML 转义 / 反转义（严格一次）
// ---------------------------------------------------------------------------

/// 出站转义：translateHtml 把请求当 HTML 解析，纯文本里的 & < > 必须编码
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
    out
}

/// 入站反转义：响应仍是 HTML 编码，只解码一次。
/// 先匹配具名实体，再匹配数字实体（&#NN; / &#xHH;）；`&amp;lt;` 这类只解一层。
fn unescape_text(s: &str) -> String {
    const NAMED: [(&str, char); 7] = [
        ("&amp;", '&'),
        ("&lt;", '<'),
        ("&gt;", '>'),
        ("&quot;", '"'),
        ("&#39;", '\''),
        ("&nbsp;", '\u{a0}'),
        ("&apos;", '\''),
    ];
    let b: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == '&' {
            let rest: String = b[i..].iter().collect();
            let mut done = None;
            for (pat, ch) in NAMED {
                if rest.starts_with(pat) {
                    done = Some((pat.len(), ch));
                    break;
                }
            }
            if let Some((len, ch)) = done {
                out.push(ch);
                i += len;
                continue;
            }
            if rest.starts_with("&#") {
                if let Some(end) = rest[2..].find(';') {
                    let body = &rest[2..2 + end];
                    let code = match body.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => body.parse::<u32>().ok(),
                    };
                    if let Some(c) = code.and_then(char::from_u32) {
                        out.push(c);
                        i += 2 + end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// 换行保留（<br data-linggo-lb="1"> 标记对）
// ---------------------------------------------------------------------------

const LB_MARKER: &str = r#"<br data-linggo-lb="1">"#;
/// 自闭合写法（部分端点/序列化会输出 `<br ... />`）
const LB_MARKER_SLASH: [&str; 2] = [r#"<br data-linggo-lb="1"/>"#, r#"<br data-linggo-lb="1" />"#];
/// 段内落单的标记（模型偶发把一个标记留在段内）：折成空格，绝不能让标签泄进译文
fn strip_lone_markers(s: &str) -> String {
    let mut t = s.to_string();
    for m in [LB_MARKER, LB_MARKER_SLASH[0], LB_MARKER_SLASH[1]] {
        t = t.replace(m, " ");
    }
    t
}

/// 段数漂移时的降级：把标记对 / 落单标记统一折成换行，只保结构不保前缀
fn markers_to_newlines(s: &str) -> String {
    let mut t = s.to_string();
    for m in [LB_MARKER, LB_MARKER_SLASH[0], LB_MARKER_SLASH[1]] {
        t = t.replace(m, "\n");
    }
    t
}

/// 源行的「缩进 + 项目符号」前缀：翻译模型会吃掉或改写它们，故响应侧统一还原成源前缀
struct PreservedLine {
    prefix: String,
    content: String,
}

/// dash 族项目符号；必须后接空白，避免 "-5°C" / "*强调*" 被误判
const BULLETS: [char; 10] = ['-', '–', '—', '•', '·', '▪', '◦', '‣', '⁃', '*'];

fn split_preserved_lines(text: &str) -> Vec<PreservedLine> {
    text.split('\n')
        .map(|raw| {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            let rest = &line[indent.len()..];
            let mut prefix = indent.clone();
            if let Some(first) = rest.chars().next() {
                if BULLETS.contains(&first) {
                    let tail = &rest[first.len_utf8()..];
                    let ws: String = tail.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                    if !ws.is_empty() {
                        prefix.push(first);
                        prefix.push_str(&ws);
                    }
                }
            }
            PreservedLine { prefix, content: rest.to_string() }
        })
        .collect()
}

/// 剥掉译文自带的缩进 / 项目符号，换回源行前缀
fn reassemble_line(line: &PreservedLine, translation: &str) -> String {
    let t = translation.trim_start_matches([' ', '\t']);
    let t = match t.chars().next() {
        Some(c) if BULLETS.contains(&c) => t[c.len_utf8()..].trim_start_matches([' ', '\t']),
        _ => t,
    };
    format!("{}{}", line.prefix, t)
}

// ---------------------------------------------------------------------------
// 系统代理（curl 不继承 WinINET；本机 Clash/V2Ray 场景必须显式 --proxy）
// ---------------------------------------------------------------------------

fn proxy_slot() -> &'static Mutex<(Option<Instant>, Vec<String>)> {
    static P: std::sync::OnceLock<Mutex<(Option<Instant>, Vec<String>)>> = std::sync::OnceLock::new();
    P.get_or_init(|| Mutex::new((None, Vec::new())))
}

/// 环境变量里的代理（标准写法，优先于系统设置）
fn proxy_from_env() -> Option<String> {
    for k in [
        "https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY", "http_proxy", "HTTP_PROXY",
    ] {
        if let Ok(v) = std::env::var(k) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// WinINET 系统代理：`ProxyEnable=1` 时读 `ProxyServer`
/// 形如 `127.0.0.1:7897` 或 `http=host:p;https=host:p;socks=host:p`（优先 https，其次 socks/http）
fn proxy_from_wininet() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let k = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    let enabled: u32 = k.get_value("ProxyEnable").ok()?;
    if enabled == 0 {
        return None;
    }
    let server: String = k.get_value("ProxyServer").ok()?;
    let server = server.trim();
    if server.is_empty() {
        return None;
    }
    if !server.contains('=') {
        return Some(normalize_proxy(server));
    }
    let pick = |key: &str| {
        server
            .split(';')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.trim().to_string())
    };
    pick("https")
        .or_else(|| pick("socks"))
        .or_else(|| pick("http"))
        .map(|v| normalize_proxy(&v))
}

/// 补全 scheme（curl 认 `http://host:port`；裸 `host:port` 也行但显式更稳）
fn normalize_proxy(v: &str) -> String {
    let v = v.trim();
    if v.starts_with("http://") || v.starts_with("https://") || v.starts_with("socks5://")
        || v.starts_with("socks5h://")
    {
        v.to_string()
    } else {
        format!("http://{v}")
    }
}

/// 取 `--proxy <addr>` 参数（带 60s 缓存；无代理返回空）
fn proxy_args() -> Vec<String> {
    let mut g = proxy_slot().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(at) = g.0 {
        if at.elapsed() < PROXY_TTL {
            return g.1.clone();
        }
    }
    let addr = proxy_from_env().or_else(proxy_from_wininet);
    let args = match addr {
        Some(a) => vec!["--proxy".to_string(), a],
        None => Vec::new(),
    };
    *g = (Some(Instant::now()), args.clone());
    args
}

fn proxy_display() -> String {
    let a = proxy_args();
    if a.len() == 2 {
        a[1].clone()
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// HTTP（curl.exe 子进程；CREATE_NO_WINDOW 静默；body 走 stdin 不落临时文件）
// ---------------------------------------------------------------------------

fn curl_exe() -> PathBuf {
    if cfg!(target_arch = "x86_64") {
        PathBuf::from(r"C:\Windows\System32\curl.exe")
    } else {
        PathBuf::from(r"C:\Windows\SysWOW64\curl.exe")
    }
}

struct HttpOut {
    ok: bool,
    body: String,
    err: String,
}

fn curl(url: &str, method: &str, headers: &[(&str, &str)], body: Option<&str>, timeout: &str) -> HttpOut {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;

    let exe = curl_exe();
    // 优先系统 curl（Win10 1803+ 自带）；缺失时退回 PATH 里的 curl.exe
    let mut cmd = if exe.is_file() {
        std::process::Command::new(&exe)
    } else {
        std::process::Command::new("curl.exe")
    };
    cmd.creation_flags(0x0800_0000) // CREATE_NO_WINDOW：不弹控制台窗口
        .args(["--silent", "--show-error", "--no-buffer", "-X", method])
        .args(["--connect-timeout", CONNECT_TIMEOUT_SECS])
        .args(["--max-time", timeout])
        .args(proxy_args())
        // 必须显式接管 stdout/stderr：否则 curl 的响应体直接继承控制台，
        // wait_with_output() 只能拿到空管道，解析恒为 EOF。
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in headers {
        cmd.arg("-H").arg(format!("{k}: {v}"));
    }
    if body.is_some() {
        cmd.args(["--data-binary", "@-"]).stdin(Stdio::piped());
    }
    cmd.arg(url);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return HttpOut { ok: false, body: String::new(), err: format!("curl 启动失败: {e}") }
        }
    };
    if let Some(b) = body {
        if let Some(mut si) = child.stdin.take() {
            if let Err(e) = si.write_all(b.as_bytes()).and_then(|_| si.flush()) {
                let _ = child.kill();
                let _ = child.wait();
                return HttpOut { ok: false, body: String::new(), err: format!("写入请求体失败: {e}") };
            }
        }
    }
    match child.wait_with_output() {
        Ok(o) if o.status.success() => HttpOut {
            ok: true,
            body: String::from_utf8_lossy(&o.stdout).to_string(),
            err: String::new(),
        },
        Ok(o) => {
            let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
            HttpOut {
                ok: false,
                body: String::new(),
                err: if e.is_empty() { format!("curl 退出码 {:?}", o.status.code()) } else { e },
            }
        }
        Err(e) => HttpOut { ok: false, body: String::new(), err: format!("curl 等待失败: {e}") },
    }
}

fn post_html(request_text: &str, sl: &str, tl: &str, timeout: &str) -> HttpOut {
    // 载荷形状：[[[<转义后的文本>], <sl>, <tl>], "wt_lib"]
    let payload = serde_json::json!([
        [ [request_text], sl, tl ],
        HTML_CLIENT
    ]);
    let body = serde_json::to_string(&payload).unwrap_or_else(|_| "[]".to_string());
    curl(
        HTML_URL,
        "POST",
        &[
            ("X-Goog-API-Key", HTML_API_KEY),
            ("Content-Type", "application/json+protobuf"),
        ],
        Some(&body),
        timeout,
    )
}

fn get_legacy(text: &str, sl: &str, tl: &str, timeout: &str) -> HttpOut {
    let url = format!(
        "{LEGACY_URL}?client=gtx&sl={}&tl={}&dt=t&strip=1&nonced=1&q={}",
        url_encode(sl),
        url_encode(tl),
        url_encode(text)
    );
    curl(&url, "GET", &[], None, timeout)
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 响应解析
// ---------------------------------------------------------------------------

/// 主通道响应：`[["译文"]]`（外层被 strip 成 `[[...]]`）
fn parse_html_resp(body: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("响应不是合法 JSON: {e}"))?;
    let seg = v
        .as_array()
        .and_then(|a| a.first())
        .and_then(|a| a.as_array())
        .ok_or("响应结构异常（缺少译文数组）")?;
    let text = seg.first().and_then(|x| x.as_str()).ok_or("响应结构异常（译文不是字符串）")?;
    Ok(text.to_string())
}

/// 兜底响应解析。真实 `translate_a/single` 形状为
/// `[[["译文","原文",...],["译文2","原文2",...]],null,"en",...]`，
/// 即首元素是「句子数组」、每个句子是「译文优先」的数组；同时兼容扁平的
/// `[["片段1","片段2"],null,"en"]`（此时各片段都是译文，需整体拼接）。
fn parse_legacy_resp(body: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("响应不是合法 JSON: {e}"))?;
    let arr = v.as_array().ok_or("响应结构异常（不是数组）")?;
    let mut out = String::new();
    if let Some(first) = arr.first().and_then(|x| x.as_array()) {
        let nested = first.iter().any(|seg| seg.is_array());
        for seg in first.iter() {
            let picked = if nested {
                seg.as_array().and_then(|a| a.first()).and_then(|x| x.as_str())
            } else {
                seg.as_str()
            };
            if let Some(t) = picked {
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        // 兜底：扫描所有顶层数组元素的首元素
        for seg in arr.iter() {
            if let Some(t) = seg.as_array().and_then(|c| c.first()).and_then(|x| x.as_str()) {
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        return Err("响应结构异常（译文为空）".to_string());
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 核心：保留换行地请求一次并还原行结构
// ---------------------------------------------------------------------------

/// 把 text 送出并拿回译文：多行走标记对粘成一个请求项，响应按标记对切回多行。
fn request_text(text: &str, sl: &str, tl: &str, timeout: &str) -> Result<String, String> {
    let lines = split_preserved_lines(text);
    let multiline = lines.len() > 1;
    let payload = if multiline {
        lines
            .iter()
            .map(|l| escape_text(&l.content))
            .collect::<Vec<_>>()
            .join(&format!("{LB_MARKER}{LB_MARKER}"))
    } else {
        escape_text(&lines[0].content)
    };

    // 主通道；失败或结构异常 → 兜底通道
    let raw = {
        let p = post_html(&payload, sl, tl, timeout);
        if p.ok {
            match parse_html_resp(&p.body) {
                Ok(t) => t,
                Err(e) => {
                    let l = get_legacy(text, sl, tl, timeout);
                    if l.ok {
                        parse_legacy_resp(&l.body)
                            .map_err(|e2| format!("Google 翻译失败（{e}；{e2}）"))?
                    } else {
                        return Err(format!("Google 翻译失败（{e}；{}）", l.err));
                    }
                }
            }
        } else {
            let l = get_legacy(text, sl, tl, timeout);
            if l.ok {
                parse_legacy_resp(&l.body)
                    .map_err(|e| format!("Google 翻译不可用（{}；{e}）", p.err))?
            } else {
                return Err(format!("Google 翻译不可用：{}；{}", p.err, l.err));
            }
        }
    };

    if !multiline {
        return Ok(unescape_text(&strip_lone_markers(&raw)).trim().to_string());
    }

    // 先在「仍编码」的原文上按标记对切段（避免源文本里字面写着标记时与转义产物混淆），
    // 再逐段反转义。
    let segs = split_marker_pairs(&raw);
    if segs.len() == lines.len() {
        Ok(lines
            .iter()
            .zip(segs)
            .map(|(l, seg)| {
                if l.content.trim().is_empty() {
                    l.prefix.clone()
                } else {
                    let body = unescape_text(&strip_lone_markers(&seg));
                    reassemble_line(l, body.trim())
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string())
    } else {
        // 段数漂移（模型合并/复制了标记对，采样中极少见）：降级为「只保换行」
        Ok(unescape_text(&markers_to_newlines(&raw)).trim().to_string())
    }
}

/// 按标记对切段；容忍标记自闭合写法与标记之间被塞入的水平空白
fn split_marker_pairs(raw: &str) -> Vec<String> {
    let sep = '\u{0}';
    // 统一三种标记写法为规范形式
    let mut unified = raw.to_string();
    for m in [LB_MARKER_SLASH[0], LB_MARKER_SLASH[1]] {
        unified = unified.replace(m, LB_MARKER);
    }
    let mut norm = String::with_capacity(unified.len());
    let mut rest = unified.as_str();
    // 相邻两标记之间允许出现水平空白（切分器有时会插入空格）
    while let Some(p) = find_marker_pair(rest) {
        norm.push_str(&rest[..p.0]);
        norm.push(sep);
        rest = &rest[p.1..];
    }
    norm.push_str(rest);
    norm.split(sep).map(|s| s.to_string()).collect()
}

/// 找到第一处「标记 + 水平空白* + 标记」，返回 (起始, 结束)
fn find_marker_pair(s: &str) -> Option<(usize, usize)> {
    let mut from = 0usize;
    while let Some(i) = s[from..].find(LB_MARKER) {
        let start = from + i;
        let after = start + LB_MARKER.len();
        let ws_len = s[after..].chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| c.len_utf8()).sum::<usize>();
        if s[after + ws_len..].starts_with(LB_MARKER) {
            return Some((start, after + ws_len + LB_MARKER.len()));
        }
        from = after;
    }
    None
}

// ---------------------------------------------------------------------------
// 对外翻译入口
// ---------------------------------------------------------------------------

/// 语言码归一：Linggo 的 33 语种码即 ISO-639-1，Google 直接可用；"auto" 透传（Google 支持自动识别）
fn g_lang(code: &str) -> String {
    let c = code.trim().to_ascii_lowercase();
    if c.is_empty() { "auto".to_string() } else { c }
}

/// Google 引擎翻译一批文本（逐行对齐，失败行退回原文）。
///
/// 返回语义与 `mt_engine::translate_lines_engine` 保持一致：
///   - `Ok(Some(v))`  命中，v 与 texts 等长等序
///   - `Ok(None)`    本引擎不适用 / 处于失败冷却 → 上层按引擎排序回落
///   - `Err(msg)`    真实错误（上层记下继续试下一个引擎，全部失败才报这个）
pub fn translate_lines(
    _app: &tauri::AppHandle,
    texts: &[String],
    source: &str,
    target: &str,
) -> Result<Option<Vec<String>>, String> {
    if texts.is_empty() {
        return Ok(None);
    }
    let sl = g_lang(source);
    let tl = g_lang(target);
    if tl == "auto" || (sl != "auto" && sl == tl) {
        return Ok(None);
    }
    if cooling() {
        return Ok(None);
    }

    // 单行（F5 / F1–F4 弹窗）走单请求；多行（F3 覆盖原文）限流并发，逐行独立保证行数对齐
    let results: Vec<Result<String, String>> = if texts.len() == 1 {
        vec![request_text(&texts[0], &sl, &tl, MAX_TIME_SECS)]
    } else {
        parallel_map(texts, BATCH_CONCURRENCY, |t| request_text(t, &sl, &tl, MAX_TIME_SECS))
    };

    let mut out: Vec<String> = Vec::with_capacity(texts.len());
    let mut ok_count = 0usize;
    let mut first_err: Option<String> = None;
    for (i, r) in results.into_iter().enumerate() {
        match r {
            Ok(t) if !t.trim().is_empty() => {
                ok_count += 1;
                out.push(t);
            }
            Ok(_) => out.push(texts[i].clone()),
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
                out.push(texts[i].clone());
            }
        }
    }
    if ok_count == 0 {
        record(false);
        return Err(first_err.unwrap_or_else(|| "Google 翻译未返回有效译文".to_string()));
    }
    record(true);
    Ok(Some(out))
}

/// Google 引擎单条翻译
pub fn translate_text(
    app: &tauri::AppHandle,
    text: &str,
    source: &str,
    target: &str,
) -> Result<Option<String>, String> {
    let texts = vec![text.to_string()];
    Ok(translate_lines(app, &texts, source, target)?.map(|mut v| v.remove(0)))
}

/// 有界并发 map：结果与输入等长等序（std::thread::scope，无额外依赖）
fn parallel_map<T, R, F>(items: &[T], concurrency: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let n = items.len();
    let slots: Vec<Mutex<Option<R>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let workers = concurrency.clamp(1, n.max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let r = f(&items[i]);
                *slots[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            });
        }
    });
    slots
        .into_iter()
        .map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner()).expect("slot filled"))
        .collect()
}

// ---------------------------------------------------------------------------
// 命令面
// ---------------------------------------------------------------------------

/// 设置面板「检测 Google 连通性」：跑一次最小真实翻译（en→zh "hello"）
#[tauri::command]
pub fn google_probe() -> GoogleStatus {
    let ok = request_text("hello", "en", "zh", PROBE_TIMEOUT_SECS)
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false);
    record(ok);
    status()
}

/// 设置面板读状态（不触网）
#[tauri::command]
pub fn google_status() -> GoogleStatus {
    status()
}

// ---------------------------------------------------------------------------
// 单元测试（全部离线，不触网）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_amp_and_tags() {
        assert_eq!(escape_text("if x <b then stop"), "if x &lt;b then stop");
        assert_eq!(escape_text("a & b"), "a &amp; b");
        assert_eq!(escape_text("write &amp; for < and >"), "write &amp;amp; for &lt; and &gt;");
    }

    #[test]
    fn unescapes_exactly_once() {
        assert_eq!(unescape_text("a &amp; b"), "a & b");
        assert_eq!(unescape_text("&lt;b&gt;"), "<b>");
        assert_eq!(unescape_text("It&#39;s"), "It's");
        assert_eq!(unescape_text("&#x4F60;&#22909;"), "你好");
        assert_eq!(unescape_text("&amp;lt;"), "&lt;");
    }

    #[test]
    fn split_lines_captures_indent_and_bullet() {
        let lines = split_preserved_lines("  - hello\nplain\n\n-5C\n*bold* text\n* real bullet");
        assert_eq!(lines[0].prefix, "  - ");
        assert_eq!(lines[0].content, "- hello");
        assert_eq!(lines[1].prefix, "");
        assert_eq!(lines[1].content, "plain");
        assert_eq!(lines[2].prefix, "");
        assert_eq!(lines[2].content, "");
        assert_eq!(lines[3].prefix, ""); // "-5C" 无空白，不是项目符号
        assert_eq!(lines[3].content, "-5C");
        assert_eq!(lines[4].prefix, ""); // "*bold*" 是强调，不是项目符号
        assert_eq!(lines[5].prefix, "* ");
        assert_eq!(lines[5].content, "* real bullet");
    }

    #[test]
    fn reassemble_restores_source_prefix() {
        let line = PreservedLine { prefix: "  - ".into(), content: "- hi".into() };
        assert_eq!(reassemble_line(&line, "你好"), "  - 你好");
        assert_eq!(reassemble_line(&line, "- 你好"), "  - 你好");
    }

    #[test]
    fn marker_pair_split_tolerates_spacing_and_self_closing() {
        let raw = format!("a{LB_MARKER}{LB_MARKER}b");
        assert_eq!(split_marker_pairs(&raw), vec!["a", "b"]);
        let spaced = format!("a{LB_MARKER} {LB_MARKER}b");
        assert_eq!(split_marker_pairs(&spaced), vec!["a", "b"]);
        let closed = format!("a{}{}b", LB_MARKER_SLASH[0], LB_MARKER_SLASH[1]);
        assert_eq!(split_marker_pairs(&closed), vec!["a", "b"]);
        // 无标记对 → 整段一段
        assert_eq!(split_marker_pairs("only one"), vec!["only one"]);
        // 三行 → 三段
        let three = format!("a{LB_MARKER}{LB_MARKER}b{LB_MARKER}{LB_MARKER}c");
        assert_eq!(split_marker_pairs(&three), vec!["a", "b", "c"]);
    }

    #[test]
    fn lone_marker_never_leaks_markup() {
        assert_eq!(strip_lone_markers(&format!("x{LB_MARKER}y")), "x y");
        assert_eq!(unescape_text(&strip_lone_markers("x&lt;br&gt;y")), "x<br>y");
    }

    #[test]
    fn url_encodes_reserved_chars() {
        assert_eq!(url_encode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(url_encode("zh"), "zh");
    }

    #[test]
    fn parses_both_response_shapes() {
        assert_eq!(parse_html_resp(r#"[["你好"]]"#).unwrap(), "你好");
        // 扁平形状：各元素都是译文片段
        assert_eq!(parse_legacy_resp(r#"[["你","好"],null,["en"]]"#).unwrap(), "你好");
        // 真实形状：首元素是句子数组，每句 [译文,原文,...] 只取译文
        assert_eq!(
            parse_legacy_resp(r#"[[["Hello.","你好。"],["world","世界"]],null,"en"]"#).unwrap(),
            "Hello.world"
        );
        assert!(parse_html_resp(r#"{"x":1}"#).is_err());
        assert!(parse_legacy_resp("not json").is_err());
        assert!(parse_legacy_resp("[null,null]").is_err());
    }

    #[test]
    fn parallel_map_preserves_order() {
        let items: Vec<usize> = (0..20).collect();
        let out: Vec<usize> = parallel_map(&items, 6, |v| {
            std::thread::sleep(Duration::from_millis(if *v % 2 == 0 { 4 } else { 1 }));
            v * 2
        });
        assert_eq!(out, items.iter().map(|v| v * 2).collect::<Vec<_>>());
    }

    #[test]
    fn proxy_normalize_adds_scheme() {
        assert_eq!(normalize_proxy("127.0.0.1:7897"), "http://127.0.0.1:7897");
        assert_eq!(normalize_proxy("http://127.0.0.1:7897"), "http://127.0.0.1:7897");
        assert_eq!(normalize_proxy("socks5://127.0.0.1:1080"), "socks5://127.0.0.1:1080");
    }

    /// 真实网络：主通道单行翻译（需外网/系统代理，默认忽略）
    #[test]
    #[ignore = "需要外网或系统代理"]
    fn live_single_line_translates() {
        let out = request_text("Hello <b>world</b> & friends", "auto", "zh", MAX_TIME_SECS)
            .expect("Google 主通道应可用");
        assert!(out.contains('你'), "译文应含中文: {out}");
        assert!(!out.contains("&lt;"), "HTML 实体应已反转义: {out}");
        assert!(!out.contains(LB_MARKER), "单行不应残留标记: {out}");
    }

    /// 真实网络：多行 + 标记对必须逐行还原（行数、缩进、项目符号）
    #[test]
    #[ignore = "需要外网或系统代理"]
    fn live_multiline_restores_lines() {
        let src = "Hello <b>world</b>\n  - second line with <i>markup</i>";
        let out = request_text(src, "auto", "zh", MAX_TIME_SECS).expect("Google 主通道应可用");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "行数应保持: {out}");
        assert!(lines[0].contains('你'), "第 1 行应已翻译: {out}");
        assert!(lines[1].contains('第'), "第 2 行应已翻译: {out}");
        assert!(lines[1].starts_with("  - "), "第 2 行应保留缩进与项目符号: {out}");
        assert!(!out.contains(LB_MARKER), "标记不应泄漏: {out}");
    }
}
