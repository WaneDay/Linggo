// Linggo 模型市场（Package Index）：按 argos-translate 的「包索引」模型实现——
// 索引（远程 URL 或内置 JSON）→ 可选语言包列表 → 下载安装到 models 目录 → 已装列表可删除。
//
// 与 argos-translate 的对应：
//   ARGOS_PACKAGE_INDEX                -> settings.package_index_url（留空 = 内置索引）
//   update_package_index()             -> pkg_list(force=true) 时下载索引到 %APPDATA%\Linggo\pkg_index.json
//   get_available_packages()           -> pkg_list()
//   AvailablePackage.download+install  -> pkg_download()（curl 逐文件下载到 models/<id>）
//   uninstall(pkg)                     -> pkg_delete()（递归删除 models/<id>）
//
// 索引条目 schema（serde camelCase）：
//   { id, kind("opus"/"nllb"/"gguf"/...), name, fromCode, toCode,
//     baseUrl, files[], sizeBytes, version }
// 下载目标目录名 = id；opus 类校验 model.bin+source.spm+target.spm，nllb 校验 model.bin+tokenizer.json，
// gguf 校验目录内含 *.gguf（大模型，下载后 model_path 为空时自动发现）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter};

/// 内置默认索引（无需联网即可看到可装语言包）
const EMBEDDED_INDEX: &str = include_str!("../pkg_index.json");

/// 下载取消标志：pkg_cancel_download 置位，pkg_download 协程在文件间隙轮询，置位则清理半成品目录
static CANCEL_DOWNLOAD: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

// ---------------------------------------------------------------------------
// 类型
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PkgEntry {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub from_code: String,
    pub to_code: String,
    pub base_url: String,
    pub files: Vec<String>,
    pub size_bytes: u64,
    pub version: String,
}

impl Default for PkgEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: String::new(),
            name: String::new(),
            from_code: String::new(),
            to_code: String::new(),
            base_url: String::new(),
            files: Vec::new(),
            size_bytes: 0,
            version: String::new(),
        }
    }
}

impl PkgEntry {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPkg {
    /// 目录名（与索引 id 对应）
    pub id: String,
    /// 友好名
    pub name: String,
    pub kind: String,
    /// 绝对路径
    pub dir: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PkgList {
    pub available: Vec<PkgEntry>,
    pub installed: Vec<InstalledPkg>,
    /// 当前可用列表来源："内置索引" / "缓存索引" / "远程索引（url）"
    pub source: String,
}

/// 下载进度事件负载（pkg-download 事件）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PkgProgress {
    pub id: String,
    pub file: String,
    pub status: String,
    pub index: usize,
    pub total: usize,
    /// 已下载得字节数（含当前文件已写入的部分），进度条按 bytes/bytesTotal 计算
    pub bytes: u64,
    pub bytes_total: u64,
}

// ---------------------------------------------------------------------------
// 索引解析 / 获取
// ---------------------------------------------------------------------------

fn cache_path() -> PathBuf {
    crate::settings::app_data_dir().join("pkg_index.json")
}

/// curl 拉取文本（系统 curl.exe 带重试；失败返回 None）
fn fetch_text(url: &str) -> Option<String> {
    let tmp = std::env::temp_dir().join(format!("linggo_pkg_idx_{}.json", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    if !crate::mt_engine::download_file(url, &tmp) {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    let s = std::fs::read_to_string(&tmp).ok();
    let _ = std::fs::remove_file(&tmp);
    s
}

fn parse_entries(payload: &str) -> Result<Vec<PkgEntry>, String> {
    serde_json::from_str(payload).map_err(|e| format!("索引解析失败: {e}"))
}

/// 取可用列表：(条目, 来源说明)。
/// 留空索引地址 → 始终用内置索引；配置了地址 → 力刷/缓存缺失时拉远程，失败回落缓存/内置。
fn resolve_index(app: &AppHandle, force: bool) -> Result<(Vec<PkgEntry>, String), String> {
    let url = crate::settings::current(app).package_index_url.trim().to_string();
    if url.is_empty() {
        return Ok((parse_entries(EMBEDDED_INDEX)?, "内置索引".to_string()));
    }
    let cache = cache_path();
    if force || !cache.exists() {
        if let Some(s) = fetch_text(&url) {
            let _ = std::fs::write(&cache, &s);
            return Ok((parse_entries(&s)?, format!("远程索引（{url}）")));
        }
    }
    if let Ok(s) = std::fs::read_to_string(&cache) {
        return Ok((parse_entries(&s)?, "缓存索引".to_string()));
    }
    Ok((
        parse_entries(EMBEDDED_INDEX)?,
        "内置索引（远程不可达）".to_string(),
    ))
}

// ---------------------------------------------------------------------------
// 已安装扫描 / 目录工具
// ---------------------------------------------------------------------------

fn same_path(a: &str, b: &Path) -> bool {
    Path::new(a.strip_prefix(r"\\?\").unwrap_or(a)) == b
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_dir() {
                total += dir_size(&p);
            } else if let Ok(md) = p.metadata() {
                total += md.len();
            }
        }
    }
    total
}

fn human_name(dir_name: &str, kind: &str) -> String {
    match kind {
        "opus" => {
            // opus-mt-aa-bb-ct2 / opus-mt-aa-bb
            let inner = dir_name
                .strip_prefix("opus-mt-")
                .unwrap_or(dir_name)
                .trim_end_matches("-ct2");
            let parts: Vec<&str> = inner.split('-').collect();
            if parts.len() >= 2 && parts[0].len() == 2 && parts[1].len() == 2 {
                format!("OPUS-MT {} → {}", parts[0], parts[1])
            } else {
                dir_name.to_string()
            }
        }
        "nllb" => {
            if dir_name.contains("1.3B") {
                "NLLB-200 1.3B (CT2)".to_string()
            } else {
                "NLLB-200 (CT2)".to_string()
            }
        }
        "gguf" => {
            if dir_name.to_ascii_lowercase().contains("hy-mt2") {
                "Hy-MT2-1.8B (GGUF)".to_string()
            } else {
                "GGUF 大模型".to_string()
            }
        }
        _ => dir_name.to_string(),
    }
}

/// 大模型目录：含任意 *.gguf 文件（如模型市场下载的 Hy-MT2）
fn is_valid_gguf_dir(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    rd.flatten().any(|e| {
        e.path()
            .extension()
            .map(|x| x.eq_ignore_ascii_case("gguf"))
            .unwrap_or(false)
    })
}

/// 扫描 models 根目录，列出已安装语言包（目录名 = id）
fn scan_installed() -> Vec<InstalledPkg> {
    let Some(base) = crate::mt_engine::models_base() else {
        return Vec::new();
    };
    let Ok(rd) = base.read_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let p = ent.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if crate::mt_engine::SKIP_DIRS.contains(&name) {
            continue;
        }
        let (kind, name_str) = if crate::mt_engine::is_valid_opus_dir(&p) {
            ("opus", human_name(name, "opus"))
        } else if crate::mt_engine::is_valid_nllb_dir(&p) {
            ("nllb", human_name(name, "nllb"))
        } else if is_valid_gguf_dir(&p) {
            ("gguf", human_name(name, "gguf"))
        } else {
            continue;
        };
        out.push(InstalledPkg {
            id: name.to_string(),
            name: name_str,
            kind: kind.to_string(),
            dir: p.to_string_lossy().to_string(),
            size_bytes: dir_size(&p),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

// ---------------------------------------------------------------------------
// 命令面
// ---------------------------------------------------------------------------

/// 列出「可用语言包 + 已安装语言包」。force=true 时重新拉取远程索引。
#[tauri::command]
pub fn pkg_list(app: AppHandle, force: bool) -> Result<PkgList, String> {
    let (available, source) = resolve_index(&app, force)?;
    Ok(PkgList {
        available,
        installed: scan_installed(),
        source,
    })
}

/// 下载文件名白名单：拒绝空名、路径分隔符、相对穿越（..）与盘符冒号，
/// 防止恶意远程索引条目把文件写到 models 目录之外。
fn safe_file_name(f: &str) -> Option<String> {
    if f.is_empty()
        || f.contains('/')
        || f.contains('\\')
        || f.contains("..")
        || f.contains(':')
    {
        return None;
    }
    Some(f.to_string())
}

/// 下载并安装指定语言包到 models/<id>（后台 curl 逐文件，进度走 pkg-download 事件）
#[tauri::command]
pub fn pkg_download(app: AppHandle, id: String) -> Result<(), String> {
    let (entries, source) = resolve_index(&app, false)?;
    let entry = entries
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("索引中没有该语言包「{id}」（当前来源：{source}）"))?;
    if entry.files.is_empty() {
        return Err("索引条目未声明 files 列表".to_string());
    }
    let base = crate::mt_engine::models_base()
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(|p| p.join("models")))
        })
        .or_else(|| std::env::current_dir().ok().map(|c| c.join("models")))
        .ok_or("无法确定 models 目录")?;
    let _ = std::fs::create_dir_all(&base);
    let entry = entry.clone();
    tauri::async_runtime::spawn(async move {
        let dst = base.join(&entry.id);
        let _ = std::fs::create_dir_all(&dst);
        let total = entry.files.len();
        let bytes_total = entry.size_bytes;
        CANCEL_DOWNLOAD.store(false, Ordering::Relaxed);
        let mut done_bytes = 0u64;
        let mut aborted = false;
        for (i, f) in entry.files.iter().enumerate() {
            // 文件间隙检查：上一轮已请求取消则直接中止
            if CANCEL_DOWNLOAD.load(Ordering::Relaxed) {
                aborted = true;
                break;
            }
            // 下载目标文件名白名单：拒绝路径分隔符/相对穿越/盘符等非安全名，
            // 防恶意远程索引把文件写到 models 目录之外。
            let fname = match safe_file_name(f) {
                Some(n) => n,
                None => {
                    let _ = app.emit(
                        "pkg-download",
                        &PkgProgress {
                            id: entry.id.clone(),
                            file: f.clone(),
                            status: "failed".to_string(),
                            index: i + 1,
                            total,
                            bytes: done_bytes,
                            bytes_total,
                        },
                    );
                    aborted = true;
                    break;
                }
            };
            let url = format!("{}/{}", entry.base_url.trim_end_matches('/'), fname);
            let target = dst.join(&fname);
            // 真实字节进度：已完成的文件大小之和 + 当前文件已写入的部分（-C - 断点续传）
            let partial = target.metadata().map(|m| m.len()).unwrap_or(0);
            let _ = app.emit(
                "pkg-download",
                &PkgProgress {
                    id: entry.id.clone(),
                    file: fname.clone(),
                    status: "downloading".to_string(),
                    index: i + 1,
                    total,
                    bytes: done_bytes + partial,
                    bytes_total,
                },
            );
            // 下载期间后台每 300ms 轮询文件大小推真实字节进度（大文件不卡在固定百分比）
            let base_bytes = done_bytes;
            let done_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let poller_app = app.clone();
            let poller_id = entry.id.clone();
            let poller_file = fname.clone();
            let poller_target = target.clone();
            let poller_flag = done_flag.clone();
            let poller = std::thread::spawn(move || {
                while !poller_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    let cur = poller_target.metadata().map(|m| m.len()).unwrap_or(0);
                    let _ = poller_app.emit(
                        "pkg-download",
                        &PkgProgress {
                            id: poller_id.clone(),
                            file: poller_file.clone(),
                            status: "downloading".to_string(),
                            index: i + 1,
                            total,
                            bytes: base_bytes + cur,
                            bytes_total,
                        },
                    );
                }
            });
            // 可取消下载：取消即强杀当前 curl 子进程（≤250ms 内生效）
            let ok = crate::mt_engine::download_file_cancel(&url, &target, &CANCEL_DOWNLOAD);
            done_flag.store(true, Ordering::Relaxed);
            let _ = poller.join();
            let got = target.metadata().map(|m| m.len()).unwrap_or(0);
            if ok {
                done_bytes += got;
            }
            // 失败/取消一律进入终止分支：清理已下载内容并广播对应状态
            if !ok || CANCEL_DOWNLOAD.load(Ordering::Relaxed) {
                let st = if CANCEL_DOWNLOAD.load(Ordering::Relaxed) {
                    "cancelled"
                } else {
                    "failed"
                };
                let _ = std::fs::remove_dir_all(&dst);
                let _ = app.emit(
                    "pkg-download",
                    &PkgProgress {
                        id: entry.id.clone(),
                        file: fname.clone(),
                        status: st.to_string(),
                        index: i + 1,
                        total,
                        bytes: if ok { done_bytes } else { done_bytes + partial },
                        bytes_total,
                    },
                );
                aborted = true;
                break;
            }
            let _ = app.emit(
                "pkg-download",
                &PkgProgress {
                    id: entry.id.clone(),
                    file: fname.clone(),
                    status: "done".to_string(),
                    index: i + 1,
                    total,
                    bytes: done_bytes,
                    bytes_total,
                },
            );
        }
        // 文件间隙被中断（尚未进入任何文件下载）也清理空目录
        if aborted && dst.exists() {
            let _ = std::fs::remove_dir_all(&dst);
        }
        CANCEL_DOWNLOAD.store(false, Ordering::Relaxed);
        crate::mt_engine::emit_status(&app);
    });
    Ok(())
}

/// 取消进行中的下载（置位取消标志；进行中协程在文件间隙响应并删除半成品目录）。
#[tauri::command]
pub fn pkg_cancel_download() -> Result<(), String> {
    CANCEL_DOWNLOAD.store(true, Ordering::Relaxed);
    Ok(())
}

/// 删除（卸载）已安装语言包：先释放引擎句柄、清空指向该目录的设置项，再递归删除。
#[tauri::command]
pub fn pkg_delete(app: AppHandle, id: String) -> Result<(), String> {
    let base = crate::mt_engine::models_base().ok_or("未找到 models 目录")?;
    let dir = base.join(&id);
    if !dir.is_dir() {
        return Err(format!("未找到已安装语言包「{id}」（请先刷新列表）"));
    }
    // 释放可能占用模型文件句柄的 NMT 引擎
    crate::mt_engine::unload_all();
    // 若设置仍显式指向该目录则一并清空（避免残留失效路径）
    let mut s = crate::settings::current(&app);
    let mut changed = false;
    for key in ["nmt_opus_zh_en", "nmt_opus_en_zh", "nmt_nllb_dir"] {
        let val = match key {
            "nmt_opus_zh_en" => &mut s.nmt_opus_zh_en,
            "nmt_opus_en_zh" => &mut s.nmt_opus_en_zh,
            _ => &mut s.nmt_nllb_dir,
        };
        if same_path(val, &dir) {
            *val = String::new();
            changed = true;
        }
    }
    // GGUF 大模型：model_path 指向该目录内文件时一并清空
    let stored = s.model_path.strip_prefix(r"\\?\").unwrap_or(&s.model_path);
    if !stored.is_empty() && Path::new(stored).starts_with(&dir) {
        s.model_path = String::new();
        changed = true;
    }
    if changed {
        crate::settings::apply(&app, s)?;
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("删除失败: {e}"))?;
    crate::mt_engine::emit_status(&app);
    Ok(())
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_index_parses() {
        let entries = parse_entries(EMBEDDED_INDEX).expect("内置索引应可解析");
        assert!(!entries.is_empty());
        for e in &entries {
            assert!(!e.id.is_empty());
            assert!(!e.base_url.is_empty());
            assert!(!e.files.is_empty());
            assert!(e.base_url.ends_with('/') == false);
        }
    }

    #[test]
    fn opus_names() {
        assert_eq!(human_name("opus-mt-zh-en-ct2", "opus"), "OPUS-MT zh → en");
        assert_eq!(human_name("opus-mt-en-zh-ct2", "opus"), "OPUS-MT en → zh");
        assert_eq!(
            human_name("nllb-200-distilled-1.3B-ct2-int8", "nllb"),
            "NLLB-200 1.3B (CT2)"
        );
    }

    #[test]
    fn safe_file_names_reject_traversal() {
        assert_eq!(safe_file_name("model.bin"), Some("model.bin".to_string()));
        assert_eq!(safe_file_name("dir-1/tok.json"), None);
        assert_eq!(safe_file_name(r"..\evil.exe"), None);
        assert_eq!(safe_file_name("../escape"), None);
        assert_eq!(safe_file_name(r"C:\esc"), None);
        assert_eq!(safe_file_name("a:b"), None);
        assert_eq!(safe_file_name(".."), None);
        assert_eq!(safe_file_name(""), None);
    }

    #[test]
    fn gguf_kind_detects_and_big_models_have_sizes() {
        assert_eq!(
            human_name("hy-mt2-1.8b-q4-k-m-gguf", "gguf"),
            "Hy-MT2-1.8B (GGUF)"
        );
        let root = std::env::temp_dir().join(format!("linggo_gguf_{}", std::process::id()));
        let d = root.join("hy-mt2-1.8b-q4-k-m-gguf");
        std::fs::create_dir_all(&d).unwrap();
        assert!(!is_valid_gguf_dir(&d));
        std::fs::write(d.join("Hy-MT2-1.8B-Q4_K_M.gguf"), b"x").unwrap();
        assert!(is_valid_gguf_dir(&d));
        let entries = parse_entries(EMBEDDED_INDEX).expect("内置索引应可解析");
        for e in entries {
            if e.kind == "gguf" || e.kind == "nllb" {
                assert!(e.size_bytes > 1_000_000_000, "大模型条目需声明体积");
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}